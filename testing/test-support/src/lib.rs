use std::net::SocketAddr;
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::{Duration as ChronoDuration, Utc};
use kernel::event::{Event, Platform, RawEvent};
use kernel::worker::{Worker, WorkerContext};
use memory::graph::{CognitiveGraph, SocialDelta};
use memory::{ConversationKey, MemoryMessage, ShortTermMemory};
use mcp::{build_mcp_router, registry::ToolRegistry};
use state::{EventDeltaRequest, StateStore};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::time::{timeout, Duration};

pub async fn in_memory_graph() -> CognitiveGraph {
    CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize")
}

pub async fn seeded_social_graph(user_id: &str) -> CognitiveGraph {
    let graph = in_memory_graph().await;
    graph
        .update_social_graph(
            user_id,
            SocialDelta {
                delta_affinity: 0.3,
                delta_attachment: 0.15,
                delta_trust: 0.35,
                delta_safety: 0.2,
                delta_tension: 0.2,
            },
        )
        .await
        .expect("social graph should update");
    graph
        .update_illusion_graph(
            user_id,
            SocialDelta {
                delta_affinity: 0.1,
                delta_attachment: 0.05,
                delta_trust: 0.2,
                delta_safety: 0.1,
                delta_tension: 0.05,
            },
        )
        .await
        .expect("illusion graph should update");
    let _ = graph
        .get_or_project_social_tree_snapshot(user_id, 0.2)
        .await
        .expect("social tree snapshot should project");
    graph
}

pub fn build_test_mcp_router(graph: CognitiveGraph) -> Router {
    build_mcp_router(Arc::new(ToolRegistry::default()), graph, 2_000)
}

pub fn tool_call_request(name: &str, input: Value) -> axum::http::Request<axum::body::Body> {
    axum::http::Request::builder()
        .uri("/api/mcp/tools/call")
        .method("POST")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            serde_json::json!({
                "name": name,
                "input": input,
            })
            .to_string(),
        ))
        .expect("tool call request should build")
}

pub fn tools_list_request() -> axum::http::Request<axum::body::Body> {
    axum::http::Request::builder()
        .uri("/api/mcp/tools")
        .method("GET")
        .body(axum::body::Body::empty())
        .expect("tools list request should build")
}

pub async fn read_json(response: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    serde_json::from_slice(&body).expect("json body expected")
}

pub fn mention_event(username: &str, content: &str) -> RawEvent {
    mention_event_in_channel(username, "cli", content)
}

pub fn mention_event_in_channel(username: &str, channel_id: &str, content: &str) -> RawEvent {
    RawEvent {
        platform: Platform::Cli,
        channel_id: channel_id.to_string(),
        message_id: format!("msg-{}-{}", channel_id, username),
        user_id: username.to_string(),
        username: username.to_string(),
        content: content.to_string(),
        is_mention: true,
        is_dm: true,
        timestamp: Utc::now(),
    }
}

pub fn history_message(username: &str, channel_id: &str, content: &str) -> MemoryMessage {
    MemoryMessage::from_raw(&RawEvent {
        platform: Platform::Cli,
        channel_id: channel_id.to_string(),
        message_id: format!(
            "history-{}-{}-{}",
            channel_id,
            username,
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ),
        user_id: username.to_string(),
        username: username.to_string(),
        content: content.to_string(),
        is_mention: false,
        is_dm: true,
        timestamp: Utc::now(),
    })
}

pub fn bot_history_message(
    channel_id: &str,
    content: &str,
    reply_to_user: Option<&str>,
) -> MemoryMessage {
    MemoryMessage::bot_response(
        Platform::Cli,
        channel_id.to_string(),
        content.to_string(),
        None,
        reply_to_user.map(|value| value.to_string()),
    )
}

pub fn seeded_short_term_memory(
    channel_id: &str,
    messages: Vec<MemoryMessage>,
) -> Arc<Mutex<ShortTermMemory>> {
    let mut short_term = ShortTermMemory::new();
    let normalized = messages
        .into_iter()
        .map(|mut msg| {
            msg.platform = Platform::Cli;
            msg.channel_id = channel_id.to_string();
            msg
        })
        .collect();
    short_term.load_history(normalized);
    Arc::new(Mutex::new(short_term))
}

pub async fn formatted_history_context(
    short_term: &Arc<Mutex<ShortTermMemory>>,
    channel_id: &str,
) -> Option<String> {
    let guard = short_term.lock().await;
    guard.format_context(&ConversationKey::new(Platform::Cli, channel_id.to_string()))
}

pub fn in_memory_state_store() -> Result<StateStore> {
    StateStore::load_default()
}

pub async fn seeded_state_store(requests: Vec<EventDeltaRequest>) -> Result<StateStore> {
    let store = StateStore::load_default()?;
    store.apply_event_deltas(&requests).await?;
    Ok(store)
}

pub fn worker_context_channels(
    capacity: usize,
) -> (
    WorkerContext,
    mpsc::Receiver<Event>,
    broadcast::Sender<Event>,
    broadcast::Sender<()>,
) {
    let (event_tx, event_rx) = mpsc::channel(capacity);
    let (broadcast_tx, _) = broadcast::channel(capacity);
    let (shutdown_tx, _) = broadcast::channel(1);
    (
        WorkerContext {
            event_tx,
            broadcast_rx: broadcast_tx.clone(),
            shutdown: shutdown_tx.clone(),
        },
        event_rx,
        broadcast_tx,
        shutdown_tx,
    )
}

pub async fn wait_for_shutdown_subscription(shutdown_tx: &broadcast::Sender<()>) {
    timeout(Duration::from_secs(2), async {
        loop {
            if shutdown_tx.receiver_count() > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("worker should subscribe to shutdown");
}

pub async fn set_tree_updated_at(graph: &CognitiveGraph, user_id: &str, updated_at: &str) {
    graph
        .db
        .query("UPDATE social_tree_root SET meta.updated_at = $updated_at WHERE user_id = $user_id;")
        .bind(("updated_at", updated_at.to_string()))
        .bind(("user_id", user_id.to_string()))
        .await
        .expect("updated_at should update");
}

pub fn stale_timestamp(hours_ago: i64) -> String {
    (Utc::now() - ChronoDuration::hours(hours_ago)).to_rfc3339()
}

#[derive(Clone)]
pub struct MockChatServerState {
    pub requests: Arc<StdMutex<Vec<Value>>>,
    pub responses: Arc<StdMutex<Vec<String>>>,
}

async fn mock_models() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "data": [] })))
}

async fn mock_chat_completions(
    State(state): State<MockChatServerState>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    state.requests.lock().expect("requests lock").push(payload);
    let body = state.responses.lock().expect("responses lock").remove(0);
    (StatusCode::OK, body)
}

pub async fn spawn_mock_chat_server(responses: Vec<String>) -> (SocketAddr, Arc<StdMutex<Vec<Value>>>) {
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let state = MockChatServerState {
        requests: Arc::clone(&requests),
        responses: Arc::new(StdMutex::new(responses)),
    };
    let app = Router::new()
        .route("/models", get(mock_models))
        .route("/chat/completions", post(mock_chat_completions))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock chat server");
    let addr = listener.local_addr().expect("mock chat server addr");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("mock chat server should run");
    });
    (addr, requests)
}

pub async fn recv_event_within(event_rx: &mut mpsc::Receiver<Event>, duration: Duration) -> Event {
    timeout(duration, event_rx.recv())
        .await
        .expect("event should arrive in time")
        .expect("event channel should remain open")
}

pub async fn expect_no_event_within(event_rx: &mut mpsc::Receiver<Event>, duration: Duration) {
    let recv_result = timeout(duration, event_rx.recv()).await;
    assert!(recv_result.is_err(), "event should not arrive");
}

pub async fn start_dialogue_worker(
    worker: cognitive::DialogueEngineWorker,
    capacity: usize,
) -> (
    tokio::task::JoinHandle<cognitive::DialogueEngineWorker>,
    mpsc::Receiver<Event>,
    broadcast::Sender<Event>,
    broadcast::Sender<()>,
) {
    let (ctx, event_rx, broadcast_tx, shutdown_tx) = worker_context_channels(capacity);
    let handle = tokio::spawn(async move {
        let mut worker = worker;
        worker.start(ctx).await.expect("dialogue worker should run");
        worker
    });
    wait_for_shutdown_subscription(&shutdown_tx).await;
    (handle, event_rx, broadcast_tx, shutdown_tx)
}

pub async fn shutdown_dialogue_worker(
    handle: tokio::task::JoinHandle<cognitive::DialogueEngineWorker>,
    shutdown_tx: &broadcast::Sender<()>,
) -> cognitive::DialogueEngineWorker {
    shutdown_tx.send(()).expect("shutdown should send");
    timeout(Duration::from_secs(2), handle)
        .await
        .expect("worker should stop in time")
        .expect("worker task should finish")
}

pub fn streaming_chunks(parts: &[&str]) -> String {
    let mut body = String::new();
    for part in parts {
        body.push_str(&format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{}\"}}}}]}}\n\n",
            part
        ));
    }
    body.push_str("data: [DONE]\n\n");
    body
}

pub fn planning_then_streaming_responses(arguments: &str, parts: &[&str]) -> Vec<String> {
    vec![
        planning_response_with_tool_call(arguments),
        planning_response_without_tool_calls(),
        streaming_chunks(parts),
    ]
}

pub fn plain_streaming_responses(parts: &[&str]) -> Vec<String> {
    vec![streaming_chunks(parts)]
}

pub fn planning_response_with_tool_call(arguments: &str) -> String {
    serde_json::json!({
        "choices": [{
            "message": {
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {
                        "name": "social.get_dialogue_summary",
                        "arguments": arguments
                    }
                }]
            }
        }]
    })
    .to_string()
}

pub fn planning_response_without_tool_calls() -> String {
    serde_json::json!({
        "choices": [{
            "message": {
                "tool_calls": []
            }
        }]
    })
    .to_string()
}

pub fn final_streaming_response(content: &str) -> String {
    format!(
        "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{}\"}}}}]}}\n\ndata: [DONE]\n\n",
        content
    )
}
