use anyhow::{Context, Result};
use pa_core::agent_profile::{get_agent_profile, sanitize_component};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use surrealdb::engine::any::connect;
use surrealdb::engine::any::Any;
use surrealdb::Surreal;

#[derive(Clone)]
pub struct CognitiveGraph {
    pub db: Surreal<Any>,
    agent_id: String,
    display_name: String,
    self_node_id: String,
}

impl CognitiveGraph {
    pub async fn new(path: &str) -> Result<Self> {
        let endpoint = if path == "memory" {
            "mem://".to_string()
        } else {
            format!("surrealkv://{}", path)
        };

        let db = connect(&endpoint)
            .await
            .context("Failed to connect to SurrealDB endpoint")?;

        db.use_ns("polyverse").use_db("cognitive").await?;

        let profile = get_agent_profile();

        Ok(Self {
            db,
            agent_id: sanitize_component(&profile.agent_id),
            display_name: profile.display_name.clone(),
            self_node_id: profile.graph_self_id.clone(),
        })
    }

    pub fn self_node_id(&self) -> &str {
        &self.self_node_id
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Person {
    pub id: Option<String>,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AttitudesTowards {
    pub affinity: f32,
    pub attachment: f32,
    pub trust: f32,
    pub safety: f32,
    pub tension: f32,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct IllusionOf {
    pub affinity: f32,
    pub attachment: f32,
    pub trust: f32,
    pub safety: f32,
    pub tension: f32,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SocialTreeRelationshipCore {
    pub affinity: f32,
    pub attachment: f32,
    pub trust: f32,
    pub safety: f32,
    pub tension: f32,
    pub familiarity: f32,
    pub boundary_reliability: f32,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SocialTreeDynamicState {
    pub tension_live: f32,
    pub warmth_live: f32,
    pub recent_shift: f32,
    pub last_turn_impact: f32,
    pub unresolved_friction_score: f32,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SocialTreeSelfOtherModel {
    pub perceived_user_affinity: f32,
    pub perceived_user_attachment: f32,
    pub perceived_user_trust: f32,
    pub perceived_user_safety: f32,
    pub perceived_user_tension: f32,
    pub confidence: f32,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SocialTreeDerivedSummaries {
    pub dialogue_summary_short: String,
    pub familiarity_bucket: String,
    pub trust_state: String,
    pub tension_state: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SocialTreeMeta {
    pub schema_version: String,
    pub updated_at: String,
    pub decay_policy: String,
    pub writer_version: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SocialTreeSnapshot {
    pub user_id: String,
    pub relationship_core: SocialTreeRelationshipCore,
    pub dynamic_state: SocialTreeDynamicState,
    pub self_other_model: SocialTreeSelfOtherModel,
    pub derived_summaries: SocialTreeDerivedSummaries,
    pub meta: SocialTreeMeta,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Entity {
    pub id: Option<String>,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct FeelsAbout {
    pub preference: f32,
    pub stress: f32,
    pub fascination: f32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RelationshipGraphSnapshot {
    pub self_node_id: String,
    pub self_display_name: String,
    pub nodes: Vec<RelationshipNode>,
    pub edges: Vec<RelationshipEdge>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RelationshipNode {
    pub id: String,
    pub label: String,
    pub kind: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RelationshipEdge {
    pub id: String,
    pub kind: String,
    pub source: String,
    pub target: String,
    pub affinity: Option<f32>,
    pub attachment: Option<f32>,
    pub trust: Option<f32>,
    pub safety: Option<f32>,
    pub tension: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct RawRelationshipEdge {
    source: String,
    target: String,
    #[serde(default)]
    affinity: Option<f32>,
    #[serde(default)]
    attachment: Option<f32>,
    #[serde(default)]
    trust: Option<f32>,
    #[serde(default)]
    safety: Option<f32>,
    #[serde(default)]
    tension: Option<f32>,
}

const MAX_DELTA: f32 = 0.30;

fn clamp_delta(v: f32) -> f32 {
    v.clamp(-MAX_DELTA, MAX_DELTA)
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SocialDelta {
    #[serde(default)]
    pub delta_affinity: f32,
    #[serde(default)]
    pub delta_attachment: f32,
    #[serde(default)]
    pub delta_trust: f32,
    #[serde(default)]
    pub delta_safety: f32,
    #[serde(default)]
    pub delta_tension: f32,
}

impl SocialDelta {
    pub fn clamped(self) -> Self {
        Self {
            delta_affinity: clamp_delta(self.delta_affinity),
            delta_attachment: clamp_delta(self.delta_attachment),
            delta_trust: clamp_delta(self.delta_trust),
            delta_safety: clamp_delta(self.delta_safety),
            delta_tension: clamp_delta(self.delta_tension),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EmotionDelta {
    pub entity_name: String,
    #[serde(default)]
    pub delta_preference: f32,
    #[serde(default)]
    pub delta_stress: f32,
    #[serde(default)]
    pub delta_fascination: f32,
}

impl EmotionDelta {
    pub fn clamped(self) -> Self {
        Self {
            entity_name: self.entity_name,
            delta_preference: clamp_delta(self.delta_preference),
            delta_stress: clamp_delta(self.delta_stress),
            delta_fascination: clamp_delta(self.delta_fascination),
        }
    }
}

fn clamp_unit(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

fn familiarity_bucket(context_depth: f32) -> &'static str {
    if context_depth >= 0.66 {
        "close"
    } else if context_depth >= 0.25 {
        "known"
    } else {
        "new"
    }
}

fn trust_state(trust: f32) -> &'static str {
    if trust >= 0.35 {
        "stable"
    } else if trust <= -0.20 {
        "fragile"
    } else {
        "neutral"
    }
}

fn tension_state(tension: f32) -> &'static str {
    if tension >= 0.55 {
        "high"
    } else if tension >= 0.25 {
        "medium"
    } else {
        "low"
    }
}

impl CognitiveGraph {
    pub async fn project_social_tree(&self, user_id: &str, memory_hint: f32) -> Result<SocialTreeSnapshot> {
        let safe_user_id = sanitize_component(user_id);
        let att_edge_id = format!("{}_{}", self.agent_id, safe_user_id);
        let ill_edge_id = format!("{}_{}", safe_user_id, self.agent_id);
        let root_id = safe_user_id.clone();

        let query = format!(
            r#"
            SELECT VALUE affinity FROM attitudes_towards:`{att}` LIMIT 1;
            SELECT VALUE attachment FROM attitudes_towards:`{att}` LIMIT 1;
            SELECT VALUE trust FROM attitudes_towards:`{att}` LIMIT 1;
            SELECT VALUE safety FROM attitudes_towards:`{att}` LIMIT 1;
            SELECT VALUE tension FROM attitudes_towards:`{att}` LIMIT 1;
            SELECT VALUE affinity FROM illusion_of:`{ill}` LIMIT 1;
            SELECT VALUE attachment FROM illusion_of:`{ill}` LIMIT 1;
            SELECT VALUE trust FROM illusion_of:`{ill}` LIMIT 1;
            SELECT VALUE safety FROM illusion_of:`{ill}` LIMIT 1;
            SELECT VALUE tension FROM illusion_of:`{ill}` LIMIT 1;
            SELECT VALUE string::concat("", last_updated) FROM attitudes_towards:`{att}` LIMIT 1;
            SELECT VALUE string::concat("", last_updated) FROM illusion_of:`{ill}` LIMIT 1;
            SELECT VALUE string::concat("", meta.updated_at) FROM social_tree_root:`{root}` LIMIT 1;
        "#,
            att = att_edge_id,
            ill = ill_edge_id,
            root = root_id
        );

        let mut response = self.db.query(&query).await?;

        fn extract_f32(response: &mut surrealdb::IndexedResults, idx: usize) -> f32 {
            let val: Result<Vec<f64>, _> = response.take(idx);
            match val {
                Ok(v) => v.first().copied().unwrap_or(0.0) as f32,
                Err(_) => 0.0,
            }
        }

        fn extract_string(response: &mut surrealdb::IndexedResults, idx: usize) -> String {
            response
                .take::<Vec<String>>(idx)
                .unwrap_or_default()
                .first()
                .cloned()
                .unwrap_or_default()
        }

        fn calc_decay(timestamp_str: &str) -> f32 {
            if timestamp_str.is_empty() {
                return 1.0;
            }
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(timestamp_str) {
                let elapsed = chrono::Utc::now().signed_duration_since(dt);
                let days = elapsed.num_hours() as f64 / 24.0;
                if days > 0.0 {
                    return (0.99_f64).powf(days) as f32;
                }
            }
            1.0
        }

        let raw_att = AttitudesTowards {
            affinity: extract_f32(&mut response, 0),
            attachment: extract_f32(&mut response, 1),
            trust: extract_f32(&mut response, 2),
            safety: extract_f32(&mut response, 3),
            tension: extract_f32(&mut response, 4),
        };

        let raw_ill = IllusionOf {
            affinity: extract_f32(&mut response, 5),
            attachment: extract_f32(&mut response, 6),
            trust: extract_f32(&mut response, 7),
            safety: extract_f32(&mut response, 8),
            tension: extract_f32(&mut response, 9),
        };

        let att_ts = extract_string(&mut response, 10);
        let ill_ts = extract_string(&mut response, 11);
        let existing_updated_at = extract_string(&mut response, 12);
        let att_decay = calc_decay(&att_ts);
        let ill_decay = calc_decay(&ill_ts);

        let attitudes = AttitudesTowards {
            affinity: raw_att.affinity * att_decay,
            attachment: raw_att.attachment * att_decay,
            trust: raw_att.trust * att_decay,
            safety: raw_att.safety * att_decay,
            tension: raw_att.tension * att_decay,
        };

        let illusion = IllusionOf {
            affinity: raw_ill.affinity * ill_decay,
            attachment: raw_ill.attachment * ill_decay,
            trust: raw_ill.trust * ill_decay,
            safety: raw_ill.safety * ill_decay,
            tension: raw_ill.tension * ill_decay,
        };

        let graph_depth = (
            attitudes.affinity.abs()
                + attitudes.attachment.abs()
                + attitudes.trust.abs()
                + attitudes.safety.abs()
        ) / 4.0;
        let context_depth = clamp_unit(graph_depth + memory_hint.min(1.0));

        let familiarity = familiarity_bucket(context_depth);
        let trust = trust_state(attitudes.trust);
        let tension = tension_state(attitudes.tension);

        let snapshot = SocialTreeSnapshot {
            user_id: user_id.to_string(),
            relationship_core: SocialTreeRelationshipCore {
                affinity: attitudes.affinity,
                attachment: attitudes.attachment,
                trust: attitudes.trust,
                safety: attitudes.safety,
                tension: attitudes.tension,
                familiarity: context_depth,
                boundary_reliability: attitudes.safety,
            },
            dynamic_state: SocialTreeDynamicState {
                tension_live: clamp_unit((attitudes.tension + 1.0) / 2.0),
                warmth_live: clamp_unit((attitudes.affinity + attitudes.safety + 2.0) / 4.0),
                recent_shift: 0.0,
                last_turn_impact: 0.0,
                unresolved_friction_score: clamp_unit((attitudes.tension + 1.0) / 2.0),
            },
            self_other_model: SocialTreeSelfOtherModel {
                perceived_user_affinity: illusion.affinity,
                perceived_user_attachment: illusion.attachment,
                perceived_user_trust: illusion.trust,
                perceived_user_safety: illusion.safety,
                perceived_user_tension: illusion.tension,
                confidence: context_depth,
            },
            derived_summaries: SocialTreeDerivedSummaries {
                dialogue_summary_short: format!(
                    "[social summary] user={} familiarity={} trust={} tension={}.",
                    user_id, familiarity, trust, tension
                ),
                familiarity_bucket: familiarity.to_string(),
                trust_state: trust.to_string(),
                tension_state: tension.to_string(),
            },
            meta: SocialTreeMeta {
                schema_version: "v1".to_string(),
                updated_at: existing_updated_at,
                decay_policy: "graph_decay_0.99_per_day".to_string(),
                writer_version: "graph_projection_v1".to_string(),
            },
        };

        let upsert_query = format!(
            r#"
            UPSERT social_tree_root:`{root}` CONTENT {{
                user_id: $user_id,
                relationship_core: {{
                    affinity: $rc_affinity,
                    attachment: $rc_attachment,
                    trust: $rc_trust,
                    safety: $rc_safety,
                    tension: $rc_tension,
                    familiarity: $rc_familiarity,
                    boundary_reliability: $rc_boundary_reliability
                }},
                dynamic_state: {{
                    tension_live: $ds_tension_live,
                    warmth_live: $ds_warmth_live,
                    recent_shift: $ds_recent_shift,
                    last_turn_impact: $ds_last_turn_impact,
                    unresolved_friction_score: $ds_unresolved_friction_score
                }},
                self_other_model: {{
                    perceived_user_affinity: $so_perceived_user_affinity,
                    perceived_user_attachment: $so_perceived_user_attachment,
                    perceived_user_trust: $so_perceived_user_trust,
                    perceived_user_safety: $so_perceived_user_safety,
                    perceived_user_tension: $so_perceived_user_tension,
                    confidence: $so_confidence
                }},
                derived_summaries: {{
                    dialogue_summary_short: $dsm_dialogue_summary_short,
                    familiarity_bucket: $dsm_familiarity_bucket,
                    trust_state: $dsm_trust_state,
                    tension_state: $dsm_tension_state
                }},
                meta: {{
                    schema_version: "v1",
                    updated_at: time::now(),
                    decay_policy: "graph_decay_0.99_per_day",
                    writer_version: "graph_projection_v1"
                }}
            }};
        "#,
            root = root_id
        );

        self.db
            .query(&upsert_query)
            .bind(("user_id", user_id.to_string()))
            .bind(("rc_affinity", snapshot.relationship_core.affinity))
            .bind(("rc_attachment", snapshot.relationship_core.attachment))
            .bind(("rc_trust", snapshot.relationship_core.trust))
            .bind(("rc_safety", snapshot.relationship_core.safety))
            .bind(("rc_tension", snapshot.relationship_core.tension))
            .bind(("rc_familiarity", snapshot.relationship_core.familiarity))
            .bind((
                "rc_boundary_reliability",
                snapshot.relationship_core.boundary_reliability,
            ))
            .bind(("ds_tension_live", snapshot.dynamic_state.tension_live))
            .bind(("ds_warmth_live", snapshot.dynamic_state.warmth_live))
            .bind(("ds_recent_shift", snapshot.dynamic_state.recent_shift))
            .bind(("ds_last_turn_impact", snapshot.dynamic_state.last_turn_impact))
            .bind((
                "ds_unresolved_friction_score",
                snapshot.dynamic_state.unresolved_friction_score,
            ))
            .bind((
                "so_perceived_user_affinity",
                snapshot.self_other_model.perceived_user_affinity,
            ))
            .bind((
                "so_perceived_user_attachment",
                snapshot.self_other_model.perceived_user_attachment,
            ))
            .bind((
                "so_perceived_user_trust",
                snapshot.self_other_model.perceived_user_trust,
            ))
            .bind((
                "so_perceived_user_safety",
                snapshot.self_other_model.perceived_user_safety,
            ))
            .bind((
                "so_perceived_user_tension",
                snapshot.self_other_model.perceived_user_tension,
            ))
            .bind(("so_confidence", snapshot.self_other_model.confidence))
            .bind((
                "dsm_dialogue_summary_short",
                snapshot.derived_summaries.dialogue_summary_short.clone(),
            ))
            .bind((
                "dsm_familiarity_bucket",
                snapshot.derived_summaries.familiarity_bucket.clone(),
            ))
            .bind((
                "dsm_trust_state",
                snapshot.derived_summaries.trust_state.clone(),
            ))
            .bind((
                "dsm_tension_state",
                snapshot.derived_summaries.tension_state.clone(),
            ))
            .await
            .context("Failed to UPSERT social tree root")?;

        let mut returned = snapshot;
        returned.meta.updated_at = chrono::Utc::now().to_rfc3339();
        Ok(returned)
    }

    pub async fn get_social_tree_snapshot(&self, user_id: &str) -> Result<SocialTreeSnapshot> {
        let query = r#"
            SELECT user_id, relationship_core, dynamic_state, self_other_model, derived_summaries, meta
            FROM social_tree_root
            WHERE user_id = $user_id
            LIMIT 1;
        "#;

        let mut response = self
            .db
            .query(query)
            .bind(("user_id", user_id.to_string()))
            .await?;
        let rows: Vec<serde_json::Value> = response.take(0).unwrap_or_default();

        if let Some(raw) = rows.into_iter().next() {
            let user_id = raw
                .get("user_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| user_id.to_string());

            let relationship_core = raw
                .get("relationship_core")
                .cloned()
                .and_then(|v| serde_json::from_value::<SocialTreeRelationshipCore>(v).ok())
                .unwrap_or_default();

            let dynamic_state = raw
                .get("dynamic_state")
                .cloned()
                .and_then(|v| serde_json::from_value::<SocialTreeDynamicState>(v).ok())
                .unwrap_or_default();

            let self_other_model = raw
                .get("self_other_model")
                .cloned()
                .and_then(|v| serde_json::from_value::<SocialTreeSelfOtherModel>(v).ok())
                .unwrap_or_default();

            let derived_summaries = raw
                .get("derived_summaries")
                .cloned()
                .and_then(|v| serde_json::from_value::<SocialTreeDerivedSummaries>(v).ok())
                .unwrap_or_default();

            let meta = raw
                .get("meta")
                .cloned()
                .and_then(|v| serde_json::from_value::<SocialTreeMeta>(v).ok())
                .unwrap_or_default();

            let mut snapshot = SocialTreeSnapshot {
                user_id,
                relationship_core,
                dynamic_state,
                self_other_model,
                derived_summaries,
                meta,
            };

            if snapshot.meta.schema_version.is_empty() {
                snapshot.meta.schema_version = "v1".to_string();
            }
            if snapshot.meta.updated_at.is_empty() {
                snapshot.meta.updated_at = chrono::Utc::now().to_rfc3339();
            }
            if snapshot.meta.decay_policy.is_empty() {
                snapshot.meta.decay_policy = "graph_decay_0.99_per_day".to_string();
            }
            if snapshot.meta.writer_version.is_empty() {
                snapshot.meta.writer_version = "graph_projection_v1".to_string();
            }

            return Ok(snapshot);
        }

        anyhow::bail!("social tree root not found")
    }

    pub async fn get_or_project_social_tree_snapshot(
        &self,
        user_id: &str,
        memory_hint: f32,
    ) -> Result<SocialTreeSnapshot> {
        match self.get_social_tree_snapshot(user_id).await {
            Ok(snapshot) => Ok(snapshot),
            Err(_) => self.project_social_tree(user_id, memory_hint).await,
        }
    }

    pub async fn update_social_graph(&self, user_id: &str, delta: SocialDelta) -> Result<()> {
        let delta = delta.clamped();
        let safe_user_id = sanitize_component(user_id);
        let edge_id = format!("{}_{}", self.agent_id, safe_user_id);

        let ensure_query = format!(
            r#"
            CREATE attitudes_towards:`{}` CONTENT {{
                in: {},
                out: person:`{}`,
                affinity: 0.0,
                attachment: 0.0,
                trust: 0.0,
                safety: 0.0,
                tension: 0.0,
                last_updated: time::now()
            }};
        "#,
            edge_id, self.self_node_id, safe_user_id
        );
        let _ = self.db.query(&ensure_query).await;

        let update_query = format!(
            r#"
            UPDATE attitudes_towards:`{}` SET
                affinity = math::clamp(affinity + $delta_affinity, -1.0, 1.0),
                attachment = math::clamp(attachment + $delta_attachment, -1.0, 1.0),
                trust = math::clamp(trust + $delta_trust, -1.0, 1.0),
                safety = math::clamp(safety + $delta_safety, -1.0, 1.0),
                tension = math::clamp(tension + $delta_tension, -1.0, 1.0),
                last_updated = time::now();
        "#,
            edge_id
        );

        self.db
            .query(&update_query)
            .bind(("delta_affinity", delta.delta_affinity))
            .bind(("delta_attachment", delta.delta_attachment))
            .bind(("delta_trust", delta.delta_trust))
            .bind(("delta_safety", delta.delta_safety))
            .bind(("delta_tension", delta.delta_tension))
            .await
            .context("Failed to UPDATE social graph")?;

        tracing::info!(user = %user_id, self_node = %self.self_node_id, "Social graph updated");

        Ok(())
    }

    pub async fn update_illusion_graph(&self, user_id: &str, delta: SocialDelta) -> Result<()> {
        let delta = delta.clamped();
        let safe_user_id = sanitize_component(user_id);
        let edge_id = format!("{}_{}", safe_user_id, self.agent_id);

        let ensure_query = format!(
            r#"
            CREATE illusion_of:`{}` CONTENT {{
                in: person:`{}`,
                out: {},
                affinity: 0.0,
                attachment: 0.0,
                trust: 0.0,
                safety: 0.0,
                tension: 0.0,
                last_updated: time::now()
            }};
        "#,
            edge_id, safe_user_id, self.self_node_id
        );
        let _ = self.db.query(&ensure_query).await;

        let update_query = format!(
            r#"
            UPDATE illusion_of:`{}` SET
                affinity = math::clamp(affinity + $delta_affinity, -1.0, 1.0),
                attachment = math::clamp(attachment + $delta_attachment, -1.0, 1.0),
                trust = math::clamp(trust + $delta_trust, -1.0, 1.0),
                safety = math::clamp(safety + $delta_safety, -1.0, 1.0),
                tension = math::clamp(tension + $delta_tension, -1.0, 1.0),
                last_updated = time::now();
        "#,
            edge_id
        );

        self.db
            .query(&update_query)
            .bind(("delta_affinity", delta.delta_affinity))
            .bind(("delta_attachment", delta.delta_attachment))
            .bind(("delta_trust", delta.delta_trust))
            .bind(("delta_safety", delta.delta_safety))
            .bind(("delta_tension", delta.delta_tension))
            .await
            .context("Failed to UPDATE illusion graph")?;

        tracing::info!(user = %user_id, self_node = %self.self_node_id, "Illusion graph updated");

        Ok(())
    }

    pub async fn update_emotion_graph(&self, entity_name: &str, delta: EmotionDelta) -> Result<()> {
        let delta = delta.clamped();
        let edge_id = format!("{}_{}", self.agent_id, sanitize_component(entity_name));
        let query = format!(
            r#"
            UPDATE feels_about:`{}` SET
                in = {},
                out = entity:`{}`,
                preference = math::clamp((preference OR 0.0) + $delta_preference, -1.0, 1.0),
                stress = math::clamp((stress OR 0.0) + $delta_stress, -1.0, 1.0),
                fascination = math::clamp((fascination OR 0.0) + $delta_fascination, -1.0, 1.0);
        "#,
            edge_id, self.self_node_id, entity_name
        );

        self.db
            .query(&query)
            .bind(("delta_preference", delta.delta_preference))
            .bind(("delta_stress", delta.delta_stress))
            .bind(("delta_fascination", delta.delta_fascination))
            .await?;

        Ok(())
    }

    pub async fn update_observed_dynamic(&self, from_user: &str, to_user: &str, tension: f32) -> Result<()> {
        let edge_id = format!(
            "{}_{}",
            sanitize_component(from_user),
            sanitize_component(to_user)
        );
        let query = format!(
            r#"
            UPDATE interacts_with:`{}` SET
                in = person:`{}`,
                out = person:`{}`,
                tension = math::clamp((tension OR 0.0) + $tension, -1.0, 1.0);
        "#,
            edge_id, from_user, to_user
        );

        self.db.query(&query).bind(("tension", tension)).await?;

        Ok(())
    }

    pub async fn get_social_context(&self, user_id: &str) -> Result<(AttitudesTowards, IllusionOf)> {
        let safe_user_id = sanitize_component(user_id);
        let att_edge_id = format!("{}_{}", self.agent_id, safe_user_id);
        let ill_edge_id = format!("{}_{}", safe_user_id, self.agent_id);

        let query = format!(
            r#"
            SELECT VALUE affinity FROM attitudes_towards:`{att}` LIMIT 1;
            SELECT VALUE attachment FROM attitudes_towards:`{att}` LIMIT 1;
            SELECT VALUE trust FROM attitudes_towards:`{att}` LIMIT 1;
            SELECT VALUE safety FROM attitudes_towards:`{att}` LIMIT 1;
            SELECT VALUE tension FROM attitudes_towards:`{att}` LIMIT 1;
            SELECT VALUE affinity FROM illusion_of:`{ill}` LIMIT 1;
            SELECT VALUE attachment FROM illusion_of:`{ill}` LIMIT 1;
            SELECT VALUE trust FROM illusion_of:`{ill}` LIMIT 1;
            SELECT VALUE safety FROM illusion_of:`{ill}` LIMIT 1;
            SELECT VALUE tension FROM illusion_of:`{ill}` LIMIT 1;
            SELECT VALUE string::concat("", last_updated) FROM attitudes_towards:`{att}` LIMIT 1;
            SELECT VALUE string::concat("", last_updated) FROM illusion_of:`{ill}` LIMIT 1;
        "#,
            att = att_edge_id,
            ill = ill_edge_id
        );

        let mut response = self.db.query(&query).await?;

        fn extract_f32(response: &mut surrealdb::IndexedResults, idx: usize) -> f32 {
            let val: Result<Vec<f64>, _> = response.take(idx);
            match val {
                Ok(v) => v.first().copied().unwrap_or(0.0) as f32,
                Err(_) => 0.0,
            }
        }

        fn extract_string(response: &mut surrealdb::IndexedResults, idx: usize) -> String {
            response
                .take::<Vec<String>>(idx)
                .unwrap_or_default()
                .first()
                .cloned()
                .unwrap_or_default()
        }

        fn calc_decay(timestamp_str: &str) -> f32 {
            if timestamp_str.is_empty() {
                return 1.0;
            }
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(timestamp_str) {
                let elapsed = chrono::Utc::now().signed_duration_since(dt);
                let days = elapsed.num_hours() as f64 / 24.0;
                if days > 0.0 {
                    return (0.99_f64).powf(days) as f32;
                }
            }
            1.0
        }

        let raw_att = AttitudesTowards {
            affinity: extract_f32(&mut response, 0),
            attachment: extract_f32(&mut response, 1),
            trust: extract_f32(&mut response, 2),
            safety: extract_f32(&mut response, 3),
            tension: extract_f32(&mut response, 4),
        };

        let raw_ill = IllusionOf {
            affinity: extract_f32(&mut response, 5),
            attachment: extract_f32(&mut response, 6),
            trust: extract_f32(&mut response, 7),
            safety: extract_f32(&mut response, 8),
            tension: extract_f32(&mut response, 9),
        };

        let att_ts = extract_string(&mut response, 10);
        let ill_ts = extract_string(&mut response, 11);
        let att_decay = calc_decay(&att_ts);
        let ill_decay = calc_decay(&ill_ts);

        let attitudes = AttitudesTowards {
            affinity: raw_att.affinity * att_decay,
            attachment: raw_att.attachment * att_decay,
            trust: raw_att.trust * att_decay,
            safety: raw_att.safety * att_decay,
            tension: raw_att.tension * att_decay,
        };

        let illusion = IllusionOf {
            affinity: raw_ill.affinity * ill_decay,
            attachment: raw_ill.attachment * ill_decay,
            trust: raw_ill.trust * ill_decay,
            safety: raw_ill.safety * ill_decay,
            tension: raw_ill.tension * ill_decay,
        };

        Ok((attitudes, illusion))
    }

    pub async fn snapshot_relationship_graph(&self) -> Result<RelationshipGraphSnapshot> {
        let query = r#"
            SELECT
                string::concat("", `in`) AS source,
                string::concat("", `out`) AS target,
                affinity,
                attachment,
                trust,
                safety,
                tension
            FROM attitudes_towards;
            SELECT
                string::concat("", `in`) AS source,
                string::concat("", `out`) AS target,
                affinity,
                attachment,
                trust,
                safety,
                tension
            FROM illusion_of;
            SELECT
                string::concat("", `in`) AS source,
                string::concat("", `out`) AS target,
                tension
            FROM interacts_with;
        "#;

        let mut response = self.db.query(query).await?;
        let social_edges = take_relationship_edges(&mut response, 0);
        let illusion_edges = take_relationship_edges(&mut response, 1);
        let dynamic_edges = take_relationship_edges(&mut response, 2);

        let mut edges = Vec::new();
        let mut nodes = HashMap::new();

        for (kind, raw_edges) in [
            ("social", social_edges),
            ("illusion", illusion_edges),
            ("observed_dynamic", dynamic_edges),
        ] {
            for raw in raw_edges {
                let source = normalize_record_id(&raw.source);
                let target = normalize_record_id(&raw.target);

                nodes.entry(source.clone()).or_insert_with(|| {
                    build_relationship_node(&source, &self.self_node_id, &self.display_name)
                });
                nodes.entry(target.clone()).or_insert_with(|| {
                    build_relationship_node(&target, &self.self_node_id, &self.display_name)
                });

                edges.push(RelationshipEdge {
                    id: format!("{}:{}->{}", kind, source, target),
                    kind: kind.to_string(),
                    source,
                    target,
                    affinity: raw.affinity,
                    attachment: raw.attachment,
                    trust: raw.trust,
                    safety: raw.safety,
                    tension: raw.tension,
                });
            }
        }

        if !nodes.contains_key(&self.self_node_id) {
            nodes.insert(
                self.self_node_id.clone(),
                build_relationship_node(&self.self_node_id, &self.self_node_id, &self.display_name),
            );
        }

        let mut node_list: Vec<RelationshipNode> = nodes.into_values().collect();
        node_list.sort_by(|a, b| a.id.cmp(&b.id));
        edges.sort_by(|a, b| a.id.cmp(&b.id));

        Ok(RelationshipGraphSnapshot {
            self_node_id: self.self_node_id.clone(),
            self_display_name: self.display_name.clone(),
            nodes: node_list,
            edges,
        })
    }
}

fn normalize_record_id(value: &str) -> String {
    value.trim_matches('"').to_string()
}

fn take_relationship_edges(
    response: &mut surrealdb::IndexedResults,
    idx: usize,
) -> Vec<RawRelationshipEdge> {
    let raw_values: Vec<serde_json::Value> = response.take(idx).unwrap_or_default();
    raw_values
        .into_iter()
        .filter_map(|value| serde_json::from_value::<RawRelationshipEdge>(value).ok())
        .collect()
}

fn build_relationship_node(id: &str, self_node_id: &str, self_display_name: &str) -> RelationshipNode {
    let kind = if id == self_node_id {
        "agent"
    } else if id.starts_with("person:") {
        "person"
    } else if id.starts_with("entity:") {
        "entity"
    } else {
        "unknown"
    };

    let label = if id == self_node_id {
        self_display_name.to_string()
    } else {
        id.rsplit(':').next().unwrap_or(id).to_string()
    };

    RelationshipNode {
        id: id.to_string(),
        label,
        kind: kind.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_surreal_extraction() -> Result<()> {
        let graph = CognitiveGraph::new("memory").await?;

        let query = format!(
            r#"
            RELATE {}->attitudes_towards->person:tester
            SET affinity = 0.5, attachment = 0.2;

            SELECT affinity, attachment, trust, safety, tension FROM {}->attitudes_towards WHERE out = person:tester;
        "#,
            graph.self_node_id, graph.self_node_id
        );

        let mut response = graph.db.query(query).await?;

        let extracted: Option<serde_json::Value> = response.take(1)?;
        println!("Extracted SQL Value: {:#?}", extracted);

        Ok(())
    }

    #[tokio::test]
    async fn test_upsert_extraction() -> Result<()> {
        let graph = CognitiveGraph::new("memory").await?;

        graph
            .db
            .query("DELETE person; DELETE attitudes_towards;")
            .await
            .unwrap();

        let s_delta = SocialDelta {
            delta_affinity: 0.25,
            delta_attachment: 0.1,
            delta_trust: 0.1,
            delta_safety: 0.1,
            delta_tension: 0.1,
        };

        graph.update_social_graph("tester_upsert", s_delta.clone()).await?;

        let (attitudes, _) = graph.get_social_context("tester_upsert").await?;
        println!("Extracted Attitudes: {:#?}", attitudes);

        graph.update_social_graph("tester_upsert", s_delta).await?;
        let (attitudes2, _) = graph.get_social_context("tester_upsert").await?;
        println!("Accumulated Attitudes: {:#?}", attitudes2);

        assert!(attitudes2.affinity > 0.0);
        Ok(())
    }

    #[tokio::test]
    async fn test_social_tree_projection_and_readback() -> Result<()> {
        let graph = CognitiveGraph::new("memory").await?;

        graph
            .db
            .query("DELETE person; DELETE attitudes_towards; DELETE illusion_of; DELETE social_tree_root;")
            .await
            .unwrap();

        graph
            .update_social_graph(
                "alice",
                SocialDelta {
                    delta_affinity: 0.4,
                    delta_attachment: 0.2,
                    delta_trust: -0.1,
                    delta_safety: 0.15,
                    delta_tension: 0.3,
                },
            )
            .await?;

        graph
            .update_illusion_graph(
                "alice",
                SocialDelta {
                    delta_affinity: 0.1,
                    delta_attachment: 0.05,
                    delta_trust: -0.05,
                    delta_safety: 0.1,
                    delta_tension: 0.2,
                },
            )
            .await?;

        let projected = graph.project_social_tree("alice", 0.12).await?;
        assert_eq!(projected.user_id, "alice");
        assert!(projected.relationship_core.affinity > 0.0);
        assert!(projected.relationship_core.familiarity >= 0.12);
        assert!(!projected.derived_summaries.dialogue_summary_short.is_empty());

        let readback = graph
            .get_or_project_social_tree_snapshot("alice", 0.12)
            .await?;
        assert_eq!(readback.user_id, "alice");
        assert_eq!(readback.meta.schema_version, "v1");
        assert_eq!(
            readback.derived_summaries.familiarity_bucket,
            projected.derived_summaries.familiarity_bucket
        );
        Ok(())
    }
}
