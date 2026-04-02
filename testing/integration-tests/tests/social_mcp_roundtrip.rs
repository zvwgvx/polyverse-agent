use memory::graph::SocialDelta;
use test_support::{
    build_test_mcp_router, in_memory_graph, read_json, seeded_social_graph, set_tree_updated_at,
    stale_timestamp, tool_call_request, tools_list_request,
};
use tower::util::ServiceExt;

#[tokio::test]
async fn mcp_router_roundtrip_returns_tree_backed_dialogue_summary() {
    let graph = seeded_social_graph("alice").await;
    let app = build_test_mcp_router(graph);

    let tools = app
        .clone()
        .oneshot(tools_list_request())
        .await
        .expect("tools request should succeed");
    let tools_payload = read_json(tools).await;
    let tools_array = tools_payload.as_array().expect("tools payload should be array");
    assert_eq!(tools_array.len(), 2);

    let response = app
        .oneshot(tool_call_request(
            "social.get_dialogue_summary",
            serde_json::json!({
                "user_id": "alice",
                "memory_hint": 0.2
            }),
        ))
        .await
        .expect("tool call should succeed");

    let payload = read_json(response).await;
    assert_eq!(payload.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        payload
            .get("result")
            .and_then(|v| v.get("user_id"))
            .and_then(|v| v.as_str()),
        Some("alice")
    );
    assert_eq!(
        payload
            .get("result")
            .and_then(|v| v.get("meta"))
            .and_then(|v| v.get("source"))
            .and_then(|v| v.as_str()),
        Some("tree_fresh")
    );
}

#[tokio::test]
async fn mcp_router_roundtrip_returns_tree_backed_affect_context() {
    let graph = seeded_social_graph("alice").await;
    let app = build_test_mcp_router(graph);

    let response = app
        .oneshot(tool_call_request(
            "social.get_affect_context",
            serde_json::json!({
                "user_id": "alice",
                "memory_hint": 0.2
            }),
        ))
        .await
        .expect("tool call should succeed");

    let payload = read_json(response).await;
    assert_eq!(payload.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        payload
            .get("result")
            .and_then(|v| v.get("known"))
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        payload
            .get("result")
            .and_then(|v| v.get("meta"))
            .and_then(|v| v.get("source"))
            .and_then(|v| v.as_str()),
        Some("tree_fresh")
    );
}

#[tokio::test]
async fn mcp_router_reprojects_stale_tree_when_stale_fallback_is_disabled() {
    let graph = seeded_social_graph("alice").await;
    set_tree_updated_at(&graph, "alice", &stale_timestamp(2)).await;
    let app = build_test_mcp_router(graph);

    let response = app
        .oneshot(tool_call_request(
            "social.get_dialogue_summary",
            serde_json::json!({
                "user_id": "alice",
                "memory_hint": 0.2,
                "max_staleness_ms": 1,
                "allow_stale_fallback": false
            }),
        ))
        .await
        .expect("tool call should succeed");

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
async fn mcp_router_projects_graph_data_into_a_fresh_tree_on_first_read() {
    let graph = in_memory_graph().await;
    graph
        .update_social_graph(
            "alice",
            SocialDelta {
                delta_trust: 0.30,
                delta_tension: 0.26,
                ..Default::default()
            },
        )
        .await
        .expect("social graph should update");
    let app = build_test_mcp_router(graph);

    let response = app
        .oneshot(tool_call_request(
            "social.get_dialogue_summary",
            serde_json::json!({
                "user_id": "alice",
                "memory_hint": 0.0,
                "max_staleness_ms": 1,
                "allow_stale_fallback": false
            }),
        ))
        .await
        .expect("tool call should succeed");

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
            .and_then(|v| v.get("trust_state"))
            .and_then(|v| v.as_str()),
        Some("neutral")
    );
    assert_eq!(
        payload
            .get("result")
            .and_then(|v| v.get("tension_state"))
            .and_then(|v| v.as_str()),
        Some("medium")
    );
}

#[tokio::test]
async fn mcp_router_refreshes_stale_tree_before_returning_result() {
    let graph = seeded_social_graph("alice").await;
    set_tree_updated_at(&graph, "alice", &stale_timestamp(2)).await;
    let app = build_test_mcp_router(graph);

    let response = app
        .oneshot(tool_call_request(
            "social.get_affect_context",
            serde_json::json!({
                "user_id": "alice",
                "memory_hint": 0.2,
                "max_staleness_ms": 1,
                "allow_stale_fallback": true,
                "force_project": false
            }),
        ))
        .await
        .expect("tool call should succeed");

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
