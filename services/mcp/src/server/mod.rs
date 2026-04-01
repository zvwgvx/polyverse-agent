use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use axum::extract::{rejection::JsonRejection, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use kernel::worker::{Worker, WorkerContext, WorkerStatus};
use memory::graph::CognitiveGraph;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tracing::{info, warn};

use crate::config::McpConfig;
use crate::registry::ToolRegistry;

#[derive(Clone)]
struct AppState {
    registry: Arc<ToolRegistry>,
    graph: CognitiveGraph,
    request_timeout_ms: u64,
    executor: ToolCallExecutor,
}

#[derive(Clone)]
struct ToolCallExecutor {
    run: Arc<dyn Fn(String, Value, CognitiveGraph) -> ToolCallFuture + Send + Sync>,
}

type ToolCallFuture = std::pin::Pin<Box<dyn Future<Output = anyhow::Result<Value>> + Send>>;

impl ToolCallExecutor {
    fn from_registry(registry: Arc<ToolRegistry>) -> Self {
        Self {
            run: Arc::new(move |name, input, graph| {
                let registry = Arc::clone(&registry);
                Box::pin(async move { registry.execute(&name, input, &graph).await })
            }),
        }
    }

    async fn execute(&self, name: String, input: Value, graph: CognitiveGraph) -> anyhow::Result<Value> {
        (self.run)(name, input, graph).await
    }
}

fn build_mcp_router_with_executor(
    registry: Arc<ToolRegistry>,
    graph: CognitiveGraph,
    request_timeout_ms: u64,
    executor: ToolCallExecutor,
) -> Router {
    let app_state = AppState {
        registry,
        graph,
        request_timeout_ms,
        executor,
    };

    Router::new()
        .route("/api/mcp/tools", get(get_tools))
        .route("/api/mcp/tools/call", post(post_tool_call))
        .with_state(app_state)
}

#[cfg(test)]
pub(crate) fn build_mcp_router_for_tests<F, Fut>(
    registry: Arc<ToolRegistry>,
    graph: CognitiveGraph,
    request_timeout_ms: u64,
    executor: F,
) -> Router
where
    F: Fn(String, Value, CognitiveGraph) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<Value>> + Send + 'static,
{
    let executor = ToolCallExecutor {
        run: Arc::new(move |name, input, graph| Box::pin(executor(name, input, graph))),
    };

    build_mcp_router_with_executor(registry, graph, request_timeout_ms, executor)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolCallRequest {
    name: String,
    #[serde(default = "default_input_value")]
    input: Value,
}

fn default_input_value() -> Value {
    Value::Object(serde_json::Map::new())
}

#[derive(Debug, Serialize)]
struct ToolCallSuccess {
    ok: bool,
    result: Value,
}

#[derive(Debug, Serialize)]
struct ToolCallError {
    ok: bool,
    error: String,
}

pub struct McpWorker {
    config: McpConfig,
    graph: CognitiveGraph,
    registry: ToolRegistry,
    status: WorkerStatus,
}

impl McpWorker {
    pub fn new(config: McpConfig, graph: CognitiveGraph) -> Self {
        Self {
            config,
            graph,
            registry: ToolRegistry::default(),
            status: WorkerStatus::NotStarted,
        }
    }

    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    pub fn graph(&self) -> &CognitiveGraph {
        &self.graph
    }
}

pub fn build_mcp_router(
    registry: Arc<ToolRegistry>,
    graph: CognitiveGraph,
    request_timeout_ms: u64,
) -> Router {
    let executor = ToolCallExecutor::from_registry(Arc::clone(&registry));
    build_mcp_router_with_executor(registry, graph, request_timeout_ms, executor)
}

#[async_trait]
impl Worker for McpWorker {
    fn name(&self) -> &str {
        "mcp"
    }

    async fn start(&mut self, ctx: WorkerContext) -> Result<()> {
        if !self.config.enabled {
            self.status = WorkerStatus::Stopped;
            info!("MCP worker is disabled by config");
            return Ok(());
        }

        let bind_addr: SocketAddr = self
            .config
            .bind_addr
            .parse()
            .with_context(|| format!("invalid MCP_BIND address: {}", self.config.bind_addr))?;

        let app = build_mcp_router(
            Arc::new(self.registry.clone()),
            self.graph.clone(),
            self.config.request_timeout_ms,
        );

        let listener = TcpListener::bind(bind_addr)
            .await
            .with_context(|| format!("failed to bind MCP API on {}", bind_addr))?;

        let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
        let server_handle = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = server_shutdown_rx.await;
                })
                .await
            {
                warn!(error = %e, "MCP API server exited with error");
            }
        });

        info!(
            bind = %bind_addr,
            max_tool_calls_per_turn = self.config.max_tool_calls_per_turn,
            request_timeout_ms = self.config.request_timeout_ms,
            tools = self.registry.list().len(),
            "MCP worker started"
        );

        self.status = WorkerStatus::Healthy;

        let mut shutdown_rx = ctx.subscribe_shutdown();
        let _ = shutdown_rx.recv().await;

        let _ = server_shutdown_tx.send(());
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), server_handle).await;

        self.status = WorkerStatus::Stopped;
        info!("MCP worker stopped");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.status = WorkerStatus::Stopped;
        Ok(())
    }

    fn health_check(&self) -> WorkerStatus {
        self.status.clone()
    }
}

async fn get_tools(State(state): State<AppState>) -> Json<Vec<crate::registry::ToolDescriptor>> {
    Json(state.registry.list().to_vec())
}

fn bad_request_json(error: String) -> (axum::http::StatusCode, Json<Value>) {
    let body = serde_json::to_value(ToolCallError {
        ok: false,
        error,
    })
    .unwrap_or_else(|_| serde_json::json!({"ok": false, "error": "serialization_error"}));
    (axum::http::StatusCode::BAD_REQUEST, Json(body))
}

fn normalize_json_rejection_message(message: String) -> String {
    let stripped = message
        .strip_prefix("Failed to deserialize the JSON body into the target type: ")
        .or_else(|| message.strip_prefix("Failed to parse the request body as JSON: "))
        .unwrap_or(&message);
    stripped.split(" at line ").next().unwrap_or(stripped).to_string()
}

async fn post_tool_call(
    State(state): State<AppState>,
    req: Result<Json<ToolCallRequest>, JsonRejection>,
) -> (axum::http::StatusCode, Json<Value>) {
    let req = match req {
        Ok(Json(req)) => req,
        Err(err) => return bad_request_json(normalize_json_rejection_message(err.body_text())),
    };
    let timeout = std::time::Duration::from_millis(state.request_timeout_ms.max(50));
    let name = req.name.trim().to_string();

    if name.is_empty() {
        return bad_request_json("tool name is required".to_string());
    }

    if !req.input.is_object() {
        return bad_request_json("invalid type: expected a JSON object for input".to_string());
    }

    let input = req.input;

    if let Some(object) = input.as_object() {
        if let Some(extra) = object.keys().find(|key| {
            !matches!(
                key.as_str(),
                "user_id" | "memory_hint" | "max_staleness_ms" | "allow_stale_fallback" | "force_project"
            )
        }) {
            return bad_request_json(format!("unknown field `{}`", extra));
        }
    }


    let graph = state.graph.clone();
    let fut = state.executor.execute(name, input, graph);
    match tokio::time::timeout(timeout, fut).await {
        Ok(Ok(result)) => {
            let body = serde_json::to_value(ToolCallSuccess { ok: true, result })
                .unwrap_or_else(|_| serde_json::json!({"ok": false, "error": "serialization_error"}));
            (axum::http::StatusCode::OK, Json(body))
        }
        Ok(Err(err)) => {
            let body = serde_json::to_value(ToolCallError {
                ok: false,
                error: err.to_string(),
            })
            .unwrap_or_else(|_| serde_json::json!({"ok": false, "error": "serialization_error"}));
            (axum::http::StatusCode::BAD_REQUEST, Json(body))
        }
        Err(_) => {
            let body = serde_json::to_value(ToolCallError {
                ok: false,
                error: "tool call timeout".to_string(),
            })
            .unwrap_or_else(|_| serde_json::json!({"ok": false, "error": "serialization_error"}));
            (axum::http::StatusCode::REQUEST_TIMEOUT, Json(body))
        }
    }
}
