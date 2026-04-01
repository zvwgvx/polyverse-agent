use kernel::event::Event;
use tokio::sync::{broadcast, mpsc};

pub struct EventBusConfig {
    pub mpsc_capacity: usize,
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

pub struct EventBus {
    pub event_tx: mpsc::Sender<Event>,

    event_rx: Option<mpsc::Receiver<Event>>,

    pub broadcast_tx: broadcast::Sender<Event>,

    pub shutdown_tx: broadcast::Sender<()>,
}

impl EventBus {
    pub fn new() -> Self {
        Self::with_config(EventBusConfig::default())
    }

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

    pub fn take_event_rx(&mut self) -> Option<mpsc::Receiver<Event>> {
        self.event_rx.take()
    }

    pub fn worker_context(&self) -> kernel::WorkerContext {
        kernel::WorkerContext {
            event_tx: self.event_tx.clone(),
            broadcast_rx: self.broadcast_tx.clone(),
            shutdown: self.shutdown_tx.clone(),
        }
    }

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
    use kernel::event::{Platform, RawEvent, SystemEvent};

    #[tokio::test]
    async fn test_event_bus_mpsc() {
        let mut bus = EventBus::new();
        let ctx = bus.worker_context();

        let event = Event::System(SystemEvent::WorkerStarted {
            name: "test".to_string(),
        });
        ctx.emit(event).await.unwrap();

        let mut rx = bus.take_event_rx().unwrap();

        drop(ctx);
        drop(bus.event_tx);

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

        let event = Event::Raw(RawEvent {
            platform: Platform::Discord,
            channel_id: "ch1".to_string(),
            message_id: "m1".to_string(),
            user_id: "u1".to_string(),
            username: "user".to_string(),
            content: "hello".to_string(),
            is_mention: false,
            is_dm: false,
            timestamp: chrono::Utc::now(),
        });

        bus.broadcast_tx.send(event).unwrap();

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
