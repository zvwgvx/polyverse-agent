use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use pa_memory::graph::CognitiveGraph;
use pa_mcp::{build_mcp_router, registry::ToolRegistry};
use serde_json::{json, Value};
use tower::util::ServiceExt;

async fn test_app() -> axum::Router {
    let graph = CognitiveGraph::new("memory")
        .await
        .expect("in-memory graph should initialize");

    build_mcp_router(Arc::new(ToolRegistry::default()), graph, 2000)
}

#[tokio::test]
async fn tools_endpoint_lists_two_read_tools() {
    let app = test_app().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/mcp/tools")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("json body expected");

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
        .oneshot(
            Request::builder()
                .uri("/api/mcp/tools/call")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "social.get_dialogue_summary",
                        "input": { "user_id": "alice", "memory_hint": 0.2 }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .expect("request should succeed");

    assert_eq!(ok_resp.status(), StatusCode::OK);
    let ok_body = axum::body::to_bytes(ok_resp.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let ok_payload: Value = serde_json::from_slice(&ok_body).expect("json body expected");
    assert_eq!(ok_payload.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert!(ok_payload
        .get("result")
        .and_then(|v| v.get("meta"))
        .is_some());

    let err_resp = app
        .oneshot(
            Request::builder()
                .uri("/api/mcp/tools/call")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "social.unknown",
                        "input": {}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .expect("request should succeed");

    assert_eq!(err_resp.status(), StatusCode::BAD_REQUEST);
    let err_body = axum::body::to_bytes(err_resp.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let err_payload: Value = serde_json::from_slice(&err_body).expect("json body expected");
    assert_eq!(err_payload.get("ok").and_then(|v| v.as_bool()), Some(false));
    let msg = err_payload
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert!(msg.contains("unknown MCP tool"));
}

#[tokio::test]
async fn tools_call_endpoint_rejects_empty_tool_name() {
    let app = test_app().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/mcp/tools/call")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "   ",
                        "input": {}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("json body expected");
    assert_eq!(payload.get("ok").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(
        payload.get("error").and_then(|v| v.as_str()),
        Some("tool name is required")
    );
}
