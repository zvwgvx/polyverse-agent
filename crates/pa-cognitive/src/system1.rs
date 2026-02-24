use std::sync::Arc;
use anyhow::{Context, Result};
use async_trait::async_trait;
use pa_core::event::{Event, RawEvent, Platform};
use pa_core::worker::{Worker, WorkerContext, WorkerStatus};
use pa_memory::graph::{CognitiveGraph, SocialDelta, EmotionDelta};
use pa_memory::short_term::ShortTermMemory;
use pa_memory::{episodic::EpisodicStore, embedder::MemoryEmbedder};
use pa_memory::types::ConversationKey;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

// ─── Config ──────────────────────────────────────────────────

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

// ─── API Types ───────────────────────────────────────────────

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
    observation: String,
    estimated_tension: f32,
}

#[derive(Debug, Deserialize)]
struct EntityUpdate {
    entity_name: String,
    delta_preference: f32,
    delta_stress: f32,
    delta_fascination: f32,
}

// ─── Worker ──────────────────────────────────────────────────

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

        let persona_prompt = std::fs::read_to_string("instruct.txt")
            .unwrap_or_else(|_| "Mày là Ryuuko...".to_string());

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
        r#"Nhiệm vụ của bạn là Trình Đánh Giá Cảm Xúc (System 1).
Nhiệm vụ duy nhất: Đọc đoạn chat, tính toán sự biến thiên trọng số tâm lý (delta) lên các mục tiêu.

Trả về STRICT JSON với cấu trúc sau:
{
  "social_updates": [
    {
      "target_user": "Tên user",
      "role": "chat_partner hoặc mentioned_person",
      "actual_perception_delta": {
        "delta_affinity": f32,
        "delta_attachment": f32,
        "delta_trust": f32,
        "delta_safety": f32,
        "delta_tension": f32
      },
      "projected_illusion_delta": {
        "expected_delta_affinity": f32,
        "expected_delta_attachment": f32,
        "expected_delta_trust": f32,
        "expected_delta_safety": f32,
        "expected_delta_tension": f32
      }
    }
  ],
  "observed_dynamics": [ 
    {
      "from_user": "Tên",
      "to_user": "Tên",
      "observation": "Nhận xét",
      "estimated_tension": f32
    }
  ],
  "entity_updates": [ 
    {
      "entity_name": "Tên sự vật/chủ đề",
      "delta_preference": f32,
      "delta_stress": f32,
      "delta_fascination": f32
    }
  ]
}

=== QUY TẮC DELTA BẮT BUỘC ===
Mỗi giá trị delta BẮT BUỘC nằm trong khoảng [-0.30, +0.30]. Vi phạm sẽ bị hệ thống cắt bớt.

BẢNG HƯỚNG DẪN DELTA (tuân thủ nghiêm ngặt):
| Loại tương tác                          | Delta phù hợp       |
|-----------------------------------------|----------------------|
| Chat bình thường, hỏi thăm, small talk | 0.000 ~ ±0.003      |
| Chia sẻ sở thích chung, đồng ý nhẹ    | ±0.003 ~ ±0.008     |
| Giúp đỡ, chia sẻ cảm xúc sâu         | ±0.008 ~ ±0.015     |
| Hiểu lầm nhẹ, bất đồng quan điểm     | ±0.010 ~ ±0.020     |
| Xúc phạm, phản bội, cứu giúp lớn     | ±0.020 ~ ±0.050     |
| SHOCK: phản bội sâu, cứu mạng         | ±0.050 ~ ±0.150     |
| PANIC: đe doạ tính mạng, phản bội hủy hoại, sự kiện thay đổi hoàn toàn mối quan hệ | ±0.150 ~ ±0.300     |

NGUYÊN TẮC:
- 90% tin nhắn bình thường nên có delta ≈ 0.001 ~ 0.005
- Delta = 0.000 là HOÀN TOÀN HỢP LÝ cho tin nhắn không có ý nghĩa cảm xúc
- Delta >= 0.03 chỉ dùng cho sự kiện ĐẶC BIỆT
- Delta >= 0.10 chỉ dùng cho sự kiện GÂY SỐC (rất hiếm)
- Delta >= 0.20 chỉ dùng cho sự kiện PANIC CỰC ĐỘ (gần như không bao giờ xảy ra)
- Quan hệ thay đổi CHẬM — cần hàng trăm tin nhắn để xây dựng niềm tin thực sự
- KHÔNG bao giờ cho delta > 0.30 hoặc < -0.30

Trường projected_illusion_delta có thể null nếu role là mentioned_person. Các mảng observed_dynamics và entity_updates có thể null hoặc rỗng nếu không có.
Không sinh ra bất kỳ văn bản nào ngoài JSON Object này."#.to_string()
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
                            
                            // Get history from short term memory
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
                            
                            // Spawn evaluate
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
        
        // System 1 trigger is concurrent with System 2, so append the current message manually
        let mut formatted_log = String::new();
        for (role, user, content) in history.iter().rev().take(4).rev() {
            let name = if role == "assistant" { "Ryuuko" } else { user.as_str() };
            formatted_log.push_str(&format!("{}: {}\n", name, content));
        }
        formatted_log.push_str(&format!("{}: {}\n", user_id, current_msg));
        
        // Construct standard unified prompt
        let mut composite_system_prompt = format!(
            "DƯỚI ĐÂY LÀ CON NGƯỜI VÀ TÍNH CÁCH CỦA BẠN (Tên: Ryuuko):\n{}\n\n{}\n\n{}\n",
            persona, 
            cognitive_context.time_and_history_text,
            system_prompt
        );
        
        if let Some(mem) = cognitive_context.memory_text {
            composite_system_prompt.push_str(&mem);
            composite_system_prompt.push('\n');
        }
        composite_system_prompt.push_str(&cognitive_context.social_text);

        let user_prompt = format!(
            "Dựa trên log hội thoại vắn tắt sau đây, hãy trích xuất trực giác và lượng biến thiên tâm lý (delta):\n{}\n",
            formatted_log
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
        
        // Try parsing JSON block
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
                
                // Process social updates (R -> U and U -> R)
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
                
                // Process observed dynamics (U1 -> U2)
                if let Some(dynamics) = data.observed_dynamics {
                    for dyn_update in dynamics {
                        if let Err(e) = graph.update_observed_dynamic(&dyn_update.from_user, &dyn_update.to_user, dyn_update.estimated_tension).await {
                            error!("Failed to update observed dynamics: {}", e);
                        }
                    }
                }
                
                // Process entity updates (R -> E)
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
