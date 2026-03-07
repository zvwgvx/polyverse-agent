#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use anyhow::Result;
use pa_core::get_agent_profile;
use serde::Deserialize;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use pa_cognitive::{
    AffectEvaluatorConfig, AffectEvaluatorWorker, DialogueEngineConfig, DialogueEngineWorker,
};
use pa_cockpit_api::{CockpitApiConfig, CockpitWorker};
use pa_memory::MemoryWorker;
use pa_runtime::{Coordinator, Supervisor};
use pa_sensory::{DiscordWorker, TelegramWorker, discord::SelfbotWsWorker};
use pa_state::StateStore;

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
    let agent_profile = get_agent_profile().clone();

    let default_filter = format!("{},ort=warn,fastembed=warn,lance=warn", config.agent.log_level);
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
    use pa_memory::{episodic::EpisodicStore, embedder::MemoryEmbedder, compressor::SemanticCompressor};

    let lancedb_path = agent_profile.episodic_db_path.clone();
    
    if let Some(parent) = std::path::Path::new(&lancedb_path).parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }

    info!("Initializing Episodic Memory and Embedding Engine...");
    let episodic = Arc::new(EpisodicStore::open(&lancedb_path, "episodic_memory").await?);
    let embedder = Arc::new(MemoryEmbedder::new()?);
    let compressor_opt = SemanticCompressor::new().ok().map(Arc::new);

    info!("Initializing SurrealDB Cognitive Graph...");
    let cognitive_graph = pa_memory::graph::CognitiveGraph::new(&agent_profile.graph_db_path).await?;

    if compressor_opt.is_none() {
        warn!("SLM Compressor missing configs, episodic memory will not ingest new events.");
    }

    let memory_db_path = agent_profile.memory_db_path.clone();

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

    if cockpit_enabled {
        if let Some(store) = state_store {
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

    let chat_max_tokens = std::env::var("CHAT_MAX_TOKENS")
        .unwrap_or_else(|_| "2048".to_string())
        .parse::<u32>()
        .unwrap_or(2048);

    let dialogue_engine_config = DialogueEngineConfig {
        api_base: config.dialogue_engine.api_base.clone(),
        api_key: config.dialogue_engine.api_key.clone(),
        model: config.dialogue_engine.model.clone(),
        chat_max_tokens,
        reasoning: config.dialogue_engine.reasoning.clone(),
    };

    if dialogue_engine_config.is_valid() {
        info!(
            api_base = %dialogue_engine_config.api_base,
            model = %dialogue_engine_config.model,
            "Registering dialogue engine worker"
        );
        supervisor.register(
            DialogueEngineWorker::new(dialogue_engine_config)
                .with_memory(Arc::clone(&short_term_handle))
                .with_episodic(Arc::clone(&episodic))
                .with_embedder(Arc::clone(&embedder))
                .with_graph(cognitive_graph.clone()),
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
            supervisor.register(AffectEvaluatorWorker::new(
                affect_evaluator_config,
                cognitive_graph.clone(),
                Arc::clone(&short_term_handle),
                Some(Arc::clone(&episodic)),
                Some(Arc::clone(&embedder)),
            ));
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
