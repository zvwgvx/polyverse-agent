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
use serde::Serialize;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tracing::{info, warn};

use crate::config::{McpConfig, McpTransport};
use crate::dispatch::{McpDispatcher, ToolCallExecutor, ToolCallFailureKind, ToolCallRequest};
use crate::registry::ToolRegistry;
use crate::stdio::serve_process_stdio;

#[derive(Clone)]
struct AppState {
    dispatcher: McpDispatcher,
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

fn build_mcp_router_with_dispatcher(dispatcher: McpDispatcher) -> Router {
    let app_state = AppState { dispatcher };

    Router::new()
        .route("/api/mcp/tools", get(get_tools))
        .route("/api/mcp/tools/call", post(post_tool_call))
        .with_state(app_state)
}

pub fn build_mcp_router_for_tests<F, Fut>(
    registry: Arc<ToolRegistry>,
    graph: CognitiveGraph,
    request_timeout_ms: u64,
    executor: F,
) -> Router
where
    F: Fn(String, Value, CognitiveGraph) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<Value>> + Send + 'static,
{
    let executor = ToolCallExecutor::new(executor);
    let dispatcher = McpDispatcher::with_executor(registry, graph, request_timeout_ms, executor);
    build_mcp_router_with_dispatcher(dispatcher)
}

pub fn build_mcp_dispatcher(
    registry: Arc<ToolRegistry>,
    graph: CognitiveGraph,
    request_timeout_ms: u64,
) -> McpDispatcher {
    McpDispatcher::new(registry, graph, request_timeout_ms)
}

pub fn build_mcp_router(
    registry: Arc<ToolRegistry>,
    graph: CognitiveGraph,
    request_timeout_ms: u64,
) -> Router {
    let dispatcher = build_mcp_dispatcher(registry, graph, request_timeout_ms);
    build_mcp_router_with_dispatcher(dispatcher)
}

pub fn build_mcp_router_from_dispatcher(dispatcher: McpDispatcher) -> Router {
    build_mcp_router_with_dispatcher(dispatcher)
}

fn normalize_json_rejection_message(message: String) -> String {
    let stripped = message
        .strip_prefix("Failed to deserialize the JSON body into the target type: ")
        .or_else(|| message.strip_prefix("Failed to parse the request body as JSON: "))
        .unwrap_or(&message);
    stripped.split(" at line ").next().unwrap_or(stripped).to_string()
}

fn error_json(status: axum::http::StatusCode, error: String) -> (axum::http::StatusCode, Json<Value>) {
    let body = serde_json::to_value(ToolCallError {
        ok: false,
        error,
    })
    .unwrap_or_else(|_| serde_json::json!({"ok": false, "error": "serialization_error"}));
    (status, Json(body))
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

        let dispatcher = build_mcp_dispatcher(
            Arc::new(self.registry.clone()),
            self.graph.clone(),
            self.config.request_timeout_ms,
        );

        match self.config.transport {
            McpTransport::Http => {
                let bind_addr: SocketAddr = self
                    .config
                    .bind_addr
                    .parse()
                    .with_context(|| format!("invalid MCP_BIND address: {}", self.config.bind_addr))?;

                let app = build_mcp_router_from_dispatcher(dispatcher);
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
                    transport = "http",
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
            }
            McpTransport::Stdio => {
                info!(
                    transport = "stdio",
                    max_tool_calls_per_turn = self.config.max_tool_calls_per_turn,
                    request_timeout_ms = self.config.request_timeout_ms,
                    tools = self.registry.list().len(),
                    "MCP worker started"
                );
                self.status = WorkerStatus::Healthy;
                serve_process_stdio(dispatcher).await?;
            }
        }

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
    Json(state.dispatcher.list_tools())
}

async fn post_tool_call(
    State(state): State<AppState>,
    req: Result<Json<ToolCallRequest>, JsonRejection>,
) -> (axum::http::StatusCode, Json<Value>) {
    let req = match req {
        Ok(Json(req)) => req,
        Err(err) => {
            return error_json(
                axum::http::StatusCode::BAD_REQUEST,
                normalize_json_rejection_message(err.body_text()),
            )
        }
    };

    match state.dispatcher.dispatch(req).await {
        Ok(result) => {
            let body = serde_json::to_value(ToolCallSuccess { ok: true, result })
                .unwrap_or_else(|_| serde_json::json!({"ok": false, "error": "serialization_error"}));
            (axum::http::StatusCode::OK, Json(body))
        }
        Err(err) => match err.kind {
            ToolCallFailureKind::BadRequest => error_json(axum::http::StatusCode::BAD_REQUEST, err.message),
            ToolCallFailureKind::Timeout => error_json(axum::http::StatusCode::REQUEST_TIMEOUT, err.message),
        },
    }
}
