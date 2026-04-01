use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use kernel::biology::BiologyState;
use kernel::event::{
    BiologyEvent, BiologyEventKind, BotTurnCompletion, Event, Platform, RawEvent, ResponseEvent,
    ResponseSource, SystemEvent,
};
use kernel::worker::{Worker, WorkerStatus};
use runtime::{Coordinator, EventBus};
use test_support::in_memory_graph;
use tokio::sync::{broadcast, RwLock};
use tokio::time::{timeout, Duration};

fn raw_event(content: &str) -> Event {
    Event::Raw(RawEvent {
        platform: Platform::Cli,
        channel_id: "cli".to_string(),
        message_id: "m1".to_string(),
        user_id: "u1".to_string(),
        username: "tester".to_string(),
        content: content.to_string(),
        is_mention: false,
        is_dm: true,
        timestamp: Utc::now(),
    })
}

fn response_event(content: &str) -> Event {
    Event::Response(ResponseEvent {
        platform: Platform::Cli,
        channel_id: "cli".to_string(),
        reply_to_message_id: Some("m1".to_string()),
        reply_to_user: Some("tester".to_string()),
        is_dm: true,
        content: content.to_string(),
        source: ResponseSource::CloudLLM,
    })
}

fn completion_event(content: &str) -> Event {
    Event::BotTurnCompletion(BotTurnCompletion {
        platform: Platform::Cli,
        channel_id: "cli".to_string(),
        reply_to_message_id: Some("m1".to_string()),
        reply_to_user: Some("tester".to_string()),
        content: content.to_string(),
    })
}

fn biology_event(delta: f32) -> Event {
    Event::Biology(BiologyEvent {
        kind: BiologyEventKind::EnergyChanged {
            delta,
            reason: "integration-test".to_string(),
        },
        timestamp: Utc::now(),
    })
}

fn sleep_event(sleeping: bool) -> Event {
    Event::Biology(BiologyEvent {
        kind: if sleeping {
            BiologyEventKind::SleepStarted
        } else {
            BiologyEventKind::SleepEnded
        },
        timestamp: Utc::now(),
    })
}

async fn spawn_coordinator(
    bus: &mut EventBus,
) -> Result<(
    tokio::task::JoinHandle<Result<()>>,
    broadcast::Receiver<Event>,
    Arc<RwLock<BiologyState>>,
)> {
    let event_rx = bus.take_event_rx().expect("event receiver should exist");
    let broadcast_rx = bus.worker_context().subscribe_events();
    let shutdown_rx = bus.worker_context().subscribe_shutdown();

    let mut coordinator = Coordinator::new(bus.broadcast_tx.clone());
    let biology = coordinator.biology_state();
    let handle = tokio::spawn(async move { coordinator.run(event_rx, shutdown_rx).await });
    Ok((handle, broadcast_rx, biology))
}

async fn shutdown_coordinator(
    bus: &EventBus,
    handle: tokio::task::JoinHandle<Result<()>>,
) -> Result<()> {
    bus.signal_shutdown();
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("coordinator should stop in time")??;
    Ok(())
}

async fn wait_for_broadcast(mut rx: broadcast::Receiver<Event>) -> Event {
    timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("broadcast should arrive in time")
        .expect("broadcast channel should remain open")
}

async fn wait_for_biology(
    biology: &Arc<RwLock<BiologyState>>,
    expected_energy: f32,
    expected_sleeping: bool,
) {
    timeout(Duration::from_secs(1), async {
        loop {
            let bio = biology.read().await;
            if (bio.energy - expected_energy).abs() < 1e-6 && bio.is_sleeping == expected_sleeping {
                break;
            }
            drop(bio);
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("biology updates should settle in time");
}

#[tokio::test]
async fn event_bus_context_can_start_and_shutdown_mcp_worker() -> Result<()> {
    let graph = in_memory_graph().await;

    let probe_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let bind_addr = probe_listener.local_addr()?;
    drop(probe_listener);

    let mut worker = mcp::McpWorker::new(
        mcp::McpConfig {
            enabled: true,
            bind_addr: bind_addr.to_string(),
            ..mcp::McpConfig::default()
        },
        graph,
    );
    let bus = EventBus::new();
    let ctx = bus.worker_context();

    let handle = tokio::spawn(async move {
        worker.start(ctx).await.expect("mcp worker should run");
        worker
    });

    timeout(Duration::from_secs(2), async {
        loop {
            if tokio::net::TcpStream::connect(bind_addr).await.is_ok() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("mcp worker should bind in time");

    bus.signal_shutdown();

    let worker = timeout(Duration::from_secs(3), handle)
        .await
        .expect("mcp worker should stop in time")
        .expect("mcp worker task should finish");

    assert_eq!(worker.health_check(), WorkerStatus::Stopped);
    Ok(())
}

#[tokio::test]
async fn coordinator_broadcasts_raw_events() -> Result<()> {
    let mut bus = EventBus::new();
    let (handle, broadcast_rx, _biology) = spawn_coordinator(&mut bus).await?;

    bus.event_tx.send(raw_event("hello runtime")).await?;

    let event = wait_for_broadcast(broadcast_rx).await;
    match event {
        Event::Raw(raw) => {
            assert_eq!(raw.platform, Platform::Cli);
            assert_eq!(raw.username, "tester");
            assert_eq!(raw.content, "hello runtime");
        }
        other => panic!("expected raw event broadcast, got {other:?}"),
    }

    shutdown_coordinator(&bus, handle).await?;
    Ok(())
}

#[tokio::test]
async fn coordinator_broadcasts_response_events() -> Result<()> {
    let mut bus = EventBus::new();
    let (handle, broadcast_rx, _biology) = spawn_coordinator(&mut bus).await?;

    bus.event_tx.send(response_event("hello reply")).await?;

    let event = wait_for_broadcast(broadcast_rx).await;
    match event {
        Event::Response(response) => {
            assert_eq!(response.platform, Platform::Cli);
            assert_eq!(response.channel_id, "cli");
            assert_eq!(response.reply_to_user.as_deref(), Some("tester"));
            assert_eq!(response.content, "hello reply");
            assert_eq!(response.source, ResponseSource::CloudLLM);
        }
        other => panic!("expected response event broadcast, got {other:?}"),
    }

    shutdown_coordinator(&bus, handle).await?;
    Ok(())
}

#[tokio::test]
async fn coordinator_broadcasts_turn_completion_events() -> Result<()> {
    let mut bus = EventBus::new();
    let (handle, broadcast_rx, _biology) = spawn_coordinator(&mut bus).await?;

    bus.event_tx.send(completion_event("done")).await?;

    let event = wait_for_broadcast(broadcast_rx).await;
    match event {
        Event::BotTurnCompletion(done) => {
            assert_eq!(done.platform, Platform::Cli);
            assert_eq!(done.channel_id, "cli");
            assert_eq!(done.reply_to_user.as_deref(), Some("tester"));
            assert_eq!(done.content, "done");
        }
        other => panic!("expected bot completion broadcast, got {other:?}"),
    }

    shutdown_coordinator(&bus, handle).await?;
    Ok(())
}

#[tokio::test]
async fn coordinator_does_not_broadcast_system_events() -> Result<()> {
    let mut bus = EventBus::new();
    let (handle, mut broadcast_rx, _biology) = spawn_coordinator(&mut bus).await?;

    bus.event_tx
        .send(Event::System(SystemEvent::ShutdownRequested))
        .await?;

    let recv_result = timeout(Duration::from_millis(300), broadcast_rx.recv()).await;
    assert!(recv_result.is_err(), "system events should not be rebroadcast");

    shutdown_coordinator(&bus, handle).await?;
    Ok(())
}

#[tokio::test]
async fn coordinator_applies_biology_events_before_shutdown() -> Result<()> {
    let mut bus = EventBus::new();
    let (handle, _broadcast_rx, biology) = spawn_coordinator(&mut bus).await?;

    bus.event_tx.send(biology_event(-25.0)).await?;
    bus.event_tx.send(sleep_event(true)).await?;
    bus.event_tx.send(sleep_event(false)).await?;
    bus.event_tx.send(biology_event(10.0)).await?;

    wait_for_biology(&biology, 85.0, false).await;
    shutdown_coordinator(&bus, handle).await?;

    let bio = biology.read().await;
    assert!((bio.energy - 85.0).abs() < 1e-6);
    assert!(!bio.is_sleeping);
    Ok(())
}
