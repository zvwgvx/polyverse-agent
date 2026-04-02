use std::sync::Arc;
use std::time::Instant;
use anyhow::Result;
use async_trait::async_trait;
use kernel::get_agent_profile;
use kernel::event::Event;
use kernel::prompt_registry::{get_prompt_or, render_prompt_or};
use kernel::worker::{Worker, WorkerContext, WorkerStatus};
use memory::graph::{CognitiveGraph, SocialDelta, EmotionDelta};
use memory::short_term::ShortTermMemory;
use memory::{episodic::EpisodicStore, embedder::MemoryEmbedder};
use memory::types::ConversationKey;
use state::{EventDeltaRequest, StateStore};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Semaphore};
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone)]
pub struct AffectEvaluatorConfig {
    pub api_base: String,
    pub api_key: String,
    pub model: String,
    pub reasoning: Option<String>,
}

impl AffectEvaluatorConfig {
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
    stream: Option<bool>,
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
struct AffectEvaluatorResponseFormat {
    social_updates: Vec<SocialTargetUpdate>,
    observed_dynamics: Option<Vec<ObservedDynamic>>,
    entity_updates: Option<Vec<EntityUpdate>>,
    emotion_delta: Option<EmotionDeltaReq>,
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
    #[serde(alias = "delta_affinity")]
    expected_delta_affinity: f32,
    #[serde(alias = "delta_attachment")]
    expected_delta_attachment: f32,
    #[serde(alias = "delta_trust")]
    expected_delta_trust: f32,
    #[serde(alias = "delta_safety")]
    expected_delta_safety: f32,
    #[serde(alias = "delta_tension")]
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

#[derive(Debug, Deserialize)]
struct EmotionDeltaReq {
    delta_valence: f32,
    delta_arousal: f32,
    delta_anxiety: f32,
    delta_anger: f32,
    delta_joy: f32,
    delta_sadness: f32,
    delta_confidence: f32,
    delta_stability: f32,
}

#[derive(Debug, Clone, Copy, Default)]
struct AffectLatency {
    total_ms: u128,
    shared_context_ms: u128,
    embed_ms: u128,
    episodic_search_ms: u128,
    chunk_count_ms: u128,
    graph_ms: u128,
    context_format_ms: u128,
    shared_context_cache_hit: bool,
    http_send_ms: u128,
    parse_ms: u128,
    graph_update_ms: u128,
    state_update_ms: u128,
}

pub struct AffectEvaluatorWorker {
    pub config: AffectEvaluatorConfig,
    status: WorkerStatus,
    http_client: Client,
    pub graph: CognitiveGraph,
    pub short_term: Arc<Mutex<ShortTermMemory>>,
    pub persona_prompt: String,
    pub episodic: Option<Arc<EpisodicStore>>,
    pub embedder: Option<Arc<MemoryEmbedder>>,
    pub state_store: Option<StateStore>,
    affect_limiter: Arc<Semaphore>,
}

impl AffectEvaluatorWorker {
    fn log_latency_summary(
        config: &AffectEvaluatorConfig,
        message_id: &str,
        user_id: &str,
        latency: &AffectLatency,
        status: &str,
    ) {
        info!(
            kind = "latency.affect",
            message_id,
            user = user_id,
            model = %config.model,
            total_ms = latency.total_ms,
            shared_context_ms = latency.shared_context_ms,
            embed_ms = latency.embed_ms,
            episodic_search_ms = latency.episodic_search_ms,
            chunk_count_ms = latency.chunk_count_ms,
            graph_ms = latency.graph_ms,
            context_format_ms = latency.context_format_ms,
            shared_context_cache_hit = latency.shared_context_cache_hit,
            http_send_ms = latency.http_send_ms,
            parse_ms = latency.parse_ms,
            graph_update_ms = latency.graph_update_ms,
            state_update_ms = latency.state_update_ms,
            status,
            "Affect evaluator latency summary"
        );
    }

    pub fn new(
        config: AffectEvaluatorConfig,
        graph: CognitiveGraph,
        short_term: Arc<Mutex<ShortTermMemory>>,
        episodic: Option<Arc<EpisodicStore>>,
        embedder: Option<Arc<MemoryEmbedder>>,
    ) -> Self {
        let http_client = Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_default();

        let profile = get_agent_profile();
        let fallback_persona = format!("You are {}.", profile.display_name);
        let persona_prompt = get_prompt_or("persona.base", fallback_persona.as_str());

        Self {
            config,
            status: WorkerStatus::NotStarted,
            http_client,
            graph,
            short_term,
            persona_prompt,
            episodic,
            embedder,
            state_store: None,
            affect_limiter: Arc::new(Semaphore::new(1)),
        }
    }

    pub fn with_state_store(mut self, store: StateStore) -> Self {
        self.state_store = Some(store);
        self
    }

    fn build_system_prompt(&self) -> String {
        get_prompt_or(
            "affect_evaluator.base_instruction",
            "You are an affect evaluator. Return strict JSON only.",
        )
    }
}

#[async_trait]
impl Worker for AffectEvaluatorWorker {
    fn name(&self) -> &str {
        "affect_evaluator"
    }

    async fn start(&mut self, ctx: WorkerContext) -> Result<()> {
        info!("Affect evaluator starting...");

        if !self.config.is_valid() {
            warn!("Affect evaluator config invalid, disabling worker.");
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
        let state_store = self.state_store.clone();
        let affect_limiter = self.affect_limiter.clone();

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
                            let message_id = raw.message_id.clone();
                            let current_msg = raw.content.clone();
                            let c = config.clone();
                            let h = http_client.clone();
                            let sp = system_prompt.clone();
                            let pp = persona_prompt.clone();
                            let g = graph.clone();
                            let st = state_store.clone();
                            let limiter = affect_limiter.clone();

		                            let e = self.episodic.clone();
		                            let em = self.embedder.clone();
		                            
	                            active_tasks.spawn(async move {
	                                let _permit = match limiter.acquire_owned().await {
	                                    Ok(permit) => permit,
	                                    Err(_) => return,
	                                };
	                                Self::evaluate_turn(&h, &c, &sp, &pp, &g, st, &target_user, &message_id, history, &current_msg, e, em).await;
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
        info!("Affect evaluator stopped");
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

impl AffectEvaluatorWorker {
    async fn evaluate_turn(
        client: &Client,
        config: &AffectEvaluatorConfig,
        system_prompt: &str,
        persona: &str,
        graph: &CognitiveGraph,
        state_store: Option<StateStore>,
        user_id: &str,
        message_id: &str,
        history: Vec<(String, String, String)>,
        current_msg: &str,
        episodic: Option<Arc<EpisodicStore>>,
        embedder: Option<Arc<MemoryEmbedder>>,
    ) {
        let overall_started = Instant::now();
        let mut latency = AffectLatency::default();
        let mut final_status = "success";

        let context_started = Instant::now();
        let cognitive_context = crate::context::build_shared_cognitive_context(
            message_id,
            &history,
            episodic.as_ref(),
            embedder.as_ref(),
            user_id,
            current_msg,
        ).await;
        latency.shared_context_ms = context_started.elapsed().as_millis();
        latency.embed_ms = cognitive_context.timing.embed_ms;
        latency.episodic_search_ms = cognitive_context.timing.episodic_search_ms;
        latency.chunk_count_ms = cognitive_context.timing.chunk_count_ms;
        latency.graph_ms = cognitive_context.timing.graph_ms;
        latency.context_format_ms = cognitive_context.timing.format_ms;
        latency.shared_context_cache_hit = cognitive_context.timing.cache_hit;

        let mut formatted_log = String::new();
        let profile = get_agent_profile();
        for (role, user, content) in history.iter().rev().take(4).rev() {
            let name = if role == "assistant" {
                profile.display_name.as_str()
            } else {
                user.as_str()
            };
            formatted_log.push_str(&format!("{}: {}\n", name, content));
        }
        formatted_log.push_str(&format!("{}: {}\n", user_id, current_msg));

        let mut composite_system_prompt = render_prompt_or(
            "affect_evaluator.composite_header",
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
        let memory_hint = (history.len() as f32 / 12.0).min(0.15);
        let affect_social_context = crate::social_context::load_affect_social_context(
            graph,
            user_id,
            memory_hint,
        ).await;
        composite_system_prompt.push_str(&affect_social_context.to_prompt_text());

        let user_prompt = render_prompt_or(
            "affect_evaluator.user_extract",
            &[("formatted_log", formatted_log.as_str())],
            "Based on the short dialogue log below, extract intuition and deltas:\n{{formatted_log}}\n",
        );

        debug!("Triggering affect evaluator JSON pass for user {}", user_id);

        let req = ChatRequest {
            model: config.model.clone(),
            messages: vec![
                ChatMessage { role: "system".to_string(), content: composite_system_prompt },
                ChatMessage { role: "user".to_string(), content: user_prompt },
            ],
            response_format: Some(serde_json::json!({ "type": "json_object" })),
            temperature: Some(0.3),
            stream: Some(false),
            reasoning: config.reasoning.clone().map(|effort| ReasoningConfig { effort }),
            provider: Some(ProviderConfig {
                order: None,
                allow_fallbacks: true,
            }),
        };

        let url = format!("{}/chat/completions", config.api_base.trim_end_matches('/'));

        let http_started = Instant::now();
        let res = match client.post(&url)
            .header("Authorization", format!("Bearer {}", config.api_key))
            .json(&req).send().await {
                Ok(r) => {
                    latency.http_send_ms = http_started.elapsed().as_millis();
                    r
                }
                Err(e) => {
                    latency.http_send_ms = http_started.elapsed().as_millis();
                    final_status = "request_error";
                    warn!("Affect evaluator request failed: {}", e);
                    latency.total_ms = overall_started.elapsed().as_millis();
                    Self::log_latency_summary(config, message_id, user_id, &latency, final_status);
                    return;
                }
            };

        if !res.status().is_success() {
            final_status = "api_error";
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            warn!(status = %status, api_error = %body, user = user_id, "Affect evaluator returned API error");
            latency.total_ms = overall_started.elapsed().as_millis();
            Self::log_latency_summary(config, message_id, user_id, &latency, final_status);
            return;
        }

        let parse_started = Instant::now();
        let response_text = match res.text().await {
            Ok(text) => text,
            Err(e) => {
                final_status = "parse_error";
                latency.parse_ms = parse_started.elapsed().as_millis();
                warn!(error = %e, user = user_id, "Affect evaluator failed to read response body");
                latency.total_ms = overall_started.elapsed().as_millis();
                Self::log_latency_summary(config, message_id, user_id, &latency, final_status);
                return;
            }
        };
        let body: serde_json::Value = match serde_json::from_str(&response_text) {
            Ok(b) => b,
            Err(e) => {
                final_status = "parse_error";
                latency.parse_ms = parse_started.elapsed().as_millis();
                warn!(error = %e, user = user_id, raw_response = %response_text, "Affect evaluator returned non-JSON response body");
                latency.total_ms = overall_started.elapsed().as_millis();
                Self::log_latency_summary(config, message_id, user_id, &latency, final_status);
                return;
            }
        };


        let content_str = body["choices"][0]["message"]["content"].as_str().unwrap_or("{}");

        let clean_json = if content_str.starts_with("```json") {
            content_str.trim_start_matches("```json").trim_end_matches("```")
        } else {
            content_str
        };

        let parsed: Result<AffectEvaluatorResponseFormat, _> = serde_json::from_str(clean_json);
        latency.parse_ms = parse_started.elapsed().as_millis();
        if let Err(ref e) = parsed {
            warn!(error = %e, user = user_id, raw_content = %content_str, clean_json = %clean_json, "Affect evaluator returned unparsable content");
        }
        match parsed {
            Ok(data) => {
                debug!("Affect evaluator completed");

                let graph_update_started = Instant::now();
                let mut session_delta: Option<SocialDeltaReq> = None;
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

                    let projection_hint = if social.role == "chat_partner" || social.target_user == user_id {
                        (history.len() as f32 / 12.0).min(0.15)
                    } else {
                        0.0
                    };
                    if let Err(e) = graph.project_social_tree(&social.target_user, projection_hint).await {
                        warn!("Failed to project social tree for {}: {}", social.target_user, e);
                    }

                    if social.role == "chat_partner" || social.target_user == user_id {
                        session_delta = Some(social.actual_perception_delta);
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

                let mut pref_sum = 0.0f64;
                let mut stress_sum = 0.0f64;
                let mut fasc_sum = 0.0f64;
                let mut pref_count = 0usize;
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
                        pref_sum += entity.delta_preference as f64;
                        stress_sum += entity.delta_stress as f64;
                        fasc_sum += entity.delta_fascination as f64;
                        pref_count += 1;
                    }
                }
                latency.graph_update_ms = graph_update_started.elapsed().as_millis();

                let state_update_started = Instant::now();
                if let Some(store) = state_store {
                    if !store.mark_event_if_new("affect", message_id).await {
                        final_status = "duplicate";
                        latency.state_update_ms = state_update_started.elapsed().as_millis();
                        latency.total_ms = overall_started.elapsed().as_millis();
                        Self::log_latency_summary(config, message_id, user_id, &latency, final_status);
                        return;
                    }
                    let mut updates = Vec::new();
                    if let Some(delta) = session_delta {
                        updates.push(EventDeltaRequest {
                            dimension_id: "session_social.affinity".to_string(),
                            delta: delta.delta_affinity as f64,
                            reason: "affect_evaluator".to_string(),
                            actor: user_id.to_string(),
                            source: "affect_evaluator".to_string(),
                        });
                        updates.push(EventDeltaRequest {
                            dimension_id: "session_social.attachment".to_string(),
                            delta: delta.delta_attachment as f64,
                            reason: "affect_evaluator".to_string(),
                            actor: user_id.to_string(),
                            source: "affect_evaluator".to_string(),
                        });
                        updates.push(EventDeltaRequest {
                            dimension_id: "session_social.trust".to_string(),
                            delta: delta.delta_trust as f64,
                            reason: "affect_evaluator".to_string(),
                            actor: user_id.to_string(),
                            source: "affect_evaluator".to_string(),
                        });
                        updates.push(EventDeltaRequest {
                            dimension_id: "session_social.safety".to_string(),
                            delta: delta.delta_safety as f64,
                            reason: "affect_evaluator".to_string(),
                            actor: user_id.to_string(),
                            source: "affect_evaluator".to_string(),
                        });
                        updates.push(EventDeltaRequest {
                            dimension_id: "session_social.tension".to_string(),
                            delta: delta.delta_tension as f64,
                            reason: "affect_evaluator".to_string(),
                            actor: user_id.to_string(),
                            source: "affect_evaluator".to_string(),
                        });
                    }

                    if let Some(delta) = data.emotion_delta {
                        updates.push(EventDeltaRequest {
                            dimension_id: "emotion.valence".to_string(),
                            delta: delta.delta_valence as f64,
                            reason: "affect_evaluator".to_string(),
                            actor: user_id.to_string(),
                            source: "affect_evaluator".to_string(),
                        });
                        updates.push(EventDeltaRequest {
                            dimension_id: "emotion.arousal".to_string(),
                            delta: delta.delta_arousal as f64,
                            reason: "affect_evaluator".to_string(),
                            actor: user_id.to_string(),
                            source: "affect_evaluator".to_string(),
                        });
                        updates.push(EventDeltaRequest {
                            dimension_id: "emotion.anxiety".to_string(),
                            delta: delta.delta_anxiety as f64,
                            reason: "affect_evaluator".to_string(),
                            actor: user_id.to_string(),
                            source: "affect_evaluator".to_string(),
                        });
                        updates.push(EventDeltaRequest {
                            dimension_id: "emotion.anger".to_string(),
                            delta: delta.delta_anger as f64,
                            reason: "affect_evaluator".to_string(),
                            actor: user_id.to_string(),
                            source: "affect_evaluator".to_string(),
                        });
                        updates.push(EventDeltaRequest {
                            dimension_id: "emotion.joy".to_string(),
                            delta: delta.delta_joy as f64,
                            reason: "affect_evaluator".to_string(),
                            actor: user_id.to_string(),
                            source: "affect_evaluator".to_string(),
                        });
                        updates.push(EventDeltaRequest {
                            dimension_id: "emotion.sadness".to_string(),
                            delta: delta.delta_sadness as f64,
                            reason: "affect_evaluator".to_string(),
                            actor: user_id.to_string(),
                            source: "affect_evaluator".to_string(),
                        });
                        updates.push(EventDeltaRequest {
                            dimension_id: "emotion.confidence".to_string(),
                            delta: delta.delta_confidence as f64,
                            reason: "affect_evaluator".to_string(),
                            actor: user_id.to_string(),
                            source: "affect_evaluator".to_string(),
                        });
                        updates.push(EventDeltaRequest {
                            dimension_id: "emotion.stability".to_string(),
                            delta: delta.delta_stability as f64,
                            reason: "affect_evaluator".to_string(),
                            actor: user_id.to_string(),
                            source: "affect_evaluator".to_string(),
                        });
                    }
                    if pref_count > 0 {
                        let count = pref_count as f64;
                        let avg_pref = pref_sum / count;
                        let avg_stress = stress_sum / count;
                        let avg_fasc = fasc_sum / count;
                        updates.push(EventDeltaRequest {
                            dimension_id: "preference.curiosity".to_string(),
                            delta: avg_pref,
                            reason: "affect_evaluator".to_string(),
                            actor: user_id.to_string(),
                            source: "affect_evaluator".to_string(),
                        });
                        updates.push(EventDeltaRequest {
                            dimension_id: "preference.fascination".to_string(),
                            delta: avg_fasc,
                            reason: "affect_evaluator".to_string(),
                            actor: user_id.to_string(),
                            source: "affect_evaluator".to_string(),
                        });
                        updates.push(EventDeltaRequest {
                            dimension_id: "preference.stress".to_string(),
                            delta: avg_stress,
                            reason: "affect_evaluator".to_string(),
                            actor: user_id.to_string(),
                            source: "affect_evaluator".to_string(),
                        });
                    }

                    if !updates.is_empty() {
                        if let Err(e) = store.apply_event_deltas(&updates).await {
                            warn!("Failed to update session/emotion state: {}", e);
                        }
                    }
                }
                latency.state_update_ms = state_update_started.elapsed().as_millis();

                debug!("Multi-target graph updated successfully via affect evaluator");
            }
            Err(e) => {
                final_status = "parse_error";
                warn!("Affect evaluator JSON parse error: {}\nContent: {}", e, clean_json);
            }
        }

        latency.total_ms = overall_started.elapsed().as_millis();
        Self::log_latency_summary(config, message_id, user_id, &latency, final_status);
    }
}
