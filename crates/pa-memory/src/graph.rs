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

/// Used to parse the exact JSON field names output by Gemini Flash evaluation
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SocialDelta {
    #[serde(default)] pub delta_affinity: f32,
    #[serde(default)] pub delta_attachment: f32,
    #[serde(default)] pub delta_trust: f32,
    #[serde(default)] pub delta_safety: f32,
    #[serde(default)] pub delta_tension: f32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EmotionDelta {
    pub entity_name: String,
    #[serde(default)] pub delta_preference: f32,
    #[serde(default)] pub delta_stress: f32,
    #[serde(default)] pub delta_fascination: f32,
}

impl CognitiveGraph {
    /// Update Ryuuko's attitude towards a User (R -> U) using clamped Delta Accumulation
    pub async fn update_social_graph(&self, user_id: &str, delta: SocialDelta) -> Result<()> {
        let query = format!(r#"
            RELATE person:ryuuko->attitudes_towards->person:{} 
            SET 
                affinity = math::clamp((affinity OR 0.0) + $delta_affinity, -1.0, 1.0),
                attachment = math::clamp((attachment OR 0.0) + $delta_attachment, -1.0, 1.0),
                trust = math::clamp((trust OR 0.0) + $delta_trust, -1.0, 1.0),
                safety = math::clamp((safety OR 0.0) + $delta_safety, -1.0, 1.0),
                tension = math::clamp((tension OR 0.0) + $delta_tension, -1.0, 1.0);
        "#, user_id);
        
        let _response = self.db.query(&query)
            .bind(("delta_affinity", delta.delta_affinity))
            .bind(("delta_attachment", delta.delta_attachment))
            .bind(("delta_trust", delta.delta_trust))
            .bind(("delta_safety", delta.delta_safety))
            .bind(("delta_tension", delta.delta_tension))
            .await?;
            
        Ok(())
    }
    
    /// Update Ryuuko's projection of how User perceives her (U -> R)
    pub async fn update_illusion_graph(&self, user_id: &str, delta: SocialDelta) -> Result<()> {
         let query = format!(r#"
            RELATE person:{}->illusion_of->person:ryuuko 
            SET 
                affinity = math::clamp((affinity OR 0.0) + $delta_affinity, -1.0, 1.0),
                attachment = math::clamp((attachment OR 0.0) + $delta_attachment, -1.0, 1.0),
                trust = math::clamp((trust OR 0.0) + $delta_trust, -1.0, 1.0),
                safety = math::clamp((safety OR 0.0) + $delta_safety, -1.0, 1.0),
                tension = math::clamp((tension OR 0.0) + $delta_tension, -1.0, 1.0);
        "#, user_id);
        
        let _response = self.db.query(&query)
            .bind(("delta_affinity", delta.delta_affinity))
            .bind(("delta_attachment", delta.delta_attachment))
            .bind(("delta_trust", delta.delta_trust))
            .bind(("delta_safety", delta.delta_safety))
            .bind(("delta_tension", delta.delta_tension))
            .await?;
            
        Ok(())
    }
    
    /// Update Ryuuko's feelings towards an entity (R -> Entity)
    pub async fn update_emotion_graph(&self, entity_name: &str, delta: EmotionDelta) -> Result<()> {
         let query = format!(r#"
            RELATE person:ryuuko->feels_about->entity:{} 
            SET 
                preference = math::clamp((preference OR 0.0) + $delta_preference, -1.0, 1.0),
                stress = math::clamp((stress OR 0.0) + $delta_stress, -1.0, 1.0),
                fascination = math::clamp((fascination OR 0.0) + $delta_fascination, -1.0, 1.0);
        "#, entity_name);
        
        let _response = self.db.query(&query)
            .bind(("delta_preference", delta.delta_preference))
            .bind(("delta_stress", delta.delta_stress))
            .bind(("delta_fascination", delta.delta_fascination))
            .await?;
            
        Ok(())
    }
    
    /// Extract context for System 1 (Frontline Roleplay)
    pub async fn get_social_context(&self, user_id: &str) -> Result<(AttitudesTowards, IllusionOf)> {
        let query = format!(r#"
            SELECT affinity, attachment, trust, safety, tension FROM person:ryuuko->attitudes_towards WHERE out = person:{};
            SELECT affinity, attachment, trust, safety, tension FROM person:{}->illusion_of WHERE out = person:ryuuko;
        "#, user_id, user_id);
        
        let mut response = self.db.query(&query).await?;
        
        let mut attitudes = AttitudesTowards::default();
        let mut illusion = IllusionOf::default();
        
        // Helper to extract f32 or fallback to 0.0
        let get_f32 = |val: &serde_json::Value, key: &str| -> f32 {
            val.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32
        };

        if let Ok(Some(val)) = response.take::<Option<serde_json::Value>>(0) {
            attitudes = AttitudesTowards {
                affinity: get_f32(&val, "affinity"),
                attachment: get_f32(&val, "attachment"),
                trust: get_f32(&val, "trust"),
                safety: get_f32(&val, "safety"),
                tension: get_f32(&val, "tension"),
            };
        }
        if let Ok(Some(val)) = response.take::<Option<serde_json::Value>>(1) {
            illusion = IllusionOf {
                affinity: get_f32(&val, "affinity"),
                attachment: get_f32(&val, "attachment"),
                trust: get_f32(&val, "trust"),
                safety: get_f32(&val, "safety"),
                tension: get_f32(&val, "tension"),
            };
        }
        
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
}
