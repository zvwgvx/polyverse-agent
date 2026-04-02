use anyhow::bail;
use memory::graph::CognitiveGraph;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug, warn};

use crate::social_context::{
    query_social_context, SocialQueryIntent, SocialQueryOptions, SocialQueryResult,
    SocialQuerySource,
};

pub const SOCIAL_GET_AFFECT_CONTEXT_TOOL: &str = "social.get_affect_context";
pub const SOCIAL_GET_DIALOGUE_SUMMARY_TOOL: &str = "social.get_dialogue_summary";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolNamespace {
    Read,
    Action,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolDescriptor {
    pub namespace: ToolNamespace,
    pub name: &'static str,
    pub read_only: bool,
}

#[derive(Debug, Clone)]
pub struct DialogueToolRegistry {
    tools: Vec<ToolDescriptor>,
}

impl Default for DialogueToolRegistry {
    fn default() -> Self {
        Self {
            tools: vec![
                ToolDescriptor {
                    namespace: ToolNamespace::Read,
                    name: SOCIAL_GET_AFFECT_CONTEXT_TOOL,
                    read_only: true,
                },
                ToolDescriptor {
                    namespace: ToolNamespace::Read,
                    name: SOCIAL_GET_DIALOGUE_SUMMARY_TOOL,
                    read_only: true,
                },
            ],
        }
    }
}

impl DialogueToolRegistry {
    pub fn list(&self) -> &[ToolDescriptor] {
        &self.tools
    }

    pub fn get(&self, name: &str) -> Option<&ToolDescriptor> {
        self.tools.iter().find(|tool| tool.name == name)
    }

    pub async fn execute(
        &self,
        name: &str,
        input: Value,
        graph: &CognitiveGraph,
    ) -> anyhow::Result<Value> {
        let Some(tool) = self.get(name) else {
            bail!("unknown MCP tool: {name}");
        };

        if tool.namespace != ToolNamespace::Read {
            bail!("tool is not callable in read-only MCP mode: {name}");
        }

        match name {
            SOCIAL_GET_AFFECT_CONTEXT_TOOL => execute_social_get_affect_context(input, graph).await,
            SOCIAL_GET_DIALOGUE_SUMMARY_TOOL => {
                execute_social_get_dialogue_summary(input, graph).await
            }
            _ => bail!("unsupported MCP tool handler: {name}"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct DialogueToolInput {
    user_id: String,
    #[serde(default)]
    memory_hint: Option<f32>,
    #[serde(default)]
    max_staleness_ms: Option<i64>,
    #[serde(default)]
    allow_stale_fallback: Option<bool>,
    #[serde(default)]
    force_project: Option<bool>,
}

#[derive(Debug, Serialize)]
struct AffectToolOutput {
    user_id: String,
    known: bool,
    metrics: AffectMetrics,
    illusion: AffectIllusion,
    meta: ToolMeta,
}

#[derive(Debug, Serialize)]
struct AffectMetrics {
    affinity: f32,
    attachment: f32,
    trust: f32,
    safety: f32,
    tension: f32,
    context_depth: f32,
}

#[derive(Debug, Serialize)]
struct AffectIllusion {
    affinity: f32,
    attachment: f32,
    trust: f32,
    safety: f32,
    tension: f32,
}

#[derive(Debug, Serialize)]
struct DialogueSummaryToolOutput {
    user_id: String,
    familiarity: Option<String>,
    trust_state: Option<String>,
    tension_state: Option<String>,
    summary: Option<String>,
    meta: ToolMeta,
}

#[derive(Debug, Serialize)]
struct ToolMeta {
    source: &'static str,
    stale: bool,
    schema_version: Option<String>,
    updated_at: Option<String>,
}

fn clamp_memory_hint(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

fn sanitize_staleness(value: Option<i64>) -> Option<i64> {
    value.map(|v| v.max(0))
}

fn source_to_str(source: SocialQuerySource) -> &'static str {
    match source {
        SocialQuerySource::TreeFresh => "tree_fresh",
        SocialQuerySource::TreeStale => "tree_stale",
        SocialQuerySource::GraphFallback => "graph_fallback",
        SocialQuerySource::DefaultFallback => "default_fallback",
    }
}

fn log_tool_call_start(tool: &str, user_id: &str) {
    debug!(
        kind = "mcp.tool",
        phase = "start",
        tool = tool,
        user = user_id,
        "MCP read tool call started"
    );
}

fn log_tool_call_success(tool: &str, user_id: &str) {
    debug!(
        kind = "mcp.tool",
        phase = "success",
        tool = tool,
        user = user_id,
        "MCP read tool call succeeded"
    );
}

fn log_tool_call_failure(tool: &str, user_id: &str, error: &str) {
    warn!(
        kind = "mcp.tool",
        phase = "failure",
        tool = tool,
        user = user_id,
        error = error,
        "MCP read tool call failed"
    );
}

async fn execute_social_get_affect_context(
    input: Value,
    graph: &CognitiveGraph,
) -> anyhow::Result<Value> {
    let input: DialogueToolInput = serde_json::from_value(input)?;
    let user_id = input.user_id.trim().to_string();
    if user_id.is_empty() {
        bail!("user_id is required");
    }

    let memory_hint = clamp_memory_hint(input.memory_hint.unwrap_or(0.0));
    let options = SocialQueryOptions::for_affect(memory_hint)
        .with_max_staleness_ms(sanitize_staleness(input.max_staleness_ms))
        .with_allow_stale_fallback(input.allow_stale_fallback.unwrap_or(true))
        .with_force_project(input.force_project.unwrap_or(false));

    log_tool_call_start(SOCIAL_GET_AFFECT_CONTEXT_TOOL, &user_id);

    let result = query_social_context(graph, SocialQueryIntent::AffectRich, &user_id, options).await;

    let SocialQueryResult::Affect { context, meta } = result else {
        let err = "unexpected query result for affect context";
        log_tool_call_failure(SOCIAL_GET_AFFECT_CONTEXT_TOOL, &user_id, err);
        bail!(err);
    };

    let output = AffectToolOutput {
        user_id: context.user_id,
        known: context.known,
        metrics: AffectMetrics {
            affinity: context.metrics.affinity,
            attachment: context.metrics.attachment,
            trust: context.metrics.trust,
            safety: context.metrics.safety,
            tension: context.metrics.tension,
            context_depth: context.metrics.context_depth,
        },
        illusion: AffectIllusion {
            affinity: context.illusion.affinity,
            attachment: context.illusion.attachment,
            trust: context.illusion.trust,
            safety: context.illusion.safety,
            tension: context.illusion.tension,
        },
        meta: ToolMeta {
            source: source_to_str(meta.source),
            stale: meta.stale,
            schema_version: meta.schema_version,
            updated_at: meta.updated_at,
        },
    };

    log_tool_call_success(SOCIAL_GET_AFFECT_CONTEXT_TOOL, &user_id);
    Ok(serde_json::to_value(output)?)
}

async fn execute_social_get_dialogue_summary(
    input: Value,
    graph: &CognitiveGraph,
) -> anyhow::Result<Value> {
    let input: DialogueToolInput = serde_json::from_value(input)?;
    let user_id = input.user_id.trim().to_string();
    if user_id.is_empty() {
        bail!("user_id is required");
    }

    let memory_hint = clamp_memory_hint(input.memory_hint.unwrap_or(0.0));
    let options = SocialQueryOptions::for_dialogue(memory_hint)
        .with_max_staleness_ms(sanitize_staleness(input.max_staleness_ms))
        .with_allow_stale_fallback(input.allow_stale_fallback.unwrap_or(true))
        .with_force_project(input.force_project.unwrap_or(false));

    log_tool_call_start(SOCIAL_GET_DIALOGUE_SUMMARY_TOOL, &user_id);

    let result =
        query_social_context(graph, SocialQueryIntent::DialogueSummary, &user_id, options).await;

    let SocialQueryResult::Dialogue { summary, meta } = result else {
        let err = "unexpected query result for dialogue summary";
        log_tool_call_failure(SOCIAL_GET_DIALOGUE_SUMMARY_TOOL, &user_id, err);
        bail!(err);
    };

    let output = DialogueSummaryToolOutput {
        user_id,
        familiarity: summary.as_ref().map(|s| s.familiarity.to_string()),
        trust_state: summary.as_ref().map(|s| s.trust_state.to_string()),
        tension_state: summary.as_ref().map(|s| s.tension_state.to_string()),
        summary: summary.map(|s| s.summary),
        meta: ToolMeta {
            source: source_to_str(meta.source),
            stale: meta.stale,
            schema_version: meta.schema_version,
            updated_at: meta.updated_at,
        },
    };

    log_tool_call_success(SOCIAL_GET_DIALOGUE_SUMMARY_TOOL, &output.user_id);
    Ok(serde_json::to_value(output)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn approx_eq(left: f64, right: f64) {
        assert!((left - right).abs() < 1e-6, "left={left} right={right}");
    }

    fn assert_meta_shape(value: &Value) {
        let meta = value.get("meta").expect("meta should exist");
        assert!(meta.get("source").and_then(|v| v.as_str()).is_some());
        assert!(meta.get("stale").and_then(|v| v.as_bool()).is_some());
        assert!(meta.get("schema_version").and_then(|v| v.as_str()).is_some());
        assert!(meta.get("updated_at").and_then(|v| v.as_str()).is_some());
    }

    #[test]
    fn clamp_memory_hint_clamps_to_unit_interval() {
        approx_eq(clamp_memory_hint(-1.0) as f64, 0.0);
        approx_eq(clamp_memory_hint(0.35) as f64, 0.35);
        approx_eq(clamp_memory_hint(5.0) as f64, 1.0);
    }

    #[test]
    fn sanitize_staleness_clamps_negative_values() {
        assert_eq!(sanitize_staleness(None), None);
        assert_eq!(sanitize_staleness(Some(-123)), Some(0));
        assert_eq!(sanitize_staleness(Some(456)), Some(456));
    }

    #[tokio::test]
    async fn registry_lists_two_read_tools() {
        let registry = DialogueToolRegistry::default();
        let tools = registry.list();
        assert_eq!(tools.len(), 2);
        assert!(tools.iter().all(|tool| tool.namespace == ToolNamespace::Read));
        assert!(tools.iter().all(|tool| tool.read_only));
        assert!(tools.iter().any(|tool| tool.name == SOCIAL_GET_AFFECT_CONTEXT_TOOL));
        assert!(tools
            .iter()
            .any(|tool| tool.name == SOCIAL_GET_DIALOGUE_SUMMARY_TOOL));
    }

    #[tokio::test]
    async fn registry_rejects_unknown_tool() {
        let graph = CognitiveGraph::new("memory")
            .await
            .expect("in-memory graph should initialize");
        let registry = DialogueToolRegistry::default();
        let result = registry.execute("social.unknown", json!({}), &graph).await;
        assert!(result.is_err());
        assert!(result.err().unwrap().to_string().contains("unknown MCP tool"));
    }

    #[tokio::test]
    async fn affect_tool_clamps_memory_hint_for_projected_context() {
        let graph = CognitiveGraph::new("memory")
            .await
            .expect("in-memory graph should initialize");
        let registry = DialogueToolRegistry::default();

        let high = registry
            .execute(
                SOCIAL_GET_AFFECT_CONTEXT_TOOL,
                json!({ "user_id": "alice", "memory_hint": 5.0, "force_project": true }),
                &graph,
            )
            .await
            .expect("tool call should succeed");
        let low = registry
            .execute(
                SOCIAL_GET_AFFECT_CONTEXT_TOOL,
                json!({ "user_id": "bob", "memory_hint": -5.0, "force_project": true }),
                &graph,
            )
            .await
            .expect("tool call should succeed");

        let high_depth = high
            .get("metrics")
            .and_then(|v| v.get("context_depth"))
            .and_then(|v| v.as_f64())
            .expect("high context_depth should exist");
        let low_depth = low
            .get("metrics")
            .and_then(|v| v.get("context_depth"))
            .and_then(|v| v.as_f64())
            .expect("low context_depth should exist");

        approx_eq(high_depth, 1.0);
        approx_eq(low_depth, 0.0);
    }

    #[tokio::test]
    async fn affect_tool_returns_expected_default_shape() {
        let graph = CognitiveGraph::new("memory")
            .await
            .expect("in-memory graph should initialize");
        let registry = DialogueToolRegistry::default();

        let result = registry
            .execute(
                SOCIAL_GET_AFFECT_CONTEXT_TOOL,
                json!({ "user_id": "alice", "memory_hint": 0.2 }),
                &graph,
            )
            .await
            .expect("tool call should succeed");

        assert_eq!(result.get("user_id").and_then(|v| v.as_str()), Some("alice"));
        assert_eq!(result.get("known").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(
            result
                .get("metrics")
                .and_then(|v| v.get("affinity"))
                .and_then(|v| v.as_f64()),
            Some(0.0)
        );
        assert_eq!(
            result
                .get("illusion")
                .and_then(|v| v.get("trust"))
                .and_then(|v| v.as_f64()),
            Some(0.0)
        );
        assert_meta_shape(&result);
        assert_eq!(
            result
                .get("meta")
                .and_then(|v| v.get("source"))
                .and_then(|v| v.as_str()),
            Some("tree_fresh")
        );
    }

    #[tokio::test]
    async fn dialogue_summary_tool_trims_user_id_and_returns_expected_shape() {
        let graph = CognitiveGraph::new("memory")
            .await
            .expect("in-memory graph should initialize");
        let registry = DialogueToolRegistry::default();

        let result = registry
            .execute(
                SOCIAL_GET_DIALOGUE_SUMMARY_TOOL,
                json!({ "user_id": "  alice  ", "memory_hint": 0.2 }),
                &graph,
            )
            .await
            .expect("tool call should succeed");

        assert_eq!(result.get("user_id").and_then(|v| v.as_str()), Some("alice"));
        assert_eq!(result.get("familiarity").and_then(|v| v.as_str()), Some("new"));
        assert_eq!(result.get("trust_state").and_then(|v| v.as_str()), Some("neutral"));
        assert_eq!(result.get("tension_state").and_then(|v| v.as_str()), Some("low"));
        assert!(result
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .contains("[social summary] user=alice"));
        assert_meta_shape(&result);
        assert_eq!(
            result
                .get("meta")
                .and_then(|v| v.get("source"))
                .and_then(|v| v.as_str()),
            Some("tree_fresh")
        );
    }

    #[tokio::test]
    async fn registry_get_returns_only_known_read_tools() {
        let registry = DialogueToolRegistry::default();

        let affect = registry
            .get(SOCIAL_GET_AFFECT_CONTEXT_TOOL)
            .expect("affect tool should exist");
        assert_eq!(affect.namespace, ToolNamespace::Read);
        assert!(affect.read_only);

        let dialogue = registry
            .get(SOCIAL_GET_DIALOGUE_SUMMARY_TOOL)
            .expect("dialogue tool should exist");
        assert_eq!(dialogue.namespace, ToolNamespace::Read);
        assert!(dialogue.read_only);

        assert!(registry.get("social.unknown").is_none());
    }

    #[tokio::test]
    async fn affect_tool_rejects_empty_user_id() {
        let graph = CognitiveGraph::new("memory")
            .await
            .expect("in-memory graph should initialize");
        let registry = DialogueToolRegistry::default();

        let result = registry
            .execute(
                SOCIAL_GET_AFFECT_CONTEXT_TOOL,
                json!({ "user_id": "   " }),
                &graph,
            )
            .await;

        assert!(result.is_err());
        assert!(result.err().unwrap().to_string().contains("user_id is required"));
    }

    #[tokio::test]
    async fn dialogue_summary_tool_rejects_empty_user_id() {
        let graph = CognitiveGraph::new("memory")
            .await
            .expect("in-memory graph should initialize");
        let registry = DialogueToolRegistry::default();

        let result = registry
            .execute(
                SOCIAL_GET_DIALOGUE_SUMMARY_TOOL,
                json!({ "user_id": "" }),
                &graph,
            )
            .await;

        assert!(result.is_err());
        assert!(result.err().unwrap().to_string().contains("user_id is required"));
    }

    #[tokio::test]
    async fn affect_tool_rejects_missing_user_id_field() {
        let graph = CognitiveGraph::new("memory")
            .await
            .expect("in-memory graph should initialize");
        let registry = DialogueToolRegistry::default();

        let result = registry
            .execute(SOCIAL_GET_AFFECT_CONTEXT_TOOL, json!({}), &graph)
            .await;

        assert!(result.is_err());
        assert!(result
            .err()
            .unwrap()
            .to_string()
            .contains("missing field `user_id`"));
    }

    #[tokio::test]
    async fn dialogue_summary_tool_sanitizes_negative_staleness_and_disables_stale_fallback() {
        let graph = CognitiveGraph::new("memory")
            .await
            .expect("in-memory graph should initialize");
        let registry = DialogueToolRegistry::default();

        graph
            .update_social_graph(
                "alice",
                memory::graph::SocialDelta {
                    delta_trust: 0.3,
                    ..Default::default()
                },
            )
            .await
            .expect("social graph should update");
        let _ = graph
            .get_or_project_social_tree_snapshot("alice", 0.2)
            .await
            .expect("snapshot should project");
        graph
            .db
            .query("UPDATE social_tree_root SET meta.updated_at = $updated_at WHERE user_id = $user_id;")
            .bind(("updated_at", "2000-01-01T00:00:00Z".to_string()))
            .bind(("user_id", "alice".to_string()))
            .await
            .expect("updated_at should update");

        let result = registry
            .execute(
                SOCIAL_GET_DIALOGUE_SUMMARY_TOOL,
                json!({
                    "user_id": "alice",
                    "memory_hint": 0.2,
                    "max_staleness_ms": -1,
                    "allow_stale_fallback": false
                }),
                &graph,
            )
            .await
            .expect("tool call should succeed");

        assert_eq!(
            result
                .get("meta")
                .and_then(|v| v.get("source"))
                .and_then(|v| v.as_str()),
            Some("tree_fresh")
        );
        assert_eq!(
            result
                .get("meta")
                .and_then(|v| v.get("stale"))
                .and_then(|v| v.as_bool()),
            Some(false)
        );
    }
}
