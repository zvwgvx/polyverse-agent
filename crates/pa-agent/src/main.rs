#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use anyhow::Result;
use serde::Deserialize;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use pa_cognitive::{LlmConfig, LlmWorker, System1Config, System1Worker};
use pa_memory::MemoryWorker;
use pa_runtime::{Coordinator, Supervisor};
use pa_sensory::{DiscordWorker, TelegramWorker, discord::SelfbotWsWorker};

// ─── Configuration ───────────────────────────────────────────

/// Agent configuration.
/// Priority: .env file → environment variables → config.toml → defaults
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
    #[serde(default)]
    llm: LlmFileConfig,
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

/// LLM config from config.toml (can be overridden by env vars)
#[derive(Debug, Default, Deserialize)]
struct LlmFileConfig {
    #[serde(default)]
    api_base: String,
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    model: String,
}

fn default_name() -> String {
    "PolyverseAgent".to_string()
}

fn default_log_level() -> String {
    "info".to_string()
}

/// Load configuration with the following priority:
/// 1. `.env` file (loaded into process env vars via dotenvy)
/// 2. Existing environment variables (override .env)
/// 3. `config.toml` file (base config)
/// 4. Defaults
fn load_config() -> Result<Config> {
    // Step 1: Load .env file (silently ignore if not found)
    match dotenvy::dotenv() {
        Ok(path) => info!(path = %path.display(), "Loaded .env file"),
        Err(dotenvy::Error::Io(_)) => {
            info!("No .env file found, using environment variables only")
        }
        Err(e) => warn!(error = %e, "Failed to parse .env file"),
    }

    // Step 2: Load config.toml as base
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
            llm: LlmFileConfig::default(),
        }
    };

    // Step 3: Environment variables OVERRIDE config.toml values

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

    // LLM env overrides (SYS2 is the Conscious Roleplay model)
    if let Ok(base) = std::env::var("SYS2_API_BASE").or_else(|_| std::env::var("OPENAI_API_BASE")).or_else(|_| std::env::var("API_BASE")) {
        config.llm.api_base = base;
    }
    if let Ok(key) = std::env::var("SYS2_API_KEY").or_else(|_| std::env::var("OPENAI_API_KEY")).or_else(|_| std::env::var("API_KEY")) {
        config.llm.api_key = key;
    }
    if let Ok(model) = std::env::var("SYS2_MODEL").or_else(|_| std::env::var("OPENAI_MODEL")).or_else(|_| std::env::var("MODEL")) {
        config.llm.model = model;
    }

    Ok(config)
}

// ─── Main ────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    // Load config (env + file)
    let config = load_config()?;

    // Initialize tracing/logging
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
        "=== Polyverse Agent Starting ==="
    );

    // Create supervisor
    let mut supervisor = Supervisor::new();

    // Register sensory workers based on config
    let mut worker_count = 0;

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

    let lancedb_path = std::env::var("LANCE_DB_PATH")
        .unwrap_or_else(|_| "data/ryuuko_lancedb".to_string());
    
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
    let cognitive_graph = pa_memory::graph::CognitiveGraph::new("data/ryuuko_graph").await?;

    if compressor_opt.is_none() {
        warn!("SLM Compressor missing configs, episodic memory will not ingest new events.");
    }

    // Register Memory worker (always — memory is core)
    let mut memory_worker = MemoryWorker::new("data/ryuuko_memory.db")
        .with_episodic(Arc::clone(&episodic))
        .with_embedder(Arc::clone(&embedder));
    
    if let Some(comp) = &compressor_opt {
        memory_worker = memory_worker.with_compressor(Arc::clone(comp));
    }

    let short_term_handle = memory_worker.short_term_handle();
    supervisor.register(memory_worker);
    worker_count += 1;
    info!("Registered Memory worker");

    // Register LLM worker
    let chat_max_tokens = std::env::var("CHAT_MAX_TOKENS")
        .unwrap_or_else(|_| "2048".to_string())
        .parse::<u32>()
        .unwrap_or(2048);

    let llm_config = LlmConfig {
        api_base: config.llm.api_base.clone(),
        api_key: config.llm.api_key.clone(),
        model: config.llm.model.clone(),
        chat_max_tokens,
    };

    if llm_config.is_valid() {
        info!(
            api_base = %llm_config.api_base,
            model = %llm_config.model,
            "Registering LLM worker"
        );
        supervisor.register(
            LlmWorker::new(llm_config)
                .with_memory(Arc::clone(&short_term_handle))
                .with_episodic(Arc::clone(&episodic))
                .with_embedder(Arc::clone(&embedder)),
        );
        worker_count += 1;
    } else {
        warn!("LLM not configured (set API_BASE, API_KEY, MODEL in .env)");
    }

    // Register System 1 Evaluator
    if let (Ok(base), Ok(key), Ok(model)) = (
        std::env::var("SYS1_API_BASE").or_else(|_| std::env::var("OPENAI_API_BASE")).or_else(|_| std::env::var("API_BASE")),
        std::env::var("SYS1_API_KEY").or_else(|_| std::env::var("OPENAI_API_KEY")).or_else(|_| std::env::var("API_KEY")),
        std::env::var("SYS1_MODEL")
    ) {
        let sys1_config = System1Config {
            api_base: base,
            api_key: key,
            model,
        };
        if sys1_config.is_valid() {
            info!(
                api_base = %sys1_config.api_base,
                model = %sys1_config.model,
                "Registering System 1 JSON Evaluator"
            );
            supervisor.register(System1Worker::new(
                sys1_config,
                cognitive_graph.clone(),
                Arc::clone(&short_term_handle),
            ));
            worker_count += 1;
        }
    } else {
        warn!("System 1 not configured (set SYS1_API_BASE, SYS1_API_KEY, SYS1_MODEL in .env)");
    }

    if worker_count == 0 {
        info!("No workers enabled. Running in headless mode.");
        info!("Configure workers via .env file.");
    }

    // Create and spawn the coordinator
    let broadcast_tx = supervisor.event_bus().broadcast_tx.clone();
    let shutdown_rx = supervisor.event_bus().shutdown_tx.subscribe();
    let mut coordinator = Coordinator::new(broadcast_tx);

    // Take the event receiver from the bus — coordinator owns it
    let event_rx = supervisor
        .event_bus_mut()
        .take_event_rx()
        .expect("event_rx already taken");

    let coordinator_handle = tokio::spawn(async move {
        if let Err(e) = coordinator.run(event_rx, shutdown_rx).await {
            error!(error = %e, "Coordinator error");
        }
    });

    // Start all workers
    supervisor.start_all().await?;

    info!(
        workers = worker_count,
        "=== Polyverse Agent Running ==="
    );
    info!("Press Ctrl+C to shutdown");

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    info!("Shutdown signal received");

    // Graceful shutdown
    supervisor.shutdown().await?;
    coordinator_handle.abort();

    info!("=== Polyverse Agent Stopped ===");
    Ok(())
}
