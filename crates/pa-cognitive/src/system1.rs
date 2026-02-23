use std::sync::Arc;
use anyhow::{Context, Result};
use async_trait::async_trait;
use pa_core::event::{Event, BotTurnCompletion, Platform};
use pa_core::worker::{Worker, WorkerContext, WorkerStatus};
use pa_memory::graph::{CognitiveGraph, SocialDelta, EmotionDelta};
use pa_memory::short_term::ShortTermMemory;
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
    response_format: Option<serde_json::Value>,
    temperature: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct System1ResponseFormat {
    internal_monologue: String,
    social_delta: Option<SocialDeltaReq>,
    emotion_delta: Option<EmotionDeltaReq>,
    target_entity: Option<String>,
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
struct EmotionDeltaReq {
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
}

impl System1Worker {
    pub fn new(config: System1Config, graph: CognitiveGraph, short_term: Arc<Mutex<ShortTermMemory>>) -> Self {
        let http_client = Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_default();

        Self {
            config,
            status: WorkerStatus::NotStarted,
            http_client,
            graph,
            short_term,
        }
    }

    fn build_system_prompt(&self) -> String {
        r#"Mày là Hệ thống 1 (Trực giác Bản năng) của Ryuuko.
Nhiệm vụ của mày: Đánh giá Nhanh chóng một đoạn chat log (giữa user và Ryuuko), để trích xuất CẢM XÚC THỰC SỰ của Ryuuko (internal_monologue) và TÁC ĐỘNG TỚI ĐỘ THÂN THIẾT (delta).
Phải dùng ngôn ngữ của Gen Z Việt Nam (chửi rủa, tục tĩu thoải mái, cực kỳ thật).

Trả về STRICT JSON với cấu trúc sau:
{
  "internal_monologue": "những gì Ryuuko đang nghĩ trong đầu (VD: đm thằng này nói chuyện hãm thế, hoặc ôi dễ thương vãi~)",
  "social_delta": {
    "delta_affinity": f32,  // Thích (0.1), Ghét (-0.1)
    "delta_attachment": f32, // Bám dính (0.1), Rời xa (-0.1)
    "delta_trust": f32,      // Tin tưởng (0.1), Nghi ngờ (-0.1)
    "delta_safety": f32,     // An toàn (0.1), Nguy hiểm (-0.1)
    "delta_tension": f32     // Căng thẳng (0.1), Thoải mái (-0.1)
  },
  "emotion_delta": null,
  "target_entity": null
}

Giá trị delta dao động từ -0.5 đến +0.5. Đa số thời gian nên để giá trị nhỏ (-0.05, 0.05).
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
        let graph = self.graph.clone();
        let short_term = self.short_term.clone();
        
        let mut active_tasks = tokio::task::JoinSet::new();

        loop {
            tokio::select! {
                Some(_) = active_tasks.join_next() => {}
                result = broadcast_rx.recv() => {
                    match result {
                        Ok(Event::BotTurnCompletion(complete)) => {
                            if complete.reply_to_user.is_none() {
                                continue;
                            }
                            
                            let user_id = complete.reply_to_user.clone().unwrap();
                            let key = ConversationKey::new(complete.platform, complete.channel_id.clone());
                            
                            // Get history from short term memory
                            let history = {
                                let stm = short_term.lock().await;
                                let dummy_id = "".to_string(); 
                                stm.get_history_for_prompt(&key, &dummy_id)
                            };
                            
                            if history.is_empty() {
                                continue;
                            }
                            
                            let target_user = user_id.clone();
                            let c = config.clone();
                            let h = http_client.clone();
                            let sp = system_prompt.clone();
                            let g = graph.clone();
                            
                            // Spawn evaluate
                            active_tasks.spawn(async move {
                                Self::evaluate_turn(&h, &c, &sp, &g, &target_user, history).await;
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
        graph: &CognitiveGraph,
        user_id: &str,
        history: Vec<(String, String, String)>
    ) {
        let mut formatted_log = String::new();
        for (role, _user, content) in history.iter().rev().take(4).rev() {
            formatted_log.push_str(&format!("{}: {}\n", role, content));
        }
        
        let prompt = format!(
            "Dựa trên log hội thoại vắn tắt sau đây, hãy trích xuất trực giác và delta:\n{}\n",
            formatted_log
        );
        
        debug!("Triggering System 1 JSON evaluator for user {}", user_id);
        
        let req = ChatRequest {
            model: config.model.clone(),
            messages: vec![
                ChatMessage { role: "system".to_string(), content: system_prompt.to_string() },
                ChatMessage { role: "user".to_string(), content: prompt },
            ],
            response_format: Some(serde_json::json!({ "type": "json_object" })),
            temperature: Some(0.3),
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
                info!("System 1 Monologue: {}", data.internal_monologue);
                if let Some(social) = data.social_delta {
                    let s_delta = SocialDelta {
                        delta_affinity: social.delta_affinity,
                        delta_attachment: social.delta_attachment,
                        delta_trust: social.delta_trust,
                        delta_safety: social.delta_safety,
                        delta_tension: social.delta_tension,
                    };
                    if let Err(e) = graph.update_social_graph(user_id, s_delta).await {
                        error!("Failed to update social graph: {}", e);
                    } else {
                        debug!("Social graph updated successfully via System 1");
                    }
                }
            }
            Err(e) => {
                warn!("System 1 JSON parse error: {}\nContent: {}", e, clean_json);
            }
        }
    }
}
