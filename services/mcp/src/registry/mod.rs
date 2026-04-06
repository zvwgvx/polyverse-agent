pub use cognitive::dialogue_tools::{ToolDescriptor, ToolNamespace};
use std::sync::Arc;

use anyhow::bail;
use memory::graph::CognitiveGraph;
use serde_json::Value;

use crate::provider::{default_providers, RegisteredTool, ToolProvider};

#[derive(Clone)]
pub struct ToolRegistry {
    providers: Vec<Arc<dyn ToolProvider>>,
    tools: Vec<ToolDescriptor>,
    registered_tools: Vec<RegisteredTool>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new(default_providers())
    }
}

impl ToolRegistry {
    pub fn new(providers: Vec<Arc<dyn ToolProvider>>) -> Self {
        let registered_tools: Vec<RegisteredTool> = providers
            .iter()
            .flat_map(|provider| provider.tools().iter().cloned())
            .collect();
        let tools = registered_tools
            .iter()
            .map(|tool| tool.descriptor.clone())
            .collect();

        Self {
            providers,
            tools,
            registered_tools,
        }
    }

    pub fn list(&self) -> &[ToolDescriptor] {
        &self.tools
    }

    pub fn list_registered(&self) -> &[RegisteredTool] {
        &self.registered_tools
    }

    pub fn get(&self, name: &str) -> Option<&ToolDescriptor> {
        self.tools.iter().find(|tool| tool.name == name)
    }

    pub fn get_registered(&self, name: &str) -> Option<&RegisteredTool> {
        self.registered_tools.iter().find(|tool| tool.descriptor.name == name)
    }

    pub async fn execute(
        &self,
        name: &str,
        input: Value,
        graph: &CognitiveGraph,
    ) -> anyhow::Result<Value> {
        for provider in &self.providers {
            if let Some(result) = provider.execute(name, input.clone(), graph).await {
                return result;
            }
        }

        bail!("unknown MCP tool: {name}")
    }
}
