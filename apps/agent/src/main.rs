#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use anyhow::Result;
use kernel::get_agent_profile;
use serde::Deserialize;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

const SETTINGS_JSON_PATH: &str = "settings.json";

#[derive(Debug, Default, Deserialize)]
struct SettingsJson {
    #[serde(default)]
    debug_mode: Option<bool>,
    #[serde(default)]
    log_level: Option<String>,
    #[serde(default)]
    chat_max_tokens: Option<u32>,
    #[serde(default)]
    semantic_max_tokens: Option<u32>,
    #[serde(default)]
    dialogue_tool_calling_enabled: Option<bool>,
    #[serde(default)]
    dialogue_tool_max_calls_per_turn: Option<usize>,
    #[serde(default)]
    dialogue_tool_timeout_ms: Option<u64>,
    #[serde(default)]
    dialogue_tool_max_candidate_users: Option<usize>,
}

fn parse_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn load_settings_json() -> SettingsJson {
    let path = std::path::Path::new(SETTINGS_JSON_PATH);
    if !path.exists() {
        return SettingsJson::default();
    }

    match std::fs::read_to_string(path) {
        Ok(content) => match serde_json::from_str::<SettingsJson>(&content) {
            Ok(settings) => settings,
            Err(e) => {
                warn!(path = SETTINGS_JSON_PATH, error = %e, "Failed to parse settings.json");
                SettingsJson::default()
            }
        },
        Err(e) => {
            warn!(path = SETTINGS_JSON_PATH, error = %e, "Failed to read settings.json");
            SettingsJson::default()
        }
    }
}

fn resolve_log_level(config: &Config, settings: &SettingsJson) -> String {
    if let Ok(level) = std::env::var("PA_LOG_LEVEL") {
        let trimmed = level.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    if let Some(level) = &settings.log_level {
        let trimmed = level.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    config.agent.log_level.trim().to_string()
}

fn resolve_chat_max_tokens(settings: &SettingsJson) -> u32 {
    std::env::var("CHAT_MAX_TOKENS")
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .or(settings.chat_max_tokens)
        .unwrap_or(2048)
}

fn resolve_semantic_max_tokens(settings: &SettingsJson) -> u32 {
    std::env::var("SEMANTIC_MAX_TOKENS")
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .or(settings.semantic_max_tokens)
        .unwrap_or(4096)
}

fn resolve_debug_mode(settings: &SettingsJson) -> bool {
    std::env::var("DEBUG_MODE")
        .ok()
        .map(|v| parse_truthy(&v))
        .or(settings.debug_mode)
        .unwrap_or(false)
}

fn resolve_dialogue_tool_calling(settings: &SettingsJson) -> DialogueToolCallingConfig {
    DialogueToolCallingConfig {
        enabled: settings.dialogue_tool_calling_enabled.unwrap_or(false),
        max_calls_per_turn: settings.dialogue_tool_max_calls_per_turn.unwrap_or(2).max(1),
        timeout_ms: settings.dialogue_tool_timeout_ms.unwrap_or(1_500).max(100),
        max_candidate_users: settings
            .dialogue_tool_max_candidate_users
            .unwrap_or(3)
            .max(1),
    }
}

fn apply_non_api_settings_to_env(settings: &SettingsJson) {
    if std::env::var("SEMANTIC_MAX_TOKENS").is_err() {
        std::env::set_var(
            "SEMANTIC_MAX_TOKENS",
            resolve_semantic_max_tokens(settings).to_string(),
        );
    }

    if std::env::var("CHAT_MAX_TOKENS").is_err() {
        std::env::set_var("CHAT_MAX_TOKENS", resolve_chat_max_tokens(settings).to_string());
    }

    if std::env::var("PA_LOG_LEVEL").is_err() {
        if let Some(level) = &settings.log_level {
            let trimmed = level.trim();
            if !trimmed.is_empty() {
                std::env::set_var("PA_LOG_LEVEL", trimmed);
            }
        }
    }

    if std::env::var("DEBUG_MODE").is_err() {
        std::env::set_var(
            "DEBUG_MODE",
            if settings.debug_mode.unwrap_or(false) { "1" } else { "0" },
        );
    }
}

fn load_settings_and_apply_env() -> SettingsJson {
    let settings = load_settings_json();
    apply_non_api_settings_to_env(&settings);
    settings
}

use cognitive::{
    AffectEvaluatorConfig, AffectEvaluatorWorker, DialogueEngineConfig, DialogueEngineWorker,
};
use cognitive::dialogue_engine::DialogueToolCallingConfig;
use cockpit_api::{CockpitApiConfig, CockpitWorker};
use mcp::{McpConfig, McpWorker};
use memory::MemoryWorker;
use runtime::{Coordinator, Supervisor};
use sensory::{DiscordWorker, TelegramWorker, discord::SelfbotWsWorker};
use state::{
    StateCommandWorker, StateDriftWorker, StateEnvironmentWorker, StateGoalWorker, StateIntentWorker,
    StateStore, StateSystemWorker, StateUserWorker,
};
use std::time::Duration;

#[derive(Debug, Deserialize)]
struct Config {
    #[serde(default)]
    discord_bot: DiscordBotConfig,
    #[serde(default)]
    discord_selfbot: DiscordSelfbotConfig,
    #[serde(default)]
    telegram: TelegramConfig,
    #[serde(default)]
    agent: AgentConfig,
    #[serde(default, alias = "llm")]
    dialogue_engine: DialogueEngineFileConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            discord_bot: DiscordBotConfig::default(),
            discord_selfbot: DiscordSelfbotConfig::default(),
            telegram: TelegramConfig::default(),
            agent: AgentConfig::default(),
            dialogue_engine: DialogueEngineFileConfig::default(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct DiscordBotConfig {
    #[serde(default)]
    token: String,
    #[serde(default)]
    enabled: bool,
}

#[derive(Debug, Default, Deserialize)]
struct DiscordSelfbotConfig {
    #[serde(default)]
    token: String,
    #[serde(default)]
    enabled: bool,
}

#[derive(Debug, Default, Deserialize)]
struct TelegramConfig {
    #[serde(default)]
    token: String,
    #[serde(default)]
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct AgentConfig {
    #[serde(default = "default_name")]
    name: String,
    #[serde(default = "default_log_level")]
    log_level: String,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: default_name(),
            log_level: default_log_level(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct DialogueEngineFileConfig {
    #[serde(default)]
    api_base: String,
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    reasoning: Option<String>,
}

fn default_name() -> String {
    "PolyverseAgent".to_string()
}

fn default_log_level() -> String {
    "info".to_string()
}

fn parse_env_bool(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(default)
}

fn resolve_mcp_enabled() -> bool {
    parse_env_bool("MCP_ENABLED", false)
}

fn resolve_mcp_bind() -> String {
    std::env::var("MCP_BIND")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "127.0.0.1:4790".to_string())
}

fn resolve_mcp_request_timeout_ms() -> u64 {
    std::env::var("MCP_REQUEST_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|v| v.max(100))
        .unwrap_or(2_000)
}

fn resolve_mcp_max_tool_calls_per_turn() -> usize {
    std::env::var("MCP_MAX_TOOL_CALLS_PER_TURN")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .map(|v| v.max(1))
        .unwrap_or(4)
}

fn load_mcp_config() -> McpConfig {
    let mut config = McpConfig::from_env();
    config.enabled = resolve_mcp_enabled();
    config.bind_addr = resolve_mcp_bind();
    config.request_timeout_ms = resolve_mcp_request_timeout_ms();
    config.max_tool_calls_per_turn = resolve_mcp_max_tool_calls_per_turn();
    config
}

fn load_config() -> Result<Config> {
    match dotenvy::dotenv() {
        Ok(path) => info!(path = %path.display(), "Loaded .env file"),
        Err(dotenvy::Error::Io(_)) => {
            info!("No .env file found, using environment variables only")
        }
        Err(e) => warn!(error = %e, "Failed to parse .env file"),
    }

    let config_path =
        std::env::var("PA_CONFIG").unwrap_or_else(|_| "config.toml".to_string());

    let mut config: Config = if std::path::Path::new(&config_path).exists() {
        let content = std::fs::read_to_string(&config_path)?;
        info!(path = %config_path, "Loaded config file");
        toml::from_str(&content)?
    } else {
        Config {
            discord_bot: DiscordBotConfig::default(),
            discord_selfbot: DiscordSelfbotConfig::default(),
            telegram: TelegramConfig::default(),
            agent: AgentConfig::default(),
            dialogue_engine: DialogueEngineFileConfig::default(),
        }
    };

    if let Ok(token) = std::env::var("DISCORD_BOT_TOKEN") {
        config.discord_bot.token = token;
        config.discord_bot.enabled = true;
    }

    if let Ok(token) = std::env::var("DISCORD_SELFBOT_TOKEN") {
        config.discord_selfbot.token = token;
        config.discord_selfbot.enabled = true;
    }

    if let Ok(token) = std::env::var("TELEGRAM_TOKEN") {
        config.telegram.token = token;
        config.telegram.enabled = true;
    }

    if let Ok(name) = std::env::var("PA_AGENT_NAME") {
        config.agent.name = name;
    }

    if let Ok(level) = std::env::var("PA_LOG_LEVEL") {
        config.agent.log_level = level;
    }

    if let Ok(base) = std::env::var("DIALOGUE_ENGINE_API_BASE")
        .or_else(|_| std::env::var("OPENAI_API_BASE"))
        .or_else(|_| std::env::var("API_BASE"))
    {
        config.dialogue_engine.api_base = base;
    }
    if let Ok(key) = std::env::var("DIALOGUE_ENGINE_API_KEY")
        .or_else(|_| std::env::var("OPENAI_API_KEY"))
        .or_else(|_| std::env::var("API_KEY"))
    {
        config.dialogue_engine.api_key = key;
    }
    if let Ok(model) = std::env::var("DIALOGUE_ENGINE_MODEL")
        .or_else(|_| std::env::var("OPENAI_MODEL"))
        .or_else(|_| std::env::var("MODEL"))
    {
        config.dialogue_engine.model = model;
    }
    if let Ok(reasoning) = std::env::var("DIALOGUE_ENGINE_REASONING")
        .or_else(|_| std::env::var("OPENAI_REASONING"))
        .or_else(|_| std::env::var("REASONING"))
    {
        config.dialogue_engine.reasoning = Some(reasoning);
    }

    Ok(config)
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = load_config()?;
    let settings = load_settings_and_apply_env();
    let agent_profile = get_agent_profile().clone();

    if resolve_debug_mode(&settings) {
        info!(path = SETTINGS_JSON_PATH, "Debug mode is enabled");
    }

    let log_level = resolve_log_level(&config, &settings);
    let default_filter = format!("{},ort=warn,fastembed=warn,lance=warn", log_level);
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_filter));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_thread_ids(true)
        .init();

    info!(
        name = %config.agent.name,
        agent_id = %agent_profile.agent_id,
        display_name = %agent_profile.display_name,
        "=== Polyverse Agent Starting ==="
    );

    let mut supervisor = Supervisor::new();

    let mut worker_count = 0;
    
    let cockpit_enabled = parse_env_bool("COCKPIT_ENABLED", true);
    let cockpit_bind = std::env::var("COCKPIT_BIND").unwrap_or_else(|_| "127.0.0.1:4787".to_string());
    let cockpit_max_recent_events = std::env::var("COCKPIT_MAX_RECENT_EVENTS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(300);
    let state_system_enabled = parse_env_bool("STATE_SYSTEM_ENABLED", true);
    let state_system_interval_ms = std::env::var("STATE_SYSTEM_INTERVAL_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(1_000);
    let state_schema_path =
        std::env::var("STATE_SCHEMA_PATH").unwrap_or_else(|_| "config/state_schema.v0.json".to_string());

    let state_store = match StateStore::load_from_file(&state_schema_path) {
        Ok(store) => Some(store),
        Err(e) => {
            warn!(
                path = %state_schema_path,
                error = %e,
                "Failed to load state schema, cockpit state API will be disabled"
            );
            None
        }
    };

    if let Some(store) = &state_store {
        let _ = store.recompute_derived().await;
    }

    if config.discord_bot.enabled && !config.discord_bot.token.is_empty() {
        info!("Registering Discord Bot worker");
        supervisor.register(DiscordWorker::new(config.discord_bot.token.clone()));
        worker_count += 1;
    } else if config.discord_bot.enabled {
        warn!("Discord Bot enabled but no token provided (set DISCORD_BOT_TOKEN in .env)");
    }

    if config.discord_selfbot.enabled {
        info!("Registering Discord Selfbot Websocket worker");
        supervisor.register(SelfbotWsWorker::new(9000));
        worker_count += 1;
    }

    if config.telegram.enabled && !config.telegram.token.is_empty() {
        info!("Registering Telegram worker");
        supervisor.register(TelegramWorker::new(config.telegram.token.clone()));
        worker_count += 1;
    } else if config.telegram.enabled {
        warn!("Telegram enabled but no token provided (set TELEGRAM_TOKEN in .env)");
    }

    use std::sync::Arc;
    use memory::{episodic::EpisodicStore, embedder::MemoryEmbedder, compressor::SemanticCompressor};

    let memory_db_path = agent_profile.memory_db_path.clone();
    if let Some(parent) = std::path::Path::new(&memory_db_path).parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let graph_db_path = agent_profile.graph_db_path.clone();
    if graph_db_path != "memory" {
        let graph_path = std::path::Path::new(&graph_db_path);
        if !graph_path.exists() {
            std::fs::create_dir_all(graph_path)?;
        }
    }

    let lancedb_path = agent_profile.episodic_db_path.clone();
    
    let lancedb_path_obj = std::path::Path::new(&lancedb_path);
    if !lancedb_path_obj.exists() {
        std::fs::create_dir_all(lancedb_path_obj)?;
    }

    info!("Initializing Episodic Memory and Embedding Engine...");
    let episodic = Arc::new(EpisodicStore::open(&lancedb_path, "episodic_memory").await?);
    let embedder_pool_size = std::env::var("EMBEDDER_POOL_SIZE")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .map(|value| value.clamp(1, 3))
        .unwrap_or(1);
    let embedder = Arc::new(MemoryEmbedder::new_with_pool_size(embedder_pool_size)?);
    info!(pool_size = embedder.pool_size(), "Embedding pool initialized");
    let compressor_opt = SemanticCompressor::new().ok().map(Arc::new);

    info!("Initializing SurrealDB Cognitive Graph...");
    let cognitive_graph = memory::graph::CognitiveGraph::new(&graph_db_path).await?;
    let mcp_config = load_mcp_config();

    if mcp_config.enabled {
        info!(
            max_tool_calls_per_turn = mcp_config.max_tool_calls_per_turn,
            "Registering MCP worker"
        );
        supervisor.register(McpWorker::new(mcp_config.clone(), cognitive_graph.clone()));
        worker_count += 1;
    }

    if compressor_opt.is_none() {
        warn!("SLM Compressor missing configs, episodic memory will not ingest new events.");
    }

    let mut memory_worker = MemoryWorker::new(&memory_db_path)
        .with_episodic(Arc::clone(&episodic))
        .with_embedder(Arc::clone(&embedder));
    
    if let Some(comp) = &compressor_opt {
        memory_worker = memory_worker.with_compressor(Arc::clone(comp));
    }

    let short_term_handle = memory_worker.short_term_handle();
    supervisor.register(memory_worker);
    worker_count += 1;
    info!("Registered Memory worker");

    let state_store_for_cockpit = state_store.clone();
    let state_store_for_affect = state_store.clone();
    let state_store_for_dialogue = state_store.clone();
    let state_store_for_drift = state_store.clone();
    let state_store_for_intent = state_store.clone();
    let state_store_for_command = state_store.clone();
    let state_store_for_user = state_store.clone();
    let state_store_for_goal = state_store.clone();
    let state_store_for_env = state_store.clone();
    let state_store_for_system = state_store.clone();

    if let Some(store) = state_store_for_drift {
        supervisor.register(StateDriftWorker::new(store));
        worker_count += 1;
    }
    if let Some(store) = state_store_for_intent {
        supervisor.register(StateIntentWorker::new(store));
        worker_count += 1;
    }
    if let Some(store) = state_store_for_command {
        supervisor.register(StateCommandWorker::new(store));
        worker_count += 1;
    }
    if let Some(store) = state_store_for_user {
        supervisor.register(StateUserWorker::new(store));
        worker_count += 1;
    }
    if let Some(store) = state_store_for_goal {
        supervisor.register(StateGoalWorker::new(store));
        worker_count += 1;
    }
    if let Some(store) = state_store_for_env {
        supervisor.register(StateEnvironmentWorker::new(store));
        worker_count += 1;
    }
    if state_system_enabled {
        if let Some(store) = state_store_for_system {
            supervisor.register(
                StateSystemWorker::new(store)
                    .with_interval(Duration::from_millis(state_system_interval_ms.max(200))),
            );
            worker_count += 1;
        }
    }

    if cockpit_enabled {
        if let Some(store) = state_store_for_cockpit {
            info!(bind = %cockpit_bind, "Registering local cockpit API worker");
            supervisor.register(
                CockpitWorker::new(
                    CockpitApiConfig {
                        enabled: true,
                        bind_addr: cockpit_bind,
                        max_recent_events: cockpit_max_recent_events,
                    },
                    store,
                )
                .with_memory_db_path(memory_db_path.clone())
                .with_short_term(Arc::clone(&short_term_handle))
                .with_episodic(Arc::clone(&episodic))
                .with_graph(cognitive_graph.clone()),
            );
            worker_count += 1;
        } else {
            warn!("Cockpit is enabled but state schema is unavailable.");
        }
    }

    let chat_max_tokens = resolve_chat_max_tokens(&settings);
    let dialogue_tool_calling = resolve_dialogue_tool_calling(&settings);

    let dialogue_engine_config = DialogueEngineConfig {
        api_base: config.dialogue_engine.api_base.clone(),
        api_key: config.dialogue_engine.api_key.clone(),
        model: config.dialogue_engine.model.clone(),
        chat_max_tokens,
        reasoning: config.dialogue_engine.reasoning.clone(),
        tool_calling: dialogue_tool_calling,
    };

    if dialogue_engine_config.is_valid() {
        info!(
            api_base = %dialogue_engine_config.api_base,
            model = %dialogue_engine_config.model,
            "Registering dialogue engine worker"
        );
        supervisor.register(
            {
                let mut worker = DialogueEngineWorker::new(dialogue_engine_config)
                    .with_memory(Arc::clone(&short_term_handle))
                    .with_episodic(Arc::clone(&episodic))
                    .with_embedder(Arc::clone(&embedder))
                    .with_graph(cognitive_graph.clone());
                if let Some(store) = state_store_for_dialogue {
                    worker = worker.with_state_store(store);
                }
                worker
            },
        );
        worker_count += 1;
    } else {
        warn!(
            "Dialogue engine not configured (set DIALOGUE_ENGINE_API_BASE, DIALOGUE_ENGINE_API_KEY, DIALOGUE_ENGINE_MODEL in .env)"
        );
    }

    if let (Ok(base), Ok(key), Ok(model)) = (
        std::env::var("AFFECT_EVALUATOR_API_BASE")
            .or_else(|_| std::env::var("OPENAI_API_BASE"))
            .or_else(|_| std::env::var("API_BASE")),
        std::env::var("AFFECT_EVALUATOR_API_KEY")
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .or_else(|_| std::env::var("API_KEY")),
        std::env::var("AFFECT_EVALUATOR_MODEL")
            .or_else(|_| std::env::var("OPENAI_MODEL"))
            .or_else(|_| std::env::var("MODEL"))
    ) {
        let reasoning = std::env::var("AFFECT_EVALUATOR_REASONING")
            .or_else(|_| std::env::var("OPENAI_REASONING"))
            .or_else(|_| std::env::var("REASONING"))
            .ok();

        let affect_evaluator_config = AffectEvaluatorConfig {
            api_base: base,
            api_key: key,
            model,
            reasoning,
        };
        if affect_evaluator_config.is_valid() {
            info!(
                api_base = %affect_evaluator_config.api_base,
                model = %affect_evaluator_config.model,
                "Registering Affect Evaluator JSON worker"
            );
            let mut affect_worker = AffectEvaluatorWorker::new(
                affect_evaluator_config,
                cognitive_graph.clone(),
                Arc::clone(&short_term_handle),
                Some(Arc::clone(&episodic)),
                Some(Arc::clone(&embedder)),
            );
            if let Some(store) = state_store_for_affect {
                affect_worker = affect_worker.with_state_store(store);
            }
            supervisor.register(affect_worker);
            worker_count += 1;
        }
    } else {
        warn!(
            "Affect evaluator not configured (set AFFECT_EVALUATOR_API_BASE, AFFECT_EVALUATOR_API_KEY, AFFECT_EVALUATOR_MODEL in .env)"
        );
    }

    if worker_count == 0 {
        info!("No workers enabled. Running in headless mode.");
        info!("Configure workers via .env file.");
    }

    let broadcast_tx = supervisor.event_bus().broadcast_tx.clone();
    let shutdown_rx = supervisor.event_bus().shutdown_tx.subscribe();
    let mut coordinator = Coordinator::new(broadcast_tx);

    let event_rx = supervisor
        .event_bus_mut()
        .take_event_rx()
        .expect("event_rx already taken");

    let coordinator_handle = tokio::spawn(async move {
        if let Err(e) = coordinator.run(event_rx, shutdown_rx).await {
            error!(error = %e, "Coordinator error");
        }
    });

    supervisor.start_all().await?;

    info!(
        workers = worker_count,
        "=== Polyverse Agent Running ==="
    );
    info!("Press Ctrl+C to shutdown");

    tokio::signal::ctrl_c().await?;
    info!("Shutdown signal received");

    supervisor.shutdown().await?;
    coordinator_handle.abort();

    info!("=== Polyverse Agent Stopped ===");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    struct CurrentDirGuard {
        previous: PathBuf,
    }

    impl CurrentDirGuard {
        fn enter(path: &Path) -> Self {
            let previous = std::env::current_dir().expect("cwd should exist");
            std::env::set_current_dir(path).expect("should enter temp dir");
            Self { previous }
        }
    }

    impl Drop for CurrentDirGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.previous);
        }
    }

    fn unique_path(prefix: &str, ext: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("agent-{prefix}-{nanos}.{ext}"))
    }

    fn remove_file_if_exists(path: &Path) {
        if path.exists() {
            let _ = fs::remove_file(path);
        }
    }

    fn set_env<K: AsRef<OsStr>, V: AsRef<OsStr>>(key: K, value: V) {
        unsafe { std::env::set_var(key, value) }
    }

    fn remove_env<K: AsRef<OsStr>>(key: K) {
        unsafe { std::env::remove_var(key) }
    }

    #[test]
    fn parse_truthy_accepts_expected_values() {
        for value in ["1", "true", "TRUE", "yes", "on", " On "] {
            assert!(parse_truthy(value), "value should be truthy: {value}");
        }
        for value in ["0", "false", "no", "off", "", " maybe "] {
            assert!(!parse_truthy(value), "value should be falsy: {value}");
        }
    }

    #[test]
    fn resolve_dialogue_tool_calling_clamps_minimums() {
        let settings = SettingsJson {
            dialogue_tool_calling_enabled: Some(true),
            dialogue_tool_max_calls_per_turn: Some(0),
            dialogue_tool_timeout_ms: Some(10),
            dialogue_tool_max_candidate_users: Some(0),
            ..Default::default()
        };

        let config = resolve_dialogue_tool_calling(&settings);
        assert!(config.enabled);
        assert_eq!(config.max_calls_per_turn, 1);
        assert_eq!(config.timeout_ms, 100);
        assert_eq!(config.max_candidate_users, 1);
    }

    #[test]
    fn resolve_log_level_prefers_env_then_settings_then_config() {
        let _guard = env_guard();
        remove_env("PA_LOG_LEVEL");

        let config = Config {
            agent: AgentConfig {
                name: "PolyverseAgent".to_string(),
                log_level: "warn".to_string(),
            },
            ..Config::default()
        };
        let settings = SettingsJson {
            log_level: Some("debug".to_string()),
            ..Default::default()
        };

        assert_eq!(resolve_log_level(&config, &settings), "debug");
        set_env("PA_LOG_LEVEL", "trace");
        assert_eq!(resolve_log_level(&config, &settings), "trace");
        remove_env("PA_LOG_LEVEL");
    }

    #[test]
    fn resolve_mcp_config_applies_env_overrides_and_clamps() {
        let _guard = env_guard();
        remove_env("MCP_ENABLED");
        remove_env("MCP_BIND");
        remove_env("MCP_REQUEST_TIMEOUT_MS");
        remove_env("MCP_MAX_TOOL_CALLS_PER_TURN");

        set_env("MCP_ENABLED", "yes");
        set_env("MCP_BIND", "0.0.0.0:9999");
        set_env("MCP_REQUEST_TIMEOUT_MS", "50");
        set_env("MCP_MAX_TOOL_CALLS_PER_TURN", "0");

        let config = load_mcp_config();
        assert!(config.enabled);
        assert_eq!(config.bind_addr, "0.0.0.0:9999");
        assert_eq!(config.request_timeout_ms, 100);
        assert_eq!(config.max_tool_calls_per_turn, 1);

        remove_env("MCP_ENABLED");
        remove_env("MCP_BIND");
        remove_env("MCP_REQUEST_TIMEOUT_MS");
        remove_env("MCP_MAX_TOOL_CALLS_PER_TURN");
    }

    #[test]
    fn load_settings_json_returns_default_for_missing_invalid_and_valid_files() {
        let _guard = env_guard();
        let temp_dir = unique_path("settings-dir", "tmp");
        fs::create_dir_all(&temp_dir).expect("temp settings dir should be created");
        let _cwd_guard = CurrentDirGuard::enter(&temp_dir);

        let missing = load_settings_json();
        assert!(missing.log_level.is_none());
        assert!(missing.debug_mode.is_none());

        fs::write(temp_dir.join(SETTINGS_JSON_PATH), "{not-json}")
            .expect("invalid settings should be written");
        let invalid = load_settings_json();
        assert!(invalid.log_level.is_none());
        assert!(invalid.debug_mode.is_none());

        fs::write(
            temp_dir.join(SETTINGS_JSON_PATH),
            r#"{"debug_mode":true,"log_level":"debug","chat_max_tokens":777,"semantic_max_tokens":888,"dialogue_tool_calling_enabled":true,"dialogue_tool_max_calls_per_turn":5,"dialogue_tool_timeout_ms":2500,"dialogue_tool_max_candidate_users":4}"#,
        )
        .expect("valid settings should be written");
        let valid = load_settings_json();
        assert_eq!(valid.debug_mode, Some(true));
        assert_eq!(valid.log_level.as_deref(), Some("debug"));
        assert_eq!(valid.chat_max_tokens, Some(777));
        assert_eq!(valid.semantic_max_tokens, Some(888));
        assert_eq!(valid.dialogue_tool_calling_enabled, Some(true));
        assert_eq!(valid.dialogue_tool_max_calls_per_turn, Some(5));
        assert_eq!(valid.dialogue_tool_timeout_ms, Some(2500));
        assert_eq!(valid.dialogue_tool_max_candidate_users, Some(4));

        remove_file_if_exists(&temp_dir.join(SETTINGS_JSON_PATH));
        let _ = fs::remove_dir(&temp_dir);
    }

    #[test]
    fn load_config_prefers_env_over_file_values() {
        let _guard = env_guard();
        let config_path = unique_path("config", "toml");
        fs::write(
            &config_path,
            r#"
[discord_bot]
enabled = false
token = "from-file-discord"

[discord_selfbot]
enabled = false
token = "from-file-selfbot"

[telegram]
enabled = false
token = "from-file-telegram"

[agent]
name = "FileAgent"
log_level = "warn"

[dialogue_engine]
api_base = "http://file-base"
api_key = "file-key"
model = "file-model"
reasoning = "file-reasoning"
"#,
        )
        .expect("config file should be written");

        set_env("PA_CONFIG", &config_path);
        set_env("DISCORD_BOT_TOKEN", "env-discord");
        set_env("DISCORD_SELFBOT_TOKEN", "env-selfbot");
        set_env("TELEGRAM_TOKEN", "env-telegram");
        set_env("PA_AGENT_NAME", "EnvAgent");
        set_env("PA_LOG_LEVEL", "debug");
        set_env("DIALOGUE_ENGINE_API_BASE", "http://env-base");
        set_env("DIALOGUE_ENGINE_API_KEY", "env-key");
        set_env("DIALOGUE_ENGINE_MODEL", "env-model");
        set_env("DIALOGUE_ENGINE_REASONING", "env-reasoning");

        let config = load_config().expect("config should load");
        assert!(config.discord_bot.enabled);
        assert_eq!(config.discord_bot.token, "env-discord");
        assert!(config.discord_selfbot.enabled);
        assert_eq!(config.discord_selfbot.token, "env-selfbot");
        assert!(config.telegram.enabled);
        assert_eq!(config.telegram.token, "env-telegram");
        assert_eq!(config.agent.name, "EnvAgent");
        assert_eq!(config.agent.log_level, "debug");
        assert_eq!(config.dialogue_engine.api_base, "http://env-base");
        assert_eq!(config.dialogue_engine.api_key, "env-key");
        assert_eq!(config.dialogue_engine.model, "env-model");
        assert_eq!(config.dialogue_engine.reasoning.as_deref(), Some("env-reasoning"));

        remove_env("PA_CONFIG");
        remove_env("DISCORD_BOT_TOKEN");
        remove_env("DISCORD_SELFBOT_TOKEN");
        remove_env("TELEGRAM_TOKEN");
        remove_env("PA_AGENT_NAME");
        remove_env("PA_LOG_LEVEL");
        remove_env("DIALOGUE_ENGINE_API_BASE");
        remove_env("DIALOGUE_ENGINE_API_KEY");
        remove_env("DIALOGUE_ENGINE_MODEL");
        remove_env("DIALOGUE_ENGINE_REASONING");
        remove_file_if_exists(&config_path);
    }

    #[test]
    fn resolve_scalar_settings_use_env_then_settings_then_defaults() {
        let _guard = env_guard();
        remove_env("CHAT_MAX_TOKENS");
        remove_env("SEMANTIC_MAX_TOKENS");
        remove_env("DEBUG_MODE");

        let settings = SettingsJson {
            debug_mode: Some(true),
            chat_max_tokens: Some(777),
            semantic_max_tokens: Some(888),
            ..Default::default()
        };

        assert_eq!(resolve_chat_max_tokens(&settings), 777);
        assert_eq!(resolve_semantic_max_tokens(&settings), 888);
        assert!(resolve_debug_mode(&settings));

        set_env("CHAT_MAX_TOKENS", "999");
        set_env("SEMANTIC_MAX_TOKENS", "1111");
        set_env("DEBUG_MODE", "off");
        assert_eq!(resolve_chat_max_tokens(&settings), 999);
        assert_eq!(resolve_semantic_max_tokens(&settings), 1111);
        assert!(!resolve_debug_mode(&settings));

        set_env("CHAT_MAX_TOKENS", "invalid");
        set_env("SEMANTIC_MAX_TOKENS", "invalid");
        remove_env("DEBUG_MODE");
        assert_eq!(resolve_chat_max_tokens(&settings), 777);
        assert_eq!(resolve_semantic_max_tokens(&settings), 888);
        assert!(resolve_debug_mode(&settings));

        let defaults = SettingsJson::default();
        assert_eq!(resolve_chat_max_tokens(&defaults), 2048);
        assert_eq!(resolve_semantic_max_tokens(&defaults), 4096);
        assert!(!resolve_debug_mode(&defaults));

        remove_env("CHAT_MAX_TOKENS");
        remove_env("SEMANTIC_MAX_TOKENS");
        remove_env("DEBUG_MODE");
    }

    #[test]
    fn apply_non_api_settings_to_env_only_fills_missing_values() {
        let _guard = env_guard();
        remove_env("SEMANTIC_MAX_TOKENS");
        remove_env("CHAT_MAX_TOKENS");
        remove_env("PA_LOG_LEVEL");
        remove_env("DEBUG_MODE");

        let settings = SettingsJson {
            debug_mode: Some(true),
            log_level: Some("debug".to_string()),
            chat_max_tokens: Some(777),
            semantic_max_tokens: Some(888),
            ..Default::default()
        };

        apply_non_api_settings_to_env(&settings);
        assert_eq!(std::env::var("SEMANTIC_MAX_TOKENS").as_deref(), Ok("888"));
        assert_eq!(std::env::var("CHAT_MAX_TOKENS").as_deref(), Ok("777"));
        assert_eq!(std::env::var("PA_LOG_LEVEL").as_deref(), Ok("debug"));
        assert_eq!(std::env::var("DEBUG_MODE").as_deref(), Ok("1"));

        set_env("SEMANTIC_MAX_TOKENS", "999");
        set_env("CHAT_MAX_TOKENS", "1234");
        set_env("PA_LOG_LEVEL", "trace");
        set_env("DEBUG_MODE", "0");
        apply_non_api_settings_to_env(&settings);
        assert_eq!(std::env::var("SEMANTIC_MAX_TOKENS").as_deref(), Ok("999"));
        assert_eq!(std::env::var("CHAT_MAX_TOKENS").as_deref(), Ok("1234"));
        assert_eq!(std::env::var("PA_LOG_LEVEL").as_deref(), Ok("trace"));
        assert_eq!(std::env::var("DEBUG_MODE").as_deref(), Ok("0"));

        remove_env("SEMANTIC_MAX_TOKENS");
        remove_env("CHAT_MAX_TOKENS");
        remove_env("PA_LOG_LEVEL");
        remove_env("DEBUG_MODE");
    }

    #[test]
    fn load_config_accepts_fallback_dialogue_env_aliases() {
        let _guard = env_guard();
        let temp_dir = unique_path("config-alias-dir", "tmp");
        fs::create_dir_all(&temp_dir).expect("temp config dir should be created");
        let _cwd_guard = CurrentDirGuard::enter(&temp_dir);
        let config_path = temp_dir.join("missing-config.toml");
        remove_file_if_exists(&config_path);

        set_env("PA_CONFIG", &config_path);
        remove_env("DIALOGUE_ENGINE_API_BASE");
        remove_env("DIALOGUE_ENGINE_API_KEY");
        remove_env("DIALOGUE_ENGINE_MODEL");
        remove_env("DIALOGUE_ENGINE_REASONING");
        remove_env("OPENAI_API_KEY");
        remove_env("OPENAI_MODEL");
        remove_env("API_BASE");
        remove_env("REASONING");
        set_env("OPENAI_API_BASE", "http://alias-base");
        set_env("API_KEY", "alias-key");
        set_env("MODEL", "alias-model");
        set_env("OPENAI_REASONING", "alias-reasoning");

        let config = load_config().expect("config should load from env aliases");
        assert_eq!(config.dialogue_engine.api_base, "http://alias-base");
        assert_eq!(config.dialogue_engine.api_key, "alias-key");
        assert_eq!(config.dialogue_engine.model, "alias-model");
        assert_eq!(
            config.dialogue_engine.reasoning.as_deref(),
            Some("alias-reasoning")
        );

        remove_env("PA_CONFIG");
        remove_env("OPENAI_API_BASE");
        remove_env("API_KEY");
        remove_env("MODEL");
        remove_env("OPENAI_REASONING");
        let _ = fs::remove_dir(&temp_dir);
    }
}

































































































































































































































































