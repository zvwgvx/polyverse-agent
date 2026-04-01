use std::collections::BTreeSet;
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
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::dialogue_tools::{DialogueToolRegistry, SOCIAL_GET_DIALOGUE_SUMMARY_TOOL};

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
    pub tool_calling: DialogueToolCallingConfig,
}

#[derive(Debug, Clone)]
pub struct DialogueToolCallingConfig {
    pub enabled: bool,
    pub max_calls_per_turn: usize,
    pub timeout_ms: u64,
    pub max_candidate_users: usize,
}

impl Default for DialogueToolCallingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_calls_per_turn: 2,
            timeout_ms: 1_500,
            max_candidate_users: 3,
        }
    }
}

impl DialogueToolCallingConfig {
    fn enabled(&self) -> bool {
        self.enabled && self.max_calls_per_turn > 0 && self.max_candidate_users > 0
    }
}

impl DialogueEngineConfig {
    pub fn is_valid(&self) -> bool {
        !self.api_base.is_empty()
            && !self.api_key.is_empty()
            && !self.model.is_empty()
            && !self.api_key.starts_with("your_")
    }
}

#[derive(Debug, Clone)]
struct DialoguePromptBundle {
    merged_system_prompt: String,
    messages: Vec<ChatMessage>,
    social_mode: &'static str,
    social_gate_open: bool,
    memory_hint: f32,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    #[serde(default)]
    message: Option<AssistantMessage>,
}

#[derive(Debug, Deserialize)]
struct AssistantMessage {
    #[serde(default)]
    tool_calls: Option<Vec<RequestedToolCall>>,
}

#[derive(Debug, Deserialize, Clone)]
struct RequestedToolCall {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    r#type: Option<String>,
    function: RequestedFunctionCall,
}

#[derive(Debug, Deserialize, Clone)]
struct RequestedFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize)]
struct ToolChatRequest {
    model: String,
    messages: Vec<Value>,
    tools: Vec<ToolDefinition>,
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
struct FinalChatRequest {
    model: String,
    messages: Vec<Value>,
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
struct ToolDefinition {
    #[serde(rename = "type")]
    kind: &'static str,
    function: ToolFunctionDefinition,
}

#[derive(Debug, Serialize)]
struct ToolFunctionDefinition {
    name: &'static str,
    description: &'static str,
    parameters: Value,
}

#[derive(Debug, Serialize)]
struct ToolMessage {
    role: &'static str,
    tool_call_id: String,
    name: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct AssistantToolCallMessage {
    role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    tool_calls: Vec<AssistantToolCallPayload>,
}

#[derive(Debug, Serialize)]
struct AssistantToolCallPayload {
    id: String,
    #[serde(rename = "type")]
    kind: &'static str,
    function: AssistantToolCallFunction,
}

#[derive(Debug, Serialize)]
struct AssistantToolCallFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize)]
struct ToolResultEnvelope {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Clone)]
struct ToolLoopOutcome {
    messages: Vec<Value>,
    executed_calls: usize,
    degraded: bool,
}

#[derive(Debug, Deserialize)]
struct DialogueToolArguments {
    user_id: String,
    #[serde(default)]
    memory_hint: Option<f32>,
    #[serde(default)]
    max_staleness_ms: Option<i64>,
    #[serde(default)]
    allow_stale_fallback: Option<bool>,
    #[serde(default)]
    force_project: Option<bool>,
}

const DIALOGUE_TOOL_POLICY_FALLBACK: &str = "### INTERNAL TOOL POLICY
Use internal read-only social tools only when they are available in this turn. Never mention tools to the user. Query only the provided candidate users and use the smallest number of tool calls needed. If tool use is unnecessary, answer normally.
";

const DIALOGUE_TOOL_DESCRIPTION: &str = "Read a compact social summary for one allowed conversation participant so the assistant can ground tone and continuity.";

fn default_tool_definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition {
        kind: "function",
        function: ToolFunctionDefinition {
            name: SOCIAL_GET_DIALOGUE_SUMMARY_TOOL,
            description: DIALOGUE_TOOL_DESCRIPTION,
            parameters: json!({
                "type": "object",
                "properties": {
                    "user_id": {
                        "type": "string",
                        "description": "One allowed candidate user to inspect."
                    },
                    "memory_hint": {
                        "type": "number",
                        "description": "Optional memory hint between 0 and 1."
                    },
                    "max_staleness_ms": {
                        "type": "integer",
                        "description": "Optional staleness budget in milliseconds."
                    },
                    "allow_stale_fallback": {
                        "type": "boolean"
                    },
                    "force_project": {
                        "type": "boolean"
                    }
                },
                "required": ["user_id"],
                "additionalProperties": false
            }),
        },
    }]
}

fn message_to_value<T: Serialize>(message: T) -> Value {
    serde_json::to_value(message).unwrap_or_else(|_| json!({}))
}

fn make_system_message(content: String) -> Value {
    json!({
        "role": "system",
        "content": content,
    })
}

fn make_user_message(name: Option<String>, content: String) -> Value {
    match name {
        Some(name) => json!({
            "role": "user",
            "name": name,
            "content": content,
        }),
        None => json!({
            "role": "user",
            "content": content,
        }),
    }
}

fn sanitize_requested_tool_kind(kind: Option<String>) -> &'static str {
    match kind.as_deref() {
        Some("function") | None => "function",
        _ => "function",
    }
}

fn make_assistant_tool_call_message(tool_calls: Vec<RequestedToolCall>) -> Value {
    let payload = tool_calls
        .into_iter()
        .enumerate()
        .map(|(idx, call)| AssistantToolCallPayload {
            id: call.id.unwrap_or_else(|| format!("tool-call-{}", idx + 1)),
            kind: sanitize_requested_tool_kind(call.r#type),
            function: AssistantToolCallFunction {
                name: call.function.name,
                arguments: call.function.arguments,
            },
        })
        .collect::<Vec<_>>();

    message_to_value(AssistantToolCallMessage {
        role: "assistant",
        content: None,
        tool_calls: payload,
    })
}

fn make_tool_result_message(tool_call_id: String, name: String, content: String) -> Value {
    message_to_value(ToolMessage {
        role: "tool",
        tool_call_id,
        name,
        content,
    })
}

fn tool_result_ok(result: Value) -> String {
    serde_json::to_string(&ToolResultEnvelope {
        ok: true,
        result: Some(result),
        error: None,
    })
    .unwrap_or_else(|_| "{\"ok\":true}".to_string())
}

fn tool_result_err(error: String) -> String {
    serde_json::to_string(&ToolResultEnvelope {
        ok: false,
        result: None,
        error: Some(error),
    })
    .unwrap_or_else(|_| "{\"ok\":false,\"error\":\"serialization_error\"}".to_string())
}

fn clamp_tool_memory_hint(value: Option<f32>, fallback: f32) -> f32 {
    value.unwrap_or(fallback).clamp(0.0, 1.0)
}

fn build_allowed_candidate_users(
    current_username: &str,
    history: &[(String, String, String)],
    max_candidate_users: usize,
) -> Vec<String> {
    let limit = max_candidate_users.max(1);
    let mut users = BTreeSet::new();

    let current = current_username.trim();
    if !current.is_empty() {
        users.insert(current.to_string());
    }

    for (role, username, _) in history.iter().rev() {
        if role != "user" {
            continue;
        }
        let trimmed = username.trim();
        if trimmed.is_empty() {
            continue;
        }
        users.insert(trimmed.to_string());
        if users.len() >= limit {
            break;
        }
    }

    users.into_iter().take(limit).collect()
}

fn tool_policy_block(candidate_users: &[String]) -> String {
    let joined = candidate_users.join(", ");
    let prompt = pa_core::prompt_registry::render_prompt_or(
        "dialogue_engine.tool_policy",
        &[("candidate_users", joined.as_str())],
        DIALOGUE_TOOL_POLICY_FALLBACK,
    );

    if joined.is_empty() {
        prompt
    } else {
        format!("{}\nAllowed candidate users: {}", prompt, joined)
    }
}

fn memory_hint_from_history(history_len: usize) -> f32 {
    (history_len as f32 / 12.0).min(0.15)
}

fn should_use_legacy_summary_path(
    config: &DialogueEngineConfig,
    decision: SocialFetchDecision,
) -> bool {
    decision.should_fetch && !config.tool_calling.enabled()
}

fn should_attempt_tool_loop(config: &DialogueEngineConfig, decision: SocialFetchDecision) -> bool {
    config.tool_calling.enabled() && decision.should_fetch
}

fn tool_calling_limits(config: &DialogueToolCallingConfig) -> (usize, u64, usize) {
    (
        config.max_calls_per_turn.max(1),
        config.timeout_ms.max(100),
        config.max_candidate_users.max(1),
    )
}

fn build_final_messages(
    bundle: &DialoguePromptBundle,
    user_name: String,
    user_content: String,
) -> Vec<Value> {
    let mut messages = Vec::with_capacity(bundle.messages.len() + 2);
    messages.push(make_system_message(bundle.merged_system_prompt.clone()));
    for message in &bundle.messages {
        messages.push(message_to_value(message.clone()));
    }
    messages.push(make_user_message(Some(user_name), user_content));
    messages
}

fn build_tool_planning_messages(
    bundle: &DialoguePromptBundle,
    candidate_users: &[String],
    user_name: String,
    user_content: String,
) -> Vec<Value> {
    let mut messages = Vec::with_capacity(bundle.messages.len() + 2);
    messages.push(make_system_message(format!(
        "{}\n\n{}",
        bundle.merged_system_prompt,
        tool_policy_block(candidate_users)
    )));
    for message in &bundle.messages {
        messages.push(message_to_value(message.clone()));
    }
    messages.push(make_user_message(Some(user_name), user_content));
    messages
}

fn build_streaming_request(config: &DialogueEngineConfig, messages: Vec<Value>) -> FinalChatRequest {
    FinalChatRequest {
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
    }
}

fn build_tool_planning_request(
    config: &DialogueEngineConfig,
    messages: Vec<Value>,
) -> ToolChatRequest {
    ToolChatRequest {
        model: config.model.clone(),
        messages,
        tools: default_tool_definitions(),
        temperature: Some(0.3),
        max_tokens: Some(config.chat_max_tokens),
        stream: Some(false),
        reasoning: config.reasoning.clone().map(|effort| ReasoningConfig { effort }),
        provider: Some(ProviderConfig {
            order: None,
            allow_fallbacks: true,
        }),
    }
}

fn parse_dialogue_tool_arguments(raw: &str) -> Result<DialogueToolArguments> {
    serde_json::from_str(raw).context("failed to parse dialogue tool arguments")
}

async fn execute_dialogue_tool_loop(
    http_client: &Client,
    config: &DialogueEngineConfig,
    graph: &CognitiveGraph,
    planning_messages: Vec<Value>,
    candidate_users: &[String],
    memory_hint: f32,
) -> Result<ToolLoopOutcome> {
    let registry = DialogueToolRegistry::default();
    let (max_calls_per_turn, timeout_ms, _) = tool_calling_limits(&config.tool_calling);
    let mut messages = planning_messages;
    let mut executed_calls = 0usize;

    while executed_calls < max_calls_per_turn {
        let request = build_tool_planning_request(config, messages.clone());
        let url = format!("{}/chat/completions", config.api_base.trim_end_matches('/'));
        let response = tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            http_client
                .post(&url)
                .header("Authorization", format!("Bearer {}", config.api_key))
                .header("Content-Type", "application/json")
                .json(&request)
                .send(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("dialogue tool planning timeout"))??;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(anyhow::anyhow!(
                "dialogue tool planning API error ({}): {}",
                status,
                body
            ));
        }

        let parsed: ChatCompletionResponse = serde_json::from_str(&body)
            .context("failed to parse dialogue tool planning response")?;
        let Some(choice) = parsed.choices.into_iter().next() else {
            return Err(anyhow::anyhow!("dialogue tool planning returned no choices"));
        };
        let Some(message) = choice.message else {
            return Err(anyhow::anyhow!("dialogue tool planning returned no message"));
        };

        let tool_calls = message.tool_calls.unwrap_or_default();
        if tool_calls.is_empty() {
            return Ok(ToolLoopOutcome {
                messages,
                executed_calls,
                degraded: false,
            });
        }

        messages.push(make_assistant_tool_call_message(tool_calls.clone()));

        for call in tool_calls {
            if executed_calls >= max_calls_per_turn {
                break;
            }

            let tool_call_id = call
                .id
                .clone()
                .unwrap_or_else(|| format!("tool-call-{}", executed_calls + 1));
            let tool_name = call.function.name.clone();
            if tool_name != SOCIAL_GET_DIALOGUE_SUMMARY_TOOL {
                return Err(anyhow::anyhow!(
                    "unsupported dialogue tool requested: {}",
                    tool_name
                ));
            }

            let args = parse_dialogue_tool_arguments(&call.function.arguments)?;
            let requested_user = args.user_id.trim().to_string();
            if requested_user.is_empty() {
                return Err(anyhow::anyhow!("dialogue tool user_id is required"));
            }
            if !candidate_users.iter().any(|user| user == &requested_user) {
                return Err(anyhow::anyhow!(
                    "dialogue tool target is not in allowed candidate set: {}",
                    requested_user
                ));
            }

            let input = json!({
                "user_id": requested_user,
                "memory_hint": clamp_tool_memory_hint(args.memory_hint, memory_hint),
                "max_staleness_ms": args.max_staleness_ms,
                "allow_stale_fallback": args.allow_stale_fallback,
                "force_project": args.force_project,
            });

            let result = tokio::time::timeout(
                std::time::Duration::from_millis(timeout_ms),
                registry.execute(&tool_name, input, graph),
            )
            .await
            .map_err(|_| anyhow::anyhow!("dialogue tool execution timeout"))?;

            let content = match result {
                Ok(value) => tool_result_ok(value),
                Err(err) => tool_result_err(err.to_string()),
            };

            messages.push(make_tool_result_message(tool_call_id, tool_name, content));
            executed_calls += 1;
        }
    }

    Ok(ToolLoopOutcome {
        messages,
        executed_calls,
        degraded: false,
    })
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

#[derive(Debug, Clone, Copy)]
struct SocialFetchDecision {
    should_fetch: bool,
    score: u8,
    has_strong_social_cue: bool,
    has_continuity_cue: bool,
    recent_social_signal: bool,
    short_follow_up: bool,
}

impl DialogueEngineWorker {
    fn contains_any(text: &str, cues: &[&str]) -> bool {
        cues.iter().any(|cue| text.contains(cue))
    }

    fn social_fetch_decision(
        raw_event: &pa_core::event::RawEvent,
        history: &[(String, String, String)],
    ) -> SocialFetchDecision {
        if history.is_empty() {
            return SocialFetchDecision {
                should_fetch: false,
                score: 0,
                has_strong_social_cue: false,
                has_continuity_cue: false,
                recent_social_signal: false,
                short_follow_up: false,
            };
        }

        let lowered = raw_event.content.to_lowercase();
        let short_follow_up = lowered.trim().chars().count() <= 28;

        const STRONG_SOCIAL_CUES: &[&str] = &[
            "xin lỗi",
            "sorry",
            "giận",
            "tin tưởng",
            "trust",
            "ghét",
            "hate",
            "yêu",
            "love",
            "nhớ",
            "miss",
            "buồn",
            "sad",
            "căng",
            "tension",
            "đừng",
            "đừng có",
            "stop",
            "khó chịu",
            "hurt",
            "thất vọng",
            "disappoint",
            "tha thứ",
            "forgive",
            "quan hệ",
            "mối quan hệ",
            "cách m nghĩ về t",
            "cách mày nghĩ về tao",
        ];

        const CONTINUITY_CUES: &[&str] = &[
            "hôm qua",
            "lúc nãy",
            "vừa nãy",
            "nãy",
            "ban nãy",
            "như t nói",
            "như tao nói",
            "như m nói",
            "như mày nói",
            "again",
            "still",
            "as i said",
            "as you said",
        ];

        let has_strong_social_cue = Self::contains_any(&lowered, STRONG_SOCIAL_CUES);
        let has_continuity_cue = Self::contains_any(&lowered, CONTINUITY_CUES);

        let recent_social_signal = history
            .iter()
            .rev()
            .take(4)
            .any(|(_, _, content)| {
                let recent = content.to_lowercase();
                Self::contains_any(&recent, STRONG_SOCIAL_CUES)
                    || Self::contains_any(&recent, CONTINUITY_CUES)
            });

        let mut score = 0_u8;
        if has_strong_social_cue {
            score += 3;
        }
        if has_continuity_cue {
            score += 2;
        }
        if recent_social_signal {
            score += 2;
        }
        if short_follow_up && recent_social_signal {
            score += 1;
        }
        if raw_event.is_dm && (has_continuity_cue || recent_social_signal) {
            score += 1;
        }

        SocialFetchDecision {
            should_fetch: score >= 3,
            score,
            has_strong_social_cue,
            has_continuity_cue,
            recent_social_signal,
            short_follow_up,
        }
    }

    async fn call_dialogue_engine(
        http_client: &reqwest::Client,
        config: &DialogueEngineConfig,
        system_prompt: &str,
        history: Vec<(String, String, String)>,
        episodic: Arc<EpisodicStore>,
        embedder: Arc<MemoryEmbedder>,
        _graph: CognitiveGraph,
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

        let mut system_blocks: Vec<String> = vec![system_prompt.to_string()];
        let cognitive_context = crate::context::build_shared_cognitive_context(
            &raw_event.message_id,
            &history,
            Some(&episodic),
            Some(&embedder),
            current_username,
            &raw_event.content,
        )
        .await;

        if let Some(memory_text) = cognitive_context.memory_text {
            system_blocks.push(memory_text);
        }

        let mut social_mode = "none";
        let mut social_fetch_decision = None;
        let memory_hint = memory_hint_from_history(history.len());
        let clean_content = strip_mention_tags(&raw_event.content, &raw_event.username);

        if let Some(dialogue_social_text) = cognitive_context.dialogue_social_text {
            social_mode = "full";
            debug!(
                kind = "prompt.social",
                user = %current_username,
                mode = social_mode,
                reason = "explicit_dialogue_social_text",
                "Dialogue social context injected"
            );
            system_blocks.push(dialogue_social_text);
        } else {
            let decision = Self::social_fetch_decision(raw_event, &history);
            social_fetch_decision = Some(decision);
            debug!(
                kind = "prompt.social",
                user = %current_username,
                mode = social_mode,
                should_fetch_social = decision.should_fetch,
                score = decision.score,
                has_strong_social_cue = decision.has_strong_social_cue,
                has_continuity_cue = decision.has_continuity_cue,
                recent_social_signal = decision.recent_social_signal,
                short_follow_up = decision.short_follow_up,
                history_len = history.len(),
                is_dm = raw_event.is_dm,
                "Dialogue social context decision"
            );

            if should_use_legacy_summary_path(config, decision) {
                if let Some(summary) = crate::social_context::load_dialogue_social_summary(
                    &_graph,
                    current_username,
                    memory_hint,
                )
                .await
                {
                    social_mode = "summary";
                    debug!(
                        kind = "prompt.social",
                        user = %summary.user_id,
                        mode = social_mode,
                        familiarity = summary.familiarity,
                        trust = summary.trust_state,
                        tension = summary.tension_state,
                        "Injected dialogue social summary context"
                    );
                    system_blocks.push(summary.summary);
                } else {
                    debug!(
                        kind = "prompt.social",
                        user = %current_username,
                        mode = social_mode,
                        "Dialogue social summary fetch returned none"
                    );
                }
            }
        }

        let social_context_insert_index = system_blocks.len();

        if let Some(store) = state_store {
            let rows = store.rows().await;
            let snapshot = format_state_snapshot(&rows, &state_prompt.snapshot);
            if !snapshot.is_empty() {
                if state_prompt.legend.enabled {
                    let legend_text = pa_core::prompt_registry::render_prompt_or(
                        state_prompt.legend.prompt_key.as_str(),
                        &[],
                        "[STATE INTERPRETATION]
Ranges vary by dimension. Most are 0..1; some are -1..1 (negative to positive).
Use states as soft signals to shape tone, effort, and prioritization; do not mention them explicitly.

session_social: affinity/attachment/trust/safety are -1..1 (negative to positive); tension is 0..1 (higher = more friction).
emotion: valence is -1..1; arousal/joy/sadness/anger/anxiety/confidence/stability are 0..1 (higher = stronger).
system: energy (capacity), fatigue (strain), responsiveness (readiness).
preference: curiosity/stress/depth/brevity/directness/empathy_bias/risk_tolerance are 0..1; fascination is -1..1 (negative to positive interest).
style: warmth/playfulness/formality/brevity control response tone.
cognition (derived): clarity/focus/coherence/creativity/decisiveness/consistency, higher = stronger.
risk (derived): safety/privacy/escalation risk, higher = higher risk -> be more cautious.
user: engagement/familiarity are 0..1; trust/reliability/boundary_respect/sentiment are -1..1.
goal: focus/commitment/clarity/urgency/constraint_pressure/satisfaction/progress, higher = stronger.
environment: load/noise/time_pressure higher = more friction; channel_quality higher = better conditions.
",
                    );
                    system_blocks.push(legend_text);
                }
                let state_text = pa_core::prompt_registry::render_prompt_or(
                    "context.state.snapshot",
                    &[("state_lines", snapshot.as_str())],
                    "### INTERNAL STATE SNAPSHOT
{{state_lines}}
",
                );
                system_blocks.push(state_text);
            }
        }

        system_blocks.push(cognitive_context.time_and_history_text);

        let history_messages: Vec<ChatMessage> = history
            .iter()
            .cloned()
            .map(|(role, username, content)| ChatMessage {
                role,
                name: if username.is_empty() { None } else { Some(username) },
                content,
            })
            .collect();

        let social_gate_open = social_fetch_decision
            .map(|decision| decision.should_fetch)
            .unwrap_or(false);
        let mut tool_loop_degraded = false;
        let mut tool_loop_executed_calls = 0usize;

        let mut merged_system_prompt = system_blocks
            .iter()
            .filter(|block| !block.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join("

");

        let mut bundle = DialoguePromptBundle {
            merged_system_prompt: merged_system_prompt.clone(),
            messages: history_messages.clone(),
            social_mode,
            social_gate_open,
            memory_hint,
        };

        let mut final_messages = build_final_messages(
            &bundle,
            current_username.to_string(),
            clean_content.clone(),
        );

        if let Some(decision) = social_fetch_decision {
            if should_attempt_tool_loop(config, decision) {
                let (_, _, max_candidate_users) = tool_calling_limits(&config.tool_calling);
                let candidate_users = build_allowed_candidate_users(
                    current_username,
                    &history,
                    max_candidate_users,
                );
                let planning_messages = build_tool_planning_messages(
                    &bundle,
                    &candidate_users,
                    current_username.to_string(),
                    clean_content.clone(),
                );

                match execute_dialogue_tool_loop(
                    http_client,
                    config,
                    &_graph,
                    planning_messages,
                    &candidate_users,
                    memory_hint,
                )
                .await
                {
                    Ok(outcome) => {
                        tool_loop_degraded = outcome.degraded;
                        tool_loop_executed_calls = outcome.executed_calls;
                        if !tool_loop_degraded {
                            if tool_loop_executed_calls > 0 {
                                social_mode = "tool_loop";
                            }
                            final_messages = outcome.messages;
                            if !final_messages.is_empty() {
                                final_messages[0] = make_system_message(merged_system_prompt.clone());
                            }
                            debug!(
                                kind = "prompt.social",
                                user = %current_username,
                                mode = social_mode,
                                executed_calls = tool_loop_executed_calls,
                                candidate_users = ?candidate_users,
                                "Dialogue tool loop completed before final streamed answer"
                            );
                        }
                    }
                    Err(error) => {
                        tool_loop_degraded = true;
                        warn!(
                            error = %error,
                            user = %current_username,
                            "Dialogue tool loop failed; falling back to legacy social summary path"
                        );
                    }
                }

                if tool_loop_degraded {
                    if let Some(summary) = crate::social_context::load_dialogue_social_summary(
                        &_graph,
                        current_username,
                        memory_hint,
                    )
                    .await
                    {
                        social_mode = "summary_fallback";
                        system_blocks.insert(
                            social_context_insert_index.min(system_blocks.len()),
                            summary.summary,
                        );
                        merged_system_prompt = system_blocks
                            .iter()
                            .filter(|block| !block.trim().is_empty())
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("

");
                        bundle.merged_system_prompt = merged_system_prompt.clone();
                        final_messages = build_final_messages(
                            &bundle,
                            current_username.to_string(),
                            clean_content.clone(),
                        );
                        debug!(
                            kind = "prompt.social",
                            user = %current_username,
                            mode = social_mode,
                            executed_calls = tool_loop_executed_calls,
                            "Dialogue tool loop degraded to legacy summary context"
                        );
                    }
                }
            }
        }

        bundle.social_mode = social_mode;
        bundle.social_gate_open = social_gate_open;
        bundle.memory_hint = memory_hint;

        debug!(
            kind = "prompt.social",
            user = %current_username,
            mode = social_mode,
            social_gate_open = social_gate_open,
            tool_loop_degraded = tool_loop_degraded,
            tool_loop_executed_calls = tool_loop_executed_calls,
            system_block_count = system_blocks.len(),
            merged_system_prompt_len = merged_system_prompt.len(),
            "Dialogue social context path finalized"
        );

        let request = build_streaming_request(config, final_messages);

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
                                    } else if let Some(start) = content.find("<think>") {
                                        is_thinking = true;
                                        output_buffer.push_str(&content[..start]);
                                    } else {
                                        output_buffer.push_str(content);
                                    }

                                    output_buffer = output_buffer.replace("

", "
");

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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::post, Json, Router};
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex as StdMutex};

    #[derive(Clone)]
    struct MockToolServerState {
        requests: Arc<StdMutex<Vec<Value>>>,
        responses: Arc<StdMutex<Vec<String>>>,
    }

    async fn mock_chat_completions(
        State(state): State<MockToolServerState>,
        Json(payload): Json<Value>,
    ) -> impl IntoResponse {
        state.requests.lock().unwrap().push(payload);
        let body = state
            .responses
            .lock()
            .unwrap()
            .remove(0);
        (StatusCode::OK, body)
    }

    async fn spawn_mock_tool_server(responses: Vec<String>) -> (SocketAddr, Arc<StdMutex<Vec<Value>>>) {
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let state = MockToolServerState {
            requests: Arc::clone(&requests),
            responses: Arc::new(StdMutex::new(responses)),
        };
        let app = Router::new()
            .route("/chat/completions", post(mock_chat_completions))
            .with_state(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock server");
        let addr = listener.local_addr().expect("mock server addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("mock server should run");
        });
        (addr, requests)
    }

    fn enabled_test_config(api_base: String) -> DialogueEngineConfig {
        DialogueEngineConfig {
            api_base,
            api_key: "test-key".to_string(),
            model: "test-model".to_string(),
            chat_max_tokens: 128,
            reasoning: None,
            tool_calling: DialogueToolCallingConfig {
                enabled: true,
                max_calls_per_turn: 2,
                timeout_ms: 1_500,
                max_candidate_users: 3,
            },
        }
    }

    #[test]
    fn candidate_users_include_current_and_respect_limit() {
        let history = vec![
            ("assistant".to_string(), "".to_string(), "hello".to_string()),
            ("user".to_string(), "alice".to_string(), "a".to_string()),
            ("user".to_string(), "bob".to_string(), "b".to_string()),
            ("user".to_string(), "alice".to_string(), "c".to_string()),
        ];

        let users = build_allowed_candidate_users("zang", &history, 3);
        assert!(users.iter().any(|u| u == "zang"));
        assert!(users.iter().any(|u| u == "alice") || users.iter().any(|u| u == "bob"));
        assert!(users.len() <= 3);
    }

    #[test]
    fn parse_dialogue_tool_arguments_rejects_invalid_json() {
        let result = parse_dialogue_tool_arguments("{not-json}");
        assert!(result.is_err());
    }

    #[test]
    fn social_fetch_gate_respects_tool_flag() {
        let enabled_config = DialogueEngineConfig {
            api_base: "http://localhost".to_string(),
            api_key: "key".to_string(),
            model: "model".to_string(),
            chat_max_tokens: 128,
            reasoning: None,
            tool_calling: DialogueToolCallingConfig {
                enabled: true,
                max_calls_per_turn: 2,
                timeout_ms: 1500,
                max_candidate_users: 3,
            },
        };
        let disabled_config = DialogueEngineConfig {
            tool_calling: DialogueToolCallingConfig {
                enabled: false,
                max_calls_per_turn: 2,
                timeout_ms: 1500,
                max_candidate_users: 3,
            },
            ..enabled_config.clone()
        };
        let decision = SocialFetchDecision {
            should_fetch: true,
            score: 3,
            has_strong_social_cue: true,
            has_continuity_cue: false,
            recent_social_signal: false,
            short_follow_up: false,
        };

        assert!(should_attempt_tool_loop(&enabled_config, decision));
        assert!(!should_use_legacy_summary_path(&enabled_config, decision));
        assert!(!should_attempt_tool_loop(&disabled_config, decision));
        assert!(should_use_legacy_summary_path(&disabled_config, decision));
    }

    #[tokio::test]
    async fn execute_dialogue_tool_loop_calls_allowed_dialogue_summary_tool() {
        let planning_response = serde_json::json!({
            "choices": [{
                "message": {
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "social.get_dialogue_summary",
                            "arguments": "{\"user_id\":\"alice\"}"
                        }
                    }]
                }
            }]
        })
        .to_string();
        let final_response = serde_json::json!({
            "choices": [{
                "message": {
                    "tool_calls": []
                }
            }]
        })
        .to_string();
        let (addr, requests) = spawn_mock_tool_server(vec![planning_response, final_response]).await;
        let config = enabled_test_config(format!("http://{}", addr));
        let http_client = Client::new();
        let graph = CognitiveGraph::new("memory").await.expect("graph init");
        let bundle = DialoguePromptBundle {
            merged_system_prompt: "system".to_string(),
            messages: vec![],
            social_mode: "none",
            social_gate_open: true,
            memory_hint: 0.0,
        };
        let planning_messages = build_tool_planning_messages(
            &bundle,
            &["alice".to_string()],
            "alice".to_string(),
            "hello".to_string(),
        );

        let outcome = execute_dialogue_tool_loop(
            &http_client,
            &config,
            &graph,
            planning_messages,
            &["alice".to_string()],
            0.0,
        )
        .await
        .expect("tool loop should succeed");

        assert_eq!(outcome.executed_calls, 1);
        assert!(!outcome.degraded);
        let sent_requests = requests.lock().unwrap();
        assert_eq!(sent_requests.len(), 2);
        assert_eq!(sent_requests[0].get("tools").and_then(|v| v.as_array()).map(|v| v.len()), Some(1));
        let last_message = outcome.messages.last().expect("tool result message");
        assert_eq!(last_message.get("role").and_then(|v| v.as_str()), Some("tool"));
        assert_eq!(last_message.get("name").and_then(|v| v.as_str()), Some("social.get_dialogue_summary"));
    }

    #[tokio::test]
    async fn execute_dialogue_tool_loop_rejects_disallowed_candidate_user() {
        let planning_response = serde_json::json!({
            "choices": [{
                "message": {
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "social.get_dialogue_summary",
                            "arguments": "{\"user_id\":\"mallory\"}"
                        }
                    }]
                }
            }]
        })
        .to_string();
        let (addr, _requests) = spawn_mock_tool_server(vec![planning_response]).await;
        let config = enabled_test_config(format!("http://{}", addr));
        let http_client = Client::new();
        let graph = CognitiveGraph::new("memory").await.expect("graph init");
        let bundle = DialoguePromptBundle {
            merged_system_prompt: "system".to_string(),
            messages: vec![],
            social_mode: "none",
            social_gate_open: true,
            memory_hint: 0.0,
        };
        let planning_messages = build_tool_planning_messages(
            &bundle,
            &["alice".to_string()],
            "alice".to_string(),
            "hello".to_string(),
        );

        let error = execute_dialogue_tool_loop(
            &http_client,
            &config,
            &graph,
            planning_messages,
            &["alice".to_string()],
            0.0,
        )
        .await
        .expect_err("tool loop should reject disallowed target");

        assert!(error.to_string().contains("allowed candidate set"));
    }
}
