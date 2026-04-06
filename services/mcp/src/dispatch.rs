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

fn validate_input_against_schema(input: &Value, input_schema: &Value) -> Result<(), ToolCallFailure> {
    if !input.is_object() {
        return Err(ToolCallFailure::bad_request(
            "invalid type: expected a JSON object for input",
        ));
    }

    let disallow_additional = input_schema
        .get("additionalProperties")
        .and_then(|v| v.as_bool())
        == Some(false);

    if !disallow_additional {
        return Ok(());
    }

    let Some(allowed_properties) = input_schema.get("properties").and_then(|v| v.as_object()) else {
        return Ok(());
    };

    let Some(input_object) = input.as_object() else {
        return Ok(());
    };

    if let Some(extra) = input_object
        .keys()
        .find(|key| !allowed_properties.contains_key(key.as_str()))
    {
        return Err(ToolCallFailure::bad_request(format!("unknown field `{}`", extra)));
    }

    Ok(())
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

    pub fn supports_execution_tools(&self) -> bool {
        self.registry
            .list()
            .iter()
            .any(|tool| tool.name.starts_with("execution."))
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

        let Some(registered_tool) = self.registry.get_registered(&name) else {
            return Err(ToolCallFailure::bad_request(format!("unknown MCP tool: {name}")));
        };

        validate_input_against_schema(&req.input, &registered_tool.input_schema)?;

        let graph = self.graph.clone();
        let fut = self.executor.execute(name, req.input, graph);
        match tokio::time::timeout(timeout, fut).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(err)) => Err(ToolCallFailure::bad_request(err.to_string())),
            Err(_) => Err(ToolCallFailure::timeout("tool call timeout")),
        }
    }
}
