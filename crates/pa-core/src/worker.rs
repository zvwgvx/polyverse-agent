use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};

use crate::event::Event;

/// Status of a worker, reported during health checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkerStatus {
    /// Worker is running normally.
    Healthy,
    /// Worker is running but experiencing issues.
    Degraded { reason: String },
    /// Worker has stopped (either gracefully or due to error).
    Stopped,
    /// Worker has not been started yet.
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

/// The context provided to each worker when it starts.
/// Contains channel handles for communication with the event bus.
#[derive(Clone)]
pub struct WorkerContext {
    /// Send events into the central event bus (many workers -> one bus).
    pub event_tx: mpsc::Sender<Event>,

    /// Receive broadcast events from the bus (one bus -> many workers).
    /// Each worker gets its own receiver via `subscribe()`.
    pub broadcast_rx: broadcast::Sender<Event>,

    /// Sender for shutdown signal. Workers listen on the receiver side.
    pub shutdown: broadcast::Sender<()>,
}

impl WorkerContext {
    /// Subscribe to the broadcast event stream.
    /// Each call creates a new independent receiver.
    pub fn subscribe_events(&self) -> broadcast::Receiver<Event> {
        self.broadcast_rx.subscribe()
    }

    /// Subscribe to the shutdown signal.
    pub fn subscribe_shutdown(&self) -> broadcast::Receiver<()> {
        self.shutdown.subscribe()
    }

    /// Emit an event into the central event bus.
    pub async fn emit(&self, event: Event) -> Result<()> {
        self.event_tx
            .send(event)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to emit event: {}", e))
    }
}

/// The core Worker trait. Every worker in the system (sensory, cognitive,
/// biology, etc.) must implement this trait.
///
/// Workers are autonomous async tasks managed by the Supervisor.
/// They communicate with the rest of the system exclusively through
/// the `WorkerContext` channels.
#[async_trait]
pub trait Worker: Send + Sync + 'static {
    /// A unique human-readable name for this worker (e.g. "discord", "slm", "energy").
    fn name(&self) -> &str;

    /// Start the worker. This should spawn the worker's main loop.
    /// The worker should run until it receives a shutdown signal or encounters
    /// an unrecoverable error.
    ///
    /// The `ctx` provides channels for event emission and reception.
    async fn start(&mut self, ctx: WorkerContext) -> Result<()>;

    /// Gracefully stop the worker. Clean up resources, flush buffers, etc.
    /// This is called by the Supervisor during shutdown.
    async fn stop(&mut self) -> Result<()>;

    /// Report the current health status of this worker.
    fn health_check(&self) -> WorkerStatus;
}
