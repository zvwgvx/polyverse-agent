use pa_memory::graph::CognitiveGraph;
use pa_mcp::registry::ToolRegistry;
use serde_json::json;

#[tokio::test]
async fn affect_tool_rejects_empty_user_id() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");
    let registry = ToolRegistry::default();

    let result = registry
        .execute(
            "social.get_affect_context",
            json!({ "user_id": "   " }),
            &graph,
        )
        .await;

    assert!(result.is_err());
    let message = result.err().unwrap().to_string();
    assert!(message.contains("user_id is required"));
}

#[tokio::test]
async fn dialogue_tool_rejects_empty_user_id() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");
    let registry = ToolRegistry::default();

    let result = registry
        .execute(
            "social.get_dialogue_summary",
            json!({ "user_id": "" }),
            &graph,
        )
        .await;

    assert!(result.is_err());
    let message = result.err().unwrap().to_string();
    assert!(message.contains("user_id is required"));
}

#[tokio::test]
async fn unknown_tool_returns_deterministic_error() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");
    let registry = ToolRegistry::default();

    let result = registry
        .execute("social.unknown", json!({}), &graph)
        .await;

    assert!(result.is_err());
    let message = result.err().unwrap().to_string();
    assert!(message.contains("unknown MCP tool"));
}

#[tokio::test]
async fn affect_tool_response_contains_meta_fields() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");
    let registry = ToolRegistry::default();

    let result = registry
        .execute(
            "social.get_affect_context",
            json!({ "user_id": "alice", "memory_hint": 0.2 }),
            &graph,
        )
        .await
        .expect("tool call should succeed");

    assert!(result.get("meta").is_some());
    let meta = result.get("meta").unwrap();
    assert!(meta.get("source").is_some());
    assert!(meta.get("stale").is_some());
    assert!(meta.get("schema_version").is_some());
    assert!(meta.get("updated_at").is_some());
}

#[tokio::test]
async fn dialogue_tool_response_contains_meta_fields() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");
    let registry = ToolRegistry::default();

    let result = registry
        .execute(
            "social.get_dialogue_summary",
            json!({ "user_id": "alice", "memory_hint": 0.2 }),
            &graph,
        )
        .await
        .expect("tool call should succeed");

    assert!(result.get("meta").is_some());
    let meta = result.get("meta").unwrap();
    assert!(meta.get("source").is_some());
    assert!(meta.get("stale").is_some());
    assert!(meta.get("schema_version").is_some());
    assert!(meta.get("updated_at").is_some());
}
