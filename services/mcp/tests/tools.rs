use std::sync::Arc;

use memory::graph::CognitiveGraph;
use mcp::{
    registry::{ToolNamespace, ToolRegistry},
    ExecutionToolProvider, RegisteredTool, SearchProviderConfig, SearchToolProvider,
    SocialToolProvider, ToolProvider, WebFetchProviderConfig, WebFetchToolProvider,
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

fn assert_registered_tool_schema(value: &serde_json::Value) {
    assert_eq!(value.get("type").and_then(|v| v.as_str()), Some("object"));
    assert_eq!(
        value
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>()),
        Some(vec!["user_id"])
    );
    assert_eq!(
        value.get("additionalProperties").and_then(|v| v.as_bool()),
        Some(false)
    );
}

fn assert_execution_tool_schema(value: &serde_json::Value) {
    assert_eq!(value.get("type").and_then(|v| v.as_str()), Some("object"));
    assert_eq!(
        value
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>()),
        Some(vec!["command"])
    );
    assert_eq!(
        value.get("additionalProperties").and_then(|v| v.as_bool()),
        Some(false)
    );
}

fn assert_search_tool_schema(value: &serde_json::Value) {
    assert_eq!(value.get("type").and_then(|v| v.as_str()), Some("object"));
    assert_eq!(
        value
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>()),
        Some(vec!["query"])
    );
    assert_eq!(
        value.get("additionalProperties").and_then(|v| v.as_bool()),
        Some(false)
    );
}

fn assert_web_fetch_tool_schema(value: &serde_json::Value) {
    assert_eq!(value.get("type").and_then(|v| v.as_str()), Some("object"));
    assert_eq!(
        value
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>()),
        Some(vec!["url"])
    );
    assert_eq!(
        value.get("additionalProperties").and_then(|v| v.as_bool()),
        Some(false)
    );
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

fn registry_with_execution(enabled: bool) -> ToolRegistry {
    ToolRegistry::new(vec![
        Arc::new(SocialToolProvider::default()),
        Arc::new(ExecutionToolProvider::new(enabled)),
    ])
}

fn registry_with_search(enabled: bool) -> ToolRegistry {
    let search_provider = SearchToolProvider::new(SearchProviderConfig {
        enabled,
        api_key: if enabled {
            Some("test-key".to_string())
        } else {
            None
        },
        timeout_ms: 2_000,
        max_results: 5,
        brave_api_base: "https://example.invalid/search".to_string(),
    });

    ToolRegistry::new(vec![
        Arc::new(SocialToolProvider::default()),
        Arc::new(search_provider),
    ])
}

fn registry_with_web_fetch(enabled: bool) -> ToolRegistry {
    let web_fetch_provider = WebFetchToolProvider::new(WebFetchProviderConfig {
        enabled,
        timeout_ms: 2_000,
        max_bytes: 100_000,
        max_chars: 4_000,
        max_redirects: 3,
        max_key_links: 8,
    });

    ToolRegistry::new(vec![
        Arc::new(SocialToolProvider::default()),
        Arc::new(web_fetch_provider),
    ])
}

fn assert_two_read_only_social_tools(registry: &ToolRegistry) {
    let tools = registry.list();
    assert_eq!(tools.len(), 2);
    assert!(tools.iter().all(|tool| tool.namespace == ToolNamespace::Read));
    assert!(tools.iter().all(|tool| tool.read_only));
    assert!(
        tools
            .iter()
            .any(|tool| tool.name == "social.get_affect_context")
    );
    assert!(
        tools
            .iter()
            .any(|tool| tool.name == "social.get_dialogue_summary")
    );
}

fn assert_no_execution_tool(registry: &ToolRegistry) {
    assert!(
        registry
            .list()
            .iter()
            .all(|tool| !tool.name.starts_with("execution."))
    );
}

fn assert_one_execution_action_tool(registry: &ToolRegistry) {
    let tools = registry.list();
    assert!(
        tools
            .iter()
            .any(|tool| tool.name == "social.get_affect_context")
    );
    assert!(
        tools
            .iter()
            .any(|tool| tool.name == "social.get_dialogue_summary")
    );
    let execution = tools
        .iter()
        .find(|tool| tool.name == "execution.run_shell")
        .expect("execution tool should exist");
    assert_eq!(execution.namespace, ToolNamespace::Action);
    assert!(!execution.read_only);
}

fn assert_no_search_tool(registry: &ToolRegistry) {
    assert!(
        registry
            .list()
            .iter()
            .all(|tool| !tool.name.starts_with("search."))
    );
}

fn assert_search_tool_registered(registry: &ToolRegistry) {
    let tool = registry
        .get_registered("search.web")
        .expect("search.web should be registered");
    assert_eq!(tool.descriptor.namespace, ToolNamespace::Read);
    assert!(tool.descriptor.read_only);
    assert_search_tool_schema(&tool.input_schema);
}

fn assert_no_web_fetch_tool(registry: &ToolRegistry) {
    assert!(
        registry
            .list()
            .iter()
            .all(|tool| !tool.name.starts_with("web."))
    );
}

fn assert_web_fetch_tool_registered(registry: &ToolRegistry) {
    let tool = registry
        .get_registered("web.fetch")
        .expect("web.fetch should be registered");
    assert_eq!(tool.descriptor.namespace, ToolNamespace::Read);
    assert!(tool.descriptor.read_only);
    assert_web_fetch_tool_schema(&tool.input_schema);
}

fn assert_tool_descriptor_shape(tool: &RegisteredTool) {
    assert!(!tool.description.is_empty());
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
    approx_eq(
        as_f64(result.get("metrics").expect("metrics should exist"), "affinity"),
        0.0,
    );
    approx_eq(
        as_f64(result.get("illusion").expect("illusion should exist"), "trust"),
        0.0,
    );
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
    assert!(
        result
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .contains("[social summary] user=alice")
    );
    assert_meta_shape(&result);
}

#[tokio::test]
async fn affect_tool_rejects_empty_user_id() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");
    let registry = ToolRegistry::default();

    let result = registry
        .execute("social.get_affect_context", json!({ "user_id": "   " }), &graph)
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
        .execute("social.get_dialogue_summary", json!({ "user_id": "" }), &graph)
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

#[tokio::test]
async fn default_registry_does_not_expose_execution_tool() {
    let registry = ToolRegistry::default();
    assert_no_execution_tool(&registry);
    assert!(registry.get_registered("execution.run_shell").is_none());
}

#[tokio::test]
async fn enabled_execution_provider_registers_action_tool_with_schema() {
    let registry = registry_with_execution(true);
    assert_one_execution_action_tool(&registry);

    let tool = registry
        .get_registered("execution.run_shell")
        .expect("execution tool should be registered when enabled");
    assert_tool_descriptor_shape(tool);
    assert_execution_tool_schema(&tool.input_schema);
}

#[tokio::test]
async fn enabled_execution_provider_returns_not_implemented_error() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");
    let registry = registry_with_execution(true);

    let result = registry
        .execute("execution.run_shell", json!({ "command": "echo hi" }), &graph)
        .await;

    assert!(result.is_err());
    let message = result.err().unwrap().to_string();
    assert!(message.contains("not implemented"));
}

#[tokio::test]
async fn disabled_execution_provider_does_not_register_tool() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");
    let registry = registry_with_execution(false);

    assert_no_execution_tool(&registry);

    let result = registry
        .execute("execution.run_shell", json!({ "command": "echo hi" }), &graph)
        .await;

    assert!(result.is_err());
    let message = result.err().unwrap().to_string();
    assert!(
        message.contains("unknown MCP tool") || message.contains("execution tools are disabled")
    );
}

#[tokio::test]
async fn default_registry_does_not_expose_search_tool() {
    let registry = ToolRegistry::default();
    assert_no_search_tool(&registry);
    assert!(registry.get_registered("search.web").is_none());
}

#[tokio::test]
async fn enabled_search_provider_registers_read_tool_with_schema() {
    let registry = registry_with_search(true);
    assert_search_tool_registered(&registry);
}

#[tokio::test]
async fn disabled_search_provider_does_not_register_tool() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");
    let registry = registry_with_search(false);

    assert_no_search_tool(&registry);
    let result = registry.execute("search.web", json!({ "query": "rust" }), &graph).await;
    let err = result.expect_err("disabled search should not execute");
    assert!(
        err.to_string().contains("unknown MCP tool")
            || err.to_string().contains("search tools are disabled")
    );
}

#[tokio::test]
async fn enabled_search_provider_rejects_empty_query_without_network() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");
    let registry = registry_with_search(true);

    let result = registry.execute("search.web", json!({ "query": "   " }), &graph).await;
    let err = result.expect_err("empty query should fail");
    assert!(err.to_string().contains("query is required"));
}

#[tokio::test]
async fn enabled_search_provider_rejects_invalid_safesearch_without_network() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");
    let registry = registry_with_search(true);

    let result = registry
        .execute(
            "search.web",
            json!({ "query": "rust", "safesearch": "invalid" }),
            &graph,
        )
        .await;
    let err = result.expect_err("invalid safesearch should fail");
    assert!(
        err.to_string()
            .contains("safesearch must be one of: off, moderate, strict")
    );
}

#[tokio::test]
async fn default_registry_does_not_expose_web_fetch_tool() {
    let registry = ToolRegistry::default();
    assert_no_web_fetch_tool(&registry);
    assert!(registry.get_registered("web.fetch").is_none());
}

#[tokio::test]
async fn enabled_web_fetch_provider_registers_read_tool_with_schema() {
    let registry = registry_with_web_fetch(true);
    assert_web_fetch_tool_registered(&registry);
}

#[tokio::test]
async fn disabled_web_fetch_provider_does_not_register_tool() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");
    let registry = registry_with_web_fetch(false);

    assert_no_web_fetch_tool(&registry);
    let result = registry
        .execute("web.fetch", json!({ "url": "https://example.com" }), &graph)
        .await;
    let err = result.expect_err("disabled web.fetch should not execute");
    assert!(
        err.to_string().contains("unknown MCP tool")
            || err.to_string().contains("web fetch tools are disabled")
    );
}

#[tokio::test]
async fn enabled_web_fetch_provider_rejects_empty_url_without_network() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");
    let registry = registry_with_web_fetch(true);

    let result = registry
        .execute("web.fetch", json!({ "url": "   " }), &graph)
        .await;
    let err = result.expect_err("empty url should fail");
    assert!(err.to_string().contains("url is required"));
}

#[tokio::test]
async fn enabled_web_fetch_provider_rejects_invalid_scheme_without_network() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");
    let registry = registry_with_web_fetch(true);

    let result = registry
        .execute("web.fetch", json!({ "url": "file:///etc/passwd" }), &graph)
        .await;
    let err = result.expect_err("invalid scheme should fail");
    assert!(err.to_string().contains("url scheme must be http or https"));
}

#[tokio::test]
async fn enabled_web_fetch_provider_rejects_localhost_without_network() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");
    let registry = registry_with_web_fetch(true);

    let result = registry
        .execute("web.fetch", json!({ "url": "http://localhost" }), &graph)
        .await;
    let err = result.expect_err("localhost should fail");
    assert!(err.to_string().contains("url host is not allowed"));
}

#[tokio::test]
async fn enabled_web_fetch_provider_rejects_private_ip_without_network() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");
    let registry = registry_with_web_fetch(true);

    let result = registry
        .execute("web.fetch", json!({ "url": "http://10.1.2.3" }), &graph)
        .await;
    let err = result.expect_err("private ip should fail");
    assert!(err.to_string().contains("url host is not allowed"));
}

#[tokio::test]
async fn enabled_web_fetch_provider_rejects_url_with_credentials_without_network() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");
    let registry = registry_with_web_fetch(true);

    let result = registry
        .execute(
            "web.fetch",
            json!({ "url": "https://user:pass@example.com" }),
            &graph,
        )
        .await;
    let err = result.expect_err("credentialed url should fail");
    assert!(err.to_string().contains("url with credentials is not allowed"));
}

#[tokio::test]
async fn enabled_web_fetch_provider_rejects_unknown_fields_without_network() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");
    let registry = registry_with_web_fetch(true);

    let result = registry
        .execute(
            "web.fetch",
            json!({ "url": "https://example.com", "unexpected": true }),
            &graph,
        )
        .await;
    let err = result.expect_err("unknown field should fail");
    assert!(err.to_string().contains("unknown field `unexpected`"));
}

#[tokio::test]
async fn enabled_web_fetch_provider_rejects_invalid_max_chars_without_network() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");
    let registry = registry_with_web_fetch(true);

    let result = registry
        .execute(
            "web.fetch",
            json!({ "url": "https://example.com", "max_chars": "oops" }),
            &graph,
        )
        .await;
    let err = result.expect_err("max_chars wrong type should fail");
    assert!(err.to_string().contains("invalid type"));
}

#[tokio::test]
async fn enabled_web_fetch_provider_rejects_invalid_url_without_network() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");
    let registry = registry_with_web_fetch(true);

    let result = registry
        .execute("web.fetch", json!({ "url": "not a url" }), &graph)
        .await;
    let err = result.expect_err("invalid url should fail");
    assert!(err.to_string().contains("invalid url"));
}

#[tokio::test]
async fn enabled_web_fetch_provider_rejects_ipv6_doc_range_without_network() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");
    let registry = registry_with_web_fetch(true);

    let result = registry
        .execute("web.fetch", json!({ "url": "http://[2001:db8::1]" }), &graph)
        .await;
    let err = result.expect_err("ipv6 documentation range should fail");
    assert!(err.to_string().contains("url host is not allowed"));
}
