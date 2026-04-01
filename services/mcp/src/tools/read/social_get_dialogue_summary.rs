use crate::observability::{
    log_tool_call_failure, log_tool_call_start, log_tool_call_success,
};
use cognitive::social_context::{
    query_social_context, SocialQueryIntent, SocialQueryOptions, SocialQueryResult,
};
use memory::graph::CognitiveGraph;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const TOOL_NAME: &str = "social.get_dialogue_summary";

#[derive(Debug, Deserialize)]
struct Input {
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
struct Output {
    user_id: String,
    familiarity: Option<String>,
    trust_state: Option<String>,
    tension_state: Option<String>,
    summary: Option<String>,
    meta: Meta,
}

#[derive(Debug, Serialize)]
struct Meta {
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

fn source_to_str(source: cognitive::social_context::SocialQuerySource) -> &'static str {
    match source {
        cognitive::social_context::SocialQuerySource::TreeFresh => "tree_fresh",
        cognitive::social_context::SocialQuerySource::TreeStale => "tree_stale",
        cognitive::social_context::SocialQuerySource::GraphFallback => "graph_fallback",
        cognitive::social_context::SocialQuerySource::DefaultFallback => "default_fallback",
    }
}

pub async fn execute(input: Value, graph: &CognitiveGraph) -> anyhow::Result<Value> {
    let input: Input = serde_json::from_value(input)?;
    let user_id = input.user_id.trim().to_string();
    if user_id.is_empty() {
        anyhow::bail!("user_id is required");
    }

    let memory_hint = clamp_memory_hint(input.memory_hint.unwrap_or(0.0));
    let options = SocialQueryOptions::for_dialogue(memory_hint)
        .with_max_staleness_ms(sanitize_staleness(input.max_staleness_ms))
        .with_allow_stale_fallback(input.allow_stale_fallback.unwrap_or(true))
        .with_force_project(input.force_project.unwrap_or(false));

    log_tool_call_start(TOOL_NAME, &user_id);

    let result = query_social_context(
        graph,
        SocialQueryIntent::DialogueSummary,
        &user_id,
        options,
    )
    .await;

    let SocialQueryResult::Dialogue { summary, meta } = result else {
        let err = "unexpected query result for dialogue summary";
        log_tool_call_failure(TOOL_NAME, &user_id, err);
        anyhow::bail!(err);
    };

    let output = Output {
        user_id,
        familiarity: summary.as_ref().map(|s| s.familiarity.to_string()),
        trust_state: summary.as_ref().map(|s| s.trust_state.to_string()),
        tension_state: summary.as_ref().map(|s| s.tension_state.to_string()),
        summary: summary.map(|s| s.summary),
        meta: Meta {
            source: source_to_str(meta.source),
            stale: meta.stale,
            schema_version: meta.schema_version,
            updated_at: meta.updated_at,
        },
    };

    log_tool_call_success(TOOL_NAME, &output.user_id);
    Ok(serde_json::to_value(output)?)
}
