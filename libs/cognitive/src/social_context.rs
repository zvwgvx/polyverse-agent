use chrono::{DateTime, Utc};
use kernel::prompt_registry::render_prompt_or;
use memory::graph::{AttitudesTowards, CognitiveGraph, IllusionOf, SocialTreeSnapshot};
use tracing::debug;

#[derive(Debug, Clone)]
pub struct SocialCoreMetrics {
    pub affinity: f32,
    pub attachment: f32,
    pub trust: f32,
    pub safety: f32,
    pub tension: f32,
    pub context_depth: f32,
}

#[derive(Debug, Clone)]
pub struct IllusionMetrics {
    pub affinity: f32,
    pub attachment: f32,
    pub trust: f32,
    pub safety: f32,
    pub tension: f32,
}

#[derive(Debug, Clone)]
pub struct AffectSocialContext {
    pub user_id: String,
    pub known: bool,
    pub metrics: SocialCoreMetrics,
    pub illusion: IllusionMetrics,
}

#[derive(Debug, Clone)]
pub struct DialogueSocialSummary {
    pub user_id: String,
    pub familiarity: &'static str,
    pub trust_state: &'static str,
    pub tension_state: &'static str,
    pub summary: String,
}

impl AffectSocialContext {
    pub fn to_prompt_text(&self) -> String {
        let affinity = format!("{:.6}", self.metrics.affinity);
        let attachment = format!("{:.6}", self.metrics.attachment);
        let trust = format!("{:.6}", self.metrics.trust);
        let safety = format!("{:.6}", self.metrics.safety); 
        let tension = format!("{:.6}", self.metrics.tension);
        let context_depth = format!("{:.6}", self.metrics.context_depth);
        let ill_affinity = format!("{:.6}", self.illusion.affinity);
        let ill_attachment = format!("{:.6}", self.illusion.attachment);
        let ill_trust = format!("{:.6}", self.illusion.trust);
        let ill_safety = format!("{:.6}", self.illusion.safety);
        let ill_tension = format!("{:.6}", self.illusion.tension);

        if self.known {
            render_prompt_or(
                "context.social.known",
                &[
                    ("username", self.user_id.as_str()),
                    ("affinity", affinity.as_str()),
                    ("attachment", attachment.as_str()),
                    ("trust", trust.as_str()),
                    ("safety", safety.as_str()),
                    ("tension", tension.as_str()),
                    ("context_depth", context_depth.as_str()),
                    ("ill_affinity", ill_affinity.as_str()),
                    ("ill_attachment", ill_attachment.as_str()),
                    ("ill_trust", ill_trust.as_str()),
                    ("ill_safety", ill_safety.as_str()),
                    ("ill_tension", ill_tension.as_str()),
                ],
                "### EMOTIONAL AND RELATION STATE WITH {{username}}:\nAffinity: {{affinity}}\nAttachment: {{attachment}}\nTrust: {{trust}}\nSafety: {{safety}}\nTension: {{tension}}\nContext Depth: {{context_depth}}\nAssumed perception -> Affinity: {{ill_affinity}}, Attachment: {{ill_attachment}}, Trust: {{ill_trust}}, Safety: {{ill_safety}}, Tension: {{ill_tension}}\n",
            )
        } else {
            render_prompt_or(
                "context.social.default",
                &[
                    ("username", self.user_id.as_str()),
                    ("context_depth", context_depth.as_str()),
                ],
                "### EMOTIONAL AND RELATION STATE WITH {{username}}:\nAffinity: 0.000000\nAttachment: 0.000000\nTrust: 0.000000\nSafety: 0.000000\nTension: 0.000000\nContext Depth: {{context_depth}}\nAssumed perception -> Affinity: 0.000000, Attachment: 0.000000, Trust: 0.000000, Safety: 0.000000, Tension: 0.000000\n",
            )
        }
    }
}

const AFFECT_MAX_STALENESS_MS: i64 = 5 * 60 * 1000;
const DIALOGUE_MAX_STALENESS_MS: i64 = 30 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocialQueryIntent {
    AffectRich,
    DialogueSummary,
}

#[derive(Debug, Clone, Copy)]
pub struct SocialQueryOptions {
    pub memory_hint: f32,
    pub max_staleness_ms: Option<i64>,
    pub force_project: bool,
    pub allow_stale_fallback: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocialQuerySource {
    TreeFresh,
    TreeStale,
    GraphFallback,
    DefaultFallback,
}

#[derive(Debug, Clone)]
pub struct SocialQueryMeta {
    pub source: SocialQuerySource,
    pub stale: bool,
    pub schema_version: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone)]
pub enum SocialQueryResult {
    Affect {
        context: AffectSocialContext,
        meta: SocialQueryMeta,
    },
    Dialogue {
        summary: Option<DialogueSocialSummary>,
        meta: SocialQueryMeta,
    },
}

impl SocialQueryIntent {
    fn scope(self) -> &'static str {
        match self {
            SocialQueryIntent::AffectRich => "affect",
            SocialQueryIntent::DialogueSummary => "dialogue",
        }
    }
}

impl SocialQueryOptions {
    pub fn for_affect(memory_hint: f32) -> Self {
        Self {
            memory_hint,
            max_staleness_ms: Some(AFFECT_MAX_STALENESS_MS),
            force_project: false,
            allow_stale_fallback: true,
        }
    }

    pub fn for_dialogue(memory_hint: f32) -> Self {
        Self {
            memory_hint,
            max_staleness_ms: Some(DIALOGUE_MAX_STALENESS_MS),
            force_project: false,
            allow_stale_fallback: true,
        }
    }

    pub fn with_max_staleness_ms(mut self, max_staleness_ms: Option<i64>) -> Self {
        self.max_staleness_ms = max_staleness_ms;
        self
    }

    pub fn with_force_project(mut self, force_project: bool) -> Self {
        self.force_project = force_project;
        self
    }

    pub fn with_allow_stale_fallback(mut self, allow_stale_fallback: bool) -> Self {
        self.allow_stale_fallback = allow_stale_fallback;
        self
    }
}

impl SocialQuerySource {
    fn as_str(self) -> &'static str {
        match self {
            SocialQuerySource::TreeFresh => "tree_fresh",
            SocialQuerySource::TreeStale => "tree_stale",
            SocialQuerySource::GraphFallback => "graph_fallback",
            SocialQuerySource::DefaultFallback => "default_fallback",
        }
    }
}

impl SocialQueryMeta {
    fn from_tree(source: SocialQuerySource, stale: bool, tree: &SocialTreeSnapshot) -> Self {
        Self {
            source,
            stale,
            schema_version: Some(tree.meta.schema_version.clone()),
            updated_at: Some(tree.meta.updated_at.clone()),
        }
    }

    fn from_fallback(source: SocialQuerySource) -> Self {
        Self {
            source,
            stale: false,
            schema_version: None,
            updated_at: None,
        }
    }
}

pub async fn load_affect_social_context(
    graph: &CognitiveGraph,
    current_username: &str,
    memory_hint: f32,
) -> AffectSocialContext {
    let options = SocialQueryOptions::for_affect(memory_hint);
    let SocialQueryResult::Affect { context, .. } = query_social_context(
        graph,
        SocialQueryIntent::AffectRich,
        current_username,
        options,
    )
    .await
    else {
        return build_default_affect_context(current_username, memory_hint);
    };

    context
}

pub async fn load_dialogue_social_summary(
    graph: &CognitiveGraph,
    current_username: &str,
    memory_hint: f32,
) -> Option<DialogueSocialSummary> {
    let options = SocialQueryOptions::for_dialogue(memory_hint);
    let SocialQueryResult::Dialogue { summary, .. } = query_social_context(
        graph,
        SocialQueryIntent::DialogueSummary,
        current_username,
        options,
    )
    .await
    else {
        return None;
    };

    summary
}

pub async fn query_social_context(
    graph: &CognitiveGraph,
    intent: SocialQueryIntent,
    current_username: &str,
    options: SocialQueryOptions,
) -> SocialQueryResult {
    resolve_social_query(graph, intent, current_username, options).await
}

async fn resolve_social_query(
    graph: &CognitiveGraph,
    intent: SocialQueryIntent,
    current_username: &str,
    options: SocialQueryOptions,
) -> SocialQueryResult {
    if let Some(result) = resolve_tree_query(graph, intent, current_username, options).await {
        return result;
    }

    resolve_graph_or_default_query(graph, intent, current_username, options).await
}

async fn resolve_tree_query(
    graph: &CognitiveGraph,
    intent: SocialQueryIntent,
    current_username: &str,
    options: SocialQueryOptions,
) -> Option<SocialQueryResult> {
    let mut snapshot = if options.force_project {
        graph.project_social_tree(current_username, options.memory_hint).await.ok()?
    } else {
        graph
            .get_or_project_social_tree_snapshot(current_username, options.memory_hint)
            .await
            .ok()?
    };

    let initial_stale = is_snapshot_stale(
        &snapshot.meta.updated_at,
        options.max_staleness_ms,
        Utc::now(),
    );

    if initial_stale && !options.force_project {
        if let Ok(refreshed) = graph
            .project_social_tree(current_username, options.memory_hint)
            .await
        {
            snapshot = refreshed;
        }
    }

    let (source, stale) = classify_tree_snapshot(
        &snapshot.meta.updated_at,
        options.max_staleness_ms,
        options.allow_stale_fallback,
        Utc::now(),
    )?;

    let meta = SocialQueryMeta::from_tree(source, stale, &snapshot);
    log_tree_snapshot_for_request(
        intent.scope(),
        current_username,
        &snapshot,
        options.memory_hint,
    );
    log_social_query_meta(intent.scope(), current_username, &meta);

    Some(match intent {
        SocialQueryIntent::AffectRich => SocialQueryResult::Affect {
            context: build_affect_context_from_tree(current_username, &snapshot, options.memory_hint),
            meta,
        },
        SocialQueryIntent::DialogueSummary => SocialQueryResult::Dialogue {
            summary: Some(build_dialogue_summary_from_tree(current_username, &snapshot)),
            meta,
        },
    })
}

async fn resolve_graph_or_default_query(
    graph: &CognitiveGraph,
    intent: SocialQueryIntent,
    current_username: &str,
    options: SocialQueryOptions,
) -> SocialQueryResult {
    match graph.get_social_context(current_username).await {
        Ok((attitudes, illusion)) => {
            let meta = SocialQueryMeta::from_fallback(SocialQuerySource::GraphFallback);
            log_social_query_meta(intent.scope(), current_username, &meta);

            match intent {
                SocialQueryIntent::AffectRich => SocialQueryResult::Affect {
                    context: build_known_affect_context(
                        current_username,
                        &attitudes,
                        &illusion,
                        options.memory_hint,
                    ),
                    meta,
                },
                SocialQueryIntent::DialogueSummary => SocialQueryResult::Dialogue {
                    summary: Some(build_dialogue_summary_from_graph(
                        current_username,
                        &attitudes,
                        options.memory_hint,
                    )),
                    meta,
                },
            }
        }
        Err(_) => {
            let meta = SocialQueryMeta::from_fallback(SocialQuerySource::DefaultFallback);
            log_social_query_meta(intent.scope(), current_username, &meta);

            match intent {
                SocialQueryIntent::AffectRich => SocialQueryResult::Affect {
                    context: build_default_affect_context(current_username, options.memory_hint),
                    meta,
                },
                SocialQueryIntent::DialogueSummary => SocialQueryResult::Dialogue {
                    summary: None,
                    meta,
                },
            }
        }
    }
}

fn classify_tree_snapshot(
    updated_at: &str,
    max_staleness_ms: Option<i64>,
    allow_stale_fallback: bool,
    now: DateTime<Utc>,
) -> Option<(SocialQuerySource, bool)> {
    let stale = is_snapshot_stale(updated_at, max_staleness_ms, now);
    if stale && !allow_stale_fallback {
        None
    } else if stale {
        Some((SocialQuerySource::TreeStale, true))
    } else {
        Some((SocialQuerySource::TreeFresh, false))
    }
}

fn parse_updated_at_rfc3339(updated_at: &str) -> Option<DateTime<Utc>> {
    if updated_at.trim().is_empty() {
        return None;
    }

    DateTime::parse_from_rfc3339(updated_at)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn snapshot_age_ms(updated_at: &str, now: DateTime<Utc>) -> Option<i64> {
    let updated_at = parse_updated_at_rfc3339(updated_at)?;
    let age = now.signed_duration_since(updated_at).num_milliseconds();
    Some(age.max(0))
}

fn is_snapshot_stale(updated_at: &str, max_staleness_ms: Option<i64>, now: DateTime<Utc>) -> bool {
    let Some(max_staleness_ms) = max_staleness_ms else {
        return false;
    };

    let max_staleness_ms = max_staleness_ms.max(0);

    match snapshot_age_ms(updated_at, now) {
        Some(age_ms) => age_ms > max_staleness_ms,
        None => true,
    }
}

fn build_dialogue_summary_from_graph(
    current_username: &str,
    attitudes: &AttitudesTowards,
    memory_hint: f32,
) -> DialogueSocialSummary {
    let graph_depth = (attitudes.affinity.abs()
        + attitudes.attachment.abs()
        + attitudes.trust.abs()
        + attitudes.safety.abs()) / 4.0;
    let context_depth = (graph_depth + memory_hint).min(1.0);

    let familiarity = if context_depth >= 0.66 {
        "close"
    } else if context_depth >= 0.25 {
        "known"
    } else {
        "new"
    };

    let trust_state = if attitudes.trust >= 0.35 {
        "stable"
    } else if attitudes.trust <= -0.20 {
        "fragile"
    } else {
        "neutral"
    };

    let tension_state = if attitudes.tension >= 0.55 {
        "high"
    } else if attitudes.tension >= 0.25 {
        "medium"
    } else {
        "low"
    };

    DialogueSocialSummary {
        user_id: current_username.to_string(),
        familiarity,
        trust_state,
        tension_state,
        summary: format!(
            "[social summary] user={} familiarity={} trust={} tension={}.",
            current_username, familiarity, trust_state, tension_state
        ),
    }
}

fn log_tree_snapshot_for_request(scope: &str, user_id: &str, tree: &SocialTreeSnapshot, memory_hint: f32) {
    debug!(
        kind = "social.tree",
        scope = scope,
        user = %user_id,
        memory_hint,
        relationship_affinity = tree.relationship_core.affinity,
        relationship_attachment = tree.relationship_core.attachment,
        relationship_trust = tree.relationship_core.trust,
        relationship_safety = tree.relationship_core.safety,
        relationship_tension = tree.relationship_core.tension,
        relationship_familiarity = tree.relationship_core.familiarity,
        boundary_reliability = tree.relationship_core.boundary_reliability,
        tension_live = tree.dynamic_state.tension_live,
        warmth_live = tree.dynamic_state.warmth_live,
        unresolved_friction_score = tree.dynamic_state.unresolved_friction_score,
        perceived_user_affinity = tree.self_other_model.perceived_user_affinity,
        perceived_user_attachment = tree.self_other_model.perceived_user_attachment,
        perceived_user_trust = tree.self_other_model.perceived_user_trust,
        perceived_user_safety = tree.self_other_model.perceived_user_safety,
        perceived_user_tension = tree.self_other_model.perceived_user_tension,
        model_confidence = tree.self_other_model.confidence,
        familiarity_bucket = %tree.derived_summaries.familiarity_bucket,
        trust_state = %tree.derived_summaries.trust_state,
        tension_state = %tree.derived_summaries.tension_state,
        dialogue_summary = %tree.derived_summaries.dialogue_summary_short,
        schema_version = %tree.meta.schema_version,
        updated_at = %tree.meta.updated_at,
        writer_version = %tree.meta.writer_version,
        "Social tree snapshot resolved for request"
    );
}

fn log_social_query_meta(scope: &str, user_id: &str, meta: &SocialQueryMeta) {
    debug!(
        kind = "social.query",
        scope = scope,
        user = %user_id,
        source = meta.source.as_str(),
        stale = meta.stale,
        schema_version = meta.schema_version.as_deref().unwrap_or(""),
        updated_at = meta.updated_at.as_deref().unwrap_or(""),
        "Social query resolved"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory::graph::SocialDelta;

    async fn in_memory_graph() -> CognitiveGraph {
        CognitiveGraph::new("memory")
            .await
            .expect("in-memory graph should initialize")
    }

    async fn set_tree_updated_at(graph: &CognitiveGraph, user_id: &str, updated_at: &str) {
        graph
            .db
            .query("UPDATE social_tree_root SET meta.updated_at = $updated_at WHERE user_id = $user_id;")
            .bind(("updated_at", updated_at.to_string()))
            .bind(("user_id", user_id.to_string()))
            .await
            .expect("updated_at should update");
    }

    fn tree_for_mapping(familiarity: &str, trust: &str, tension: &str, summary: &str) -> SocialTreeSnapshot {
        SocialTreeSnapshot {
            user_id: "alice".to_string(),
            relationship_core: memory::graph::SocialTreeRelationshipCore {
                affinity: 0.1,
                attachment: 0.1,
                trust: 0.1,
                safety: 0.1,
                tension: 0.1,
                familiarity: 0.4,
                boundary_reliability: 0.4,
            },
            dynamic_state: memory::graph::SocialTreeDynamicState::default(),
            self_other_model: memory::graph::SocialTreeSelfOtherModel::default(),
            derived_summaries: memory::graph::SocialTreeDerivedSummaries {
                dialogue_summary_short: summary.to_string(),
                familiarity_bucket: familiarity.to_string(),
                trust_state: trust.to_string(),
                tension_state: tension.to_string(),
            },
            meta: memory::graph::SocialTreeMeta {
                schema_version: "v1".to_string(),
                updated_at: Utc::now().to_rfc3339(),
                decay_policy: "graph_decay_0.99_per_day".to_string(),
                writer_version: "graph_projection_v1".to_string(),
            },
        }
    }

    fn dialogue_meta(result: &SocialQueryResult) -> (&Option<DialogueSocialSummary>, &SocialQueryMeta) {
        match result {
            SocialQueryResult::Dialogue { summary, meta } => (summary, meta),
            SocialQueryResult::Affect { .. } => panic!("expected dialogue result"),
        }
    }

    fn affect_meta(result: &SocialQueryResult) -> (&AffectSocialContext, &SocialQueryMeta) {
        match result {
            SocialQueryResult::Affect { context, meta } => (context, meta),
            SocialQueryResult::Dialogue { .. } => panic!("expected affect result"),
        }
    }

    fn assert_tree_meta(meta: &SocialQueryMeta, source: SocialQuerySource, stale: bool) {
        assert_eq!(meta.source, source);
        assert_eq!(meta.stale, stale);
        assert_eq!(meta.schema_version.as_deref(), Some("v1"));
        assert!(meta.updated_at.as_deref().is_some());
    }

    fn assert_default_fallback_meta(meta: &SocialQueryMeta) {
        assert_eq!(meta.source, SocialQuerySource::DefaultFallback);
        assert!(!meta.stale);
        assert!(meta.schema_version.is_none());
        assert!(meta.updated_at.is_none());
    }

    fn assert_graph_fallback_meta(meta: &SocialQueryMeta) {
        assert_eq!(meta.source, SocialQuerySource::GraphFallback);
        assert!(!meta.stale);
        assert!(meta.schema_version.is_none());
        assert!(meta.updated_at.is_none());
    }

    fn approx_eq(left: f32, right: f32) {
        assert!((left - right).abs() < 1e-6, "left={left} right={right}");
    }

    #[test]
    fn parse_updated_at_rfc3339_valid_invalid_and_empty() {
        let ts = "2026-03-31T12:34:56Z";
        assert!(parse_updated_at_rfc3339(ts).is_some());
        assert!(parse_updated_at_rfc3339("").is_none());
        assert!(parse_updated_at_rfc3339("not-a-timestamp").is_none());
    }

    #[test]
    fn staleness_threshold_boundary() {
        let now = Utc::now();
        let fresh = (now - chrono::Duration::milliseconds(4999)).to_rfc3339();
        let stale = (now - chrono::Duration::milliseconds(5001)).to_rfc3339();

        assert!(!is_snapshot_stale(&fresh, Some(5000), now));
        assert!(is_snapshot_stale(&stale, Some(5000), now));
    }

    #[test]
    fn classify_tree_snapshot_allows_or_rejects_stale_by_policy() {
        let now = Utc::now();
        let stale = (now - chrono::Duration::minutes(60)).to_rfc3339();

        let allowed = classify_tree_snapshot(&stale, Some(5 * 60 * 1000), true, now);
        assert_eq!(allowed, Some((SocialQuerySource::TreeStale, true)));

        let rejected = classify_tree_snapshot(&stale, Some(5 * 60 * 1000), false, now);
        assert!(rejected.is_none());
    }

    #[tokio::test]
    async fn query_social_context_returns_tree_fresh_affect_result() {
        let graph = in_memory_graph().await;
        let result = query_social_context(
            &graph,
            SocialQueryIntent::AffectRich,
            "alice",
            SocialQueryOptions::for_affect(0.2).with_force_project(true),
        )
        .await;

        let (context, meta) = affect_meta(&result);
        assert_tree_meta(meta, SocialQuerySource::TreeFresh, false);
        assert_eq!(context.user_id, "alice");
        assert!(!context.known);
        approx_eq(context.metrics.context_depth, 0.2);
    }

    #[tokio::test]
    async fn force_project_refreshes_stale_snapshot_back_to_tree_fresh() {
        let graph = in_memory_graph().await;
        graph
            .project_social_tree("alice", 0.0)
            .await
            .expect("initial projection should succeed");
        let stale = (Utc::now() - chrono::Duration::hours(2)).to_rfc3339();
        set_tree_updated_at(&graph, "alice", &stale).await;

        let result = query_social_context(
            &graph,
            SocialQueryIntent::DialogueSummary,
            "alice",
            SocialQueryOptions::for_dialogue(0.2)
                .with_force_project(true)
                .with_max_staleness_ms(Some(1)),
        )
        .await;

        let (summary, meta) = dialogue_meta(&result);
        assert_tree_meta(meta, SocialQuerySource::TreeFresh, false);
        assert_eq!(summary.as_ref().map(|s| s.user_id.as_str()), Some("alice"));
    }

    #[tokio::test]
    async fn resolve_graph_or_default_query_returns_graph_fallback_affect_result() {
        let graph = in_memory_graph().await;
        graph
            .update_social_graph(
                "alice",
                SocialDelta {
                    delta_affinity: 0.25,
                    delta_attachment: 0.1,
                    delta_trust: 0.2,
                    delta_safety: 0.15,
                    delta_tension: 0.05,
                },
            )
            .await
            .expect("social graph should update");
        graph
            .update_illusion_graph(
                "alice",
                SocialDelta {
                    delta_affinity: 0.1,
                    delta_attachment: 0.05,
                    delta_trust: 0.15,
                    delta_safety: 0.1,
                    delta_tension: 0.0,
                },
            )
            .await
            .expect("illusion graph should update");

        let result = resolve_graph_or_default_query(
            &graph,
            SocialQueryIntent::AffectRich,
            "alice",
            SocialQueryOptions::for_affect(0.05),
        )
        .await;

        let (context, meta) = affect_meta(&result);
        assert_graph_fallback_meta(meta);
        assert!(context.known);
        approx_eq(context.metrics.affinity, 0.25);
        approx_eq(context.illusion.trust, 0.15);
        assert!(context.metrics.context_depth > 0.0);
    }

    #[tokio::test]
    async fn resolve_graph_or_default_query_returns_graph_fallback_dialogue_result() {
        let graph = in_memory_graph().await;
        graph
            .update_social_graph(
                "alice",
                SocialDelta {
                    delta_affinity: 0.3,
                    delta_attachment: 0.2,
                    delta_trust: 0.3,
                    delta_safety: 0.2,
                    delta_tension: 0.3,
                },
            )
            .await
            .expect("social graph should update");
        graph
            .update_social_graph(
                "alice",
                SocialDelta {
                    delta_affinity: 0.0,
                    delta_attachment: 0.0,
                    delta_trust: 0.1,
                    delta_safety: 0.0,
                    delta_tension: 0.0,
                },
            )
            .await
            .expect("social graph should update twice");

        let result = resolve_graph_or_default_query(
            &graph,
            SocialQueryIntent::DialogueSummary,
            "alice",
            SocialQueryOptions::for_dialogue(0.0),
        )
        .await;

        let (summary, meta) = dialogue_meta(&result);
        let summary = summary.as_ref().expect("summary should exist");
        assert_graph_fallback_meta(meta);
        assert_eq!(summary.user_id, "alice");
        assert_eq!(summary.familiarity, "known");
        assert_eq!(summary.trust_state, "stable");
        assert_eq!(summary.tension_state, "medium");
    }

    #[tokio::test]
    async fn resolve_graph_or_default_query_returns_zeroed_graph_fallback_dialogue_result() {
        let graph = in_memory_graph().await;
        let result = resolve_graph_or_default_query(
            &graph,
            SocialQueryIntent::DialogueSummary,
            "alice",
            SocialQueryOptions::for_dialogue(0.0),
        )
        .await;

        let (summary, meta) = dialogue_meta(&result);
        let summary = summary.as_ref().expect("summary should exist");
        assert_graph_fallback_meta(meta);
        assert_eq!(summary.user_id, "alice");
        assert_eq!(summary.familiarity, "new");
        assert_eq!(summary.trust_state, "neutral");
        assert_eq!(summary.tension_state, "low");
    }

    #[tokio::test]
    async fn resolve_graph_or_default_query_returns_zeroed_graph_fallback_affect_result() {
        let graph = in_memory_graph().await;
        let result = resolve_graph_or_default_query(
            &graph,
            SocialQueryIntent::AffectRich,
            "alice",
            SocialQueryOptions::for_affect(0.05),
        )
        .await;

        let (context, meta) = affect_meta(&result);
        assert_graph_fallback_meta(meta);
        assert_eq!(context.user_id, "alice");
        assert!(context.known);
        approx_eq(context.metrics.context_depth, 0.05);
        approx_eq(context.metrics.affinity, 0.0);
        approx_eq(context.illusion.trust, 0.0);
    }

    #[test]
    fn default_fallback_meta_has_expected_shape() {
        let meta = SocialQueryMeta::from_fallback(SocialQuerySource::DefaultFallback);
        assert_default_fallback_meta(&meta);
    }

    #[test]
    fn default_affect_context_has_expected_shape() {
        let context = build_default_affect_context("alice", 0.05);
        assert_eq!(context.user_id, "alice");
        assert!(!context.known);
        approx_eq(context.metrics.context_depth, 0.05);
        approx_eq(context.metrics.affinity, 0.0);
        approx_eq(context.illusion.trust, 0.0);
    }

    #[test]
    fn default_dialogue_result_is_none() {
        let result = SocialQueryResult::Dialogue {
            summary: None,
            meta: SocialQueryMeta::from_fallback(SocialQuerySource::DefaultFallback),
        };
        let (summary, meta) = dialogue_meta(&result);
        assert!(summary.is_none());
        assert_default_fallback_meta(meta);
    }

    #[test]
    fn affect_from_tree_marks_zero_signal_as_unknown() {
        let mut tree = tree_for_mapping("new", "neutral", "low", "");
        tree.relationship_core.affinity = 0.0;
        tree.relationship_core.attachment = 0.0;
        tree.relationship_core.trust = 0.0;
        tree.relationship_core.safety = 0.0;
        tree.relationship_core.tension = 0.0;
        tree.self_other_model.perceived_user_affinity = 0.0;
        tree.self_other_model.perceived_user_attachment = 0.0;
        tree.self_other_model.perceived_user_trust = 0.0;
        tree.self_other_model.perceived_user_safety = 0.0;
        tree.self_other_model.perceived_user_tension = 0.0;
        let context = build_affect_context_from_tree("alice", &tree, 0.2);
        assert_eq!(context.user_id, "alice");
        assert!(!context.known);
        approx_eq(context.metrics.context_depth, 0.4);
    }

    #[test]
    fn affect_from_tree_marks_signal_as_known() {
        let mut tree = tree_for_mapping("new", "neutral", "low", "");
        tree.relationship_core.trust = 0.2;
        let context = build_affect_context_from_tree("alice", &tree, 0.0);
        assert!(context.known);
        approx_eq(context.metrics.trust, 0.2);
    }

    #[test]
    fn tree_meta_from_tree_keeps_schema_fields() {
        let tree = tree_for_mapping("known", "stable", "medium", "summary");
        let meta = SocialQueryMeta::from_tree(SocialQuerySource::TreeFresh, false, &tree);
        assert_tree_meta(&meta, SocialQuerySource::TreeFresh, false);
    }

    #[test]
    fn source_as_str_matches_expected_values() {
        assert_eq!(SocialQuerySource::TreeFresh.as_str(), "tree_fresh");
        assert_eq!(SocialQuerySource::TreeStale.as_str(), "tree_stale");
        assert_eq!(SocialQuerySource::GraphFallback.as_str(), "graph_fallback");
        assert_eq!(SocialQuerySource::DefaultFallback.as_str(), "default_fallback");
    }

    #[test]
    fn classify_tree_snapshot_reports_tree_stale_when_policy_allows_it() {
        let now = Utc::now();
        let stale = (now - chrono::Duration::hours(2)).to_rfc3339();
        let meta = classify_tree_snapshot(&stale, Some(60_000), true, now);
        assert_eq!(meta, Some((SocialQuerySource::TreeStale, true)));
    }

    #[test]
    fn classify_tree_snapshot_reports_tree_fresh_without_staleness_limit() {
        let now = Utc::now();
        let empty = "";
        let meta = classify_tree_snapshot(empty, None, true, now);
        assert_eq!(meta, Some((SocialQuerySource::TreeFresh, false)));
    }

    #[test]
    fn snapshot_age_clamps_future_timestamps_to_zero() {
        let now = Utc::now();
        let future = (now + chrono::Duration::minutes(5)).to_rfc3339();
        assert_eq!(snapshot_age_ms(&future, now), Some(0));
    }

    #[test]
    fn negative_max_staleness_is_sanitized_by_is_snapshot_stale() {
        let now = Utc::now();
        let stale = (now - chrono::Duration::milliseconds(1)).to_rfc3339();
        assert!(is_snapshot_stale(&stale, Some(-1), now));
    }

    #[test]
    fn invalid_updated_at_is_treated_as_stale_by_classifier() {
        let now = Utc::now();
        assert_eq!(
            classify_tree_snapshot("not-a-timestamp", Some(60_000), true, now),
            Some((SocialQuerySource::TreeStale, true))
        );
        assert!(classify_tree_snapshot("not-a-timestamp", Some(60_000), false, now).is_none());
    }

    #[tokio::test]
    async fn stale_tree_without_stale_fallback_reprojects_to_tree_fresh() {
        let graph = in_memory_graph().await;
        graph
            .update_social_graph(
                "alice",
                SocialDelta {
                    delta_trust: 0.2,
                    ..Default::default()
                },
            )
            .await
            .expect("social graph should update");
        graph
            .project_social_tree("alice", 0.0)
            .await
            .expect("projection should succeed");
        let stale = (Utc::now() - chrono::Duration::hours(2)).to_rfc3339();
        set_tree_updated_at(&graph, "alice", &stale).await;

        let result = query_social_context(
            &graph,
            SocialQueryIntent::DialogueSummary,
            "alice",
            SocialQueryOptions::for_dialogue(0.1)
                .with_max_staleness_ms(Some(60_000))
                .with_allow_stale_fallback(false),
        )
        .await;

        let (summary, meta) = dialogue_meta(&result);
        let summary = summary.as_ref().expect("summary should exist");
        assert_tree_meta(meta, SocialQuerySource::TreeFresh, false);
        assert_eq!(summary.user_id, "alice");
        assert_eq!(summary.trust_state, "neutral");
    }

    #[tokio::test]
    async fn load_dialogue_social_summary_uses_tree_query_path() {
        let graph = in_memory_graph().await;
        let summary = load_dialogue_social_summary(&graph, "alice", 0.1).await;
        assert_eq!(summary.as_ref().map(|s| s.user_id.as_str()), Some("alice"));
    }

    #[tokio::test]
    async fn stale_affect_tree_without_stale_fallback_reprojects_to_tree_fresh() {
        let graph = in_memory_graph().await;
        graph
            .update_social_graph(
                "alice",
                SocialDelta {
                    delta_affinity: 0.3,
                    ..Default::default()
                },
            )
            .await
            .expect("social graph should update");
        graph
            .project_social_tree("alice", 0.0)
            .await
            .expect("projection should succeed");
        let stale = (Utc::now() - chrono::Duration::hours(2)).to_rfc3339();
        set_tree_updated_at(&graph, "alice", &stale).await;

        let result = query_social_context(
            &graph,
            SocialQueryIntent::AffectRich,
            "alice",
            SocialQueryOptions::for_affect(0.1)
                .with_max_staleness_ms(Some(60_000))
                .with_allow_stale_fallback(false),
        )
        .await;

        let (context, meta) = affect_meta(&result);
        assert_tree_meta(meta, SocialQuerySource::TreeFresh, false);
        assert!(context.known);
        approx_eq(context.metrics.affinity, 0.3);
    }

    #[tokio::test]
    async fn load_affect_social_context_uses_tree_query_path() {
        let graph = in_memory_graph().await;
        let context = load_affect_social_context(&graph, "alice", 0.1).await;
        assert_eq!(context.user_id, "alice");
        assert!(!context.known);
        approx_eq(context.metrics.context_depth, 0.1);
    }

    #[test]
    fn social_query_options_builders_preserve_overrides() {
        let affect = SocialQueryOptions::for_affect(0.2)
            .with_max_staleness_ms(Some(123))
            .with_force_project(true)
            .with_allow_stale_fallback(false);
        assert_eq!(affect.memory_hint, 0.2);
        assert_eq!(affect.max_staleness_ms, Some(123));
        assert!(affect.force_project);
        assert!(!affect.allow_stale_fallback);

        let dialogue = SocialQueryOptions::for_dialogue(0.3);
        assert_eq!(dialogue.memory_hint, 0.3);
        assert_eq!(dialogue.max_staleness_ms, Some(30 * 60 * 1000));
        assert!(!dialogue.force_project);
        assert!(dialogue.allow_stale_fallback);
    }

    #[test]
    fn social_query_intent_scope_matches_expected_values() {
        assert_eq!(SocialQueryIntent::AffectRich.scope(), "affect");
        assert_eq!(SocialQueryIntent::DialogueSummary.scope(), "dialogue");
    }
}

fn build_affect_context_from_tree(
    current_username: &str,
    tree: &SocialTreeSnapshot,
    memory_hint: f32,
) -> AffectSocialContext {
    let context_depth = tree
        .relationship_core
        .familiarity
        .max(memory_hint)
        .min(1.0);

    let has_signal = tree.relationship_core.affinity != 0.0
        || tree.relationship_core.attachment != 0.0
        || tree.relationship_core.trust != 0.0
        || tree.relationship_core.safety != 0.0
        || tree.relationship_core.tension != 0.0
        || tree.self_other_model.perceived_user_affinity != 0.0
        || tree.self_other_model.perceived_user_attachment != 0.0
        || tree.self_other_model.perceived_user_trust != 0.0
        || tree.self_other_model.perceived_user_safety != 0.0
        || tree.self_other_model.perceived_user_tension != 0.0;

    AffectSocialContext {
        user_id: current_username.to_string(),
        known: has_signal,
        metrics: SocialCoreMetrics {
            affinity: tree.relationship_core.affinity,
            attachment: tree.relationship_core.attachment,
            trust: tree.relationship_core.trust,
            safety: tree.relationship_core.safety,
            tension: tree.relationship_core.tension,
            context_depth,
        },
        illusion: IllusionMetrics {
            affinity: tree.self_other_model.perceived_user_affinity,
            attachment: tree.self_other_model.perceived_user_attachment,
            trust: tree.self_other_model.perceived_user_trust,
            safety: tree.self_other_model.perceived_user_safety,
            tension: tree.self_other_model.perceived_user_tension,
        },
    }
}

fn build_dialogue_summary_from_tree(
    current_username: &str,
    tree: &SocialTreeSnapshot,
) -> DialogueSocialSummary {
    let familiarity = match tree.derived_summaries.familiarity_bucket.as_str() {
        "close" => "close",
        "known" => "known",
        _ => "new",
    };

    let trust_state = match tree.derived_summaries.trust_state.as_str() {
        "stable" => "stable",
        "fragile" => "fragile",
        _ => "neutral",
    };

    let tension_state = match tree.derived_summaries.tension_state.as_str() {
        "high" => "high",
        "medium" => "medium",
        _ => "low",
    };

    let summary = if tree.derived_summaries.dialogue_summary_short.trim().is_empty() {
        format!(
            "[social summary] user={} familiarity={} trust={} tension={}.",
            current_username, familiarity, trust_state, tension_state
        )
    } else {
        tree.derived_summaries.dialogue_summary_short.clone()
    };

    DialogueSocialSummary {
        user_id: current_username.to_string(),
        familiarity,
        trust_state,
        tension_state,
        summary,
    }
}

fn build_known_affect_context(
    current_username: &str,
    attitudes: &AttitudesTowards,
    illusion: &IllusionOf,
    memory_hint: f32,
) -> AffectSocialContext {
    let graph_depth = (attitudes.affinity.abs()
        + attitudes.attachment.abs()
        + attitudes.trust.abs()
        + attitudes.safety.abs()) / 4.0;
    let context_depth = (graph_depth + memory_hint).min(1.0);

    AffectSocialContext {
        user_id: current_username.to_string(),
        known: true,
        metrics: SocialCoreMetrics {
            affinity: attitudes.affinity,
            attachment: attitudes.attachment,
            trust: attitudes.trust,
            safety: attitudes.safety,
            tension: attitudes.tension,
            context_depth,
        },
        illusion: IllusionMetrics {
            affinity: illusion.affinity,
            attachment: illusion.attachment,
            trust: illusion.trust,
            safety: illusion.safety,
            tension: illusion.tension,
        },
    }
}

fn build_default_affect_context(current_username: &str, memory_hint: f32) -> AffectSocialContext {
    AffectSocialContext {
        user_id: current_username.to_string(),
        known: false,
        metrics: SocialCoreMetrics {
            affinity: 0.0,
            attachment: 0.0,
            trust: 0.0,
            safety: 0.0,
            tension: 0.0,
            context_depth: memory_hint.min(1.0),
        },
        illusion: IllusionMetrics {
            affinity: 0.0,
            attachment: 0.0,
            trust: 0.0,
            safety: 0.0,
            tension: 0.0,
        },
    }
}
