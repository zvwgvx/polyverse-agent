use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use kernel::event::{Event, Intent, Platform, RawEvent, Sentiment};
use kernel::worker::{Worker, WorkerContext, WorkerStatus};
use serde::{Deserialize, Serialize};
use sysinfo::System;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSchema {
    pub version: String,
    pub dimensions: Vec<StateDimension>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateDimension {
    pub id: String,
    pub domain: String,
    pub scope: String,
    pub description: String,
    pub update_mode: String,
    pub range_min: f64,
    pub range_max: f64,
    pub default: f64,
    #[serde(default = "default_baseline")]
    pub baseline: f64,
    #[serde(default = "default_decay_k")]
    pub decay_k: f64,
    #[serde(default = "default_max_delta")]
    pub max_delta_per_turn: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct StateValue {
    pub dimension_id: String,
    pub value: f64,
    pub updated_at: DateTime<Utc>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StateRow {
    pub id: String,
    pub domain: String,
    pub scope: String,
    pub description: String,
    pub update_mode: String,
    pub range_min: f64,
    pub range_max: f64,
    pub baseline: f64,
    pub decay_k: f64,
    pub max_delta_per_turn: f64,
    pub value: f64,
    pub updated_at: DateTime<Utc>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StateDeltaLog {
    pub sequence: u64,
    pub dimension_id: String,
    pub before: f64,
    pub after: f64,
    pub reason: String,
    pub actor: String,
    pub source: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StateMetricEntry {
    pub source: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct StateMetricsSnapshot {
    pub total_updates: u64,
    pub last_updated_at: Option<DateTime<Utc>>,
    pub by_source: Vec<StateMetricEntry>,
}

#[derive(Debug, Clone)]
struct StateUpdateMetrics {
    total_updates: u64,
    last_updated_at: Option<DateTime<Utc>>,
    by_source: HashMap<String, u64>,
}

impl StateUpdateMetrics {
    fn new() -> Self {
        Self {
            total_updates: 0,
            last_updated_at: None,
            by_source: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManualPatchRequest {
    pub dimension_id: String,
    pub value: f64,
    pub reason: String,
    pub actor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManualPatchResult {
    pub applied: StateDeltaLog,
    pub row: StateRow,
}

#[derive(Debug, Clone)]
pub struct EventDeltaRequest {
    pub dimension_id: String,
    pub delta: f64,
    pub reason: String,
    pub actor: String,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct StateStore {
    schema: Arc<StateSchema>,
    values: Arc<RwLock<HashMap<String, StateValue>>>,
    history: Arc<RwLock<VecDeque<StateDeltaLog>>>,
    sequence: Arc<RwLock<u64>>,
    max_history: usize,
    event_cache: Arc<RwLock<HashMap<String, RecentIdSet>>>,
    event_cache_max: usize,
    metrics: Arc<RwLock<StateUpdateMetrics>>,
}

impl StateStore {
    pub fn from_schema(schema: StateSchema) -> Self {
        let now = Utc::now();
        let mut values = HashMap::new();
        for dim in &schema.dimensions {
            values.insert(
                dim.id.clone(),
                StateValue {
                    dimension_id: dim.id.clone(),
                    value: dim.default,
                    updated_at: now,
                    source: "schema_default".to_string(),
                },
            );
        }

        Self {
            schema: Arc::new(schema),
            values: Arc::new(RwLock::new(values)),
            history: Arc::new(RwLock::new(VecDeque::new())),
            sequence: Arc::new(RwLock::new(0)),
            max_history: 2_000,
            event_cache: Arc::new(RwLock::new(HashMap::new())),
            event_cache_max: 2_048,
            metrics: Arc::new(RwLock::new(StateUpdateMetrics::new())),
        }
    }

    pub fn load_from_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read state schema: {}", path.display()))?;
        let schema: StateSchema =
            serde_json::from_str(&raw).context("failed to parse state schema json")?;
        Ok(Self::from_schema(schema))
    }

    pub fn load_default() -> Result<Self> {
        let schema_path = find_schema_path()?;
        Self::load_from_file(schema_path)
    }

    pub fn schema(&self) -> &StateSchema {
        &self.schema
    }

    pub async fn metrics_snapshot(&self) -> StateMetricsSnapshot {
        let metrics = self.metrics.read().await;
        let mut by_source: Vec<StateMetricEntry> = metrics
            .by_source
            .iter()
            .map(|(source, count)| StateMetricEntry {
                source: source.clone(),
                count: *count,
            })
            .collect();
        by_source.sort_by(|a, b| b.count.cmp(&a.count));

        StateMetricsSnapshot {
            total_updates: metrics.total_updates,
            last_updated_at: metrics.last_updated_at,
            by_source,
        }
    }

    pub async fn rows(&self) -> Vec<StateRow> {
        let values = self.values.read().await;
        let mut rows = Vec::with_capacity(self.schema.dimensions.len());

        for dim in &self.schema.dimensions {
            if let Some(value) = values.get(&dim.id) {
                rows.push(StateRow {
                    id: dim.id.clone(),
                    domain: dim.domain.clone(),
                    scope: dim.scope.clone(),
                    description: dim.description.clone(),
                    update_mode: dim.update_mode.clone(),
                    range_min: dim.range_min,
                    range_max: dim.range_max,
                    baseline: dim.baseline,
                    decay_k: dim.decay_k,
                    max_delta_per_turn: dim.max_delta_per_turn,
                    value: value.value,
                    updated_at: value.updated_at,
                    source: value.source.clone(),
                });
            }
        }

        rows
    }

    pub async fn value(&self, id: &str) -> Option<f64> {
        let values = self.values.read().await;
        values.get(id).map(|value| value.value)
    }

    pub fn baseline_for(&self, id: &str) -> Option<f64> {
        self.schema
            .dimensions
            .iter()
            .find(|dim| dim.id == id)
            .map(|dim| dim.baseline)
    }

    pub async fn history(&self, limit: usize) -> Vec<StateDeltaLog> {
        let history = self.history.read().await;
        let count = limit.min(history.len());
        history.iter().rev().take(count).cloned().collect()
    }

    pub async fn patch_manual(&self, req: ManualPatchRequest) -> Result<ManualPatchResult> {
        let dim = self
            .schema
            .dimensions
            .iter()
            .find(|d| d.id == req.dimension_id)
            .with_context(|| format!("dimension not found: {}", req.dimension_id))?;

        let clamped = req.value.clamp(dim.range_min, dim.range_max);
        let actor = req
            .actor
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("runtime")
            .to_string();

        let mut values = self.values.write().await;
        let current = values.get(&req.dimension_id).with_context(|| {
            format!(
                "state value not initialized for dimension {}",
                req.dimension_id
            )
        })?;

        let now = Utc::now();
        let before = current.value;

        values.insert(
            req.dimension_id.clone(),
            StateValue {
                dimension_id: req.dimension_id.clone(),
                value: clamped,
                updated_at: now,
                source: "manual_patch".to_string(),
            },
        );
        drop(values);

        let mut seq = self.sequence.write().await;
        *seq += 1;
        let log = StateDeltaLog {
            sequence: *seq,
            dimension_id: req.dimension_id.clone(),
            before,
            after: clamped,
            reason: req.reason,
            actor,
            source: "manual_patch".to_string(),
            timestamp: now,
        };
        drop(seq);

        let mut history = self.history.write().await;
        history.push_back(log.clone());
        while history.len() > self.max_history {
            let _ = history.pop_front();
        }
        drop(history);

        let values = self.values.read().await;
        let current = values.get(&req.dimension_id).with_context(|| {
            format!(
                "state value not initialized after patch for dimension {}",
                req.dimension_id
            )
        })?;

        let row = StateRow {
            id: dim.id.clone(),
            domain: dim.domain.clone(),
            scope: dim.scope.clone(),
            description: dim.description.clone(),
            update_mode: dim.update_mode.clone(),
            range_min: dim.range_min,
            range_max: dim.range_max,
            baseline: dim.baseline,
            decay_k: dim.decay_k,
            max_delta_per_turn: dim.max_delta_per_turn,
            value: current.value,
            updated_at: current.updated_at,
            source: current.source.clone(),
        };

        let result = ManualPatchResult { applied: log, row };

        self.record_metrics("manual_patch", 1).await;

        if !dim.update_mode.eq_ignore_ascii_case("derived") {
            let _ = self.recompute_derived().await;
        }

        Ok(result)
    }

    pub async fn apply_event_deltas(&self, requests: &[EventDeltaRequest]) -> Result<usize> {
        if requests.is_empty() {
            return Ok(0);
        }

        struct PreparedDelta<'a> {
            req: &'a EventDeltaRequest,
            range_min: f64,
            range_max: f64,
            max_delta: f64,
        }

        let mut prepared = Vec::with_capacity(requests.len());
        for req in requests {
            let dim = self
                .schema
                .dimensions
                .iter()
                .find(|d| d.id == req.dimension_id)
                .with_context(|| format!("dimension not found: {}", req.dimension_id))?;
            prepared.push(PreparedDelta {
                req,
                range_min: dim.range_min,
                range_max: dim.range_max,
                max_delta: dim.max_delta_per_turn,
            });
        }

        let (updated, counts) = {
            let now = Utc::now();
            let mut values = self.values.write().await;
            let mut seq = self.sequence.write().await;
            let mut history = self.history.write().await;
            let mut updated = 0usize;
            let mut counts: HashMap<String, u64> = HashMap::new();

            for item in prepared {
                let before = values
                    .get(&item.req.dimension_id)
                    .with_context(|| {
                        format!(
                            "state value not initialized for dimension {}",
                            item.req.dimension_id
                        )
                    })?
                    .value;
                let mut delta = item.req.delta;
                if item.max_delta > 0.0 {
                    delta = delta.clamp(-item.max_delta, item.max_delta);
                }
                if delta.abs() < 1e-6 {
                    continue;
                }
                let after = (before + delta).clamp(item.range_min, item.range_max);
                if (after - before).abs() < 1e-6 {
                    continue;
                }

                let source_value = if item.req.source.trim().is_empty() {
                    "event_delta"
                } else {
                    item.req.source.trim()
                };
                let source = source_value.to_string();

                values.insert(
                    item.req.dimension_id.clone(),
                    StateValue {
                        dimension_id: item.req.dimension_id.clone(),
                        value: after,
                        updated_at: now,
                        source: source.clone(),
                    },
                );

                *seq += 1;
                history.push_back(StateDeltaLog {
                    sequence: *seq,
                    dimension_id: item.req.dimension_id.clone(),
                    before,
                    after,
                    reason: if item.req.reason.trim().is_empty() {
                        "event_delta".to_string()
                    } else {
                        item.req.reason.clone()
                    },
                    actor: if item.req.actor.trim().is_empty() {
                        "system".to_string()
                    } else {
                        item.req.actor.clone()
                    },
                    source: source.clone(),
                    timestamp: now,
                });

                *counts.entry(source).or_insert(0) += 1;
                updated += 1;
            }

            while history.len() > self.max_history {
                let _ = history.pop_front();
            }

            (updated, counts)
        };

        if updated > 0 {
            self.record_metrics_batch(&counts).await;
            let _ = self.recompute_derived().await;
        }

        Ok(updated)
    }

    pub async fn mark_event_if_new(&self, source: &str, event_id: &str) -> bool {
        if event_id.trim().is_empty() {
            return true;
        }
        let mut cache = self.event_cache.write().await;
        let entry = cache
            .entry(source.to_string())
            .or_insert_with(|| RecentIdSet::new(self.event_cache_max));
        entry.insert(event_id.to_string())
    }

    pub async fn has_recent_event(&self, source: &str, event_id: &str) -> bool {
        if event_id.trim().is_empty() {
            return false;
        }
        let cache = self.event_cache.read().await;
        cache
            .get(source)
            .map(|set| set.contains(event_id))
            .unwrap_or(false)
    }

    pub async fn apply_drift_tick(&self, dt_seconds: f64) -> Result<usize> {
        if dt_seconds <= 0.0 {
            return Ok(0);
        }

        let updated = {
            let now = Utc::now();
            let mut values = self.values.write().await;
            let mut seq = self.sequence.write().await;
            let mut history = self.history.write().await;
            let mut updated = 0usize;

        for dim in &self.schema.dimensions {
            if !is_drift_mode(&dim.update_mode) {
                continue;
            }
                if dim.decay_k <= 0.0 {
                    continue;
                }
                let before = values
                    .get(&dim.id)
                    .with_context(|| {
                        format!("state value not initialized for dimension {}", dim.id)
                    })?
                    .value;
                let factor = (dim.decay_k * dt_seconds).clamp(0.0, 1.0);
                if factor <= 0.0 {
                    continue;
                }
                let target = dim.baseline;
                let mut after = (before + (target - before) * factor)
                    .clamp(dim.range_min, dim.range_max);
                let mut delta = after - before;
                if delta.abs() < 0.001 {
                    after = target.clamp(dim.range_min, dim.range_max);
                    delta = after - before;
                }
                if delta.abs() < 1e-6 {
                    continue;
                }

                values.insert(
                    dim.id.clone(),
                    StateValue {
                        dimension_id: dim.id.clone(),
                        value: after,
                        updated_at: now,
                        source: "drift_tick".to_string(),
                    },
                );

                *seq += 1;
                history.push_back(StateDeltaLog {
                    sequence: *seq,
                    dimension_id: dim.id.clone(),
                    before,
                    after,
                    reason: "drift_tick".to_string(),
                    actor: "system".to_string(),
                    source: "drift_tick".to_string(),
                    timestamp: now,
                });

                updated += 1;
            }

            while history.len() > self.max_history {
                let _ = history.pop_front();
            }

            updated
        };

        if updated > 0 {
            self.record_metrics("drift_tick", updated as u64).await;
            let _ = self.recompute_derived().await;
        }

        Ok(updated)
    }

    pub async fn recompute_derived(&self) -> Result<usize> {
        let values = self.values.read().await;
        let get = |id: &str| values.get(id).map(|v| v.value);

        let safety = match get("session_social.safety") {
            Some(v) => v,
            None => return Ok(0),
        };
        let trust = get("session_social.trust").unwrap_or(0.0);
        let tension = get("session_social.tension").unwrap_or(0.0);
        let valence = get("emotion.valence").unwrap_or(0.0);
        let arousal = get("emotion.arousal").unwrap_or(0.0);
        let anxiety = get("emotion.anxiety").unwrap_or(0.0);
        let anger = get("emotion.anger").unwrap_or(0.0);
        let confidence = get("emotion.confidence").unwrap_or(0.0);
        let stability = get("emotion.stability").unwrap_or(0.0);
        let energy = get("system.energy").unwrap_or(0.0);
        let responsiveness = get("system.responsiveness").unwrap_or(0.0);
        let goal_focus = get("goal.current_focus").unwrap_or(0.0);
        let goal_commitment = get("goal.commitment").unwrap_or(0.0);
        let goal_clarity = get("goal.clarity").unwrap_or(0.0);

        drop(values);

        let valence_norm = clamp01((valence + 1.0) * 0.5);
        let safety_norm = clamp01((safety + 1.0) * 0.5);
        let trust_norm = clamp01((trust + 1.0) * 0.5);

        let mut computed: HashMap<String, f64> = HashMap::new();
        computed.insert(
            "cognition.clarity".to_string(),
            clamp01(0.4 * stability + 0.3 * energy + 0.3 * safety_norm),
        );
        computed.insert(
            "cognition.focus".to_string(),
            clamp01(0.4 * responsiveness + 0.3 * (1.0 - tension) + 0.3 * confidence),
        );
        computed.insert(
            "cognition.coherence".to_string(),
            clamp01(0.4 * stability + 0.3 * confidence + 0.3 * (1.0 - anxiety)),
        );
        computed.insert(
            "cognition.creativity".to_string(),
            clamp01(0.4 * valence_norm + 0.3 * arousal + 0.3 * (1.0 - anxiety)),
        );
        computed.insert(
            "cognition.decisiveness".to_string(),
            clamp01(0.5 * confidence + 0.3 * energy + 0.2 * (1.0 - tension)),
        );
        computed.insert(
            "cognition.consistency".to_string(),
            clamp01(0.5 * stability + 0.3 * (1.0 - arousal) + 0.2 * (1.0 - tension)),
        );
        computed.insert(
            "risk.safety".to_string(),
            clamp01(0.5 * (1.0 - safety_norm) + 0.3 * tension + 0.2 * anger),
        );
        computed.insert(
            "risk.privacy".to_string(),
            clamp01(0.6 * (1.0 - trust_norm) + 0.4 * anxiety),
        );
        computed.insert(
            "risk.escalation".to_string(),
            clamp01(0.5 * tension + 0.3 * anger + 0.2 * (1.0 - safety_norm)),
        );
        computed.insert(
            "goal.progress".to_string(),
            clamp01(0.4 * goal_focus + 0.3 * goal_commitment + 0.3 * goal_clarity),
        );

        let updated = {
            let now = Utc::now();
            let mut values = self.values.write().await;
            let mut updated = 0usize;

            for dim in &self.schema.dimensions {
                if !dim.update_mode.eq_ignore_ascii_case("derived") {
                    continue;
                }
                let Some(target) = computed.get(&dim.id) else {
                    continue;
                };
                let before = values
                    .get(&dim.id)
                    .with_context(|| {
                        format!("state value not initialized for dimension {}", dim.id)
                    })?
                    .value;
                let after = target.clamp(dim.range_min, dim.range_max);
                if (after - before).abs() < 1e-6 {
                    continue;
                }
                values.insert(
                    dim.id.clone(),
                    StateValue {
                        dimension_id: dim.id.clone(),
                        value: after,
                        updated_at: now,
                        source: "derived".to_string(),
                    },
                );
                updated += 1;
            }

            updated
        };

        if updated > 0 {
            self.record_metrics("derived", updated as u64).await;
        }

        Ok(updated)
    }

    async fn record_metrics(&self, source: &str, count: u64) {
        if count == 0 {
            return;
        }
        let mut metrics = self.metrics.write().await;
        metrics.total_updates += count;
        metrics.last_updated_at = Some(Utc::now());
        *metrics.by_source.entry(source.to_string()).or_insert(0) += count;
    }

    async fn record_metrics_batch(&self, counts: &HashMap<String, u64>) {
        if counts.is_empty() {
            return;
        }
        let mut metrics = self.metrics.write().await;
        metrics.last_updated_at = Some(Utc::now());
        for (source, count) in counts {
            metrics.total_updates += *count;
            *metrics.by_source.entry(source.clone()).or_insert(0) += count;
        }
    }
}

static SCHEMA_SEARCH_ROOT: OnceLock<PathBuf> = OnceLock::new();

pub fn set_schema_search_root(path: impl AsRef<Path>) {
    let _ = SCHEMA_SEARCH_ROOT.set(path.as_ref().to_path_buf());
}

fn find_schema_path() -> Result<PathBuf> {
    if let Some(root) = SCHEMA_SEARCH_ROOT.get() {
        let candidate = root.join("config/state_schema.v0.json");
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    let mut dir = std::env::current_dir().context("failed to read current working directory")?;
    loop {
        let candidate = dir.join("config/state_schema.v0.json");
        if candidate.exists() {
            return Ok(candidate);
        }
        if !dir.pop() {
            break;
        }
    }

    Err(anyhow::anyhow!(
        "state schema not found (expected config/state_schema.v0.json)"
    ))
}

fn default_baseline() -> f64 {
    0.0
}

fn default_decay_k() -> f64 {
    0.0
}

fn default_max_delta() -> f64 {
    0.0
}

fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

fn is_drift_mode(mode: &str) -> bool {
    let m = mode.trim().to_ascii_lowercase();
    m == "drift" || m == "drift_event" || m == "drift+event" || m == "event+drift"
}

#[derive(Debug)]
struct RecentIdSet {
    order: VecDeque<String>,
    set: HashSet<String>,
    max: usize,
}

impl RecentIdSet {
    fn new(max: usize) -> Self {
        Self {
            order: VecDeque::new(),
            set: HashSet::new(),
            max,
        }
    }

    fn contains(&self, id: &str) -> bool {
        self.set.contains(id)
    }

    fn insert(&mut self, id: String) -> bool {
        if self.set.contains(&id) {
            return false;
        }
        self.set.insert(id.clone());
        self.order.push_back(id);
        while self.order.len() > self.max {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
        true
    }
}

pub struct StateDriftWorker {
    store: StateStore,
    status: WorkerStatus,
    interval: Duration,
}

pub struct StateIntentWorker {
    store: StateStore,
    status: WorkerStatus,
}

pub struct StateCommandWorker {
    store: StateStore,
    status: WorkerStatus,
}

pub struct StateUserWorker {
    store: StateStore,
    status: WorkerStatus,
}

pub struct StateGoalWorker {
    store: StateStore,
    status: WorkerStatus,
}

pub struct StateEnvironmentWorker {
    store: StateStore,
    status: WorkerStatus,
}

pub struct StateSystemWorker {
    store: StateStore,
    status: WorkerStatus,
    interval: Duration,
    adjust_rate: f64,
}

impl StateEnvironmentWorker {
    pub fn new(store: StateStore) -> Self {
        Self {
            store,
            status: WorkerStatus::NotStarted,
        }
    }
}

impl StateSystemWorker {
    pub fn new(store: StateStore) -> Self {
        Self {
            store,
            status: WorkerStatus::NotStarted,
            interval: Duration::from_secs(1),
            adjust_rate: 0.2,
        }
    }

    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    pub fn with_adjust_rate(mut self, adjust_rate: f64) -> Self {
        self.adjust_rate = adjust_rate.clamp(0.01, 1.0);
        self
    }
}

impl StateGoalWorker {
    pub fn new(store: StateStore) -> Self {
        Self {
            store,
            status: WorkerStatus::NotStarted,
        }
    }
}

impl StateIntentWorker {
    pub fn new(store: StateStore) -> Self {
        Self {
            store,
            status: WorkerStatus::NotStarted,
        }
    }
}

impl StateCommandWorker {
    pub fn new(store: StateStore) -> Self {
        Self {
            store,
            status: WorkerStatus::NotStarted,
        }
    }
}

impl StateUserWorker {
    pub fn new(store: StateStore) -> Self {
        Self {
            store,
            status: WorkerStatus::NotStarted,
        }
    }
}

impl StateDriftWorker {
    pub fn new(store: StateStore) -> Self {
        Self {
            store,
            status: WorkerStatus::NotStarted,
            interval: Duration::from_secs(1),
        }
    }

    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }
}

#[async_trait]
impl Worker for StateDriftWorker {
    fn name(&self) -> &str {
        "state_drift"
    }

    async fn start(&mut self, ctx: WorkerContext) -> Result<()> {
        self.status = WorkerStatus::Healthy;
        let mut shutdown_rx = ctx.subscribe_shutdown();
        let mut ticker = tokio::time::interval(self.interval);

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let _ = self.store.apply_drift_tick(self.interval.as_secs_f64()).await;
                }
                _ = shutdown_rx.recv() => break,
            }
        }

        self.status = WorkerStatus::Stopped;
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

#[async_trait]
impl Worker for StateIntentWorker {
    fn name(&self) -> &str {
        "state_intent"
    }

    async fn start(&mut self, ctx: WorkerContext) -> Result<()> {
        self.status = WorkerStatus::Healthy;
        let mut shutdown_rx = ctx.subscribe_shutdown();
        let mut event_rx = ctx.subscribe_events();

        loop {
            tokio::select! {
                result = event_rx.recv() => {
                    match result {
                        Ok(Event::Intent(intent)) => {
                            if intent.confidence < 0.4 {
                                continue;
                            }
                            if self
                                .store
                                .has_recent_event("affect", &intent.source.message_id)
                                .await
                            {
                                continue;
                            }
                            if !self
                                .store
                                .mark_event_if_new("intent", &intent.source.message_id)
                                .await
                            {
                                continue;
                            }
                            let weight = match intent.intent {
                                Intent::Insult => 1.5,
                                Intent::Command => 0.2,
                                Intent::Question => 0.8,
                                Intent::ChitChat => 0.7,
                                Intent::ComplexQuery => 0.6,
                                Intent::Noise => 0.0,
                            };
                            if weight <= 0.0 {
                                continue;
                            }

                            let mut updates = Vec::new();
                            let actor = intent.source.username.clone();
                            match intent.sentiment {
                                Sentiment::Positive => {
                                    push_intent_delta(&mut updates, "emotion.valence", 0.03 * weight, &actor);
                                    push_intent_delta(&mut updates, "emotion.joy", 0.03 * weight, &actor);
                                    push_intent_delta(&mut updates, "emotion.anxiety", -0.02 * weight, &actor);
                                    push_intent_delta(&mut updates, "session_social.tension", -0.02 * weight, &actor);
                                    push_intent_delta(&mut updates, "session_social.safety", 0.02 * weight, &actor);
                                    push_intent_delta(&mut updates, "session_social.affinity", 0.02 * weight, &actor);
                                }
                                Sentiment::Negative => {
                                    push_intent_delta(&mut updates, "emotion.valence", -0.03 * weight, &actor);
                                    push_intent_delta(&mut updates, "emotion.joy", -0.02 * weight, &actor);
                                    push_intent_delta(&mut updates, "emotion.anxiety", 0.03 * weight, &actor);
                                    push_intent_delta(&mut updates, "session_social.tension", 0.03 * weight, &actor);
                                    push_intent_delta(&mut updates, "session_social.safety", -0.02 * weight, &actor);
                                    push_intent_delta(&mut updates, "session_social.affinity", -0.02 * weight, &actor);
                                }
                                Sentiment::Neutral => {}
                            }

                            if !updates.is_empty() {
                                let _ = self.store.apply_event_deltas(&updates).await;
                            }
                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = shutdown_rx.recv() => break,
            }
        }

        self.status = WorkerStatus::Stopped;
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

#[async_trait]
impl Worker for StateCommandWorker {
    fn name(&self) -> &str {
        "state_command"
    }

    async fn start(&mut self, ctx: WorkerContext) -> Result<()> {
        self.status = WorkerStatus::Healthy;
        let mut shutdown_rx = ctx.subscribe_shutdown();
        let mut event_rx = ctx.subscribe_events();

        loop {
            tokio::select! {
                result = event_rx.recv() => {
                    match result {
                        Ok(Event::Raw(raw)) => {
                            if !should_handle_command(&raw) {
                                continue;
                            }
                            let Some(command) = parse_state_command(&raw.content) else {
                                continue;
                            };
                            if !self.store.mark_event_if_new("state_command", &raw.message_id).await {
                                continue;
                            }

                            let actor = raw.username.clone();
                            for (dimension_id, value) in command.sets {
                                let _ = self.store.patch_manual(ManualPatchRequest {
                                    dimension_id,
                                    value,
                                    reason: "state_command".to_string(),
                                    actor: Some(actor.clone()),
                                }).await;
                            }

                            if !command.deltas.is_empty() {
                                let updates: Vec<EventDeltaRequest> = command
                                    .deltas
                                    .into_iter()
                                    .map(|(dimension_id, delta)| EventDeltaRequest {
                                        dimension_id,
                                        delta,
                                        reason: "state_command".to_string(),
                                        actor: actor.clone(),
                                        source: "state_command".to_string(),
                                    })
                                    .collect();
                                let _ = self.store.apply_event_deltas(&updates).await;
                            }
                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = shutdown_rx.recv() => break,
            }
        }

        self.status = WorkerStatus::Stopped;
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

#[async_trait]
impl Worker for StateUserWorker {
    fn name(&self) -> &str {
        "state_user"
    }

    async fn start(&mut self, ctx: WorkerContext) -> Result<()> {
        self.status = WorkerStatus::Healthy;
        let mut shutdown_rx = ctx.subscribe_shutdown();
        let mut event_rx = ctx.subscribe_events();

        loop {
            tokio::select! {
                result = event_rx.recv() => {
                    match result {
                        Ok(Event::Raw(raw)) => {
                            if !raw.is_mention && !raw.is_dm {
                                continue;
                            }
                            if !self.store.mark_event_if_new("state_user_raw", &raw.message_id).await {
                                continue;
                            }
                            let mut updates = Vec::new();
                            let actor = raw.username.clone();
                            push_user_delta(&mut updates, "user.engagement", 0.02, &actor, "interaction");
                            push_user_delta(&mut updates, "user.familiarity", 0.01, &actor, "interaction");
                            push_user_delta(&mut updates, "user.reliability", 0.005, &actor, "interaction");

                            if !updates.is_empty() {
                                let _ = self.store.apply_event_deltas(&updates).await;
                            }
                        }
                        Ok(Event::Intent(intent)) => {
                            if intent.confidence < 0.4 {
                                continue;
                            }
                            if !self.store.mark_event_if_new("state_user_intent", &intent.source.message_id).await {
                                continue;
                            }
                            let mut updates = Vec::new();
                            let actor = intent.source.username.clone();
                            match intent.sentiment {
                                Sentiment::Positive => {
                                    push_user_delta(&mut updates, "user.sentiment", 0.03, &actor, "intent_signal");
                                    push_user_delta(&mut updates, "user.trust", 0.01, &actor, "intent_signal");
                                    push_user_delta(&mut updates, "user.boundary_respect", 0.01, &actor, "intent_signal");
                                }
                                Sentiment::Negative => {
                                    push_user_delta(&mut updates, "user.sentiment", -0.03, &actor, "intent_signal");
                                    push_user_delta(&mut updates, "user.trust", -0.01, &actor, "intent_signal");
                                    push_user_delta(&mut updates, "user.boundary_respect", -0.01, &actor, "intent_signal");
                                }
                                Sentiment::Neutral => {}
                            }
                            if !updates.is_empty() {
                                let _ = self.store.apply_event_deltas(&updates).await;
                            }
                        }
                        Ok(Event::Response(response)) => {
                            if response.source != kernel::event::ResponseSource::CloudLLM {
                                continue;
                            }
                            let Some(reply_to) = response.reply_to_user.clone() else {
                                continue;
                            };
                            let Some(message_id) = response.reply_to_message_id.clone() else {
                                continue;
                            };
                            if !self.store.mark_event_if_new("state_user_response", &message_id).await {
                                continue;
                            }
                            let mut updates = Vec::new();
                            push_user_delta(&mut updates, "user.engagement", 0.01, &reply_to, "bot_response");
                            let _ = self.store.apply_event_deltas(&updates).await;
                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = shutdown_rx.recv() => break,
            }
        }

        self.status = WorkerStatus::Stopped;
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

#[async_trait]
impl Worker for StateGoalWorker {
    fn name(&self) -> &str {
        "state_goal"
    }

    async fn start(&mut self, ctx: WorkerContext) -> Result<()> {
        self.status = WorkerStatus::Healthy;
        let mut shutdown_rx = ctx.subscribe_shutdown();
        let mut event_rx = ctx.subscribe_events();

        loop {
            tokio::select! {
                result = event_rx.recv() => {
                    match result {
                        Ok(Event::Raw(raw)) => {
                            if !raw.is_mention && !raw.is_dm {
                                continue;
                            }
                            if !self.store.mark_event_if_new("state_goal_raw", &raw.message_id).await {
                                continue;
                            }
                            let actor = raw.username.clone();
                            let mut updates = Vec::new();
                            push_goal_delta(&mut updates, "goal.current_focus", 0.02, &actor, "interaction");
                            push_goal_delta(&mut updates, "goal.commitment", 0.01, &actor, "interaction");
                            if !updates.is_empty() {
                                let _ = self.store.apply_event_deltas(&updates).await;
                            }
                        }
                        Ok(Event::Intent(intent)) => {
                            if intent.confidence < 0.4 {
                                continue;
                            }
                            if !self.store.mark_event_if_new("state_goal_intent", &intent.source.message_id).await {
                                continue;
                            }
                            let actor = intent.source.username.clone();
                            let mut updates = Vec::new();
                            match intent.intent {
                                Intent::Question | Intent::ComplexQuery => {
                                    push_goal_delta(&mut updates, "goal.current_focus", 0.03, &actor, "intent_focus");
                                    push_goal_delta(&mut updates, "goal.clarity", 0.02, &actor, "intent_focus");
                                }
                                Intent::Command => {
                                    push_goal_delta(&mut updates, "goal.urgency", 0.03, &actor, "intent_urgency");
                                    push_goal_delta(&mut updates, "goal.constraint_pressure", 0.02, &actor, "intent_urgency");
                                }
                                Intent::Insult => {
                                    push_goal_delta(&mut updates, "goal.commitment", -0.02, &actor, "intent_disruption");
                                    push_goal_delta(&mut updates, "goal.satisfaction", -0.03, &actor, "intent_disruption");
                                }
                                Intent::ChitChat => {
                                    push_goal_delta(&mut updates, "goal.current_focus", -0.01, &actor, "intent_chitchat");
                                }
                                Intent::Noise => {}
                            }
                            if !updates.is_empty() {
                                let _ = self.store.apply_event_deltas(&updates).await;
                            }
                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = shutdown_rx.recv() => break,
            }
        }

        self.status = WorkerStatus::Stopped;
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

#[async_trait]
impl Worker for StateEnvironmentWorker {
    fn name(&self) -> &str {
        "state_environment"
    }

    async fn start(&mut self, ctx: WorkerContext) -> Result<()> {
        self.status = WorkerStatus::Healthy;
        let mut shutdown_rx = ctx.subscribe_shutdown();
        let mut event_rx = ctx.subscribe_events();

        loop {
            tokio::select! {
                result = event_rx.recv() => {
                    match result {
                        Ok(Event::Raw(raw)) => {
                            if !raw.is_mention && !raw.is_dm {
                                continue;
                            }
                            if !self.store.mark_event_if_new("state_env_raw", &raw.message_id).await {
                                continue;
                            }
                            let content_len = raw.content.len() as f64;
                            let load_delta = (content_len / 500.0).min(0.02);
                            let noise_delta = if raw.content.chars().filter(|c| !c.is_alphanumeric() && !c.is_whitespace()).count() > 10 {
                                0.02
                            } else {
                                0.0
                            };
                            let mut updates = Vec::new();
                            push_env_delta(&mut updates, "environment.load", load_delta, "runtime");
                            if noise_delta > 0.0 {
                                push_env_delta(&mut updates, "environment.noise", noise_delta, "runtime");
                            }
                            if !updates.is_empty() {
                                let _ = self.store.apply_event_deltas(&updates).await;
                            }
                        }
                        Ok(Event::Response(response)) => {
                            if response.source != kernel::event::ResponseSource::CloudLLM {
                                continue;
                            }
                            let Some(message_id) = response.reply_to_message_id.clone() else {
                                continue;
                            };
                            if !self.store.mark_event_if_new("state_env_response", &message_id).await {
                                continue;
                            }
                            let mut updates = Vec::new();
                            push_env_delta(&mut updates, "environment.load", 0.01, "runtime");
                            push_env_delta(&mut updates, "environment.channel_quality", 0.005, "runtime");
                            let _ = self.store.apply_event_deltas(&updates).await;
                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = shutdown_rx.recv() => break,
            }
        }

        self.status = WorkerStatus::Stopped;
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

#[async_trait]
impl Worker for StateSystemWorker {
    fn name(&self) -> &str {
        "state_system"
    }

    async fn start(&mut self, ctx: WorkerContext) -> Result<()> {
        self.status = WorkerStatus::Healthy;
        let mut shutdown_rx = ctx.subscribe_shutdown();
        let mut ticker = tokio::time::interval(self.interval);
        let mut system = System::new_all();

        system.refresh_cpu_usage();
        system.refresh_memory();

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    system.refresh_cpu_usage();
                    system.refresh_memory();

                    let cpu_ratio = (system.global_cpu_usage() as f64 / 100.0).clamp(0.0, 1.0);
                    let total_mem = system.total_memory() as f64;
                    let used_mem = system.used_memory() as f64;
                    let mem_ratio = if total_mem > 0.0 {
                        (used_mem / total_mem).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };

                    let load = System::load_average();
                    let cpu_count = system.cpus().len().max(1) as f64;
                    let load_ratio = (load.one / cpu_count).clamp(0.0, 2.0) / 2.0;

                    let pressure = clamp01(0.5 * cpu_ratio + 0.35 * mem_ratio + 0.15 * load_ratio);

                    let energy_baseline = self.store.baseline_for("system.energy").unwrap_or(0.7);
                    let fatigue_baseline = self.store.baseline_for("system.fatigue").unwrap_or(0.2);
                    let response_baseline = self.store.baseline_for("system.responsiveness").unwrap_or(0.75);

                    let energy_current = self.store.value("system.energy").await.unwrap_or(energy_baseline);
                    let fatigue_current = self.store.value("system.fatigue").await.unwrap_or(fatigue_baseline);
                    let response_current = self
                        .store
                        .value("system.responsiveness")
                        .await
                        .unwrap_or(response_baseline);

                    let energy_target = (energy_baseline - 0.4 * pressure).clamp(0.0, 1.0);
                    let fatigue_target = (fatigue_baseline + 0.5 * pressure).clamp(0.0, 1.0);
                    let response_target = (response_baseline - 0.55 * pressure).clamp(0.0, 1.0);

                    let mut updates = Vec::new();
                    let energy_delta = (energy_target - energy_current) * self.adjust_rate;
                    if energy_delta.abs() > 1e-6 {
                        push_system_delta(&mut updates, "system.energy", energy_delta, "system_metrics");
                    }
                    let fatigue_delta = (fatigue_target - fatigue_current) * self.adjust_rate;
                    if fatigue_delta.abs() > 1e-6 {
                        push_system_delta(&mut updates, "system.fatigue", fatigue_delta, "system_metrics");
                    }
                    let response_delta = (response_target - response_current) * self.adjust_rate;
                    if response_delta.abs() > 1e-6 {
                        push_system_delta(
                            &mut updates,
                            "system.responsiveness",
                            response_delta,
                            "system_metrics",
                        );
                    }

                    if !updates.is_empty() {
                        let _ = self.store.apply_event_deltas(&updates).await;
                    }
                }
                _ = shutdown_rx.recv() => break,
            }
        }

        self.status = WorkerStatus::Stopped;
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

fn push_intent_delta(updates: &mut Vec<EventDeltaRequest>, dimension_id: &str, delta: f64, actor: &str) {
    updates.push(EventDeltaRequest {
        dimension_id: dimension_id.to_string(),
        delta,
        reason: "intent_signal".to_string(),
        actor: actor.to_string(),
        source: "intent_signal".to_string(),
    });
}

fn push_user_delta(
    updates: &mut Vec<EventDeltaRequest>,
    dimension_id: &str,
    delta: f64,
    actor: &str,
    reason: &str,
) {
    updates.push(EventDeltaRequest {
        dimension_id: dimension_id.to_string(),
        delta,
        reason: reason.to_string(),
        actor: actor.to_string(),
        source: "user_state".to_string(),
    });
}

fn push_goal_delta(
    updates: &mut Vec<EventDeltaRequest>,
    dimension_id: &str,
    delta: f64,
    actor: &str,
    reason: &str,
) {
    updates.push(EventDeltaRequest {
        dimension_id: dimension_id.to_string(),
        delta,
        reason: reason.to_string(),
        actor: actor.to_string(),
        source: "goal_state".to_string(),
    });
}

fn push_env_delta(
    updates: &mut Vec<EventDeltaRequest>,
    dimension_id: &str,
    delta: f64,
    reason: &str,
) {
    updates.push(EventDeltaRequest {
        dimension_id: dimension_id.to_string(),
        delta,
        reason: reason.to_string(),
        actor: "system".to_string(),
        source: "environment_state".to_string(),
    });
}

fn push_system_delta(
    updates: &mut Vec<EventDeltaRequest>,
    dimension_id: &str,
    delta: f64,
    reason: &str,
) {
    updates.push(EventDeltaRequest {
        dimension_id: dimension_id.to_string(),
        delta,
        reason: reason.to_string(),
        actor: "system".to_string(),
        source: "system_metrics".to_string(),
    });
}

struct ParsedCommand {
    sets: Vec<(String, f64)>,
    deltas: Vec<(String, f64)>,
}

fn should_handle_command(raw: &RawEvent) -> bool {
    raw.is_dm || raw.is_mention || raw.platform == Platform::Cli
}

fn parse_state_command(input: &str) -> Option<ParsedCommand> {
    let trimmed = input.trim();
    let (prefix, rest) = if let Some(rest) = trimmed.strip_prefix("/state") {
        ("/state", rest)
    } else if let Some(rest) = trimmed.strip_prefix("!state") {
        ("!state", rest)
    } else if let Some(rest) = trimmed.strip_prefix("#state") {
        ("#state", rest)
    } else {
        return None;
    };

    let mut body = rest.trim();
    if body.starts_with(':') {
        body = body.trim_start_matches(':').trim_start();
    }
    if body.is_empty() {
        return None;
    }

    let mut sets = Vec::new();
    let mut deltas = Vec::new();

    for raw_token in body.split(|c: char| c.is_whitespace() || c == ',') {
        let token = raw_token.trim();
        if token.is_empty() {
            continue;
        }
        let (key, op, value) = match parse_kv_token(token) {
            Some(parsed) => parsed,
            None => continue,
        };
        let Some(dimension_id) = normalize_dimension_key(key) else {
            continue;
        };
        match op {
            TokenOp::Set => sets.push((dimension_id, value)),
            TokenOp::Delta => deltas.push((dimension_id, value)),
        }
    }

    if sets.is_empty() && deltas.is_empty() {
        return None;
    }

    let _ = prefix;
    Some(ParsedCommand { sets, deltas })
}

#[derive(Clone, Copy)]
enum TokenOp {
    Set,
    Delta,
}

fn parse_kv_token(token: &str) -> Option<(&str, TokenOp, f64)> {
    if let Some((key, value)) = token.split_once("+=") {
        let val = value.trim().parse::<f64>().ok()?;
        return Some((key.trim(), TokenOp::Delta, val));
    }
    if let Some((key, value)) = token.split_once("-=") {
        let val = value.trim().parse::<f64>().ok()?;
        return Some((key.trim(), TokenOp::Delta, -val));
    }
    if let Some((key, value)) = token.split_once('=') {
        let val = value.trim().parse::<f64>().ok()?;
        return Some((key.trim(), TokenOp::Set, val));
    }
    None
}

fn normalize_dimension_key(key: &str) -> Option<String> {
    let k = key.trim().to_ascii_lowercase();
    let mapped = match k.as_str() {
        "warmth" | "style.warmth" => "style.warmth",
        "play" | "playfulness" | "style.playfulness" => "style.playfulness",
        "formality" | "formal" | "style.formality" => "style.formality",
        "brevity" | "style.brevity" => "style.brevity",
        "pref_brevity" | "preference.brevity" => "preference.brevity",
        "curiosity" | "preference.curiosity" => "preference.curiosity",
        "fascination" | "preference.fascination" => "preference.fascination",
        "stress" | "preference.stress" => "preference.stress",
        "depth" | "preference.depth" => "preference.depth",
        "directness" | "preference.directness" => "preference.directness",
        "empathy" | "empathy_bias" | "preference.empathy_bias" => "preference.empathy_bias",
        "risk" | "risk_tolerance" | "preference.risk_tolerance" => "preference.risk_tolerance",
        _ => return None,
    };
    Some(mapped.to_string())
}
