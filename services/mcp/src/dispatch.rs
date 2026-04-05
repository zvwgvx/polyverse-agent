use std::future::Future;
use std::sync::Arc;

use memory::graph::CognitiveGraph;
use serde::Deserialize;
use serde_json::Value;

use crate::registry::{ToolDescriptor, ToolRegistry};

pub type ToolCallFuture = std::pin::Pin<Box<dyn Future<Output = anyhow::Result<Value>> + Send>>;

#[derive(Clone)]
pub struct ToolCallExecutor {
    run: Arc<dyn Fn(String, Value, CognitiveGraph) -> ToolCallFuture + Send + Sync>,
}

impl ToolCallExecutor {
    pub fn from_registry(registry: Arc<ToolRegistry>) -> Self {
        Self {
            run: Arc::new(move |name, input, graph| {
                let registry = Arc::clone(&registry);
                Box::pin(async move { registry.execute(&name, input, &graph).await })
            }),
        }
    }

    pub fn new<F, Fut>(executor: F) -> Self
    where
        F: Fn(String, Value, CognitiveGraph) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = anyhow::Result<Value>> + Send + 'static,
    {
        Self {
            run: Arc::new(move |name, input, graph| Box::pin(executor(name, input, graph))),
        }
    }

    async fn execute(&self, name: String, input: Value, graph: CognitiveGraph) -> anyhow::Result<Value> {
        (self.run)(name, input, graph).await
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCallRequest {
    pub name: String,
    #[serde(default = "default_input_value")]
    pub input: Value,
}

pub fn default_input_value() -> Value {
    Value::Object(serde_json::Map::new())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCallFailureKind {
    BadRequest,
    Timeout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallFailure {
    pub kind: ToolCallFailureKind,
    pub message: String,
}

impl ToolCallFailure {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            kind: ToolCallFailureKind::BadRequest,
            message: message.into(),
        }
    }

    fn timeout(message: impl Into<String>) -> Self {
        Self {
            kind: ToolCallFailureKind::Timeout,
            message: message.into(),
        }
    }
}

#[derive(Clone)]
pub struct McpDispatcher {
    registry: Arc<ToolRegistry>,
    graph: CognitiveGraph,
    request_timeout_ms: u64,
    executor: ToolCallExecutor,
}

impl McpDispatcher {
    pub fn new(registry: Arc<ToolRegistry>, graph: CognitiveGraph, request_timeout_ms: u64) -> Self {
        let executor = ToolCallExecutor::from_registry(Arc::clone(&registry));
        Self::with_executor(registry, graph, request_timeout_ms, executor)
    }

    pub fn with_executor(
        registry: Arc<ToolRegistry>,
        graph: CognitiveGraph,
        request_timeout_ms: u64,
        executor: ToolCallExecutor,
    ) -> Self {
        Self {
            registry,
            graph,
            request_timeout_ms,
            executor,
        }
    }

    pub fn list_tools(&self) -> Vec<ToolDescriptor> {
        self.registry.list().to_vec()
    }

    pub fn list_registered_tools(&self) -> Vec<crate::provider::RegisteredTool> {
        self.registry.list_registered().to_vec()
    }

    pub fn get_registered_tool(&self, name: &str) -> Option<crate::provider::RegisteredTool> {
        self.registry.get_registered(name).cloned()
    }

    pub fn request_timeout_ms(&self) -> u64 {
        self.request_timeout_ms
    }

    pub fn supports_tools(&self) -> bool {
        !self.registry.list().is_empty()
    }

    pub fn supports_resources(&self) -> bool {
        false
    }

    pub fn supports_prompts(&self) -> bool {
        false
    }

    pub fn supports_logging(&self) -> bool {
        false
    }

    pub fn supports_streaming(&self) -> bool {
        false
    }

    pub fn server_name(&self) -> &'static str {
        "polyverse-agent-mcp"
    }

    pub async fn dispatch(&self, req: ToolCallRequest) -> Result<Value, ToolCallFailure> {
        let timeout = std::time::Duration::from_millis(self.request_timeout_ms.max(50));
        let name = req.name.trim().to_string();

        if name.is_empty() {
            return Err(ToolCallFailure::bad_request("tool name is required"));
        }

        if !req.input.is_object() {
            return Err(ToolCallFailure::bad_request(
                "invalid type: expected a JSON object for input",
            ));
        }

        let input = req.input;

        if let Some(object) = input.as_object() {
            if let Some(extra) = object.keys().find(|key| {
                !matches!(
                    key.as_str(),
                    "user_id" | "memory_hint" | "max_staleness_ms" | "allow_stale_fallback" | "force_project"
                )
            }) {
                return Err(ToolCallFailure::bad_request(format!("unknown field `{}`", extra)));
            }
        }

        let graph = self.graph.clone();
        let fut = self.executor.execute(name, input, graph);
        match tokio::time::timeout(timeout, fut).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(err)) => Err(ToolCallFailure::bad_request(err.to_string())),
            Err(_) => Err(ToolCallFailure::timeout("tool call timeout")),
        }
    }
}
