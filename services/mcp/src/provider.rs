use std::sync::Arc;

use async_trait::async_trait;
use cognitive::DialogueToolRegistry;
use memory::graph::CognitiveGraph;
use serde_json::{json, Value};

use crate::registry::{ToolDescriptor, ToolNamespace};

#[derive(Debug, Clone)]
pub struct RegisteredTool {
    pub descriptor: ToolDescriptor,
    pub description: &'static str,
    pub input_schema: Value,
}

#[async_trait]
pub trait ToolProvider: Send + Sync {
    fn tools(&self) -> &[RegisteredTool];

    async fn execute(&self, name: &str, input: Value, graph: &CognitiveGraph) -> Option<anyhow::Result<Value>>;
}

#[derive(Debug, Clone)]
pub struct SocialToolProvider {
    registry: DialogueToolRegistry,
    tools: Vec<RegisteredTool>,
}

impl Default for SocialToolProvider {
    fn default() -> Self {
        Self {
            registry: DialogueToolRegistry::default(),
            tools: vec![
                RegisteredTool {
                    descriptor: ToolDescriptor {
                        namespace: ToolNamespace::Read,
                        name: "social.get_affect_context",
                        read_only: true,
                    },
                    description: "Read affect and relationship context for a user.",
                    input_schema: social_tool_input_schema(),
                },
                RegisteredTool {
                    descriptor: ToolDescriptor {
                        namespace: ToolNamespace::Read,
                        name: "social.get_dialogue_summary",
                        read_only: true,
                    },
                    description: "Read dialogue summary and trust/tension state for a user.",
                    input_schema: social_tool_input_schema(),
                },
            ],
        }
    }
}

#[async_trait]
impl ToolProvider for SocialToolProvider {
    fn tools(&self) -> &[RegisteredTool] {
        &self.tools
    }

    async fn execute(&self, name: &str, input: Value, graph: &CognitiveGraph) -> Option<anyhow::Result<Value>> {
        self.registry.get(name)?;
        Some(self.registry.execute(name, input, graph).await)
    }
}

fn social_tool_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "user_id": { "type": "string", "description": "User identifier to query." },
            "memory_hint": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
            "max_staleness_ms": { "type": "integer", "minimum": 0 },
            "allow_stale_fallback": { "type": "boolean" },
            "force_project": { "type": "boolean" }
        },
        "required": ["user_id"],
        "additionalProperties": false
    })
}

pub fn default_providers() -> Vec<Arc<dyn ToolProvider>> {
    vec![Arc::new(SocialToolProvider::default())]
}
