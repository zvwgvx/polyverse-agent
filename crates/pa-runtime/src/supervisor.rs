use std::collections::HashMap;

use anyhow::Result;
use pa_core::event::{Event, SystemEvent};
use pa_core::worker::Worker;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::event_bus::EventBus;

pub struct Supervisor {
    workers: Vec<Box<dyn Worker>>,

    handles: HashMap<String, JoinHandle<()>>,

    event_bus: EventBus,
}

impl Supervisor {
    pub fn new() -> Self {
        Self {
            workers: Vec::new(),
            handles: HashMap::new(),
            event_bus: EventBus::new(),
        }
    }

    pub fn with_event_bus(event_bus: EventBus) -> Self {
        Self {
            workers: Vec::new(),
            handles: HashMap::new(),
            event_bus,
        }
    }

    pub fn event_bus(&self) -> &EventBus {
        &self.event_bus
    }

    pub fn event_bus_mut(&mut self) -> &mut EventBus {
        &mut self.event_bus
    }

    pub fn register<W: Worker>(&mut self, worker: W) {
        info!(worker = worker.name(), "Registering worker");
        self.workers.push(Box::new(worker));
    }

    pub async fn start_all(&mut self) -> Result<()> {
        info!(
            count = self.workers.len(),
            "Starting all registered workers"
        );

        let workers = std::mem::take(&mut self.workers);

        for mut worker in workers {
            let name = worker.name().to_string();
            let ctx = self.event_bus.worker_context();

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

    pub async fn shutdown(&mut self) -> Result<()> {
        info!("Initiating graceful shutdown...");

        self.event_bus.signal_shutdown();

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

    pub fn worker_count(&self) -> usize {
        self.workers.len() + self.handles.len()
    }

    pub fn all_healthy(&self) -> bool {
        self.handles.values().all(|h| !h.is_finished())
    }
}

impl Default for Supervisor {
    fn default() -> Self {
        Self::new()
    }
}
