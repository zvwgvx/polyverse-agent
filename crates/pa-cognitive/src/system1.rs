use std::sync::Arc;
use anyhow::Result;
use async_trait::async_trait;
use pa_core::event::Event;
use pa_core::prompt_registry::{get_prompt_or, render_prompt_or};
use pa_core::worker::{Worker, WorkerContext, WorkerStatus};
use pa_memory::graph::{CognitiveGraph, SocialDelta, EmotionDelta};
use pa_memory::short_term::ShortTermMemory;
use pa_memory::{episodic::EpisodicStore, embedder::MemoryEmbedder};
use pa_memory::types::ConversationKey;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone)]
pub struct System1Config {
    pub api_base: String,
    pub api_key: String,
    pub model: String,
    pub reasoning: Option<String>,
}

impl System1Config {
    pub fn is_valid(&self) -> bool {
        !self.api_base.is_empty() && !self.api_key.is_empty() && !self.model.is_empty()
    }
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<ReasoningConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<ProviderConfig>,
}

#[derive(Debug, Serialize)]
pub struct ReasoningConfig {
    pub effort: String,
}

#[derive(Debug, Serialize)]
struct ProviderConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    order: Option<Vec<String>>,
    allow_fallbacks: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct System1ResponseFormat {
    social_updates: Vec<SocialTargetUpdate>,
    observed_dynamics: Option<Vec<ObservedDynamic>>,
    entity_updates: Option<Vec<EntityUpdate>>,
}

#[derive(Debug, Deserialize)]
struct SocialTargetUpdate {
    target_user: String,
    role: String,
    actual_perception_delta: SocialDeltaReq,
    projected_illusion_delta: Option<IllusionDeltaReq>,
}

#[derive(Debug, Deserialize)]
struct SocialDeltaReq {
    delta_affinity: f32,
    delta_attachment: f32,
    delta_trust: f32,
    delta_safety: f32,
    delta_tension: f32,
}

#[derive(Debug, Deserialize)]
struct IllusionDeltaReq {
    expected_delta_affinity: f32,
    expected_delta_attachment: f32,
    expected_delta_trust: f32,
    expected_delta_safety: f32,
    expected_delta_tension: f32,
}

#[derive(Debug, Deserialize)]
struct ObservedDynamic {
    from_user: String,
    to_user: String,
    estimated_tension: f32,
}

#[derive(Debug, Deserialize)]
struct EntityUpdate {
    entity_name: String,
    delta_preference: f32,
    delta_stress: f32,
    delta_fascination: f32,
}

pub struct System1Worker {
    pub config: System1Config,
    status: WorkerStatus,
    http_client: Client,
    pub graph: CognitiveGraph,
    pub short_term: Arc<Mutex<ShortTermMemory>>,
    pub persona_prompt: String,
    pub episodic: Option<Arc<EpisodicStore>>,
    pub embedder: Option<Arc<MemoryEmbedder>>,
}

impl System1Worker {
    pub fn new(
        config: System1Config, 
        graph: CognitiveGraph, 
        short_term: Arc<Mutex<ShortTermMemory>>,
        episodic: Option<Arc<EpisodicStore>>,
        embedder: Option<Arc<MemoryEmbedder>>,
    ) -> Self {
        let http_client = Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_default();

        let persona_prompt = get_prompt_or("persona.base", "You are Ryuuko.");

        Self {
            config,
            status: WorkerStatus::NotStarted,
            http_client,
            graph,
            short_term,
            persona_prompt,
            episodic,
            embedder,
        }
    }

    fn build_system_prompt(&self) -> String {
        get_prompt_or(
            "system1.base_instruction",
            "You are System 1 emotional evaluator. Return strict JSON only.",
        )
    }
}

#[async_trait]
impl Worker for System1Worker {
    fn name(&self) -> &str {
        "system1"
    }

    async fn start(&mut self, ctx: WorkerContext) -> Result<()> {
        info!("System 1 evaluator starting...");

        if !self.config.is_valid() {
            warn!("System 1 config invalid, disabling worker.");
            self.status = WorkerStatus::Stopped;
            return Ok(());
        }

        self.status = WorkerStatus::Healthy;
        
        let mut broadcast_rx = ctx.subscribe_events();
        let mut shutdown_rx = ctx.subscribe_shutdown();
        
        let http_client = self.http_client.clone();
        let config = self.config.clone();
        let system_prompt = self.build_system_prompt();
        let persona_prompt = self.persona_prompt.clone();
        let graph = self.graph.clone();
        let short_term = self.short_term.clone();
        
        let mut active_tasks = tokio::task::JoinSet::new();

        loop {
            tokio::select! {
                Some(_) = active_tasks.join_next() => {}
                result = broadcast_rx.recv() => {
                    match result {
	                        Ok(Event::Raw(raw)) if raw.is_mention => {
	                            let user_id = raw.username.clone();
	                            let key = ConversationKey::from_raw(&raw);
	                            
	                            let history = {
	                                let stm = short_term.lock().await;
	                                stm.get_history_for_prompt(&key, &raw.message_id)
	                            };
                            
                            let target_user = user_id.clone();
                            let current_msg = raw.content.clone();
                            let c = config.clone();
                            let h = http_client.clone();
                            let sp = system_prompt.clone();
                            let pp = persona_prompt.clone();
                            let g = graph.clone();
                            
	                            let e = self.episodic.clone();
	                            let em = self.embedder.clone();
	                            
	                            active_tasks.spawn(async move {
	                                Self::evaluate_turn(&h, &c, &sp, &pp, &g, &target_user, history, &current_msg, e, em).await;
	                            });
	                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = shutdown_rx.recv() => break,
            }
        }
        
        active_tasks.abort_all();
        self.status = WorkerStatus::Stopped;
        info!("System 1 Evaluator stopped");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.status = WorkerStatus::Stopped;
        Ok(())
    }

    fn health_check(&self) -> WorkerStatus {
        self.status.clone()
    }
}

impl System1Worker {
    async fn evaluate_turn(
        client: &Client,
        config: &System1Config,
        system_prompt: &str,
        persona: &str,
        graph: &CognitiveGraph,
        user_id: &str,
        history: Vec<(String, String, String)>,
        current_msg: &str,
        episodic: Option<Arc<EpisodicStore>>,
        embedder: Option<Arc<MemoryEmbedder>>,
    ) {
        let cognitive_context = crate::context::build_shared_cognitive_context(
            &history,
            episodic.as_ref(),
	        embedder.as_ref(),
	        graph,
	        user_id,
	        current_msg,
	    ).await;
	        
	    let mut formatted_log = String::new();
	    for (role, user, content) in history.iter().rev().take(4).rev() {
	        let name = if role == "assistant" { "Ryuuko" } else { user.as_str() };
	        formatted_log.push_str(&format!("{}: {}\n", name, content));
	    }
	    formatted_log.push_str(&format!("{}: {}\n", user_id, current_msg));
	        
        let mut composite_system_prompt = render_prompt_or(
            "system1.composite_header",
            &[
                ("persona", persona),
                (
                    "time_and_history_text",
                    cognitive_context.time_and_history_text.as_str(),
                ),
                ("system_prompt", system_prompt),
            ],
            "Below is your persona and context:\n{{persona}}\n\n{{time_and_history_text}}\n\n{{system_prompt}}\n",
        );
        
        if let Some(mem) = cognitive_context.memory_text {
            composite_system_prompt.push_str(&mem);
            composite_system_prompt.push('\n');
        }
        composite_system_prompt.push_str(&cognitive_context.social_text);

        let user_prompt = render_prompt_or(
            "system1.user_extract",
            &[("formatted_log", formatted_log.as_str())],
            "Based on the short dialogue log below, extract intuition and deltas:\n{{formatted_log}}\n",
        );
        
        debug!("Triggering System 1 JSON evaluator for user {}", user_id);
        
        let req = ChatRequest {
            model: config.model.clone(),
            messages: vec![
                ChatMessage { role: "system".to_string(), content: composite_system_prompt },
                ChatMessage { role: "user".to_string(), content: user_prompt },
            ],
            response_format: Some(serde_json::json!({ "type": "json_object" })),
            temperature: Some(0.3),
            reasoning: config.reasoning.clone().map(|effort| ReasoningConfig { effort }),
            provider: Some(ProviderConfig {
                order: Some(vec!["Google AI Studio".to_string()]),
                allow_fallbacks: true,
            }),
        };
        
        let url = format!("{}/chat/completions", config.api_base.trim_end_matches('/'));
        
        let res = match client.post(&url)
            .header("Authorization", format!("Bearer {}", config.api_key))
            .json(&req).send().await {
                Ok(r) => r,
                Err(e) => {
                    warn!("System 1 request failed: {}", e);
                    return;
                }
            };
            
        if !res.status().is_success() {
            warn!("System 1 returned error status: {}", res.status());
            return;
        }
        
        let body: serde_json::Value = match res.json().await {
            Ok(b) => b,
            Err(_) => return,
        };
	        
	        let content_str = body["choices"][0]["message"]["content"].as_str().unwrap_or("{}");
	        
	        let clean_json = if content_str.starts_with("```json") {
	            let s = content_str.trim_start_matches("```json").trim_end_matches("```");
	            s
	        } else {
            content_str
        };
        
        let parsed: Result<System1ResponseFormat, _> = serde_json::from_str(clean_json);
	        match parsed {
	            Ok(data) => {
	                debug!("System 1 evaluation completed");
	                
	                for social in data.social_updates {
	                    let s_delta = SocialDelta {
	                        delta_affinity: social.actual_perception_delta.delta_affinity,
	                        delta_attachment: social.actual_perception_delta.delta_attachment,
                        delta_trust: social.actual_perception_delta.delta_trust,
                        delta_safety: social.actual_perception_delta.delta_safety,
                        delta_tension: social.actual_perception_delta.delta_tension,
                    };
                    
                    if let Err(e) = graph.update_social_graph(&social.target_user, s_delta).await {
                        error!("Failed to update social graph for {}: {}", social.target_user, e);
                    }
                    
                    if social.role == "chat_partner" {
                        if let Some(illusion) = social.projected_illusion_delta {
                            let i_delta = SocialDelta {
                                delta_affinity: illusion.expected_delta_affinity,
                                delta_attachment: illusion.expected_delta_attachment,
                                delta_trust: illusion.expected_delta_trust,
                                delta_safety: illusion.expected_delta_safety,
                                delta_tension: illusion.expected_delta_tension,
                            };
                            if let Err(e) = graph.update_illusion_graph(&social.target_user, i_delta).await {
                                error!("Failed to update illusion graph for {}: {}", social.target_user, e);
                            }
	                        }
	                    }
	                }
	                
	                if let Some(dynamics) = data.observed_dynamics {
	                    for dyn_update in dynamics {
	                        if let Err(e) = graph.update_observed_dynamic(&dyn_update.from_user, &dyn_update.to_user, dyn_update.estimated_tension).await {
	                            error!("Failed to update observed dynamics: {}", e);
                        }
	                    }
	                }
	                
	                if let Some(entities) = data.entity_updates {
	                    for entity in entities {
	                        let e_delta = EmotionDelta {
	                            entity_name: entity.entity_name.clone(),
                            delta_preference: entity.delta_preference,
                            delta_stress: entity.delta_stress,
                            delta_fascination: entity.delta_fascination,
                        };
                        if let Err(e) = graph.update_emotion_graph(&entity.entity_name, e_delta).await {
                            error!("Failed to update emotion graph for {}: {}", entity.entity_name, e);
                        }
                    }
                }
                
                debug!("Multi-target Graph updated successfully via System 1");
            }
            Err(e) => {
                warn!("System 1 JSON parse error: {}\nContent: {}", e, clean_json);
            }
        }
    }
}
