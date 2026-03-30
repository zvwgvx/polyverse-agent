use pa_core::prompt_registry::render_prompt_or;
use pa_memory::graph::{AttitudesTowards, CognitiveGraph, IllusionOf};

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
    match graph.get_social_context(current_username).await {
        Ok((attitudes, illusion)) => build_known_affect_context(current_username, &attitudes, &illusion, memory_hint),
        Err(_) => build_default_affect_context(current_username, memory_hint),
    }
}

pub async fn load_dialogue_social_summary(
    graph: &CognitiveGraph,
    current_username: &str,
    memory_hint: f32,
) -> Option<DialogueSocialSummary> {
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
