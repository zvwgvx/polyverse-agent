pub use cognitive::dialogue_tools::{ToolDescriptor, ToolNamespace};
use cognitive::DialogueToolRegistry;
use memory::graph::CognitiveGraph;
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct ToolRegistry {
    inner: DialogueToolRegistry,
}

impl ToolRegistry {
    pub fn list(&self) -> &[ToolDescriptor] {
        self.inner.list()
    }

    pub fn get(&self, name: &str) -> Option<&ToolDescriptor> {
        self.inner.get(name)
    }

    pub async fn execute(
        &self,
        name: &str,
        input: Value,
        graph: &CognitiveGraph,
    ) -> anyhow::Result<Value> {
        self.inner.execute(name, input, graph).await
    }
}
