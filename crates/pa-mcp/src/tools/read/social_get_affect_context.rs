use crate::observability::{
    log_tool_call_failure, log_tool_call_start, log_tool_call_success,
};
use pa_cognitive::social_context::{
    query_social_context, SocialQueryIntent, SocialQueryOptions, SocialQueryResult,
};
use pa_memory::graph::CognitiveGraph;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const TOOL_NAME: &str = "social.get_affect_context";

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
    known: bool,
    metrics: Metrics,
    illusion: Illusion,
    meta: Meta,
}

#[derive(Debug, Serialize)]
struct Metrics {
    affinity: f32,
    attachment: f32,
    trust: f32,
    safety: f32,
    tension: f32,
    context_depth: f32,
}

#[derive(Debug, Serialize)]
struct Illusion {
    affinity: f32,
    attachment: f32,
    trust: f32,
    safety: f32,
    tension: f32,
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

fn source_to_str(source: pa_cognitive::social_context::SocialQuerySource) -> &'static str {
    match source {
        pa_cognitive::social_context::SocialQuerySource::TreeFresh => "tree_fresh",
        pa_cognitive::social_context::SocialQuerySource::TreeStale => "tree_stale",
        pa_cognitive::social_context::SocialQuerySource::GraphFallback => "graph_fallback",
        pa_cognitive::social_context::SocialQuerySource::DefaultFallback => "default_fallback",
    }
}

pub async fn execute(input: Value, graph: &CognitiveGraph) -> anyhow::Result<Value> {
    let input: Input = serde_json::from_value(input)?;
    let user_id = input.user_id.trim().to_string();
    if user_id.is_empty() {
        anyhow::bail!("user_id is required");
    }

    let memory_hint = clamp_memory_hint(input.memory_hint.unwrap_or(0.0));
    let options = SocialQueryOptions::for_affect(memory_hint)
        .with_max_staleness_ms(sanitize_staleness(input.max_staleness_ms))
        .with_allow_stale_fallback(input.allow_stale_fallback.unwrap_or(true))
        .with_force_project(input.force_project.unwrap_or(false));

    log_tool_call_start(TOOL_NAME, &user_id);

    let result = query_social_context(
        graph,
        SocialQueryIntent::AffectRich,
        &user_id,
        options,
    )
    .await;

    let SocialQueryResult::Affect { context, meta } = result else {
        let err = "unexpected query result for affect context";
        log_tool_call_failure(TOOL_NAME, &user_id, err);
        anyhow::bail!(err);
    };

    let output = Output {
        user_id: context.user_id,
        known: context.known,
        metrics: Metrics {
            affinity: context.metrics.affinity,
            attachment: context.metrics.attachment,
            trust: context.metrics.trust,
            safety: context.metrics.safety,
            tension: context.metrics.tension,
            context_depth: context.metrics.context_depth,
        },
        illusion: Illusion {
            affinity: context.illusion.affinity,
            attachment: context.illusion.attachment,
            trust: context.illusion.trust,
            safety: context.illusion.safety,
            tension: context.illusion.tension,
        },
        meta: Meta {
            source: source_to_str(meta.source),
            stale: meta.stale,
            schema_version: meta.schema_version,
            updated_at: meta.updated_at,
        },
    };

    log_tool_call_success(TOOL_NAME, &user_id);
    Ok(serde_json::to_value(output)?)
}
