use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
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
pub struct StateStore {
    schema: Arc<StateSchema>,
    values: Arc<RwLock<HashMap<String, StateValue>>>,
    history: Arc<RwLock<VecDeque<StateDeltaLog>>>,
    sequence: Arc<RwLock<u64>>,
    max_history: usize,
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
                    value: value.value,
                    updated_at: value.updated_at,
                    source: value.source.clone(),
                });
            }
        }

        rows
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
            .unwrap_or("cockpit")
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
            value: current.value,
            updated_at: current.updated_at,
            source: current.source.clone(),
        };

        Ok(ManualPatchResult { applied: log, row })
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
