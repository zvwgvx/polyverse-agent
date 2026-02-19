use pa_core::event::Event;
use tokio::sync::{broadcast, mpsc};

/// Configuration for the event bus channel sizes.
pub struct EventBusConfig {
    /// Capacity of the mpsc channel (workers -> coordinator).
    pub mpsc_capacity: usize,
    /// Capacity of the broadcast channel (coordinator -> workers).
    pub broadcast_capacity: usize,
}

impl Default for EventBusConfig {
    fn default() -> Self {
        Self {
            mpsc_capacity: 256,
            broadcast_capacity: 128,
        }
    }
}

/// The central event bus for the Polyverse Agent.
///
/// Architecture:
/// - Workers send events INTO the bus via `mpsc` (many-to-one).
/// - The coordinator reads from the mpsc receiver, processes events,
///   and may broadcast events out to all workers via `broadcast` (one-to-many).
///
/// ```text
///  Worker A ──┐
///  Worker B ──┤──► [mpsc] ──► Coordinator ──► [broadcast] ──► All Workers
///  Worker C ──┘
/// ```
pub struct EventBus {
    /// Sender side of the mpsc channel. Cloned and given to each worker.
    pub event_tx: mpsc::Sender<Event>,

    /// Receiver side of the mpsc channel. Owned by the coordinator.
    /// Use `take_event_rx()` to move this out for the coordinator.
    event_rx: Option<mpsc::Receiver<Event>>,

    /// Sender side of the broadcast channel.
    /// Workers subscribe via `broadcast_tx.subscribe()`.
    pub broadcast_tx: broadcast::Sender<Event>,

    /// Shutdown signal broadcaster.
    pub shutdown_tx: broadcast::Sender<()>,
}

impl EventBus {
    /// Create a new EventBus with default configuration.
    pub fn new() -> Self {
        Self::with_config(EventBusConfig::default())
    }

    /// Create a new EventBus with custom configuration.
    pub fn with_config(config: EventBusConfig) -> Self {
        let (event_tx, event_rx) = mpsc::channel(config.mpsc_capacity);
        let (broadcast_tx, _) = broadcast::channel(config.broadcast_capacity);
        let (shutdown_tx, _) = broadcast::channel(1);

        Self {
            event_tx,
            event_rx: Some(event_rx),
            broadcast_tx,
            shutdown_tx,
        }
    }

    /// Take the event receiver out of the bus for the coordinator.
    /// This can only be called once — subsequent calls return None.
    pub fn take_event_rx(&mut self) -> Option<mpsc::Receiver<Event>> {
        self.event_rx.take()
    }

    /// Create a WorkerContext for a worker to use.
    /// Each worker gets its own context with cloned senders.
    pub fn worker_context(&self) -> pa_core::WorkerContext {
        pa_core::WorkerContext {
            event_tx: self.event_tx.clone(),
            broadcast_rx: self.broadcast_tx.clone(),
            shutdown: self.shutdown_tx.clone(),
        }
    }

    /// Signal all workers to shut down.
    pub fn signal_shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pa_core::event::{Platform, RawEvent, SystemEvent};

    #[tokio::test]
    async fn test_event_bus_mpsc() {
        let mut bus = EventBus::new();
        let ctx = bus.worker_context();

        // Worker sends an event
        let event = Event::System(SystemEvent::WorkerStarted {
            name: "test".to_string(),
        });
        ctx.emit(event).await.unwrap();

        // Take the receiver for the coordinator
        let mut rx = bus.take_event_rx().unwrap();

        // Drop the sender side so recv completes
        drop(ctx);
        drop(bus.event_tx);

        // Coordinator receives
        let received = rx.recv().await.unwrap();
        assert!(received.is_system());
    }

    #[tokio::test]
    async fn test_event_bus_broadcast() {
        let bus = EventBus::new();
        let ctx1 = bus.worker_context();
        let ctx2 = bus.worker_context();

        let mut rx1 = ctx1.subscribe_events();
        let mut rx2 = ctx2.subscribe_events();

        // Coordinator broadcasts an event
        let event = Event::Raw(RawEvent {
            platform: Platform::Discord,
            channel_id: "ch1".to_string(),
            message_id: "m1".to_string(),
            user_id: "u1".to_string(),
            username: "user".to_string(),
            content: "hello".to_string(),
            is_mention: false,
            timestamp: chrono::Utc::now(),
        });

        bus.broadcast_tx.send(event).unwrap();

        // Both workers receive
        let e1 = rx1.recv().await.unwrap();
        let e2 = rx2.recv().await.unwrap();
        assert!(e1.is_raw());
        assert!(e2.is_raw());
    }

    #[tokio::test]
    async fn test_shutdown_signal() {
        let bus = EventBus::new();
        let ctx = bus.worker_context();
        let mut shutdown_rx = ctx.subscribe_shutdown();

        bus.signal_shutdown();

        let result = shutdown_rx.recv().await;
        assert!(result.is_ok());
    }
}
