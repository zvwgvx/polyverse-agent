use std::sync::Arc;

use anyhow::anyhow;
use mcp::{
    build_mcp_router, build_mcp_router_for_tests, registry::ToolRegistry, SearchProviderConfig,
    SearchToolProvider, WebFetchProviderConfig, WebFetchToolProvider,
};
use serde_json::{json, Value};
use test_support::{in_memory_graph, read_json, tool_call_request, tools_list_request};
use tower::util::ServiceExt;

fn web_tools_registry() -> ToolRegistry {
    ToolRegistry::new(vec![
        Arc::new(SearchToolProvider::new(SearchProviderConfig {
            enabled: true,
            api_key: Some("test-key".to_string()),
            timeout_ms: 2_000,
            max_results: 5,
            brave_api_base: "https://api.search.brave.com/res/v1/web/search".to_string(),
        })),
        Arc::new(WebFetchToolProvider::new(WebFetchProviderConfig {
            enabled: true,
            timeout_ms: 2_000,
            max_bytes: 1_000_000,
            max_chars: 20_000,
            max_redirects: 3,
            max_key_links: 8,
        })),
    ])
}

async fn real_web_tools_app() -> axum::Router {
    let graph = in_memory_graph().await;
    build_mcp_router(Arc::new(web_tools_registry()), graph, 2_000)
}

async fn stubbed_web_tools_app() -> axum::Router {
    let graph = in_memory_graph().await;

    build_mcp_router_for_tests(
        Arc::new(web_tools_registry()),
        graph,
        2_000,
        |name, input, _graph| async move {
            match name.as_str() {
                "web.fetch" => {
                    let url = input
                        .get("url")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    Ok(json!({
                        "url": url,
                        "final_url": url,
                        "status": 200,
                        "title": "stub",
                        "content_markdown": "stub content",
                        "key_links": [],
                        "meta": {
                            "source": "web_fetch",
                            "engine": "stub",
                            "cached": false,
                            "response_ms": 0,
                            "bytes": 12,
                            "content_type": "text/plain",
                            "redirect_count": 0,
                            "truncated": false,
                            "max_chars": 20000,
                            "instruction": null
                        }
                    }))
                }
                "search.web" => {
                    let query = input
                        .get("query")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    Ok(json!({
                        "query": query,
                        "engine": "brave",
                        "results": [
                            {
                                "title": "Result 1",
                                "url": "https://example.com/a",
                                "snippet": "a"
                            }
                        ],
                        "meta": {
                            "source": "brave_search",
                            "cached": false,
                            "response_ms": 0
                        }
                    }))
                }
                _ => Err(anyhow!("unknown MCP tool: {name}")),
            }
        },
    )
}

async fn assert_bad_request_contains(app: axum::Router, name: &str, input: Value, needle: &str) {
    let response = app
        .oneshot(tool_call_request(name, input))
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let payload = read_json(response).await;
    assert_eq!(payload.get("ok").and_then(|v| v.as_bool()), Some(false));
    assert!(payload
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .contains(needle));
}

#[tokio::test]
async fn mcp_router_lists_registered_web_tools() {
    let app = real_web_tools_app().await;

    let response = app
        .oneshot(tools_list_request())
        .await
        .expect("tools request should succeed");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let payload = read_json(response).await;
    let arr = payload.as_array().expect("tools payload should be array");
    assert_eq!(arr.len(), 2);

    let names: Vec<String> = arr
        .iter()
        .filter_map(|item| item.get("name").and_then(|v| v.as_str()).map(|v| v.to_string()))
        .collect();
    assert!(names.contains(&"search.web".to_string()));
    assert!(names.contains(&"web.fetch".to_string()));
}

#[tokio::test]
async fn mcp_router_roundtrip_web_tools_with_stubbed_executor() {
    let app = stubbed_web_tools_app().await;

    let search_response = app
        .clone()
        .oneshot(tool_call_request(
            "search.web",
            json!({
                "query": "rust"
            }),
        ))
        .await
        .expect("search request should succeed");
    assert_eq!(search_response.status(), axum::http::StatusCode::OK);
    let search_payload = read_json(search_response).await;
    assert_eq!(search_payload.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        search_payload
            .get("result")
            .and_then(|v| v.get("query"))
            .and_then(|v| v.as_str()),
        Some("rust")
    );

    let fetch_response = app
        .oneshot(tool_call_request(
            "web.fetch",
            json!({
                "url": "https://example.com"
            }),
        ))
        .await
        .expect("fetch request should succeed");
    assert_eq!(fetch_response.status(), axum::http::StatusCode::OK);
    let fetch_payload = read_json(fetch_response).await;
    assert_eq!(fetch_payload.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        fetch_payload
            .get("result")
            .and_then(|v| v.get("status"))
            .and_then(|v| v.as_i64()),
        Some(200)
    );
    assert!(fetch_payload
        .get("result")
        .and_then(|v| v.get("content_markdown"))
        .and_then(|v| v.as_str())
        .is_some());
}

#[tokio::test]
async fn mcp_router_rejects_web_tools_invalid_inputs_before_network_call() {
    let app = real_web_tools_app().await;

    assert_bad_request_contains(app.clone(), "search.web", json!({}), "missing field `query`").await;
    assert_bad_request_contains(
        app.clone(),
        "search.web",
        json!({"query":"   "}),
        "query is required",
    )
    .await;
    assert_bad_request_contains(
        app.clone(),
        "search.web",
        json!({"query":"rust", "safesearch":"invalid"}),
        "safesearch must be one of: off, moderate, strict",
    )
    .await;

    assert_bad_request_contains(app.clone(), "web.fetch", json!({}), "missing field `url`").await;
    assert_bad_request_contains(
        app.clone(),
        "web.fetch",
        json!({"url":"file:///etc/passwd"}),
        "url scheme must be http or https",
    )
    .await;
    assert_bad_request_contains(
        app,
        "web.fetch",
        json!({"url":"http://localhost"}),
        "url host is not allowed",
    )
    .await;
}
