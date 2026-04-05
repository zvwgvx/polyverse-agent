use cognitive::{dialogue_engine::DialogueToolCallingConfig, DialogueEngineConfig, DialogueEngineWorker};
use kernel::event::{Event, ResponseSource};
use kernel::worker::{Worker, WorkerStatus};
use memory::graph::SocialDelta;
use state::EventDeltaRequest;
use test_support::{
    bot_history_message, expect_no_event_within, formatted_history_context, history_message,
    in_memory_graph, mention_event_in_channel, mention_event_with_image, plain_streaming_responses,
    planning_then_streaming_responses, recv_event_within, seeded_short_term_memory,
    seeded_social_graph, seeded_state_store, shutdown_dialogue_worker, spawn_mock_chat_server,
    start_dialogue_worker,
};
use serde_json::Value;
use tokio::time::Duration;

fn disabled_tool_config(api_base: String) -> DialogueEngineConfig {
    DialogueEngineConfig {
        api_base,
        api_key: "test-key".to_string(),
        model: "test-model".to_string(),
        chat_max_tokens: 128,
        reasoning: None,
        tool_calling: DialogueToolCallingConfig {
            enabled: false,
            max_calls_per_turn: 2,
            timeout_ms: 1_500,
            max_candidate_users: 3,
        },
        api_timeout_secs: None,
    }
}

fn enabled_tool_config(api_base: String) -> DialogueEngineConfig {
    DialogueEngineConfig {
        api_base,
        api_key: "test-key".to_string(),
        model: "test-model".to_string(),
        chat_max_tokens: 128,
        reasoning: None,
        tool_calling: DialogueToolCallingConfig {
            enabled: true,
            max_calls_per_turn: 2,
            timeout_ms: 1_500,
            max_candidate_users: 3,
        },
        api_timeout_secs: None,
    }
}

#[tokio::test]
async fn dialogue_worker_starts_and_shuts_down_cleanly() {
    let (addr, _requests) = spawn_mock_chat_server(Vec::new()).await;
    let worker = DialogueEngineWorker::new(disabled_tool_config(format!("http://{}", addr)))
        .with_system_prompt("system".to_string());

    let (handle, _event_rx, _broadcast_tx, shutdown_tx) = start_dialogue_worker(worker, 16).await;
    let worker = shutdown_dialogue_worker(handle, &shutdown_tx).await;
    assert_eq!(worker.health_check(), WorkerStatus::Stopped);
}

#[tokio::test]
async fn dialogue_worker_ignores_non_mention_events() {
    let (addr, requests) = spawn_mock_chat_server(Vec::new()).await;
    let worker = DialogueEngineWorker::new(disabled_tool_config(format!("http://{}", addr)))
        .with_system_prompt("system".to_string());

    let (handle, mut event_rx, broadcast_tx, shutdown_tx) = start_dialogue_worker(worker, 16).await;

    let mut raw = mention_event_in_channel("alice", "quiet-channel", "ignore this");
    raw.is_mention = false;
    broadcast_tx.send(Event::Raw(raw)).expect("broadcast should send");

    expect_no_event_within(&mut event_rx, Duration::from_millis(300)).await;
    assert!(requests.lock().expect("requests lock").is_empty());

    let worker = shutdown_dialogue_worker(handle, &shutdown_tx).await;
    assert_eq!(worker.health_check(), WorkerStatus::Stopped);
}

#[tokio::test]
async fn dialogue_worker_stops_when_api_config_is_invalid() {
    let mut worker = DialogueEngineWorker::new(DialogueEngineConfig {
        api_base: String::new(),
        api_key: "test-key".to_string(),
        model: "test-model".to_string(),
        chat_max_tokens: 128,
        reasoning: None,
        tool_calling: DialogueToolCallingConfig {
            enabled: false,
            max_calls_per_turn: 2,
            timeout_ms: 1_500,
            max_candidate_users: 3,
        },
        api_timeout_secs: None,
    });

    let (ctx, _event_rx, _broadcast_tx, _shutdown_tx) = test_support::worker_context_channels(16);
    worker.start(ctx).await.expect("worker should exit cleanly");
    assert_eq!(worker.health_check(), WorkerStatus::Stopped);
}

#[tokio::test]
async fn dialogue_worker_streams_final_response_and_completion_for_mentions() {
    let (addr, requests) =
        spawn_mock_chat_server(plain_streaming_responses(&["hello from worker"]))
            .await;
    let worker = DialogueEngineWorker::new(disabled_tool_config(format!("http://{}", addr)))
        .with_system_prompt("system".to_string());

    let (handle, mut event_rx, broadcast_tx, shutdown_tx) = start_dialogue_worker(worker, 16).await;

    let raw = mention_event_in_channel("alice", "reply-channel", "<@bot> hello from user");
    let message_id = raw.message_id.clone();
    broadcast_tx.send(Event::Raw(raw)).expect("broadcast should send");

    let response = recv_event_within(&mut event_rx, Duration::from_secs(1)).await;
    match response {
        Event::Response(response) => {
            assert_eq!(response.channel_id, "reply-channel");
            assert_eq!(response.reply_to_message_id.as_deref(), Some(message_id.as_str()));
            assert_eq!(response.reply_to_user.as_deref(), Some("alice"));
            assert_eq!(response.content, "hello from worker");
            assert_eq!(response.source, ResponseSource::CloudLLM);
        }
        other => panic!("expected response event, got {other:?}"),
    }

    let completion = recv_event_within(&mut event_rx, Duration::from_secs(1)).await;
    match completion {
        Event::BotTurnCompletion(done) => {
            assert_eq!(done.channel_id, "reply-channel");
            assert_eq!(done.reply_to_message_id.as_deref(), Some(message_id.as_str()));
            assert_eq!(done.reply_to_user.as_deref(), Some("alice"));
            assert_eq!(done.content, "hello from worker");
        }
        other => panic!("expected turn completion event, got {other:?}"),
    }

    let sent_requests = requests.lock().expect("requests lock");
    assert_eq!(sent_requests.len(), 1);
    assert_eq!(sent_requests[0].get("stream").and_then(|v| v.as_bool()), Some(true));
    let messages = sent_requests[0]
        .get("messages")
        .and_then(|v| v.as_array())
        .expect("messages array should exist");
    let last = messages.last().expect("user message should exist");
    assert_eq!(last.get("role").and_then(|v| v.as_str()), Some("user"));
    assert_eq!(last.get("name").and_then(|v| v.as_str()), Some("alice"));
    assert_eq!(last.get("content").and_then(|v| v.as_str()), Some("hello from user"));

    let worker = shutdown_dialogue_worker(handle, &shutdown_tx).await;
    assert_eq!(worker.health_check(), WorkerStatus::Stopped);
}

#[tokio::test]
async fn dialogue_worker_sends_multimodal_user_content_for_image_turns() {
    let (addr, requests) =
        spawn_mock_chat_server(plain_streaming_responses(&["vision reply"]))
            .await;
    let worker = DialogueEngineWorker::new(disabled_tool_config(format!("http://{}", addr)))
        .with_system_prompt("system".to_string());

    let (handle, mut event_rx, broadcast_tx, shutdown_tx) = start_dialogue_worker(worker, 16).await;

    let raw = mention_event_with_image("alice", "vision-channel", "nhìn ảnh này nhé");
    broadcast_tx.send(Event::Raw(raw)).expect("broadcast should send");

    let _ = recv_event_within(&mut event_rx, Duration::from_secs(1)).await;
    let _ = recv_event_within(&mut event_rx, Duration::from_secs(1)).await;

    let sent_requests = requests.lock().expect("requests lock");
    assert_eq!(sent_requests.len(), 1);
    let messages = sent_requests[0]
        .get("messages")
        .and_then(|v| v.as_array())
        .expect("messages array should exist");
    let last = messages.last().expect("user message should exist");
    let content = last
        .get("content")
        .and_then(|v| v.as_array())
        .expect("multimodal content array should exist");
    assert_eq!(content[0].get("type").and_then(|v| v.as_str()), Some("text"));
    assert_eq!(content[1].get("type").and_then(|v| v.as_str()), Some("image_url"));

    let worker = shutdown_dialogue_worker(handle, &shutdown_tx).await;
    assert_eq!(worker.health_check(), WorkerStatus::Stopped);
}

#[tokio::test]
async fn dialogue_worker_executes_tool_loop_before_streaming_final_answer() {
    let (addr, requests) = spawn_mock_chat_server(planning_then_streaming_responses(
        "{\"user_id\":\"alice\"}",
        &["tool loop reply"],
    ))
    .await;
    let short_term = seeded_short_term_memory(
        "tool-channel",
        vec![
            history_message("alice", "tool-channel", "tao đang buồn"),
            bot_history_message("tool-channel", "kể tao nghe đi", Some("alice")),
        ],
    );
    let graph = seeded_social_graph("alice").await;
    let worker = DialogueEngineWorker::new(enabled_tool_config(format!("http://{}", addr)))
        .with_system_prompt("system".to_string())
        .with_memory(short_term)
        .with_graph(graph);

    let (handle, mut event_rx, broadcast_tx, shutdown_tx) = start_dialogue_worker(worker, 16).await;

    let raw = mention_event_in_channel("alice", "tool-channel", "xin lỗi vì lúc nãy nhé");
    let message_id = raw.message_id.clone();
    broadcast_tx.send(Event::Raw(raw)).expect("broadcast should send");

    let response = recv_event_within(&mut event_rx, Duration::from_secs(1)).await;
    match response {
        Event::Response(response) => {
            assert_eq!(response.reply_to_message_id.as_deref(), Some(message_id.as_str()));
            assert_eq!(response.content, "tool loop reply");
            assert_eq!(response.source, ResponseSource::CloudLLM);
        }
        other => panic!("expected response event, got {other:?}"),
    }

    let completion = recv_event_within(&mut event_rx, Duration::from_secs(1)).await;
    match completion {
        Event::BotTurnCompletion(done) => {
            assert_eq!(done.reply_to_message_id.as_deref(), Some(message_id.as_str()));
            assert_eq!(done.content, "tool loop reply");
        }
        other => panic!("expected turn completion event, got {other:?}"),
    }

    let sent_requests = requests.lock().expect("requests lock");
    assert_eq!(sent_requests.len(), 3);
    assert_eq!(sent_requests[0].get("stream").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(sent_requests[1].get("stream").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(sent_requests[2].get("stream").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        sent_requests[0]
            .get("tools")
            .and_then(|v| v.as_array())
            .map(|v| v.len()),
        Some(1)
    );

    let final_messages = sent_requests[2]
        .get("messages")
        .and_then(|v| v.as_array())
        .expect("final messages should exist");
    let assistant_tool_call = final_messages
        .iter()
        .find(|message| {
            message.get("role").and_then(|v| v.as_str()) == Some("assistant")
                && message
                    .get("tool_calls")
                    .and_then(|v| v.as_array())
                    .map(|v| v.len())
                    == Some(1)
        })
        .expect("assistant tool-call message should exist");
    assert_eq!(
        assistant_tool_call
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .map(|v| v.len()),
        Some(1)
    );
    let tool_message = final_messages
        .iter()
        .find(|message| {
            message.get("role").and_then(|v| v.as_str()) == Some("tool")
                && message.get("name").and_then(|v| v.as_str())
                    == Some("social.get_dialogue_summary")
        })
        .expect("tool result message should exist");
    let tool_payload: Value = serde_json::from_str(
        tool_message
            .get("content")
            .and_then(|v| v.as_str())
            .expect("tool content should be string"),
    )
    .expect("tool content should be json");
    assert_eq!(tool_payload.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        tool_payload
            .get("result")
            .and_then(|v| v.get("user_id"))
            .and_then(|v| v.as_str()),
        Some("alice")
    );

    let worker = shutdown_dialogue_worker(handle, &shutdown_tx).await;
    assert_eq!(worker.health_check(), WorkerStatus::Stopped);
}

#[tokio::test]
async fn dialogue_worker_history_helper_formats_context() {
    let short_term = seeded_short_term_memory(
        "ctx-channel",
        vec![
            history_message("alice", "ctx-channel", "hello there"),
            bot_history_message("ctx-channel", "hi back", Some("alice")),
        ],
    );

    let formatted = formatted_history_context(&short_term, "ctx-channel")
        .await
        .expect("history should format");
    assert!(formatted.contains("alice: hello there"));
    assert!(formatted.contains("hi back"));
}

#[tokio::test]
async fn dialogue_worker_state_helper_seeds_rows() {
    let store = seeded_state_store(vec![EventDeltaRequest {
        dimension_id: "style.warmth".to_string(),
        delta: 0.1,
        reason: "test warm tone".to_string(),
        actor: "integration-test".to_string(),
        source: "integration-test".to_string(),
    }])
    .await
    .expect("state store should seed");

    let rows = store.rows().await;
    let warmth = rows
        .iter()
        .find(|row| row.id == "style.warmth")
        .expect("warmth row should exist");
    assert!(warmth.value > warmth.baseline);
}

#[tokio::test]
async fn dialogue_worker_graph_helper_can_project_snapshot() {
    let graph = in_memory_graph().await;
    graph
        .update_social_graph(
            "alice",
            SocialDelta {
                delta_trust: 0.3,
                ..Default::default()
            },
        )
        .await
        .expect("social graph should update");

    let snapshot = graph
        .get_or_project_social_tree_snapshot("alice", 0.2)
        .await
        .expect("tree snapshot should project");
    assert_eq!(snapshot.user_id, "alice");
    assert_eq!(snapshot.derived_summaries.trust_state, "neutral");
}
