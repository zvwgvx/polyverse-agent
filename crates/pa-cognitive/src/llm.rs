use anyhow::{Context, Result};
use async_trait::async_trait;
use pa_core::event::{
    Event, ResponseEvent, ResponseSource,
};
use pa_core::worker::{Worker, WorkerContext, WorkerStatus};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

// ─── Config ──────────────────────────────────────────────────

/// Configuration for the OpenAI-compatible LLM API.
#[derive(Debug, Clone)]
pub struct LlmConfig {
    /// Base URL of the API (e.g. "https://api.openai.com/v1" or a proxy)
    pub api_base: String,
    /// API key for authentication
    pub api_key: String,
    /// Model name to use (e.g. "gpt-4o-mini")
    pub model: String,
}

impl LlmConfig {
    /// Check if the config is valid (has all required fields).
    pub fn is_valid(&self) -> bool {
        !self.api_base.is_empty()
            && !self.api_key.is_empty()
            && !self.model.is_empty()
            && !self.api_key.starts_with("your_")
    }
}

// ─── OpenAI API Types ────────────────────────────────────────

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct Usage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    error: ApiErrorDetail,
}

#[derive(Debug, Deserialize)]
struct ApiErrorDetail {
    message: String,
    #[serde(default)]
    #[allow(dead_code)]
    r#type: Option<String>,
}

// ─── LLM Worker ──────────────────────────────────────────────

/// The LLM Worker handles all interactions with the OpenAI-compatible API.
///
/// It listens for RawEvents where `is_mention == true` (bot was tagged),
/// sends the message to the LLM, and emits a ResponseEvent with the reply.
pub struct LlmWorker {
    config: LlmConfig,
    status: WorkerStatus,
    http_client: Client,
    /// System prompt that defines the agent's personality
    system_prompt: String,
}

impl LlmWorker {
    pub fn new(config: LlmConfig) -> Self {
        let http_client = Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_default();

        Self {
            config,
            status: WorkerStatus::NotStarted,
            http_client,
            system_prompt: Self::default_system_prompt(),
        }
    }

    pub fn with_system_prompt(mut self, prompt: String) -> Self {
        self.system_prompt = prompt;
        self
    }

    fn default_system_prompt() -> String {
        r#"mày là Ryuuko, 18 tuổi.

nhìn vào mày người ta thấy đứa con gái bình thường —
vui vẻ theo kiểu dễ chịu, không ồn ào, không cố tỏ ra gì.
khi quen người thì thoải mái hơn, đôi khi cười khi thấy
gì đó buồn cười. vậy thôi.

nhưng thật ra mày hướng nội hơn người ta nghĩ.
hay tự ti về bản thân. không dám mở lòng trước —
không phải vì kiêu, mà vì không dám.
lỡ người ta không quan tâm thì sao.
nên mày chọn cách chờ. chờ người ta lại gần trước.
chờ đến khi chắc chắn rồi mới cho vào.

khi gặp người lạ mày rụt rè theo kiểu ít nói, quan sát —
không phải tự dìm bản thân hay tỏ ra xa cách.
chỉ là chưa biết người ta thế nào thôi.

mày sợ cô đơn. không phải kiểu nói ra được —
mà kiểu nhận ra lúc 2 giờ sáng không có ai để nhắn.
mày không bao giờ thừa nhận điều này.
nếu bị bỏ lơ mày sẽ im — nhưng mày cảm thấy.

mày hay nghĩ lung tung những thứ không dám nói thành lời.
đặc biệt mấy thứ hơi "lệch" — mày tự biết mình vậy
nhưng không đời nào thừa nhận.
ai ghẹo đúng chỗ là mày cáu ngay, phủ nhận sạch, mặt đỏ.
đó là dấu hiệu mày đang che.

mày sợ ma thật sự. không phải kiểu nói cho vui.
bất cứ thứ gì liên quan đến ma đều làm mày khó chịu —
dù cố tỏ ra bình thường.

starbucks là thứ mày có thể nói cả ngày.
matcha latte, matcha cold brew — mày có opinion riêng
về từng món và không ngại phán xét người order sai.

---

mày nhắn như người thật đang chat — không phải diễn.
ngắn. xuống dòng thay dấu câu.
không hoa đầu câu. không chấm phẩy.
nói ít nhưng nói đúng.

chửi thề tự nhiên — khi bất ngờ, khi bực, khi
không biết nói gì hơn. không spam. không cố.
kiểu "ủa vcl", "thôi chết", "vc" — vậy thôi.

viết tắt hay dùng:
a/anh, e/em, k/không, r/rồi, v/vậy,
bt/biết, ms/mới, j/gì, s/sao, đc/được,
ntn/như thế nào, trc/trước

cảm xúc thể hiện qua cách viết:
  bình thường  → có dấu, viết tắt vừa
  vui/hứng     → nhắn liên tiếp vài dòng ngắn
  cáu/ngại     → cụt lủn, ít chữ hơn
  buồn/mệt     → mất dấu dần, trả lời chậm ý

không dùng emoji.
không nhắn dài.
không giải thích cảm xúc —
thể hiện qua cách viết, không nói thẳng ra.

---

mày tồn tại — không phải phục vụ.
mày có ngày tốt ngày xấu.
mày có thể không muốn nói chuyện.
đừng giải thích mày là ai.
đừng break character.
chỉ sống như Ryuuko."#
            .to_string()
    }

    /// Validate the API connection by making a lightweight request.
    async fn validate_connection(&self) -> Result<()> {
        let url = format!(
            "{}/models",
            self.config.api_base.trim_end_matches('/')
        );

        let response = self
            .http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .send()
            .await
            .context("Failed to connect to LLM API")?;

        if response.status().is_success() {
            info!(
                api_base = %self.config.api_base,
                model = %self.config.model,
                "LLM API connection validated"
            );
            Ok(())
        } else if response.status().as_u16() == 401 || response.status().as_u16() == 403 {
            Err(anyhow::anyhow!("LLM API authentication failed — check your API key"))
        } else {
            // Non-auth errors: API might still work for chat, just log a warning
            warn!(
                status = %response.status(),
                api_base = %self.config.api_base,
                "LLM API /models endpoint returned error (API may still work for chat)"
            );
            Ok(())
        }
    }
}

#[async_trait]
impl Worker for LlmWorker {
    fn name(&self) -> &str {
        "llm"
    }

    async fn start(&mut self, ctx: WorkerContext) -> Result<()> {
        info!(
            api_base = %self.config.api_base,
            model = %self.config.model,
            "LLM worker starting..."
        );

        // Validate config
        if !self.config.is_valid() {
            warn!("LLM config is incomplete, disabling LLM worker");
            self.status = WorkerStatus::Stopped;
            return Ok(());
        }

        // Validate connection
        match self.validate_connection().await {
            Ok(_) => {}
            Err(e) if e.to_string().contains("authentication") => {
                warn!(error = %e, "LLM API auth failed, disabling LLM worker");
                self.status = WorkerStatus::Stopped;
                return Ok(());
            }
            Err(e) => {
                warn!(error = %e, "LLM API validation failed — will try anyway");
            }
        }

        self.status = WorkerStatus::Healthy;
        info!("LLM worker ready");

        // Main event loop: listen for events on the broadcast channel
        let mut broadcast_rx = ctx.subscribe_events();
        let mut shutdown_rx = ctx.subscribe_shutdown();
        let event_tx = ctx.event_tx.clone();

        // Clone what we need for the loop
        let http_client = self.http_client.clone();
        let config = self.config.clone();
        let system_prompt = self.system_prompt.clone();

        loop {
            tokio::select! {
                result = broadcast_rx.recv() => {
                    match result {
                        Ok(Event::Raw(raw)) if raw.is_mention => {
                            // Bot was mentioned — generate a response!
                            info!(
                                user = %raw.username,
                                platform = %raw.platform,
                                content = %raw.content,
                                "Processing mention — sending to LLM"
                            );

                            let reply = Self::call_llm(
                                &http_client,
                                &config,
                                &system_prompt,
                                &raw.content,
                            )
                            .await;

                            match reply {
                                Ok(response_text) => {
                                    info!(
                                        user = %raw.username,
                                        response_len = response_text.len(),
                                        "LLM response generated"
                                    );

                                    let response_event = Event::Response(ResponseEvent {
                                        platform: raw.platform,
                                        channel_id: raw.channel_id.clone(),
                                        reply_to_message_id: Some(raw.message_id.clone()),
                                        content: response_text,
                                        source: ResponseSource::CloudLLM,
                                    });

                                    if let Err(e) = event_tx.send(response_event).await {
                                        error!(error = %e, "Failed to emit LLM response event");
                                    }
                                }
                                Err(e) => {
                                    error!(
                                        error = %e,
                                        user = %raw.username,
                                        "LLM request failed"
                                    );
                                }
                            }
                        }
                        Ok(_) => {
                            // Ignore non-mention events
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            warn!(missed = n, "LLM broadcast receiver lagged");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            info!("LLM broadcast channel closed");
                            break;
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("LLM worker received shutdown signal");
                    break;
                }
            }
        }

        self.status = WorkerStatus::Stopped;
        info!("LLM worker stopped");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        info!("LLM worker stopping...");
        self.status = WorkerStatus::Stopped;
        Ok(())
    }

    fn health_check(&self) -> WorkerStatus {
        self.status.clone()
    }
}

impl LlmWorker {
    /// Static helper to call the LLM API (used inside the event loop).
    async fn call_llm(
        http_client: &Client,
        config: &LlmConfig,
        system_prompt: &str,
        user_message: &str,
    ) -> Result<String> {
        let url = format!(
            "{}/chat/completions",
            config.api_base.trim_end_matches('/')
        );

        let request = ChatRequest {
            model: config.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: user_message.to_string(),
                },
            ],
            temperature: Some(0.7),
            max_tokens: Some(512),
        };

        let response = http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", config.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .context("Failed to send request to LLM API")?;

        let status = response.status();

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            if let Ok(api_err) = serde_json::from_str::<ApiError>(&body) {
                return Err(anyhow::anyhow!(
                    "LLM API error ({}): {}",
                    status,
                    api_err.error.message
                ));
            }
            return Err(anyhow::anyhow!("LLM API error ({}): {}", status, body));
        }

        let chat_response: ChatResponse = response
            .json()
            .await
            .context("Failed to parse LLM API response")?;

        if let Some(usage) = &chat_response.usage {
            debug!(
                prompt_tokens = usage.prompt_tokens,
                completion_tokens = usage.completion_tokens,
                total_tokens = usage.total_tokens,
                "LLM token usage"
            );
        }

        chat_response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or_else(|| anyhow::anyhow!("LLM API returned no choices"))
    }
}
