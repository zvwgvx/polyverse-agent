use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::StreamExt;
use pa_core::event::{
    Event, ResponseEvent, ResponseSource,
};
use pa_core::worker::{Worker, WorkerContext, WorkerStatus};
use pa_memory::short_term::ShortTermMemory;
use pa_memory::types::ConversationKey;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
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
    /// Stream response for line-by-line delivery
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    /// OpenRouter: disable reasoning/thinking mode for models like Qwen, DeepSeek
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<ReasoningConfig>,
    /// OpenRouter: provider routing config
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<ProviderConfig>,
}

#[derive(Debug, Serialize)]
struct ReasoningConfig {
    effort: String,
}

#[derive(Debug, Serialize)]
struct ProviderConfig {
    allow_fallbacks: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
    /// OpenAI multi-participant field — identifies who sent the message.
    /// Skipped when None so it doesn't break proxies that don't support it.
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
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
    /// Shared short-term memory for conversation context
    short_term: Option<Arc<Mutex<ShortTermMemory>>>,
}

impl LlmWorker {
    pub fn new(config: LlmConfig) -> Self {
        let http_client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_default();

        Self {
            config,
            status: WorkerStatus::NotStarted,
            http_client,
            system_prompt: Self::default_system_prompt(),
            short_term: None,
        }
    }

    /// Attach a shared short-term memory handle.
    pub fn with_memory(mut self, stm: Arc<Mutex<ShortTermMemory>>) -> Self {
        self.short_term = Some(stm);
        self
    }

    pub fn with_system_prompt(mut self, prompt: String) -> Self {
        self.system_prompt = prompt;
        self
    }

    fn default_system_prompt() -> String {
        match std::fs::read_to_string("instruct.txt") {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Failed to read instruct.txt, using fallback prompt. Error: {}", e);
                r#"mày là Ryuuko, chatbot AI.
hãy trả lời ngắn gọn, tự nhiên."#.to_string()
            }
        }
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
        let short_term = self.short_term.clone();

        let mut active_tasks = tokio::task::JoinSet::new();

        loop {
            tokio::select! {
                // Reap completed tasks to free memory
                Some(_) = active_tasks.join_next() => {
                    // Task finished normally or panicked, either way we just reap it
                }
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

                            // Retrieve conversation history (exclude current msg to avoid duplicate)
                            let history = if let Some(ref stm) = short_term {
                                let key = ConversationKey::from_raw(&raw);
                                // Await safely inside the block
                                let stm_guard = stm.lock().await;
                                stm_guard.get_history_for_prompt(&key, &raw.message_id)
                            } else {
                                Vec::new()
                            };

                            if !history.is_empty() {
                                debug!(
                                    turns = history.len(),
                                    "Injecting conversation history into prompt"
                                );
                            }

                            let http_client = http_client.clone();
                            let config = config.clone();
                            let system_prompt = system_prompt.clone();
                            let event_tx = event_tx.clone();
                            let raw_clone = raw.clone();
                            let username = raw.username.clone();

                            // Spawn the LLM call into a separate async task so it doesn't block
                            // the worker's broadcast event loop. This allows concurrent processing
                            // and ensures the worker can respond to shutdown signals immediately.
                            active_tasks.spawn(async move {
                                let result = Self::call_llm(
                                    &http_client,
                                    &config,
                                    &system_prompt,
                                    history,
                                    &username,
                                    &raw_clone,
                                    &event_tx,
                                )
                                .await;

                                if let Err(e) = result {
                                    error!(
                                        error = ?e,
                                        user = %username,
                                        "LLM request failed"
                                    );
                                }
                            });
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

        // Abort any ongoing LLM requests so they don't try to send 
        // down closed channels during shutdown.
        active_tasks.abort_all();

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
        history: Vec<(String, String, String)>,
        current_username: &str,
        raw_event: &pa_core::event::RawEvent,
        event_tx: &tokio::sync::mpsc::Sender<Event>,
    ) -> Result<()> {
        let url = format!(
            "{}/chat/completions",
            config.api_base.trim_end_matches('/')
        );

        let mut messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: system_prompt.to_string(),
                name: None,
            },
        ];

        let preamble = if history.is_empty() {
            format!(
                "[context: đây là tin nhắn đầu tiên từ {}. mày chưa biết người này — đây là người lạ.]",
                current_username
            )
        } else {
            let users: Vec<&str> = history
                .iter()
                .filter(|(role, _, _)| role == "user")
                .map(|(_, username, _)| username.as_str())
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            format!(
                "[context: đã có {} tin nhắn trước đó trong cuộc hội thoại này. người tham gia: {}. hãy trả lời dựa trên mạch hội thoại ở trên.]",
                history.len(),
                users.join(", ")
            )
        };
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: preamble,
            name: None,
        });

        for (role, username, content) in history {
            messages.push(ChatMessage {
                role,
                name: if username.is_empty() { None } else { Some(username) },
                content,
            });
        }

        // Clean user content before sending
        let clean_content = strip_mention_tags(&raw_event.content, &raw_event.username);
        messages.push(ChatMessage {
            role: "user".to_string(),
            name: Some(current_username.to_string()),
            content: clean_content,
        });

        let request = ChatRequest {
            model: config.model.clone(),
            messages,
            temperature: Some(0.7),
            max_tokens: Some(512),
            stream: Some(true),
            reasoning: Some(ReasoningConfig {
                effort: "low".to_string(),
            }),
            provider: Some(ProviderConfig {
                allow_fallbacks: true,
            }),
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

        let mut stream = response.bytes_stream();
        let mut inbound_buffer = String::new();
        let mut output_buffer = String::new();
        let mut is_thinking = false;

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.context("Failed to read stream chunk")?;
            let text = String::from_utf8_lossy(&chunk);
            inbound_buffer.push_str(&text);

            while let Some(i) = inbound_buffer.find('\n') {
                let line = inbound_buffer[..i].trim().to_string();
                inbound_buffer = inbound_buffer[i + 1..].to_string();

                if line.starts_with("data: ") {
                    let data = &line["data: ".len()..];
                    if data == "[DONE]" {
                        break;
                    }

                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(choices) = json.get("choices").and_then(|c| c.as_array()) {
                            if let Some(choice) = choices.first() {
                                if let Some(content) = choice.get("delta").and_then(|d| d.get("content")).and_then(|c| c.as_str()) {
                                    if is_thinking {
                                        if let Some(end) = content.find("</think>") {
                                            is_thinking = false;
                                            output_buffer.push_str(&content[end + 8..]);
                                        }
                                    } else {
                                        if let Some(start) = content.find("<think>") {
                                            is_thinking = true;
                                            output_buffer.push_str(&content[..start]);
                                        } else {
                                            output_buffer.push_str(content);
                                        }
                                    }

                                    // Xóa khoảng trắng thừa và gom \n\n thành \n theo yêu cầu user
                                    output_buffer = output_buffer.replace("\n\n", "\n");

                                    while let Some(n) = output_buffer.find('\n') {
                                        let msg = output_buffer[..n].trim().to_string();
                                        output_buffer = output_buffer[n + 1..].to_string();

                                        if !msg.is_empty() {
                                            info!(
                                                user = %raw_event.username,
                                                content = %msg,
                                                "LLM stream emitted line"
                                            );
                                            let event = Event::Response(ResponseEvent {
                                                platform: raw_event.platform,
                                                channel_id: raw_event.channel_id.clone(),
                                                reply_to_message_id: Some(raw_event.message_id.clone()),
                                                reply_to_user: Some(raw_event.username.clone()),
                                                is_dm: raw_event.is_dm,
                                                content: msg,
                                                source: ResponseSource::CloudLLM,
                                            });
                                            if let Err(e) = event_tx.send(event).await {
                                                tracing::error!("Failed to send stream line event: {}", e);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let final_msg = output_buffer.trim().to_string();
        if !final_msg.is_empty() {
            info!(
                user = %raw_event.username,
                content = %final_msg,
                "LLM stream emitted final line"
            );
            let event = Event::Response(ResponseEvent {
                platform: raw_event.platform,
                channel_id: raw_event.channel_id.clone(),
                reply_to_message_id: Some(raw_event.message_id.clone()),
                reply_to_user: Some(raw_event.username.clone()),
                is_dm: raw_event.is_dm,
                content: final_msg,
                source: ResponseSource::CloudLLM,
            });
            if let Err(e) = event_tx.send(event).await {
                tracing::error!("Failed to send stream final line event: {}", e);
            }
        }

        Ok(())
    }
}

/// Strip Discord mention tags (e.g. `<@123456>`) from message content.
/// Returns the cleaned content, trimmed of leading/trailing whitespace.
fn strip_mention_tags(content: &str, _username: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let chars: Vec<char> = content.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<' && i + 1 < chars.len() && chars[i + 1] == '@' {
            if let Some(end) = chars[i..].iter().position(|&c| c == '>') {
                i += end + 1;
                // Skip following space
                while i < chars.len() && chars[i] == ' ' {
                    i += 1;
                }
                continue;
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result.trim().to_string()
}
