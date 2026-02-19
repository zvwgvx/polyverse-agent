use std::collections::HashMap;

use anyhow::Result;
use pa_core::event::{Event, SystemEvent};
use pa_core::worker::Worker;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::event_bus::EventBus;

/// The Supervisor manages the lifecycle of all workers in the system.
///
/// Responsibilities:
/// - Register workers before startup
/// - Start all workers concurrently
/// - Monitor worker health
/// - Graceful shutdown of all workers
pub struct Supervisor {
    /// Registered workers (not yet started).
    workers: Vec<Box<dyn Worker>>,

    /// Running worker handles, keyed by worker name.
    handles: HashMap<String, JoinHandle<()>>,

    /// The shared event bus.
    event_bus: EventBus,
}

impl Supervisor {
    /// Create a new Supervisor with a fresh event bus.
    pub fn new() -> Self {
        Self {
            workers: Vec::new(),
            handles: HashMap::new(),
            event_bus: EventBus::new(),
        }
    }

    /// Create a new Supervisor with a custom event bus.
    pub fn with_event_bus(event_bus: EventBus) -> Self {
        Self {
            workers: Vec::new(),
            handles: HashMap::new(),
            event_bus,
        }
    }

    /// Get a reference to the event bus (for coordinator to use).
    pub fn event_bus(&self) -> &EventBus {
        &self.event_bus
    }

    /// Get a mutable reference to the event bus.
    pub fn event_bus_mut(&mut self) -> &mut EventBus {
        &mut self.event_bus
    }

    /// Register a worker with the supervisor.
    /// Workers must be registered before `start_all()` is called.
    pub fn register<W: Worker>(&mut self, worker: W) {
        info!(worker = worker.name(), "Registering worker");
        self.workers.push(Box::new(worker));
    }

    /// Start all registered workers.
    /// Each worker is spawned as an independent tokio task.
    pub async fn start_all(&mut self) -> Result<()> {
        info!(
            count = self.workers.len(),
            "Starting all registered workers"
        );

        // Drain workers out so we can move them into tasks
        let workers = std::mem::take(&mut self.workers);

        for mut worker in workers {
            let name = worker.name().to_string();
            let ctx = self.event_bus.worker_context();

            // Emit system event
            let start_event = Event::System(SystemEvent::WorkerStarted {
                name: name.clone(),
            });
            let _ = self.event_bus.event_tx.send(start_event).await;

            let task_name = name.clone();
            let handle = tokio::spawn(async move {
                info!(worker = %task_name, "Worker task starting");
                if let Err(e) = worker.start(ctx).await {
                    error!(worker = %task_name, error = %e, "Worker exited with error");
                } else {
                    info!(worker = %task_name, "Worker exited gracefully");
                }
            });

            self.handles.insert(name, handle);
        }

        info!("All workers started");
        Ok(())
    }

    /// Gracefully shut down all workers.
    /// Sends shutdown signal and waits for all tasks to complete.
    pub async fn shutdown(&mut self) -> Result<()> {
        info!("Initiating graceful shutdown...");

        // Send shutdown signal to all workers
        self.event_bus.signal_shutdown();

        // Wait for all worker tasks to finish
        let handles = std::mem::take(&mut self.handles);
        for (name, handle) in handles {
            info!(worker = %name, "Waiting for worker to stop...");
            match tokio::time::timeout(std::time::Duration::from_secs(10), handle).await {
                Ok(Ok(())) => info!(worker = %name, "Worker stopped"),
                Ok(Err(e)) => error!(worker = %name, error = %e, "Worker task panicked"),
                Err(_) => warn!(worker = %name, "Worker did not stop within timeout, aborting"),
            }
        }

        info!("All workers stopped. Shutdown complete.");
        Ok(())
    }

    /// Get the number of registered/running workers.
    pub fn worker_count(&self) -> usize {
        self.workers.len() + self.handles.len()
    }

    /// Check if all running workers are still alive.
    pub fn all_healthy(&self) -> bool {
        self.handles.values().all(|h| !h.is_finished())
    }
}

impl Default for Supervisor {
    fn default() -> Self {
        Self::new()
    }
}
