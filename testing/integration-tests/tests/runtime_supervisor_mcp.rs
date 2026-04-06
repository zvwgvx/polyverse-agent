use anyhow::Result;
use kernel::event::{Event, SystemEvent};
use mcp::McpTransport;
use runtime::Supervisor;
use test_support::in_memory_graph;
use tokio::time::{timeout, Duration};

#[tokio::test]
async fn supervisor_can_start_and_shutdown_mcp_worker() -> Result<()> {
    let graph = in_memory_graph().await;

    let probe_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let bind_addr = probe_listener.local_addr()?;
    drop(probe_listener);

    let mut supervisor = Supervisor::new();
    let mut event_rx = supervisor
        .event_bus_mut()
        .take_event_rx()
        .expect("event receiver should exist");

    supervisor.register(mcp::McpWorker::new(
        mcp::McpConfig {
            enabled: true,
            transport: McpTransport::Http,
            bind_addr: bind_addr.to_string(),
            ..mcp::McpConfig::default()
        },
        graph,
    ));

    supervisor.start_all().await?;

    let started = timeout(Duration::from_secs(2), event_rx.recv())
        .await
        .expect("worker started event should arrive")
        .expect("worker started event should exist");
    match started {
        Event::System(SystemEvent::WorkerStarted { name }) => assert_eq!(name, "mcp"),
        other => panic!("expected worker started event, got {other:?}"),
    }

    timeout(Duration::from_secs(2), async {
        loop {
            if tokio::net::TcpStream::connect(bind_addr).await.is_ok() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("mcp worker should bind in time");

    supervisor.shutdown().await?;
    assert_eq!(supervisor.worker_count(), 0);
    Ok(())
}

async fn start_live_mcp_supervisor() -> Result<(Supervisor, std::net::SocketAddr)> {
    let graph = in_memory_graph().await;

    let probe_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let bind_addr = probe_listener.local_addr()?;
    drop(probe_listener);

    let mut supervisor = Supervisor::new();
    supervisor.register(mcp::McpWorker::new(
        mcp::McpConfig {
            enabled: true,
            transport: McpTransport::Http,
            bind_addr: bind_addr.to_string(),
            ..mcp::McpConfig::default()
        },
        graph,
    ));
    supervisor.start_all().await?;

    timeout(Duration::from_secs(2), async {
        loop {
            if tokio::net::TcpStream::connect(bind_addr).await.is_ok() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("mcp worker should bind in time");

    Ok((supervisor, bind_addr))
}

#[tokio::test]
async fn live_mcp_worker_serves_http_tool_requests() -> Result<()> {
    let (mut supervisor, bind_addr) = start_live_mcp_supervisor().await?;

    let client = reqwest::Client::new();
    let tools: serde_json::Value = client
        .get(format!("http://{bind_addr}/api/mcp/tools"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let tools = tools.as_array().expect("tools payload should be array");
    assert_eq!(tools.len(), 2);

    let response: serde_json::Value = client
        .post(format!("http://{bind_addr}/api/mcp/tools/call"))
        .json(&serde_json::json!({
            "name": "social.get_dialogue_summary",
            "input": {
                "user_id": "alice",
                "memory_hint": 0.2
            }
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    assert_eq!(response.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        response
            .get("result")
            .and_then(|v| v.get("user_id"))
            .and_then(|v| v.as_str()),
        Some("alice")
    );
    assert_eq!(
        response
            .get("result")
            .and_then(|v| v.get("meta"))
            .and_then(|v| v.get("source"))
            .and_then(|v| v.as_str()),
        Some("tree_fresh")
    );

    supervisor.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn live_mcp_worker_returns_http_errors_for_bad_requests() -> Result<()> {
    let (mut supervisor, bind_addr) = start_live_mcp_supervisor().await?;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{bind_addr}/api/mcp/tools/call"))
        .json(&serde_json::json!({
            "name": "social.unknown",
            "input": {}
        }))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let payload: serde_json::Value = response.json().await?;
    assert_eq!(payload.get("ok").and_then(|v| v.as_bool()), Some(false));
    assert!(payload
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .contains("unknown MCP tool"));

    supervisor.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn live_mcp_worker_rejects_empty_tool_name_over_http() -> Result<()> {
    let (mut supervisor, bind_addr) = start_live_mcp_supervisor().await?;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{bind_addr}/api/mcp/tools/call"))
        .json(&serde_json::json!({
            "name": "   ",
            "input": {}
        }))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let payload: serde_json::Value = response.json().await?;
    assert_eq!(payload.get("ok").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(
        payload.get("error").and_then(|v| v.as_str()),
        Some("tool name is required")
    );

    supervisor.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn live_mcp_worker_rejects_missing_user_id_over_http() -> Result<()> {
    let (mut supervisor, bind_addr) = start_live_mcp_supervisor().await?;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{bind_addr}/api/mcp/tools/call"))
        .json(&serde_json::json!({
            "name": "social.get_dialogue_summary",
            "input": {}
        }))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let payload: serde_json::Value = response.json().await?;
    assert_eq!(payload.get("ok").and_then(|v| v.as_bool()), Some(false));
    assert!(payload
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .contains("missing field `user_id`"));

    supervisor.shutdown().await?;
    Ok(())
}

