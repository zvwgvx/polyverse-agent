use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use memory::graph::CognitiveGraph;
use mcp::{build_mcp_router, registry::ToolRegistry};
use serde_json::{json, Value};
use tower::util::ServiceExt;

mod config {
    pub use mcp::config::*;
}

mod registry {
    pub use mcp::registry::*;
}

#[path = "../src/server/mod.rs"]
mod server_impl;

async fn test_app() -> axum::Router {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");

    build_mcp_router(Arc::new(ToolRegistry::default()), graph, 2000)
}

async fn read_json(response: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    serde_json::from_slice(&body).expect("json body expected")
}

fn tool_call_request(body: Value) -> Request<Body> {
    Request::builder()
        .uri("/api/mcp/tools/call")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn tools_list_request() -> Request<Body> {
    Request::builder()
        .uri("/api/mcp/tools")
        .method("GET")
        .body(Body::empty())
        .unwrap()
}

async fn test_app_with_timeout_executor(request_timeout_ms: u64) -> axum::Router {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");

    server_impl::build_mcp_router_for_tests(
        Arc::new(ToolRegistry::default()),
        graph,
        request_timeout_ms,
        |_name, _input, _graph| async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            Ok(json!({"ok": true}))
        },
    )
}

#[tokio::test]
async fn tools_call_endpoint_times_out_with_deterministic_error_shape() {
    let app = test_app_with_timeout_executor(50).await;

    let response = app
        .oneshot(tool_call_request(json!({
            "name": "social.get_dialogue_summary",
            "input": { "user_id": "alice" }
        })))
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    let payload = read_json(response).await;
    assert_eq!(payload.get("ok").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(
        payload.get("error").and_then(|v| v.as_str()),
        Some("tool call timeout")
    );
}


#[tokio::test]
async fn tools_endpoint_lists_two_read_tools() {
    let app = test_app().await;

    let response = app
        .oneshot(tools_list_request())
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let payload = read_json(response).await;

    let arr = payload.as_array().expect("tools payload should be array");
    assert_eq!(arr.len(), 2);

    let names: Vec<String> = arr
        .iter()
        .filter_map(|item| item.get("name").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();

    assert!(names.contains(&"social.get_affect_context".to_string()));
    assert!(names.contains(&"social.get_dialogue_summary".to_string()));

    for tool in arr {
        assert_eq!(tool.get("read_only").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(tool.get("namespace").and_then(|v| v.as_str()), Some("read"));
    }
}

#[tokio::test]
async fn tools_call_endpoint_success_and_error_paths() {
    let app = test_app().await;

    let ok_resp = app
        .clone()
        .oneshot(tool_call_request(json!({
            "name": "social.get_dialogue_summary",
            "input": { "user_id": "alice", "memory_hint": 0.2 }
        })))
        .await
        .expect("request should succeed");

    assert_eq!(ok_resp.status(), StatusCode::OK);
    let ok_payload = read_json(ok_resp).await;
    let result = ok_payload.get("result").expect("result should exist");
    let meta = result.get("meta").expect("meta should exist");

    assert_eq!(ok_payload.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(result.get("user_id").and_then(|v| v.as_str()), Some("alice"));
    assert_eq!(meta.get("source").and_then(|v| v.as_str()), Some("tree_fresh"));
    assert_eq!(meta.get("stale").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(meta.get("schema_version").and_then(|v| v.as_str()), Some("v1"));
    assert!(meta.get("updated_at").and_then(|v| v.as_str()).is_some());

    let err_resp = app
        .oneshot(tool_call_request(json!({
            "name": "social.unknown",
            "input": {}
        })))
        .await
        .expect("request should succeed");

    assert_eq!(err_resp.status(), StatusCode::BAD_REQUEST);
    let err_payload = read_json(err_resp).await;
    assert_eq!(err_payload.get("ok").and_then(|v| v.as_bool()), Some(false));
    let msg = err_payload
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert!(msg.contains("unknown MCP tool"));
}

#[tokio::test]
async fn tools_call_endpoint_rejects_malformed_json_payload() {
    let app = test_app().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/mcp/tools/call")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from("{not-json}"))
                .unwrap(),
        )
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn tools_call_endpoint_trims_tool_name_before_execution() {
    let app = test_app().await;

    let response = app
        .oneshot(tool_call_request(json!({
            "name": "  social.get_dialogue_summary  ",
            "input": { "user_id": "alice" }
        })))
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let payload = read_json(response).await;
    assert_eq!(payload.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        payload
            .get("result")
            .and_then(|v| v.get("user_id"))
            .and_then(|v| v.as_str()),
        Some("alice")
    );
}

#[tokio::test]
async fn tools_call_endpoint_returns_affect_meta_shape() {
    let app = test_app().await;

    let response = app
        .oneshot(tool_call_request(json!({
            "name": "social.get_affect_context",
            "input": { "user_id": "alice", "memory_hint": 0.2 }
        })))
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let payload = read_json(response).await;
    let result = payload.get("result").expect("result should exist");
    let meta = result.get("meta").expect("meta should exist");

    assert_eq!(payload.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(result.get("known").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(meta.get("source").and_then(|v| v.as_str()), Some("tree_fresh"));
    assert_eq!(meta.get("stale").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(meta.get("schema_version").and_then(|v| v.as_str()), Some("v1"));
    assert!(meta.get("updated_at").and_then(|v| v.as_str()).is_some());
    assert!(result
        .get("metrics")
        .and_then(|v| v.get("context_depth"))
        .and_then(|v| v.as_f64())
        .is_some());
    assert!(result
        .get("illusion")
        .and_then(|v| v.get("trust"))
        .and_then(|v| v.as_f64())
        .is_some());
}

#[tokio::test]
async fn tools_call_endpoint_rejects_empty_tool_name() {
    let app = test_app().await;

    let response = app
        .oneshot(tool_call_request(json!({
            "name": "   ",
            "input": {}
        })))
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = read_json(response).await;
    assert_eq!(payload.get("ok").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(
        payload.get("error").and_then(|v| v.as_str()),
        Some("tool name is required")
    );
}

#[tokio::test]
async fn tools_call_endpoint_rejects_missing_user_id_with_deterministic_error() {
    let app = test_app().await;

    let response = app
        .oneshot(tool_call_request(json!({
            "name": "social.get_dialogue_summary",
            "input": {}
        })))
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = read_json(response).await;
    assert_eq!(payload.get("ok").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(
        payload.get("error").and_then(|v| v.as_str()),
        Some("missing field `user_id`")
    );
}

#[tokio::test]
async fn tools_call_endpoint_rejects_empty_user_id_with_deterministic_error() {
    let app = test_app().await;

    let response = app
        .oneshot(tool_call_request(json!({
            "name": "social.get_affect_context",
            "input": { "user_id": "   " }
        })))
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = read_json(response).await;
    assert_eq!(payload.get("ok").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(
        payload.get("error").and_then(|v| v.as_str()),
        Some("user_id is required")
    );
}

#[tokio::test]
async fn tools_call_endpoint_defaults_missing_input_to_empty_object() {
    let app = test_app().await;

    let response = app
        .oneshot(tool_call_request(json!({
            "name": "social.unknown"
        })))
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = read_json(response).await;
    assert_eq!(payload.get("ok").and_then(|v| v.as_bool()), Some(false));
    assert!(payload
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .contains("unknown MCP tool"));
}

#[tokio::test]
async fn tools_call_endpoint_enforces_minimum_timeout_floor() {
    let app = test_app_with_timeout_executor(1).await;

    let response = app
        .oneshot(tool_call_request(json!({
            "name": "social.get_dialogue_summary",
            "input": { "user_id": "alice" }
        })))
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    let payload = read_json(response).await;
    assert_eq!(payload.get("ok").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(
        payload.get("error").and_then(|v| v.as_str()),
        Some("tool call timeout")
    );
}

#[tokio::test]
async fn tools_call_endpoint_rejects_wrong_json_shape() {
    let app = test_app().await;

    let response = app
        .oneshot(tool_call_request(json!([
            {
                "name": "social.get_dialogue_summary",
                "input": { "user_id": "alice" }
            }
        ])))
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = read_json(response).await;
    assert_eq!(payload.get("ok").and_then(|v| v.as_bool()), Some(false));
    assert!(payload
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .contains("invalid type"));
}

#[tokio::test]
async fn tools_call_endpoint_rejects_extra_top_level_fields() {
    let app = test_app().await;

    let response = app
        .oneshot(tool_call_request(json!({
            "name": "social.get_dialogue_summary",
            "input": { "user_id": "alice" },
            "extra": true
        })))
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = read_json(response).await;
    assert_eq!(payload.get("ok").and_then(|v| v.as_bool()), Some(false));
    assert!(payload
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .contains("unknown field `extra`"));
}

#[tokio::test]
async fn tools_endpoint_meta_shape_is_stable_across_tools() {
    let app = test_app().await;

    let dialogue = app
        .clone()
        .oneshot(tool_call_request(json!({
            "name": "social.get_dialogue_summary",
            "input": { "user_id": "alice" }
        })))
        .await
        .expect("dialogue request should succeed");
    let affect = app
        .oneshot(tool_call_request(json!({
            "name": "social.get_affect_context",
            "input": { "user_id": "alice" }
        })))
        .await
        .expect("affect request should succeed");

    let dialogue_payload = read_json(dialogue).await;
    let affect_payload = read_json(affect).await;

    let dialogue_meta = dialogue_payload
        .get("result")
        .and_then(|v| v.get("meta"))
        .expect("dialogue meta should exist");
    let affect_meta = affect_payload
        .get("result")
        .and_then(|v| v.get("meta"))
        .expect("affect meta should exist");

    for key in ["source", "stale", "schema_version", "updated_at"] {
        assert!(dialogue_meta.get(key).is_some(), "dialogue meta missing {key}");
        assert!(affect_meta.get(key).is_some(), "affect meta missing {key}");
    }
}

#[tokio::test]
async fn tools_call_endpoint_rejects_missing_name_field() {
    let app = test_app().await;

    let response = app
        .oneshot(tool_call_request(json!({
            "input": { "user_id": "alice" }
        })))
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = read_json(response).await;
    assert_eq!(payload.get("ok").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(
        payload.get("error").and_then(|v| v.as_str()),
        Some("missing field `name`")
    );
}

#[tokio::test]
async fn tools_call_endpoint_uses_normalized_staleness_options() {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");
    graph
        .update_social_graph(
            "alice",
            memory::graph::SocialDelta {
                delta_trust: 0.3,
                ..Default::default()
            },
        )
        .await
        .expect("social graph should update");
    let _ = graph
        .get_or_project_social_tree_snapshot("alice", 0.2)
        .await
        .expect("snapshot should project");
    graph
        .db
        .query("UPDATE social_tree_root SET meta.updated_at = $updated_at WHERE user_id = $user_id;")
        .bind(("updated_at", "2000-01-01T00:00:00Z".to_string()))
        .bind(("user_id", "alice".to_string()))
        .await
        .expect("updated_at should update");

    let app = build_mcp_router(Arc::new(ToolRegistry::default()), graph, 2000);

    let response = app
        .oneshot(tool_call_request(json!({
            "name": "social.get_dialogue_summary",
            "input": {
                "user_id": "alice",
                "memory_hint": 0.2,
                "max_staleness_ms": -1,
                "allow_stale_fallback": false
            }
        })))
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let payload = read_json(response).await;
    assert_eq!(payload.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        payload
            .get("result")
            .and_then(|v| v.get("meta"))
            .and_then(|v| v.get("source"))
            .and_then(|v| v.as_str()),
        Some("tree_fresh")
    );
    assert_eq!(
        payload
            .get("result")
            .and_then(|v| v.get("meta"))
            .and_then(|v| v.get("stale"))
            .and_then(|v| v.as_bool()),
        Some(false)
    );
}

#[tokio::test]
async fn tools_call_endpoint_rejects_invalid_input_type() {
    let app = test_app().await;

    let response = app
        .oneshot(tool_call_request(json!({
            "name": "social.get_dialogue_summary",
            "input": "not-an-object"
        })))
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = read_json(response).await;
    assert_eq!(payload.get("ok").and_then(|v| v.as_bool()), Some(false));
    assert!(payload
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .contains("invalid type"));
}

#[tokio::test]
async fn tools_call_endpoint_rejects_extra_unknown_input_fields() {
    let app = test_app().await;

    let response = app
        .oneshot(tool_call_request(json!({
            "name": "social.get_dialogue_summary",
            "input": {
                "user_id": "alice",
                "unexpected": true
            }
        })))
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = read_json(response).await;
    assert_eq!(payload.get("ok").and_then(|v| v.as_bool()), Some(false));
    assert!(payload
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .contains("unknown field `unexpected`"));
}

#[tokio::test]
async fn tools_call_endpoint_rejects_non_string_name_field() {
    let app = test_app().await;

    let response = app
        .oneshot(tool_call_request(json!({
            "name": 123,
            "input": { "user_id": "alice" }
        })))
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = read_json(response).await;
    assert_eq!(payload.get("ok").and_then(|v| v.as_bool()), Some(false));
    assert!(payload
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .contains("invalid type"));
}

