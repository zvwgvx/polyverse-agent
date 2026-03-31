use pa_core::prompt_registry::render_prompt_or;
use pa_memory::graph::{AttitudesTowards, CognitiveGraph, IllusionOf, SocialTreeSnapshot};
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

pub async fn load_affect_social_context(
    graph: &CognitiveGraph,
    current_username: &str,
    memory_hint: f32,
) -> AffectSocialContext {
    match graph
        .get_or_project_social_tree_snapshot(current_username, memory_hint)
        .await
    {
        Ok(snapshot) => {
            log_tree_snapshot_for_request("affect", current_username, &snapshot, memory_hint);
            build_affect_context_from_tree(current_username, &snapshot, memory_hint)
        }
        Err(_) => match graph.get_social_context(current_username).await {
            Ok((attitudes, illusion)) => {
                build_known_affect_context(current_username, &attitudes, &illusion, memory_hint)
            }
            Err(_) => build_default_affect_context(current_username, memory_hint),
        },
    }
}

pub async fn load_dialogue_social_summary(
    graph: &CognitiveGraph,
    current_username: &str,
    memory_hint: f32,
) -> Option<DialogueSocialSummary> {
    if let Ok(snapshot) = graph
        .get_or_project_social_tree_snapshot(current_username, memory_hint)
        .await
    {
        log_tree_snapshot_for_request("dialogue", current_username, &snapshot, memory_hint);
        return Some(build_dialogue_summary_from_tree(current_username, &snapshot));
    }

    let (attitudes, _) = graph.get_social_context(current_username).await.ok()?;
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

    Some(DialogueSocialSummary {
        user_id: current_username.to_string(),
        familiarity,
        trust_state,
        tension_state,
        summary: format!(
            "[social summary] user={} familiarity={} trust={} tension={}.",
            current_username, familiarity, trust_state, tension_state
        ),
    })
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
