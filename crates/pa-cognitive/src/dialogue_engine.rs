use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::StreamExt;
use pa_core::get_agent_profile;
use pa_core::event::{
    Event, ResponseEvent, ResponseSource,
};
use pa_core::worker::{Worker, WorkerContext, WorkerStatus};
use pa_memory::short_term::ShortTermMemory;
use pa_memory::types::ConversationKey;
use pa_state::{StateRow, StateStore};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use pa_memory::{
    episodic::EpisodicStore, 
    embedder::MemoryEmbedder,
    graph::CognitiveGraph,
};

#[derive(Debug, Clone)]
pub struct DialogueEngineConfig {
    pub api_base: String,
    pub api_key: String,
    pub model: String,
    pub chat_max_tokens: u32,
    pub reasoning: Option<String>,
}

impl DialogueEngineConfig {
    pub fn is_valid(&self) -> bool {
        !self.api_base.is_empty()
            && !self.api_key.is_empty()
            && !self.model.is_empty()
            && !self.api_key.starts_with("your_")
    }
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<ReasoningConfig>,
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

pub struct DialogueEngineWorker {
    pub config: DialogueEngineConfig,
    status: WorkerStatus,
    http_client: Client,
    pub system_prompt: String,
    pub short_term: Option<Arc<Mutex<ShortTermMemory>>>,
    pub episodic: Option<Arc<EpisodicStore>>,
    pub embedder: Option<Arc<MemoryEmbedder>>,
    pub graph: Option<CognitiveGraph>,
    pub state_store: Option<StateStore>,
    state_prompt: StatePromptConfig,
}

impl DialogueEngineWorker {
    pub fn new(config: DialogueEngineConfig) -> Self {
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
            state_store: None,
            state_prompt: StatePromptConfig::from_env_and_file(),
        }
    }

    pub fn with_memory(mut self, stm: Arc<Mutex<ShortTermMemory>>) -> Self {
        self.short_term = Some(stm);
        self
    }

    pub fn with_episodic(mut self, episodic: Arc<EpisodicStore>) -> Self {
        self.episodic = Some(episodic);
        self
    }

    pub fn with_embedder(mut self, embedder: Arc<MemoryEmbedder>) -> Self {
        self.embedder = Some(embedder);
        self
    }

    pub fn with_graph(mut self, graph: CognitiveGraph) -> Self {
        self.graph = Some(graph);
        self
    }

    pub fn with_state_store(mut self, store: StateStore) -> Self {
        self.state_store = Some(store);
        self
    }

    pub fn with_system_prompt(mut self, prompt: String) -> Self {
        self.system_prompt = prompt;
        self
    }

    fn default_system_prompt() -> String {
        match pa_core::prompt_registry::get_prompt("persona.base") {
            Ok(s) => s,
            Err(e) => {
                let profile = get_agent_profile();
                let fallback = format!(
                    "You are {}, an AI chatbot.\nReply briefly and naturally.\n",
                    profile.display_name
                );
                tracing::warn!(
                    "Failed to load persona.base from prompt registry, using fallback prompt. Error: {}",
                    e
                );
                pa_core::prompt_registry::get_prompt_or(
                    "dialogue_engine.fallback",
                    fallback.as_str(),
                )
            }
        }
    }

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
            .context("Failed to connect to dialogue engine API")?;

        if response.status().is_success() {
            info!(
                api_base = %self.config.api_base,
                model = %self.config.model,
                "Dialogue engine API connection validated"
            );
            Ok(())
        } else if response.status().as_u16() == 401 || response.status().as_u16() == 403 {
            Err(anyhow::anyhow!("Dialogue engine API authentication failed — check your API key"))
        } else {
            warn!(
                status = %response.status(),
                api_base = %self.config.api_base,
                "Dialogue engine API /models endpoint returned error (API may still work for chat)"
            );
            Ok(())
        }
    }
}

fn format_state_snapshot(rows: &[StateRow], config: &StateSnapshotConfig) -> String {
    if rows.is_empty() || !config.enabled {
        return String::new();
    }

    let mut lines = Vec::new();
    for domain in &config.domains {
        let mut parts = Vec::new();
        for row in rows.iter().filter(|row| row.domain == domain.as_str()) {
            if !config.include_derived && row.update_mode.eq_ignore_ascii_case("derived") {
                continue;
            }
            let key = row.id.split('.').nth(1).unwrap_or(&row.id);
            parts.push(format!(
                "{}={value:.precision$}",
                key,
                value = row.value,
                precision = config.precision
            ));
        }
        if !parts.is_empty() {
            lines.push(format!("{}: {}", domain, parts.join(", ")));
        }
    }

    lines.join("\n")
}

#[derive(Clone)]
struct StateSnapshotConfig {
    enabled: bool,
    precision: usize,
    include_derived: bool,
    domains: Vec<String>,
}

impl Default for StateSnapshotConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            precision: 3,
            include_derived: true,
            domains: vec![
                "session_social".to_string(),
                "emotion".to_string(),
                "system".to_string(),
                "preference".to_string(),
                "style".to_string(),
                "cognition".to_string(),
                "risk".to_string(),
                "user".to_string(),
                "goal".to_string(),
                "environment".to_string(),
            ],
        }
    }
}

impl StateSnapshotConfig {
    fn apply_env_overrides(&mut self) {
        if let Ok(value) = std::env::var("STATE_PROMPT_ENABLED") {
            self.enabled = matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes");
        }
        if let Ok(value) = std::env::var("STATE_PROMPT_PRECISION") {
            if let Ok(parsed) = value.parse::<usize>() {
                self.precision = parsed.clamp(0, 6);
            }
        }
        if let Ok(value) = std::env::var("STATE_PROMPT_INCLUDE_DERIVED") {
            self.include_derived = matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes");
        }
        if let Ok(value) = std::env::var("STATE_PROMPT_DOMAINS") {
            let mut domains = Vec::new();
            for raw in value.split(',') {
                let domain = raw.trim();
                if domain.is_empty() {
                    continue;
                }
                domains.push(domain.to_string());
            }
            if !domains.is_empty() {
                self.domains = domains;
            }
        }
    }
}

#[derive(Clone)]
struct StateLegendConfig {
    enabled: bool,
    prompt_key: String,
}

impl Default for StateLegendConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            prompt_key: "context.state.legend".to_string(),
        }
    }
}

#[derive(Clone)]
struct StatePromptConfig {
    snapshot: StateSnapshotConfig,
    legend: StateLegendConfig,
}

impl Default for StatePromptConfig {
    fn default() -> Self {
        Self {
            snapshot: StateSnapshotConfig::default(),
            legend: StateLegendConfig::default(),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct StatePromptFileConfig {
    snapshot: Option<StateSnapshotFileConfig>,
    legend: Option<StateLegendFileConfig>,
}

#[derive(Debug, Deserialize, Default)]
struct StateSnapshotFileConfig {
    enabled: Option<bool>,
    precision: Option<usize>,
    include_derived: Option<bool>,
    domains: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Default)]
struct StateLegendFileConfig {
    enabled: Option<bool>,
    prompt: Option<String>,
}

impl StatePromptConfig {
    fn from_env_and_file() -> Self {
        let mut config = Self::default();

        let path = std::env::var("STATE_PROMPT_CONFIG_PATH")
            .unwrap_or_else(|_| "config/state_prompt.json".to_string());
        if let Ok(raw) = std::fs::read_to_string(&path) {
            match serde_json::from_str::<StatePromptFileConfig>(&raw) {
                Ok(file) => {
                    if let Some(snapshot) = file.snapshot {
                        if let Some(enabled) = snapshot.enabled {
                            config.snapshot.enabled = enabled;
                        }
                        if let Some(precision) = snapshot.precision {
                            config.snapshot.precision = precision.clamp(0, 6);
                        }
                        if let Some(include_derived) = snapshot.include_derived {
                            config.snapshot.include_derived = include_derived;
                        }
                        if let Some(domains) = snapshot.domains {
                            let mut filtered = Vec::new();
                            for domain in domains {
                                let trimmed = domain.trim().to_string();
                                if !trimmed.is_empty() {
                                    filtered.push(trimmed);
                                }
                            }
                            if !filtered.is_empty() {
                                config.snapshot.domains = filtered;
                            }
                        }
                    }
                    if let Some(legend) = file.legend {
                        if let Some(enabled) = legend.enabled {
                            config.legend.enabled = enabled;
                        }
                        if let Some(prompt) = legend.prompt {
                            if !prompt.trim().is_empty() {
                                config.legend.prompt_key = prompt.trim().to_string();
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, path = %path, "Failed to parse state prompt config, using defaults");
                }
            }
        }

        config.snapshot.apply_env_overrides();
        config
    }
}

#[async_trait]
impl Worker for DialogueEngineWorker {
    fn name(&self) -> &str {
        "dialogue_engine"
    }

    async fn start(&mut self, ctx: WorkerContext) -> Result<()> {
        info!(
            api_base = %self.config.api_base,
            model = %self.config.model,
            "Dialogue engine worker starting..."
        );

        if !self.config.is_valid() {
            warn!("Dialogue engine config is incomplete, disabling dialogue engine worker");
            self.status = WorkerStatus::Stopped;
            return Ok(());
        }

        match self.validate_connection().await {
            Ok(_) => {}
            Err(e) if e.to_string().contains("authentication") => {
                warn!(error = %e, "Dialogue engine API auth failed, disabling Dialogue engine worker");
                self.status = WorkerStatus::Stopped;
                return Ok(());
            }
            Err(e) => {
                warn!(error = %e, "Dialogue engine API validation failed — will try anyway");
            }
        }

        self.status = WorkerStatus::Healthy;
        info!("Dialogue engine worker ready");

        let mut broadcast_rx = ctx.subscribe_events();
        let mut shutdown_rx = ctx.subscribe_shutdown();
        let event_tx = ctx.event_tx.clone();

        let http_client = self.http_client.clone();
        let config = self.config.clone();
        let system_prompt = self.system_prompt.clone();
        let short_term = self.short_term.clone();
        let episodic = self.episodic.clone();
        let embedder = self.embedder.clone();
        let graph = self.graph.clone();
        let state_store = self.state_store.clone();
        let state_prompt = self.state_prompt.clone();

        let mut active_tasks = tokio::task::JoinSet::new();

        loop {
            tokio::select! {
                Some(_) = active_tasks.join_next() => {
                }
                result = broadcast_rx.recv() => {
                    match result {
                        Ok(Event::Raw(raw)) if raw.is_mention => {
                            info!(
                                user = %raw.username,
                                platform = %raw.platform,
                                content = %raw.content,
                                "Processing mention — sending to dialogue engine"
                            );

                            let history = if let Some(ref stm) = short_term {
                                let key = ConversationKey::from_raw(&raw);
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
                            let st = state_store.clone();
                            let sp = state_prompt.clone();
                            let tx = event_tx.clone();
                            let raw_clone = raw.clone();
                            let username = raw_clone.username.clone();

                            active_tasks.spawn(async move {
                                let result = Self::call_dialogue_engine(
                                    &http_client,
                                    &cfg,
                                    &sys,
                                    history,
                                    ep.expect("Episodic store not initialized").clone(),
                                    emb.expect("Embedder not initialized").clone(),
                                    g.expect("Graph not initialized").clone(),
                                    st,
                                    sp,
                                    &username,
                                    &raw_clone,
                                    &tx,
                                )
                                .await;

                                if let Err(e) = result {
                                    error!(
                                        error = ?e,
                                        user = %username,
                                        "Dialogue engine request failed"
                                    );
                                }
                            });
                        }
                        Ok(_) => {
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            warn!(missed = n, "Dialogue engine broadcast receiver lagged");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            info!("Dialogue engine broadcast channel closed");
                            break;
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Dialogue engine worker received shutdown signal");
                    break;
                }
            }
        }

        active_tasks.abort_all();

        self.status = WorkerStatus::Stopped;
        info!("Dialogue engine worker stopped");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        info!("Dialogue engine worker stopping...");
        self.status = WorkerStatus::Stopped;
        Ok(())
    }

    fn health_check(&self) -> WorkerStatus {
        self.status.clone()
    }
}

impl DialogueEngineWorker {
    async fn call_dialogue_engine(
        http_client: &reqwest::Client,
        config: &DialogueEngineConfig,
        system_prompt: &str,
        history: Vec<(String, String, String)>,
        episodic: Arc<EpisodicStore>,
        embedder: Arc<MemoryEmbedder>,
        graph: CognitiveGraph,
        state_store: Option<StateStore>,
        state_prompt: StatePromptConfig,
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

        let cognitive_context = crate::context::build_shared_cognitive_context(
            &raw_event.message_id,
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

        if let Some(store) = state_store {
            let rows = store.rows().await;
            let snapshot = format_state_snapshot(&rows, &state_prompt.snapshot);
            if !snapshot.is_empty() {
                if state_prompt.legend.enabled {
                    let legend_text = pa_core::prompt_registry::render_prompt_or(
                        state_prompt.legend.prompt_key.as_str(),
                        &[],
                        "[STATE INTERPRETATION]\nRanges vary by dimension. Most are 0..1; some are -1..1 (negative to positive).\nUse states as soft signals to shape tone, effort, and prioritization; do not mention them explicitly.\n\nsession_social: affinity/attachment/trust/safety are -1..1 (negative to positive); tension is 0..1 (higher = more friction).\nemotion: valence is -1..1; arousal/joy/sadness/anger/anxiety/confidence/stability are 0..1 (higher = stronger).\nsystem: energy (capacity), fatigue (strain), responsiveness (readiness).\npreference: curiosity/stress/depth/brevity/directness/empathy_bias/risk_tolerance are 0..1; fascination is -1..1 (negative to positive interest).\nstyle: warmth/playfulness/formality/brevity control response tone.\ncognition (derived): clarity/focus/coherence/creativity/decisiveness/consistency, higher = stronger.\nrisk (derived): safety/privacy/escalation risk, higher = higher risk -> be more cautious.\nuser: engagement/familiarity are 0..1; trust/reliability/boundary_respect/sentiment are -1..1.\ngoal: focus/commitment/clarity/urgency/constraint_pressure/satisfaction/progress, higher = stronger.\nenvironment: load/noise/time_pressure higher = more friction; channel_quality higher = better conditions.\n",
                    );
                    messages.push(ChatMessage {
                        role: "system".to_string(),
                        content: legend_text,
                        name: None,
                    });
                }
                let state_text = pa_core::prompt_registry::render_prompt_or(
                    "context.state.snapshot",
                    &[("state_lines", snapshot.as_str())],
                    "### INTERNAL STATE SNAPSHOT\n{{state_lines}}\n",
                );
                messages.push(ChatMessage {
                    role: "system".to_string(),
                    content: state_text,
                    name: None,
                });
            }
        }

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
                order: None,
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
            .context("Failed to send request to dialogue engine API")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            warn!(status = %status, api_error = %body, user = %raw_event.username, "Dialogue engine returned API error");
            if let Ok(api_err) = serde_json::from_str::<ApiError>(&body) {
                return Err(anyhow::anyhow!(
                    "Dialogue engine API error ({}): {} | body: {}",
                    status,
                    api_err.error.message,
                    body
                ));
            }
            return Err(anyhow::anyhow!("Dialogue engine API error ({}): {}", status, body));
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

                                    output_buffer = output_buffer.replace("\n\n", "\n");

                                    while let Some(n) = output_buffer.find('\n') {
                                        let msg = output_buffer[..n].trim().to_string();
                                        output_buffer = output_buffer[n + 1..].to_string();

                                        if !msg.is_empty() {
                                            info!(
                                                user = %raw_event.username,
                                                content = %msg,
                                                "Dialogue engine stream emitted line"
                                            );
                                            full_response_buffer.push_str(&msg);
                                            full_response_buffer.push('\n');
                                            if raw_event.platform != pa_core::event::Platform::DiscordSelfbot {
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
        }

        let final_msg = output_buffer.trim().to_string();
        if !final_msg.is_empty() {
            info!(
                user = %raw_event.username,
                content = %final_msg,
                "Dialogue engine stream emitted final line"
            );
            full_response_buffer.push_str(&final_msg);
            if raw_event.platform != pa_core::event::Platform::DiscordSelfbot {
                let event = Event::Response(ResponseEvent {
                    platform: raw_event.platform,
                    channel_id: raw_event.channel_id.clone(),
                    reply_to_message_id: if is_first_chunk { Some(raw_event.message_id.clone()) } else { None },
                    reply_to_user: Some(raw_event.username.clone()),
                    is_dm: raw_event.is_dm,
                    content: final_msg,
                    source: ResponseSource::CloudLLM,
                });
                if let Err(e) = event_tx.send(event).await {
                    tracing::error!("Failed to send stream final line event: {}", e);
                }
            }
        }

        let full_response = full_response_buffer.trim().to_string();
        if raw_event.platform == pa_core::event::Platform::DiscordSelfbot && !full_response.is_empty() {
            let event = Event::Response(ResponseEvent {
                platform: raw_event.platform,
                channel_id: raw_event.channel_id.clone(),
                reply_to_message_id: Some(raw_event.message_id.clone()),
                reply_to_user: Some(raw_event.username.clone()),
                is_dm: raw_event.is_dm,
                content: full_response.clone(),
                source: ResponseSource::CloudLLM,
            });
            if let Err(e) = event_tx.send(event).await {
                tracing::error!("Failed to send selfbot final response event: {}", e);
            }
        }
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

fn strip_mention_tags(content: &str, _username: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let chars: Vec<char> = content.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<' && i + 1 < chars.len() && chars[i + 1] == '@' {
            if let Some(end) = chars[i..].iter().position(|&c| c == '>') {
                i += end + 1;
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
