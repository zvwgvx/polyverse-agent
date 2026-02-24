use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use surrealdb::engine::any::connect;
use surrealdb::engine::any::Any;
use surrealdb::Surreal;

#[derive(Clone)]
pub struct CognitiveGraph {
    pub db: Surreal<Any>,
}

impl CognitiveGraph {
    /// Initialize the SurrealDB engine for the Dual-Graph Cognitive System.
    pub async fn new(path: &str) -> Result<Self> {
        let endpoint = if path == "memory" {
            "mem://".to_string()
        } else {
            // using local embedded KV store
            format!("surrealkv://{}", path)
        };
        
        let db = connect(&endpoint)
            .await
            .context("Failed to connect to SurrealDB endpoint")?;
            
        // Use the default namespace and database for Ryuuko
        db.use_ns("polyverse").use_db("cognitive").await?;
        
        Ok(Self { db })
    }
}

// ==========================================
// 1. Social Knowledge Graph (SKGraph) Schema
// ==========================================

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Person {
    pub id: Option<String>, 
    pub name: String,
}

/// The Edge representing Ryuuko's actual attitude towards a User (R -> U)
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AttitudesTowards {
    pub affinity: f32,
    pub attachment: f32,
    pub trust: f32,
    pub safety: f32,
    pub tension: f32,
}

/// The Edge representing Ryuuko's projected illusion of how a User perceives her (U -> R)
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct IllusionOf {
    pub affinity: f32,
    pub attachment: f32,
    pub trust: f32,
    pub safety: f32,
    pub tension: f32,
}

// ==========================================
// 2. Emotion Graph Schema
// ==========================================

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Entity {
    pub id: Option<String>,
    pub name: String,
}

/// The Edge representing Ryuuko's feelings towards a concept/entity (R -> Entity)
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct FeelsAbout {
    pub preference: f32,
    pub stress: f32,
    pub fascination: f32,
}

// ==========================================
// 3. Delta Update Definitions (System 2)
// ==========================================

/// Max delta per single evaluation (hard limit)
const MAX_DELTA: f32 = 0.30;

fn clamp_delta(v: f32) -> f32 {
    v.clamp(-MAX_DELTA, MAX_DELTA)
}

/// Used to parse the exact JSON field names output by Gemini Flash evaluation
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SocialDelta {
    #[serde(default)] pub delta_affinity: f32,
    #[serde(default)] pub delta_attachment: f32,
    #[serde(default)] pub delta_trust: f32,
    #[serde(default)] pub delta_safety: f32,
    #[serde(default)] pub delta_tension: f32,
}

impl SocialDelta {
    /// Clamp all deltas to [-MAX_DELTA, +MAX_DELTA] to prevent LLM volatility
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
    /// Clamp all deltas to [-MAX_DELTA, +MAX_DELTA]
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
    /// Update Ryuuko's attitude towards a User (R -> U) using clamped Delta Accumulation
    pub async fn update_social_graph(&self, user_id: &str, delta: SocialDelta) -> Result<()> {
        let delta = delta.clamped(); // Hard clamp to prevent LLM volatility
        let edge_id = format!("ryuuko_{}", user_id.replace(['`', '"', '\''], ""));
        
        // Step 1: Ensure the record exists with zero defaults
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
        // Ignore error if already exists
        let _ = self.db.query(&ensure_query).await;
        
        // Step 2: Accumulate deltas on the existing record
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
    
    /// Update Ryuuko's projection of how User perceives her (U -> R)
    pub async fn update_illusion_graph(&self, user_id: &str, delta: SocialDelta) -> Result<()> {
        let delta = delta.clamped();
        let edge_id = format!("{}_ryuuko", user_id.replace(['`', '"', '\''], ""));
        
        // Step 1: Ensure the record exists
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
        
        // Step 2: Accumulate deltas
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
    
    /// Update Ryuuko's feelings towards an entity (R -> Entity)
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
    
    /// Update Ryuuko's observation of dynamics between two people (U1 -> U2)
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
        
        // Use SELECT VALUE to return scalar values (avoids 'Expected any, got record')
        // Also read last_updated as string for passive decay calculation
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
        
        // Helper: each SELECT VALUE returns Vec<f64> with 0 or 1 items
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
        
        // Passive decay: 1%/day toward 0 (decay_factor = 0.99^days_elapsed)
        fn calc_decay(timestamp_str: &str) -> f32 {
            if timestamp_str.is_empty() { return 1.0; }
            // SurrealDB time format: "2026-02-24T00:00:00Z"
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
        
        // This is index 1 because index 0 is the RELATE statement
        let extracted: Option<serde_json::Value> = response.take(1)?;
        println!("Extracted SQL Value: {:#?}", extracted);
        
        Ok(())
    }

    #[tokio::test]
    async fn test_upsert_extraction() -> Result<()> {
        let graph = CognitiveGraph::new("memory").await?;
        
        // Wipe to be sure
        graph.db.query("DELETE person; DELETE attitudes_towards;").await.unwrap();

        let s_delta = SocialDelta {
            delta_affinity: 0.25,
            delta_attachment: 0.1,
            delta_trust: 0.1,
            delta_safety: 0.1,
            delta_tension: 0.1,
        };

        // Call the exact update function
        graph.update_social_graph("tester_upsert", s_delta.clone()).await?;

        // Print raw DB contents
        let mut raw = graph.db.query("SELECT * FROM attitudes_towards").await.unwrap();
        let raw_val: Option<serde_json::Value> = raw.take(0).unwrap();
        println!("RAW DB CONTENTS: {:#?}", raw_val);

        // Try getting it back
        let (attitudes, _) = graph.get_social_context("tester_upsert").await?;
        println!("Extracted Attitudes: {:#?}", attitudes);

        // Add more to test accumulation
        graph.update_social_graph("tester_upsert", s_delta).await?;
        let (attitudes2, _) = graph.get_social_context("tester_upsert").await?;
        println!("Accumulated Attitudes: {:#?}", attitudes2);
        
        assert!(attitudes2.affinity > 0.0);
        Ok(())
    }
}
