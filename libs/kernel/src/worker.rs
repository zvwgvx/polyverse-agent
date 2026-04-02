use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};

use crate::event::Event;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkerStatus {
    Healthy,
    Degraded { reason: String },
    Stopped,
    NotStarted,
}

impl std::fmt::Display for WorkerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkerStatus::Healthy => write!(f, "Healthy"),
            WorkerStatus::Degraded { reason } => write!(f, "Degraded: {reason}"),
            WorkerStatus::Stopped => write!(f, "Stopped"),
            WorkerStatus::NotStarted => write!(f, "NotStarted"),
        }
    }
}

#[derive(Clone)]
pub struct WorkerContext {
    pub event_tx: mpsc::Sender<Event>,

    pub broadcast_rx: broadcast::Sender<Event>,

    pub shutdown: broadcast::Sender<()>,
}

impl WorkerContext {
    pub fn subscribe_events(&self) -> broadcast::Receiver<Event> {
        self.broadcast_rx.subscribe()
    }

    pub fn subscribe_shutdown(&self) -> broadcast::Receiver<()> {
        self.shutdown.subscribe()
    }

    pub async fn emit(&self, event: Event) -> Result<()> {
        self.event_tx
            .send(event)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to emit event: {}", e))
    }
}

#[async_trait]
pub trait Worker: Send + Sync + 'static {
    fn name(&self) -> &str;

    async fn start(&mut self, ctx: WorkerContext) -> Result<()>;

    async fn stop(&mut self) -> Result<()>;

    fn health_check(&self) -> WorkerStatus;
}
