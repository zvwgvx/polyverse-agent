use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use surrealdb::engine::any::connect;
use surrealdb::engine::any::Any;
use surrealdb::Surreal;
use std::collections::HashMap;

#[derive(Clone)]
pub struct CognitiveGraph {
    pub db: Surreal<Any>,
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
        
        Ok(Self { db })
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
    #[serde(default)] pub delta_affinity: f32,
    #[serde(default)] pub delta_attachment: f32,
    #[serde(default)] pub delta_trust: f32,
    #[serde(default)] pub delta_safety: f32,
    #[serde(default)] pub delta_tension: f32,
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
    #[serde(default)] pub delta_preference: f32,
    #[serde(default)] pub delta_stress: f32,
    #[serde(default)] pub delta_fascination: f32,
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

impl CognitiveGraph {
    pub async fn update_social_graph(&self, user_id: &str, delta: SocialDelta) -> Result<()> {
        let delta = delta.clamped();
        let edge_id = format!("ryuuko_{}", user_id.replace(['`', '"', '\''], ""));
        
        let ensure_query = format!(r#"
            CREATE attitudes_towards:`{}` CONTENT {{
                in: person:ryuuko,
                out: person:`{}`,
                affinity: 0.0,
                attachment: 0.0,
                trust: 0.0,
                safety: 0.0,
                tension: 0.0,
                last_updated: time::now()
            }};
        "#, edge_id, user_id);
        let _ = self.db.query(&ensure_query).await;
        
        let update_query = format!(r#"
            UPDATE attitudes_towards:`{}` SET 
                affinity = math::clamp(affinity + $delta_affinity, -1.0, 1.0),
                attachment = math::clamp(attachment + $delta_attachment, -1.0, 1.0),
                trust = math::clamp(trust + $delta_trust, -1.0, 1.0),
                safety = math::clamp(safety + $delta_safety, -1.0, 1.0),
                tension = math::clamp(tension + $delta_tension, -1.0, 1.0),
                last_updated = time::now();
        "#, edge_id);
        
        self.db.query(&update_query)
            .bind(("delta_affinity", delta.delta_affinity))
            .bind(("delta_attachment", delta.delta_attachment))
            .bind(("delta_trust", delta.delta_trust))
            .bind(("delta_safety", delta.delta_safety))
            .bind(("delta_tension", delta.delta_tension))
            .await
            .context("Failed to UPDATE social graph")?;
        
        tracing::info!(user = %user_id, "Social graph updated");
            
        Ok(())
    }
    
    pub async fn update_illusion_graph(&self, user_id: &str, delta: SocialDelta) -> Result<()> {
        let delta = delta.clamped();
        let edge_id = format!("{}_ryuuko", user_id.replace(['`', '"', '\''], ""));
        
        let ensure_query = format!(r#"
            CREATE illusion_of:`{}` CONTENT {{
                in: person:`{}`,
                out: person:ryuuko,
                affinity: 0.0,
                attachment: 0.0,
                trust: 0.0,
                safety: 0.0,
                tension: 0.0,
                last_updated: time::now()
            }};
        "#, edge_id, user_id);
        let _ = self.db.query(&ensure_query).await;
        
        let update_query = format!(r#"
            UPDATE illusion_of:`{}` SET 
                affinity = math::clamp(affinity + $delta_affinity, -1.0, 1.0),
                attachment = math::clamp(attachment + $delta_attachment, -1.0, 1.0),
                trust = math::clamp(trust + $delta_trust, -1.0, 1.0),
                safety = math::clamp(safety + $delta_safety, -1.0, 1.0),
                tension = math::clamp(tension + $delta_tension, -1.0, 1.0),
                last_updated = time::now();
        "#, edge_id);
        
        self.db.query(&update_query)
            .bind(("delta_affinity", delta.delta_affinity))
            .bind(("delta_attachment", delta.delta_attachment))
            .bind(("delta_trust", delta.delta_trust))
            .bind(("delta_safety", delta.delta_safety))
            .bind(("delta_tension", delta.delta_tension))
            .await
            .context("Failed to UPDATE illusion graph")?;
        
        tracing::info!(user = %user_id, "Illusion graph updated");
            
        Ok(())
    }
    
    pub async fn update_emotion_graph(&self, entity_name: &str, delta: EmotionDelta) -> Result<()> {
        let delta = delta.clamped();
        let edge_id = format!("ryuuko_{}", entity_name.replace(['`', '"', '\''], ""));
        let query = format!(r#"
            UPDATE feels_about:`{}` SET 
                in = person:ryuuko,
                out = entity:`{}`,
                preference = math::clamp((preference OR 0.0) + $delta_preference, -1.0, 1.0),
                stress = math::clamp((stress OR 0.0) + $delta_stress, -1.0, 1.0),
                fascination = math::clamp((fascination OR 0.0) + $delta_fascination, -1.0, 1.0);
        "#, edge_id, entity_name);
        
        let _response = self.db.query(&query)
            .bind(("delta_preference", delta.delta_preference))
            .bind(("delta_stress", delta.delta_stress))
            .bind(("delta_fascination", delta.delta_fascination))
            .await?;
            
        Ok(())
    }
    
    pub async fn update_observed_dynamic(&self, from_user: &str, to_user: &str, tension: f32) -> Result<()> {
        let edge_id = format!("{}_{}", from_user.replace(['`', '"', '\''], ""), to_user.replace(['`', '"', '\''], ""));
        let query = format!(r#"
            UPDATE interacts_with:`{}` SET 
                in = person:`{}`,
                out = person:`{}`,
                tension = math::clamp((tension OR 0.0) + $tension, -1.0, 1.0);
        "#, edge_id, from_user, to_user);
        
        let _response = self.db.query(&query)
            .bind(("tension", tension))
            .await?;
            
        Ok(())
    }
    
    pub async fn get_social_context(&self, user_id: &str) -> Result<(AttitudesTowards, IllusionOf)> {
        let att_edge_id = format!("ryuuko_{}", user_id.replace(['`', '"', '\''], ""));
        let ill_edge_id = format!("{}_ryuuko", user_id.replace(['`', '"', '\''], ""));
        
        let query = format!(r#"
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
        "#, att = att_edge_id, ill = ill_edge_id);
        
        let mut response = self.db.query(&query).await?;
        
        fn extract_f32(response: &mut surrealdb::IndexedResults, idx: usize) -> f32 {
            let val: Result<Vec<f64>, _> = response.take(idx);
            match val {
                Ok(v) => v.first().copied().unwrap_or(0.0) as f32,
                Err(_) => 0.0,
            }
        }
        
        fn extract_string(response: &mut surrealdb::IndexedResults, idx: usize) -> String {
            response.take::<Vec<String>>(idx)
                .unwrap_or_default()
                .first().cloned().unwrap_or_default()
        }
        
        fn calc_decay(timestamp_str: &str) -> f32 {
            if timestamp_str.is_empty() { return 1.0; }
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

                nodes
                    .entry(source.clone())
                    .or_insert_with(|| build_relationship_node(&source));
                nodes
                    .entry(target.clone())
                    .or_insert_with(|| build_relationship_node(&target));

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

        if !nodes.contains_key("person:ryuuko") {
            nodes.insert(
                "person:ryuuko".to_string(),
                build_relationship_node("person:ryuuko"),
            );
        }

        let mut node_list: Vec<RelationshipNode> = nodes.into_values().collect();
        node_list.sort_by(|a, b| a.id.cmp(&b.id));
        edges.sort_by(|a, b| a.id.cmp(&b.id));

        Ok(RelationshipGraphSnapshot {
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

fn build_relationship_node(id: &str) -> RelationshipNode {
    let label = id.rsplit(':').next().unwrap_or(id).to_string();
    let kind = if id == "person:ryuuko" {
        "agent"
    } else if id.starts_with("person:") {
        "person"
    } else if id.starts_with("entity:") {
        "entity"
    } else {
        "unknown"
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
        
        let mut response = graph.db.query(r#"
            RELATE person:ryuuko->attitudes_towards->person:tester 
            SET affinity = 0.5, attachment = 0.2;
            
            SELECT affinity, attachment, trust, safety, tension FROM person:ryuuko->attitudes_towards WHERE out = person:tester;
        "#).await?;
        
        let extracted: Option<serde_json::Value> = response.take(1)?;
        println!("Extracted SQL Value: {:#?}", extracted);
        
        Ok(())
    }

    #[tokio::test]
    async fn test_upsert_extraction() -> Result<()> {
        let graph = CognitiveGraph::new("memory").await?;
        
        graph.db.query("DELETE person; DELETE attitudes_towards;").await.unwrap();

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
}
