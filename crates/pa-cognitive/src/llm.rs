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
use serde_json::json;
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};
use tracing::{debug, error, info, warn};

use pa_memory::{
    episodic::EpisodicStore, 
    embedder::MemoryEmbedder,
    graph::CognitiveGraph,
};

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
    /// Maximum tokens for chat completions
    pub chat_max_tokens: u32,
    /// Optional reasoning effort (e.g. "low", "medium", "high")
    pub reasoning: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    order: Option<Vec<String>>,
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
    pub config: LlmConfig,
    status: WorkerStatus,
    http_client: Client,
    /// System prompt that defines the agent's personality
    pub system_prompt: String,
    /// Shared handle to short-term memory (injected by coordinator).
    pub short_term: Option<Arc<Mutex<ShortTermMemory>>>,
    /// Shared handle to Episodic Store for semantic searches.
    pub episodic: Option<Arc<EpisodicStore>>,
    /// Shared handle to Memory Embedder to vectorize user queries.
    pub embedder: Option<Arc<MemoryEmbedder>>,
    /// Shared handle to the Cognitive Graph (SurrealDB)
    pub graph: Option<CognitiveGraph>,
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
            episodic: None,
            embedder: None,
            graph: None,
        }
    }

    /// Attach a shared short-term memory handle.
    pub fn with_memory(mut self, stm: Arc<Mutex<ShortTermMemory>>) -> Self {
        self.short_term = Some(stm);
        self
    }

    /// Attach a shared episodic memory handle.
    pub fn with_episodic(mut self, episodic: Arc<EpisodicStore>) -> Self {
        self.episodic = Some(episodic);
        self
    }

    /// Attach a shared embedder handle.
    pub fn with_embedder(mut self, embedder: Arc<MemoryEmbedder>) -> Self {
        self.embedder = Some(embedder);
        self
    }

    /// Attach a shared cognitive graph handle.
    pub fn with_graph(mut self, graph: CognitiveGraph) -> Self {
        self.graph = Some(graph);
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
        let episodic = self.episodic.clone();
        let embedder = self.embedder.clone();
        let graph = self.graph.clone();

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
                            let cfg = config.clone();
                            let sys = system_prompt.clone();
                            let ep = episodic.clone();
                            let emb = embedder.clone();
                            let g = graph.clone();
                            let tx = event_tx.clone();
                            let raw_clone = raw.clone();
                            let username = raw_clone.username.clone(); // Use raw_clone for username

                            // Spawn the LLM call into a separate async task so it doesn't block
                            // the worker's broadcast event loop. This allows concurrent processing
                            // and ensures the worker can respond to shutdown signals immediately.
                            active_tasks.spawn(async move {
                                let result = Self::call_llm(
                                    &http_client,
                                    &cfg,
                                    &sys,
                                    history,
                                    ep.expect("Episodic store not initialized").clone(),
                                    emb.expect("Embedder not initialized").clone(),
                                    g.expect("Graph not initialized").clone(),
                                    &username,
                                    &raw_clone,
                                    &tx,
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
        http_client: &reqwest::Client,
        config: &LlmConfig,
        system_prompt: &str,
        history: Vec<(String, String, String)>,
        episodic: Arc<EpisodicStore>,
        embedder: Arc<MemoryEmbedder>,
        graph: CognitiveGraph,
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

        // --- Shared Cognitive Context ---
        let cognitive_context = crate::context::build_shared_cognitive_context(
            &history,
            Some(&episodic),
            Some(&embedder),
            &graph,
            current_username,
            &raw_event.content,
        ).await;

        if let Some(memory_text) = cognitive_context.memory_text {
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: memory_text,
                name: None,
            });
        }

        messages.push(ChatMessage {
            role: "system".to_string(),
            content: cognitive_context.social_text,
            name: None,
        });

        messages.push(ChatMessage {
            role: "system".to_string(),
            content: cognitive_context.time_and_history_text,
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
            max_tokens: Some(config.chat_max_tokens),
            stream: Some(true),
            reasoning: config.reasoning.clone().map(|effort| ReasoningConfig { effort }),
            provider: Some(ProviderConfig {
                order: Some(vec!["Google AI Studio".to_string()]),
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
        let mut full_response_buffer = String::new();
        let mut is_first_chunk = true;

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
                                            full_response_buffer.push_str(&msg);
                                            full_response_buffer.push('\n');
                                            let event = Event::Response(ResponseEvent {
                                                platform: raw_event.platform,
                                                channel_id: raw_event.channel_id.clone(),
                                                reply_to_message_id: if is_first_chunk { Some(raw_event.message_id.clone()) } else { None },
                                                reply_to_user: Some(raw_event.username.clone()),
                                                is_dm: raw_event.is_dm,
                                                content: msg,
                                                source: ResponseSource::CloudLLM,
                                            });
                                            is_first_chunk = false;
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
            full_response_buffer.push_str(&final_msg);
            let event = Event::Response(ResponseEvent {
                platform: raw_event.platform,
                channel_id: raw_event.channel_id.clone(),
                reply_to_message_id: if is_first_chunk { Some(raw_event.message_id.clone()) } else { None },
                reply_to_user: Some(raw_event.username.clone()),
                is_dm: raw_event.is_dm,
                content: final_msg,
                source: ResponseSource::CloudLLM,
            });
            is_first_chunk = false;
            if let Err(e) = event_tx.send(event).await {
                tracing::error!("Failed to send stream final line event: {}", e);
            }
        }

        let full_response = full_response_buffer.trim().to_string();
        if !full_response.is_empty() {
            let event = Event::BotTurnCompletion(pa_core::event::BotTurnCompletion {
                platform: raw_event.platform,
                channel_id: raw_event.channel_id.clone(),
                reply_to_message_id: Some(raw_event.message_id.clone()),
                reply_to_user: Some(raw_event.username.clone()),
                content: full_response,
            });
            let _ = event_tx.send(event).await;
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
