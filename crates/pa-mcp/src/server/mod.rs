use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use pa_core::worker::{Worker, WorkerContext, WorkerStatus};
use pa_memory::graph::CognitiveGraph;
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
}

#[derive(Debug, Deserialize)]
struct ToolCallRequest {
    name: String,
    #[serde(default)]
    input: Value,
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
    let app_state = AppState {
        registry,
        graph,
        request_timeout_ms,
    };

    Router::new()
        .route("/api/mcp/tools", get(get_tools))
        .route("/api/mcp/tools/call", post(post_tool_call))
        .with_state(app_state)
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

async fn post_tool_call(
    State(state): State<AppState>,
    Json(req): Json<ToolCallRequest>,
) -> (axum::http::StatusCode, Json<Value>) {
    let timeout = std::time::Duration::from_millis(state.request_timeout_ms.max(50));
    let name = req.name.trim().to_string();

    if name.is_empty() {
        let body = serde_json::to_value(ToolCallError {
            ok: false,
            error: "tool name is required".to_string(),
        })
        .unwrap_or_else(|_| serde_json::json!({"ok": false, "error": "serialization_error"}));
        return (axum::http::StatusCode::BAD_REQUEST, Json(body));
    }

    let registry = Arc::clone(&state.registry);
    let graph = state.graph.clone();
    let fut = async move { registry.execute(&name, req.input, &graph).await };
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
