use std::sync::{Mutex, OnceLock};

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

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn set_env_var(name: &str, value: &str) {
    unsafe {
        std::env::set_var(name, value);
    }
}

fn remove_env_var(name: &str) {
    unsafe {
        std::env::remove_var(name);
    }
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
    let tool_names: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool.get("name").and_then(|v| v.as_str()))
        .collect();
    assert!(tool_names.contains(&"social.get_affect_context"));
    assert!(tool_names.contains(&"social.get_dialogue_summary"));

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

#[tokio::test]
async fn live_mcp_worker_serves_web_tools_when_env_enabled_over_http() -> Result<()> {
    let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());

    set_env_var("MCP_SEARCH_ENABLED", "true");
    set_env_var("BRAVE_SEARCH_API_KEY", "integration-test-key");
    set_env_var("MCP_WEB_FETCH_ENABLED", "true");

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
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool.get("name").and_then(|v| v.as_str()))
        .collect();
    assert!(names.contains(&"search.web"));
    assert!(names.contains(&"web.fetch"));

    let web_fetch_response = client
        .post(format!("http://{bind_addr}/api/mcp/tools/call"))
        .json(&serde_json::json!({
            "name": "web.fetch",
            "input": { "url": "http://localhost" }
        }))
        .send()
        .await?;

    assert_eq!(web_fetch_response.status(), reqwest::StatusCode::BAD_REQUEST);
    let web_fetch_payload: serde_json::Value = web_fetch_response.json().await?;
    assert_eq!(
        web_fetch_payload.get("ok").and_then(|v| v.as_bool()),
        Some(false)
    );
    assert!(web_fetch_payload
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .contains("url host is not allowed"));

    let search_response = client
        .post(format!("http://{bind_addr}/api/mcp/tools/call"))
        .json(&serde_json::json!({
            "name": "search.web",
            "input": { "query": "   " }
        }))
        .send()
        .await?;

    assert_eq!(search_response.status(), reqwest::StatusCode::BAD_REQUEST);
    let search_payload: serde_json::Value = search_response.json().await?;
    assert_eq!(search_payload.get("ok").and_then(|v| v.as_bool()), Some(false));
    assert!(search_payload
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .contains("query is required"));

    supervisor.shutdown().await?;

    remove_env_var("MCP_SEARCH_ENABLED");
    remove_env_var("BRAVE_SEARCH_API_KEY");
    remove_env_var("MCP_WEB_FETCH_ENABLED");

    Ok(())
}

#[tokio::test]
async fn live_mcp_worker_does_not_expose_web_tools_when_env_disabled_over_http() -> Result<()> {
    let _guard = env_lock().lock().unwrap_or_else(|p| p.into_inner());

    remove_env_var("MCP_SEARCH_ENABLED");
    remove_env_var("BRAVE_SEARCH_API_KEY");
    remove_env_var("MCP_WEB_FETCH_ENABLED");

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
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool.get("name").and_then(|v| v.as_str()))
        .collect();

    assert!(!names.contains(&"search.web"));
    assert!(!names.contains(&"web.fetch"));

    supervisor.shutdown().await?;
    Ok(())
}

