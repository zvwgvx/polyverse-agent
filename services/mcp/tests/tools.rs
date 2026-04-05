use std::sync::Arc;

use memory::graph::CognitiveGraph;
use mcp::{
    registry::{ToolNamespace, ToolRegistry},
    RegisteredTool, SocialToolProvider, ToolProvider,
};
use serde_json::json;

fn as_f64(value: &serde_json::Value, key: &str) -> f64 {
    value
        .get(key)
        .and_then(|v| v.as_f64())
        .unwrap_or_else(|| panic!("missing numeric field: {key}"))
}

fn nested_as_f64(value: &serde_json::Value, parent: &str, key: &str) -> f64 {
    value
        .get(parent)
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_f64())
        .unwrap_or_else(|| panic!("missing numeric field: {parent}.{key}"))
}

fn assert_meta_shape(value: &serde_json::Value) {
    let meta = value.get("meta").expect("meta should exist");
    assert!(meta.get("source").and_then(|v| v.as_str()).is_some());
    assert!(meta.get("stale").and_then(|v| v.as_bool()).is_some());
    assert!(meta.get("schema_version").and_then(|v| v.as_str()).is_some());
    assert!(meta.get("updated_at").and_then(|v| v.as_str()).is_some());
}

fn assert_two_read_only_social_tools(registry: &ToolRegistry) {
    let tools = registry.list();
    assert_eq!(tools.len(), 2);
    assert!(tools.iter().all(|tool| tool.namespace == ToolNamespace::Read));
    assert!(tools.iter().all(|tool| tool.read_only));
    assert!(tools.iter().any(|tool| tool.name == "social.get_affect_context"));
    assert!(tools.iter().any(|tool| tool.name == "social.get_dialogue_summary"));
}

fn assert_registered_tool_schema(value: &serde_json::Value) {
    assert_eq!(value.get("type").and_then(|v| v.as_str()), Some("object"));
    assert_eq!(
        value.get("required")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>()),
        Some(vec!["user_id"])
    );
    assert_eq!(value.get("additionalProperties").and_then(|v| v.as_bool()), Some(false));
}

fn approx_eq(left: f64, right: f64) {
    assert!((left - right).abs() < 1e-6, "left={left} right={right}");
}

fn provider_tools() -> Vec<RegisteredTool> {
    let provider: Arc<dyn ToolProvider> = Arc::new(SocialToolProvider::default());
    provider.tools().to_vec()
}

fn custom_registry() -> ToolRegistry {
    ToolRegistry::new(vec![Arc::new(SocialToolProvider::default())])
}

#[tokio::test]
async fn registry_lists_two_read_only_social_tools() {
    let registry = ToolRegistry::default();
    assert_two_read_only_social_tools(&registry);
}

#[tokio::test]
async fn social_provider_exposes_metadata_and_schema() {
    let tools = provider_tools();
    assert_eq!(tools.len(), 2);
    for tool in tools {
        assert!(!tool.description.is_empty());
        assert_registered_tool_schema(&tool.input_schema);
    }
}

#[tokio::test]
async fn registry_exposes_registered_tool_metadata() {
    let registry = custom_registry();
    let tool = registry
        .get_registered("social.get_dialogue_summary")
        .expect("registered tool should exist");

    assert_eq!(tool.descriptor.name, "social.get_dialogue_summary");
    assert!(tool.description.contains("dialogue summary"));
    assert_registered_tool_schema(&tool.input_schema);
    assert_eq!(registry.list_registered().len(), registry.list().len());
}

#[tokio::test]
async fn affect_tool_clamps_memory_hint_into_unit_interval() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");
    let registry = ToolRegistry::default();

    let clamped_high = registry
        .execute(
            "social.get_affect_context",
            json!({ "user_id": "alice", "memory_hint": 5.0, "force_project": true }),
            &graph,
        )
        .await
        .expect("tool call should succeed");
    let clamped_low = registry
        .execute(
            "social.get_affect_context",
            json!({ "user_id": "bob", "memory_hint": -5.0, "force_project": true }),
            &graph,
        )
        .await
        .expect("tool call should succeed");

    approx_eq(nested_as_f64(&clamped_high, "metrics", "context_depth"), 1.0);
    approx_eq(nested_as_f64(&clamped_low, "metrics", "context_depth"), 0.0);
}

#[tokio::test]
async fn dialogue_tool_trims_user_id_and_returns_stable_meta_shape() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");
    let registry = ToolRegistry::default();

    let result = registry
        .execute(
            "social.get_dialogue_summary",
            json!({ "user_id": "  alice  ", "memory_hint": 0.2 }),
            &graph,
        )
        .await
        .expect("tool call should succeed");

    assert_eq!(result.get("user_id").and_then(|v| v.as_str()), Some("alice"));
    assert_eq!(result.get("familiarity").and_then(|v| v.as_str()), Some("new"));
    assert_eq!(result.get("trust_state").and_then(|v| v.as_str()), Some("neutral"));
    assert_eq!(result.get("tension_state").and_then(|v| v.as_str()), Some("low"));
    assert_meta_shape(&result);
    assert_eq!(
        result
            .get("meta")
            .and_then(|v| v.get("source"))
            .and_then(|v| v.as_str()),
        Some("tree_fresh")
    );
}

#[tokio::test]
async fn affect_tool_returns_expected_default_shape() {
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

    assert_eq!(result.get("known").and_then(|v| v.as_bool()), Some(false));
    approx_eq(as_f64(result.get("metrics").expect("metrics should exist"), "affinity"), 0.0);
    approx_eq(as_f64(result.get("illusion").expect("illusion should exist"), "trust"), 0.0);
    assert_meta_shape(&result);
}

#[tokio::test]
async fn dialogue_tool_returns_expected_default_shape() {
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

    assert_eq!(result.get("familiarity").and_then(|v| v.as_str()), Some("new"));
    assert_eq!(result.get("trust_state").and_then(|v| v.as_str()), Some("neutral"));
    assert_eq!(result.get("tension_state").and_then(|v| v.as_str()), Some("low"));
    assert!(result
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .contains("[social summary] user=alice"));
    assert_meta_shape(&result);
}

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

    let result = registry.execute("social.unknown", json!({}), &graph).await;

    assert!(result.is_err());
    let message = result.err().unwrap().to_string();
    assert!(message.contains("unknown MCP tool"));
}
