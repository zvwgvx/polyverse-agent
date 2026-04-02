use kernel::worker::{Worker, WorkerContext, WorkerStatus};
use memory::graph::CognitiveGraph;
use mcp::{McpConfig, McpWorker};
use std::sync::{Mutex, MutexGuard, OnceLock};
use tokio::sync::{broadcast, mpsc};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn env_guard() -> MutexGuard<'static, ()> {
    env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn clear_mcp_env() {
    unsafe {
        std::env::remove_var("MCP_ENABLED");
        std::env::remove_var("MCP_BIND");
        std::env::remove_var("MCP_REQUEST_TIMEOUT_MS");
        std::env::remove_var("MCP_MAX_TOOL_CALLS_PER_TURN");
    }
}

fn set_mcp_env(key: &str, value: &str) {
    unsafe {
        std::env::set_var(key, value);
    }
}

struct McpEnvReset;

impl Drop for McpEnvReset {
    fn drop(&mut self) {
        clear_mcp_env();
    }
}

fn reset_mcp_env() -> McpEnvReset {
    clear_mcp_env();
    McpEnvReset
}

fn test_worker_context() -> WorkerContext {
    let (event_tx, _event_rx) = mpsc::channel(1);
    let (broadcast_tx, _) = broadcast::channel(1);
    let (shutdown_tx, _) = broadcast::channel(1);
    WorkerContext {
        event_tx,
        broadcast_rx: broadcast_tx,
        shutdown: shutdown_tx,
    }
}

async fn in_memory_graph() -> CognitiveGraph {
    CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize")
}

#[tokio::test]
async fn mcp_worker_constructs_with_default_registry() {
    let graph = in_memory_graph().await;

    let worker = McpWorker::new(McpConfig::default(), graph);
    let tools = worker.registry().list();

    assert_eq!(tools.len(), 2);
    assert!(tools.iter().any(|t| t.name == "social.get_affect_context"));
    assert!(tools.iter().any(|t| t.name == "social.get_dialogue_summary"));
    assert!(tools.iter().all(|t| t.read_only));
}

#[tokio::test]
async fn mcp_worker_exposes_graph_reference() {
    let graph = in_memory_graph().await;

    let worker = McpWorker::new(McpConfig::default(), graph.clone());
    let snapshot = worker
        .graph()
        .snapshot_relationship_graph()
        .await
        .expect("graph snapshot should succeed");

    assert_eq!(snapshot.self_node_id, graph.self_node_id());
}

#[tokio::test]
async fn disabled_mcp_worker_stops_immediately() {
    let graph = in_memory_graph().await;
    let mut worker = McpWorker::new(
        McpConfig {
            enabled: false,
            ..McpConfig::default()
        },
        graph,
    );

    let ctx = test_worker_context();

    worker.start(ctx).await.expect("disabled worker should start cleanly");
    assert_eq!(worker.health_check(), WorkerStatus::Stopped);
}

#[tokio::test]
async fn stop_marks_mcp_worker_stopped() {
    let graph = in_memory_graph().await;
    let mut worker = McpWorker::new(McpConfig::default(), graph);

    worker.stop().await.expect("stop should succeed");
    assert_eq!(worker.health_check(), WorkerStatus::Stopped);
}

#[tokio::test]
async fn invalid_bind_address_errors_during_start() {
    let graph = in_memory_graph().await;
    let mut worker = McpWorker::new(
        McpConfig {
            enabled: true,
            bind_addr: "not-an-addr".to_string(),
            ..McpConfig::default()
        },
        graph,
    );

    let ctx = test_worker_context();

    let err = worker.start(ctx).await.expect_err("invalid bind should fail");
    assert!(err.to_string().contains("invalid MCP_BIND address"));
    assert_eq!(worker.health_check(), WorkerStatus::NotStarted);
}

#[test]
fn mcp_config_from_env_applies_defaults_and_clamps() {
    let _guard = env_guard();
    let _reset = reset_mcp_env();

    let defaults = McpConfig::from_env();
    assert!(!defaults.enabled);
    assert_eq!(defaults.bind_addr, "127.0.0.1:4790");
    assert_eq!(defaults.request_timeout_ms, 2_000);
    assert_eq!(defaults.max_tool_calls_per_turn, 4);

    set_mcp_env("MCP_ENABLED", "yes");
    set_mcp_env("MCP_BIND", "0.0.0.0:9999");
    set_mcp_env("MCP_REQUEST_TIMEOUT_MS", "50");
    set_mcp_env("MCP_MAX_TOOL_CALLS_PER_TURN", "0");

    let overridden = McpConfig::from_env();
    assert!(overridden.enabled);
    assert_eq!(overridden.bind_addr, "0.0.0.0:9999");
    assert_eq!(overridden.request_timeout_ms, 100);
    assert_eq!(overridden.max_tool_calls_per_turn, 1);
}

#[test]
fn mcp_config_from_env_ignores_blank_bind_and_invalid_numbers() {
    let _guard = env_guard();
    let _reset = reset_mcp_env();

    set_mcp_env("MCP_BIND", "   ");
    set_mcp_env("MCP_REQUEST_TIMEOUT_MS", "nope");
    set_mcp_env("MCP_MAX_TOOL_CALLS_PER_TURN", "bad");

    let config = McpConfig::from_env();
    assert_eq!(config.bind_addr, "127.0.0.1:4790");
    assert_eq!(config.request_timeout_ms, 2_000);
    assert_eq!(config.max_tool_calls_per_turn, 4);
}

#[tokio::test]
async fn worker_name_is_stable() {
    let graph = in_memory_graph().await;
    let worker = McpWorker::new(McpConfig::default(), graph);
    assert_eq!(worker.name(), "mcp");
}

