use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::{Component, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use async_trait::async_trait;
use axum::extract::{Query, State};
use axum::http::Method;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use pa_core::agent_profile::get_agent_profile;
use pa_core::event::{Event, SystemEvent};
use pa_core::worker::{Worker, WorkerContext, WorkerStatus};
use pa_memory::episodic::EpisodicStore;
use pa_memory::graph::{CognitiveGraph, RelationshipGraphSnapshot};
use pa_memory::short_term::{ActiveSessionSnapshot, ShortTermMemory};
use pa_memory::{MemoryMessage, MemoryStore};
use pa_state::{ManualPatchRequest, ManualPatchResult, StateMetricsSnapshot, StateStore};
use serde::{Deserialize, Serialize};
use sysinfo::{Components, Disks, System};
use tokio::net::TcpListener;
use tokio::sync::{oneshot, Mutex, RwLock};
use tower_http::cors::{Any, CorsLayer};
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct CockpitApiConfig {
    pub enabled: bool,
    pub bind_addr: String,
    pub max_recent_events: usize,
}

impl Default for CockpitApiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind_addr: "127.0.0.1:4787".to_string(),
            max_recent_events: 300,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CockpitCounter {
    pub raw_events: u64,
    pub mention_events: u64,
    pub response_events: u64,
    pub bot_turns: u64,
    pub system_events: u64,
    pub intent_events: u64,
    pub biology_events: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkerStateView {
    pub name: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentIdentityView {
    pub agent_id: String,
    pub display_name: String,
    pub graph_self_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CockpitOverview {
    pub identity: AgentIdentityView,
    pub started_at: DateTime<Utc>,
    pub uptime_seconds: u64,
    pub counters: CockpitCounter,
    pub workers: Vec<WorkerStateView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CockpitEventView {
    pub ts: DateTime<Utc>,
    pub kind: String,
    pub summary: String,
}

#[derive(Debug, Clone)]
struct CockpitMetrics {
    started_at: DateTime<Utc>,
    counters: CockpitCounter,
    worker_status: HashMap<String, String>,
    recent_events: VecDeque<CockpitEventView>,
    max_recent_events: usize,
}

impl CockpitMetrics {
    fn new(max_recent_events: usize) -> Self {
        Self {
            started_at: Utc::now(),
            counters: CockpitCounter {
                raw_events: 0,
                mention_events: 0,
                response_events: 0,
                bot_turns: 0,
                system_events: 0,
                intent_events: 0,
                biology_events: 0,
            },
            worker_status: HashMap::new(),
            recent_events: VecDeque::new(),
            max_recent_events,
        }
    }

    fn push_event(&mut self, kind: &str, summary: String) {
        self.recent_events.push_back(CockpitEventView {
            ts: Utc::now(),
            kind: kind.to_string(),
            summary,
        });
        while self.recent_events.len() > self.max_recent_events {
            let _ = self.recent_events.pop_front();
        }
    }

    fn overview(&self, identity: &AgentIdentityView) -> CockpitOverview {
        let uptime = Utc::now() - self.started_at;
        let workers = self
            .worker_status
            .iter()
            .map(|(name, status)| WorkerStateView {
                name: name.clone(),
                status: status.clone(),
            })
            .collect();

        CockpitOverview {
            identity: identity.clone(),
            started_at: self.started_at,
            uptime_seconds: uptime.num_seconds().max(0) as u64,
            counters: self.counters.clone(),
            workers,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct PromptEntry {
    id: String,
    path: String,
}

#[derive(Debug, Clone)]
struct PromptCatalog {
    root_dir: PathBuf,
    entries: Vec<PromptEntry>,
    by_id: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
struct PromptDocument {
    id: String,
    path: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct PromptDocumentQuery {
    id: String,
}

#[derive(Debug, Deserialize)]
struct PromptUpdateRequest {
    id: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct PromptRegistryFile {
    prompts: HashMap<String, String>,
}

#[derive(Clone)]
struct AppState {
    identity: AgentIdentityView,
    metrics: Arc<RwLock<CockpitMetrics>>,
    state_store: StateStore,
    prompts: Arc<PromptCatalog>,
    memory_db_path: Option<String>,
    memory_reader: Option<Arc<std::sync::Mutex<Option<MemoryStore>>>>,
    short_term: Option<Arc<Mutex<ShortTermMemory>>>,
    episodic: Option<Arc<EpisodicStore>>,
    graph: Option<CognitiveGraph>,
    system_cache: Arc<RwLock<Option<CachedSystemSnapshot>>>,
    relationship_cache: Arc<RwLock<Option<CachedRelationshipSnapshot>>>,
}

#[derive(Debug, Deserialize)]
struct EventQuery {
    limit: Option<usize>,
}

pub struct CockpitWorker {
    config: CockpitApiConfig,
    state_store: StateStore,
    metrics: Arc<RwLock<CockpitMetrics>>,
    memory_db_path: Option<String>,
    memory_reader: Option<Arc<std::sync::Mutex<Option<MemoryStore>>>>,
    short_term: Option<Arc<Mutex<ShortTermMemory>>>,
    episodic: Option<Arc<EpisodicStore>>,
    graph: Option<CognitiveGraph>,
    system_cache: Arc<RwLock<Option<CachedSystemSnapshot>>>,
    relationship_cache: Arc<RwLock<Option<CachedRelationshipSnapshot>>>,
    status: WorkerStatus,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct MemoryOverview {
    pub persisted_message_count: i64,
    pub persisted_recent_messages: Vec<MemoryMessage>,
    pub active_session_count: usize,
    pub active_sessions: Vec<ActiveSessionSnapshot>,
    pub active_recent_messages: Vec<MemoryMessage>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct EpisodicChunkView {
    pub id: String,
    pub content: String,
    pub timestamp: String,
    pub importance: f32,
    pub metadata: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct EpisodicOverview {
    pub total_chunks: usize,
    pub recent_chunks: Vec<EpisodicChunkView>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct LoadAverageView {
    pub one: f64,
    pub five: f64,
    pub fifteen: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct DiskUsageView {
    pub name: String,
    pub mount_point: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_bytes: u64,
    pub usage_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct TemperatureView {
    pub label: String,
    pub temperature_celsius: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct GpuView {
    pub name: String,
    pub utilization_percent: Option<f32>,
    pub memory_used_mb: Option<u64>,
    pub memory_total_mb: Option<u64>,
    pub temperature_celsius: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SystemSnapshot {
    pub collected_at: DateTime<Utc>,
    pub cpu_usage_percent: f32,
    pub cpu_count: usize,
    pub load_average: LoadAverageView,
    pub total_memory_bytes: u64,
    pub used_memory_bytes: u64,
    pub available_memory_bytes: u64,
    pub disks: Vec<DiskUsageView>,
    pub temperatures: Vec<TemperatureView>,
    pub gpus: Vec<GpuView>,
}

#[derive(Clone)]
struct CachedSystemSnapshot {
    cached_at: Instant,
    snapshot: SystemSnapshot,
}

#[derive(Clone)]
struct CachedRelationshipSnapshot {
    cached_at: Instant,
    snapshot: RelationshipGraphSnapshot,
}

const SYSTEM_CACHE_TTL: Duration = Duration::from_millis(1500);
const RELATIONSHIP_CACHE_TTL: Duration = Duration::from_millis(1200);

impl CockpitWorker {
    pub fn new(config: CockpitApiConfig, state_store: StateStore) -> Self {
        Self {
            metrics: Arc::new(RwLock::new(CockpitMetrics::new(config.max_recent_events))),
            config,
            state_store,
            memory_db_path: None,
            memory_reader: None,
            short_term: None,
            episodic: None,
            graph: None,
            system_cache: Arc::new(RwLock::new(None)),
            relationship_cache: Arc::new(RwLock::new(None)),
            status: WorkerStatus::NotStarted,
        }
    }

    pub fn with_memory_db_path(mut self, path: impl Into<String>) -> Self {
        self.memory_db_path = Some(path.into());
        self.memory_reader = Some(Arc::new(std::sync::Mutex::new(None)));
        self
    }

    pub fn with_short_term(mut self, short_term: Arc<Mutex<ShortTermMemory>>) -> Self {
        self.short_term = Some(short_term);
        self
    }

    pub fn with_episodic(mut self, episodic: Arc<EpisodicStore>) -> Self {
        self.episodic = Some(episodic);
        self
    }

    pub fn with_graph(mut self, graph: CognitiveGraph) -> Self {
        self.graph = Some(graph);
        self
    }

    async fn track_event(metrics: &Arc<RwLock<CockpitMetrics>>, event: Event) {
        let mut metrics = metrics.write().await;
        match event {
            Event::Raw(raw) => {
                metrics.counters.raw_events += 1;
                if raw.is_mention {
                    metrics.counters.mention_events += 1;
                }
                metrics.push_event("raw", format!("{}: {}", raw.username, truncate(&raw.content, 120)));
            }
            Event::Intent(intent) => {
                metrics.counters.intent_events += 1;
                metrics.push_event(
                    "intent",
                    format!(
                        "{} -> {:?} ({:.2})",
                        intent.source.username,
                        intent.intent,
                        intent.confidence
                    ),
                );
            }
            Event::Response(resp) => {
                metrics.counters.response_events += 1;
                metrics.push_event(
                    "response",
                    format!("{:?}: {}", resp.source, truncate(&resp.content, 120)),
                );
            }
            Event::BotTurnCompletion(done) => {
                metrics.counters.bot_turns += 1;
                metrics.push_event(
                    "turn_completion",
                    format!(
                        "to {}: {}",
                        done.reply_to_user.unwrap_or_else(|| "unknown".to_string()),
                        truncate(&done.content, 120)
                    ),
                );
            }
            Event::Biology(_) => {
                metrics.counters.biology_events += 1;
                metrics.push_event("biology", "biology update".to_string());
            }
            Event::System(system) => {
                metrics.counters.system_events += 1;
                match system {
                    SystemEvent::WorkerStarted { name } => {
                        metrics.worker_status.insert(name.clone(), "started".to_string());
                        metrics.push_event("system", format!("worker_started: {}", name));
                    }
                    SystemEvent::WorkerStopped { name } => {
                        metrics.worker_status.insert(name.clone(), "stopped".to_string());
                        metrics.push_event("system", format!("worker_stopped: {}", name));
                    }
                    SystemEvent::WorkerError { name, error } => {
                        metrics
                            .worker_status
                            .insert(name.clone(), format!("error: {}", truncate(&error, 80)));
                        metrics.push_event(
                            "system",
                            format!("worker_error: {} ({})", name, truncate(&error, 80)),
                        );
                    }
                    SystemEvent::ShutdownRequested => {
                        metrics.push_event("system", "shutdown_requested".to_string());
                    }
                    SystemEvent::HealthCheckRequest => {
                        metrics.push_event("system", "health_check_request".to_string());
                    }
                }
            }
        }
    }
}

#[async_trait]
impl Worker for CockpitWorker {
    fn name(&self) -> &str {
        "cockpit_api"
    }

    async fn start(&mut self, ctx: WorkerContext) -> Result<()> {
        if !self.config.enabled {
            self.status = WorkerStatus::Stopped;
            return Ok(());
        }

        let bind_addr: SocketAddr = self
            .config
            .bind_addr
            .parse()
            .with_context(|| format!("invalid COCKPIT_BIND address: {}", self.config.bind_addr))?;

        let prompts = Arc::new(load_prompt_catalog()?);
        let profile = get_agent_profile();
        let app_state = AppState {
            identity: AgentIdentityView {
                agent_id: profile.agent_id.clone(),
                display_name: profile.display_name.clone(),
                graph_self_id: profile.graph_self_id.clone(),
            },
            metrics: Arc::clone(&self.metrics),
            state_store: self.state_store.clone(),
            prompts,
            memory_db_path: self.memory_db_path.clone(),
            memory_reader: self.memory_reader.clone(),
            short_term: self.short_term.clone(),
            episodic: self.episodic.clone(),
            graph: self.graph.clone(),
            system_cache: Arc::clone(&self.system_cache),
            relationship_cache: Arc::clone(&self.relationship_cache),
        };

        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods([Method::GET, Method::POST])
            .allow_headers(Any);

        let app = Router::new()
            .route("/api/cockpit/overview", get(get_overview))
            .route("/api/cockpit/events", get(get_events))
            .route("/api/cockpit/states", get(get_states))
            .route("/api/cockpit/states/history", get(get_state_history))
            .route("/api/cockpit/state/metrics", get(get_state_metrics))
            .route("/api/cockpit/state/patch", post(post_state_patch))
            .route("/api/cockpit/memory", get(get_memory))
            .route("/api/cockpit/episodic", get(get_episodic))
            .route("/api/cockpit/relationships", get(get_relationships))
            .route("/api/cockpit/system", get(get_system))
            .route("/api/cockpit/prompts", get(get_prompts))
            .route("/api/cockpit/prompts/document", get(get_prompt_document))
            .route("/api/cockpit/prompts/update", post(post_prompt_update))
            .with_state(app_state)
            .layer(cors);

        let listener = TcpListener::bind(bind_addr)
            .await
            .with_context(|| format!("failed to bind cockpit API on {}", bind_addr))?;

        let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
        let server_handle = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = server_shutdown_rx.await;
                })
                .await
            {
                warn!(error = %e, "cockpit API server exited with error");
            }
        });
        info!(bind = %bind_addr, "Cockpit API started");
        self.status = WorkerStatus::Healthy;

        let mut event_rx = ctx.subscribe_events();
        let mut shutdown_rx = ctx.subscribe_shutdown();
        loop {
            tokio::select! {
                event = event_rx.recv() => {
                    if let Ok(event) = event {
                        Self::track_event(&self.metrics, event).await;
                    }
                }
                _ = shutdown_rx.recv() => {
                    break;
                }
            }
        }

        let _ = server_shutdown_tx.send(());
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), server_handle).await;
        self.status = WorkerStatus::Stopped;
        info!("Cockpit API stopped");
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

async fn get_overview(State(state): State<AppState>) -> impl IntoResponse {
    let metrics = state.metrics.read().await;
    Json(metrics.overview(&state.identity))
}

async fn get_events(
    State(state): State<AppState>,
    Query(query): Query<EventQuery>,
) -> impl IntoResponse {
    let limit = query.limit.unwrap_or(50).min(500);
    let metrics = state.metrics.read().await;
    let events: Vec<CockpitEventView> = metrics
        .recent_events
        .iter()
        .rev()
        .take(limit)
        .cloned()
        .collect();
    Json(events)
}

async fn get_states(State(state): State<AppState>) -> impl IntoResponse {
    let rows = state.state_store.rows().await;
    Json(rows)
}

async fn get_state_history(
    State(state): State<AppState>,
    Query(query): Query<EventQuery>,
) -> impl IntoResponse {
    let limit = query.limit.unwrap_or(100).min(1000);
    let history = state.state_store.history(limit).await;
    Json(history)
}

async fn get_state_metrics(State(state): State<AppState>) -> impl IntoResponse {
    let snapshot: StateMetricsSnapshot = state.state_store.metrics_snapshot().await;
    Json(snapshot)
}

async fn post_state_patch(
    State(state): State<AppState>,
    Json(req): Json<ManualPatchRequest>,
) -> Result<Json<ManualPatchResult>, (axum::http::StatusCode, String)> {
    match state.state_store.patch_manual(req).await {
        Ok(result) => Ok(Json(result)),
        Err(err) => Err((
            axum::http::StatusCode::BAD_REQUEST,
            err.to_string(),
        )),
    }
}

async fn get_prompts(State(state): State<AppState>) -> impl IntoResponse {
    Json(state.prompts.entries.clone())
}

async fn get_prompt_document(
    State(state): State<AppState>,
    Query(query): Query<PromptDocumentQuery>,
) -> Result<Json<PromptDocument>, (axum::http::StatusCode, String)> {
    let path = resolve_prompt_path(&state.prompts, &query.id).map_err(|err| {
        (
            axum::http::StatusCode::NOT_FOUND,
            err.to_string(),
        )
    })?;

    let content = std::fs::read_to_string(&path).map_err(|err| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to read prompt file {}: {}", path.display(), err),
        )
    })?;
    let _ = pa_core::prompt_registry::set_prompt_content(&query.id, content.clone());

    let rel_path = state
        .prompts
        .by_id
        .get(&query.id)
        .cloned()
        .unwrap_or_default();

    Ok(Json(PromptDocument {
        id: query.id,
        path: rel_path,
        content,
    }))
}

async fn post_prompt_update(
    State(state): State<AppState>,
    Json(req): Json<PromptUpdateRequest>,
) -> Result<Json<PromptDocument>, (axum::http::StatusCode, String)> {
    let path = resolve_prompt_path(&state.prompts, &req.id).map_err(|err| {
        (
            axum::http::StatusCode::NOT_FOUND,
            err.to_string(),
        )
    })?;

    std::fs::write(&path, &req.content).map_err(|err| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to write prompt file {}: {}", path.display(), err),
        )
    })?;
    if let Err(err) = pa_core::prompt_registry::set_prompt_content(&req.id, req.content.clone()) {
        return Err((
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to refresh prompt cache for {}: {}", req.id, err),
        ));
    }

    let rel_path = state
        .prompts
        .by_id
        .get(&req.id)
        .cloned()
        .unwrap_or_default();

    Ok(Json(PromptDocument {
        id: req.id,
        path: rel_path,
        content: req.content,
    }))
}

async fn get_memory(
    State(state): State<AppState>,
    Query(query): Query<EventQuery>,
) -> impl IntoResponse {
    let limit = query.limit.unwrap_or(20).min(100);

    let mut overview = MemoryOverview::default();

    if let (Some(reader), Some(path)) = (&state.memory_reader, &state.memory_db_path) {
        let reader = Arc::clone(reader);
        let path = path.clone();
        if let Ok(Ok(Some((count, messages)))) = tokio::task::spawn_blocking(move || {
            let mut guard = match reader.lock() {
                Ok(guard) => guard,
                Err(_) => return Ok(None),
            };

            if guard.is_none() {
                match MemoryStore::open_read_only(&path) {
                    Ok(store) => {
                        *guard = Some(store);
                    }
                    Err(_) => return Ok(None),
                }
            }

            let store = match guard.as_ref() {
                Some(store) => store,
                None => return Ok(None),
            };

            let count = store.message_count().unwrap_or(0);
            let recent = store.get_recent_all(limit).unwrap_or_default();
            Ok::<Option<(i64, Vec<MemoryMessage>)>, ()>(Some((count, recent)))
        })
        .await
        {
            overview.persisted_message_count = count;
            overview.persisted_recent_messages = messages;
        }
    }

    if let Some(short_term) = &state.short_term {
        let short_term = short_term.lock().await;
        overview.active_session_count = short_term.active_session_count();
        overview.active_sessions = short_term.active_sessions_snapshot();
        overview.active_recent_messages = short_term.recent_messages(limit);
    }

    Json(overview)
}

async fn get_episodic(
    State(state): State<AppState>,
    Query(query): Query<EventQuery>,
) -> impl IntoResponse {
    let limit = query.limit.unwrap_or(30).min(200);
    let mut overview = EpisodicOverview::default();

    if let Some(episodic) = &state.episodic {
        overview.total_chunks = episodic.count().await.unwrap_or(0);
        overview.recent_chunks = episodic
            .recent(limit)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|chunk| EpisodicChunkView {
                id: chunk.id,
                content: chunk.content,
                timestamp: DateTime::<Utc>::from_timestamp(chunk.timestamp, 0)
                    .unwrap_or_else(Utc::now)
                    .to_rfc3339(),
                importance: chunk.importance,
                metadata: chunk.metadata,
            })
            .collect();
    }

    Json(overview)
}

async fn get_relationships(State(state): State<AppState>) -> impl IntoResponse {
    if let Some(cached) = state
        .relationship_cache
        .read()
        .await
        .as_ref()
        .filter(|cached| cached.cached_at.elapsed() < RELATIONSHIP_CACHE_TTL)
        .cloned()
    {
        return Json(cached.snapshot);
    }

    if let Some(graph) = &state.graph {
        match graph.snapshot_relationship_graph().await {
            Ok(snapshot) => {
                let mut cache = state.relationship_cache.write().await;
                *cache = Some(CachedRelationshipSnapshot {
                    cached_at: Instant::now(),
                    snapshot: snapshot.clone(),
                });
                return Json(snapshot);
            }
            Err(err) => warn!(error = %err, "failed to build relationship graph snapshot"),
        }
    }

    Json(RelationshipGraphSnapshot {
        self_node_id: state.identity.graph_self_id.clone(),
        self_display_name: state.identity.display_name.clone(),
        nodes: Vec::new(),
        edges: Vec::new(),
    })
}

async fn get_system(State(state): State<AppState>) -> impl IntoResponse {
    if let Some(cached) = state
        .system_cache
        .read()
        .await
        .as_ref()
        .filter(|cached| cached.cached_at.elapsed() < SYSTEM_CACHE_TTL)
        .cloned()
    {
        return Json(cached.snapshot);
    }

    let snapshot = collect_system_snapshot().await;
    let mut cache = state.system_cache.write().await;
    *cache = Some(CachedSystemSnapshot {
        cached_at: Instant::now(),
        snapshot: snapshot.clone(),
    });
    Json(snapshot)
}

fn truncate(input: &str, max: usize) -> String {
    if input.chars().count() <= max {
        return input.to_string();
    }
    let mut out = String::with_capacity(max + 1);
    for (idx, ch) in input.chars().enumerate() {
        if idx >= max {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

async fn collect_system_snapshot() -> SystemSnapshot {
    tokio::task::spawn_blocking(collect_system_snapshot_blocking)
        .await
        .unwrap_or_else(|_| SystemSnapshot::default())
}

fn collect_system_snapshot_blocking() -> SystemSnapshot {
    let mut system = System::new_all();
    system.refresh_memory();
    system.refresh_cpu_usage();
    std::thread::sleep(Duration::from_millis(200));
    system.refresh_cpu_usage();
    system.refresh_memory();

    let disks = Disks::new_with_refreshed_list();
    let components = Components::new_with_refreshed_list();
    let load = System::load_average();

    let disks = disks
        .iter()
        .map(|disk| {
            let total_bytes = disk.total_space();
            let available_bytes = disk.available_space();
            let used_bytes = total_bytes.saturating_sub(available_bytes);
            let usage_ratio = if total_bytes == 0 {
                0.0
            } else {
                used_bytes as f64 / total_bytes as f64
            };

            DiskUsageView {
                name: disk.name().to_string_lossy().to_string(),
                mount_point: disk.mount_point().to_string_lossy().to_string(),
                total_bytes,
                available_bytes,
                used_bytes,
                usage_ratio,
            }
        })
        .collect();

    let temperatures = components
        .iter()
        .map(|component| TemperatureView {
            label: component.label().to_string(),
            temperature_celsius: component.temperature(),
        })
        .collect();

    SystemSnapshot {
        collected_at: Utc::now(),
        cpu_usage_percent: system.global_cpu_usage(),
        cpu_count: system.cpus().len(),
        load_average: LoadAverageView {
            one: load.one,
            five: load.five,
            fifteen: load.fifteen,
        },
        total_memory_bytes: system.total_memory(),
        used_memory_bytes: system.used_memory(),
        available_memory_bytes: system.available_memory(),
        disks,
        temperatures,
        gpus: read_gpu_views(),
    }
}

fn read_gpu_views() -> Vec<GpuView> {
    let output = match Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu",
            "--format=csv,noheader,nounits",
        ])
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return Vec::new(),
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(',').map(|part| part.trim()).collect();
            if fields.is_empty() {
                return None;
            }

            Some(GpuView {
                name: fields.first().copied().unwrap_or("GPU").to_string(),
                utilization_percent: parse_optional_f32(fields.get(1).copied()),
                memory_used_mb: parse_optional_u64(fields.get(2).copied()),
                memory_total_mb: parse_optional_u64(fields.get(3).copied()),
                temperature_celsius: parse_optional_f32(fields.get(4).copied()),
            })
        })
        .collect()
}

fn parse_optional_f32(value: Option<&str>) -> Option<f32> {
    value.and_then(|v| v.parse::<f32>().ok())
}

fn parse_optional_u64(value: Option<&str>) -> Option<u64> {
    value.and_then(|v| v.parse::<u64>().ok())
}

fn load_prompt_catalog() -> Result<PromptCatalog> {
    let path = find_prompt_registry_path()?;
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read prompt registry: {}", path.display()))?;
    let parsed: PromptRegistryFile =
        serde_json::from_str(&raw).context("failed to parse prompt registry json")?;

    let root_dir = path
        .parent()
        .and_then(|value| value.parent())
        .map(PathBuf::from)
        .context("failed to resolve project root from prompt registry path")?;

    let mut entries: Vec<PromptEntry> = parsed
        .prompts
        .iter()
        .map(|(id, path)| PromptEntry {
            id: id.clone(),
            path: path.clone(),
        })
        .collect();
    entries.sort_by(|a, b| a.id.cmp(&b.id));

    Ok(PromptCatalog {
        root_dir,
        entries,
        by_id: parsed.prompts,
    })
}

fn resolve_prompt_path(catalog: &PromptCatalog, id: &str) -> Result<PathBuf> {
    let rel_path = catalog
        .by_id
        .get(id)
        .with_context(|| format!("prompt id not found in registry: {}", id))?;

    let joined = catalog.root_dir.join(rel_path);
    let normalized = normalize_path(&joined);

    if !normalized.starts_with(&catalog.root_dir) {
        return Err(anyhow::anyhow!(
            "prompt path escapes project root: {}",
            normalized.display()
        ));
    }

    Ok(joined)
}

fn normalize_path(path: &std::path::Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = normalized.pop();
            }
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }

    normalized
}

fn find_prompt_registry_path() -> Result<PathBuf> {
    let mut dir = std::env::current_dir().context("failed to read current working directory")?;
    loop {
        let candidate = dir.join("config/prompt_registry.json");
        if candidate.exists() {
            return Ok(candidate);
        }
        if !dir.pop() {
            break;
        }
    }

    Err(anyhow::anyhow!(
        "prompt registry not found (expected config/prompt_registry.json)"
    ))
}
