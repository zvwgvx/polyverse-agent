use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use kernel::event::{BiologyEvent, BiologyEventKind, Event, Platform, RawEvent, SystemEvent};
use kernel::worker::{Worker, WorkerContext, WorkerStatus};
use runtime::{Coordinator, EventBus, Supervisor};
use tokio::sync::oneshot;
use tokio::time::{timeout, Duration};

struct WaitingWorker {
    name: &'static str,
    started_tx: Option<oneshot::Sender<()>>,
    stopped_tx: Option<oneshot::Sender<()>>,
}

impl WaitingWorker {
    fn new(
        name: &'static str,
        started_tx: oneshot::Sender<()>,
        stopped_tx: oneshot::Sender<()>,
    ) -> Self {
        Self {
            name,
            started_tx: Some(started_tx),
            stopped_tx: Some(stopped_tx),
        }
    }
}

#[async_trait]
impl Worker for WaitingWorker {
    fn name(&self) -> &str {
        self.name
    }

    async fn start(&mut self, ctx: WorkerContext) -> Result<()> {
        if let Some(started_tx) = self.started_tx.take() {
            let _ = started_tx.send(());
        }

        let mut shutdown_rx = ctx.subscribe_shutdown();
        let _ = shutdown_rx.recv().await;

        if let Some(stopped_tx) = self.stopped_tx.take() {
            let _ = stopped_tx.send(());
        }

        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        Ok(())
    }

    fn health_check(&self) -> WorkerStatus {
        WorkerStatus::Healthy
    }
}

fn raw_event(content: &str) -> Event {
    Event::Raw(RawEvent {
        platform: Platform::Cli,
        channel_id: "cli".to_string(),
        message_id: "m1".to_string(),
        user_id: "u1".to_string(),
        username: "tester".to_string(),
        content: content.to_string(),
        attachments: vec![],
        is_mention: false,
        is_dm: true,
        timestamp: Utc::now(),
    })
}

#[tokio::test]
async fn supervisor_emits_worker_started_and_shutdowns_worker() -> Result<()> {
    let (started_tx, started_rx) = oneshot::channel();
    let (stopped_tx, stopped_rx) = oneshot::channel();

    let mut supervisor = Supervisor::new();
    let mut event_rx = supervisor
        .event_bus_mut()
        .take_event_rx()
        .expect("event receiver should exist");

    supervisor.register(WaitingWorker::new("observer", started_tx, stopped_tx));
    assert_eq!(supervisor.worker_count(), 1);

    supervisor.start_all().await?;

    timeout(Duration::from_secs(1), started_rx)
        .await
        .expect("worker should start in time")
        .expect("worker should report start");

    let event = timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .expect("worker started event should arrive in time")
        .expect("worker started event should exist");

    match event {
        Event::System(SystemEvent::WorkerStarted { name }) => assert_eq!(name, "observer"),
        other => panic!("expected worker started system event, got {other:?}"),
    }

    assert_eq!(supervisor.worker_count(), 1);
    assert!(supervisor.all_healthy());

    supervisor.shutdown().await?;

    timeout(Duration::from_secs(1), stopped_rx)
        .await
        .expect("worker should stop in time")
        .expect("worker should report stop");

    assert_eq!(supervisor.worker_count(), 0);
    assert!(supervisor.all_healthy());
    Ok(())
}

#[tokio::test]
async fn coordinator_broadcasts_raw_events_until_shutdown() -> Result<()> {
    let mut bus = EventBus::new();
    let event_rx = bus.take_event_rx().expect("event receiver should exist");
    let mut broadcast_rx = bus.worker_context().subscribe_events();
    let shutdown_rx = bus.worker_context().subscribe_shutdown();

    let mut coordinator = Coordinator::new(bus.broadcast_tx.clone());
    let handle = tokio::spawn(async move { coordinator.run(event_rx, shutdown_rx).await });

    bus.event_tx.send(raw_event("hello runtime")).await?;

    let received = timeout(Duration::from_secs(1), broadcast_rx.recv())
        .await
        .expect("broadcast should arrive in time")
        .expect("broadcast channel should remain open");

    match received {
        Event::Raw(raw) => {
            assert_eq!(raw.platform, Platform::Cli);
            assert_eq!(raw.username, "tester");
            assert_eq!(raw.content, "hello runtime");
        }
        other => panic!("expected raw event broadcast, got {other:?}"),
    }

    bus.signal_shutdown();
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("coordinator should stop in time")??;

    Ok(())
}

#[tokio::test]
async fn coordinator_applies_biology_events_before_shutdown() -> Result<()> {
    let mut bus = EventBus::new();
    let event_rx = bus.take_event_rx().expect("event receiver should exist");
    let shutdown_rx = bus.worker_context().subscribe_shutdown();

    let mut coordinator = Coordinator::new(bus.broadcast_tx.clone());
    let biology = coordinator.biology_state();
    let handle = tokio::spawn(async move { coordinator.run(event_rx, shutdown_rx).await });

    bus.event_tx
        .send(Event::Biology(BiologyEvent {
            kind: BiologyEventKind::EnergyChanged {
                delta: -25.0,
                reason: "load".to_string(),
            },
            timestamp: Utc::now(),
        }))
        .await?;
    bus.event_tx
        .send(Event::Biology(BiologyEvent {
            kind: BiologyEventKind::SleepStarted,
            timestamp: Utc::now(),
        }))
        .await?;
    bus.event_tx
        .send(Event::Biology(BiologyEvent {
            kind: BiologyEventKind::SleepEnded,
            timestamp: Utc::now(),
        }))
        .await?;
    bus.event_tx
        .send(Event::Biology(BiologyEvent {
            kind: BiologyEventKind::EnergyChanged {
                delta: 10.0,
                reason: "rest".to_string(),
            },
            timestamp: Utc::now(),
        }))
        .await?;

    timeout(Duration::from_secs(1), async {
        loop {
            let bio = biology.read().await;
            if (bio.energy - 85.0).abs() < 1e-6 && !bio.is_sleeping {
                break;
            }
            drop(bio);
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("biology updates should settle in time");

    bus.signal_shutdown();
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("coordinator should stop in time")??;

    let bio = biology.read().await;
    assert!((bio.energy - 85.0).abs() < 1e-6);
    assert!(!bio.is_sleeping);
    Ok(())
}
