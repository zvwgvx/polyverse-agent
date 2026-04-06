//! UDS relay server — runs inside the agent process.
//!
//! Accepts connections from platform processes, forwards inbound
//! `PlatformMessage::Ingest` events onto the agent EventBus, and
//! sends back `AgentMessage::Response` frames for any `Event::Response`
//! that matches the platform served by that connection.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use kernel::event::{Event, Platform};
use kernel::worker::{Worker, WorkerContext, WorkerStatus};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{debug, error, info, warn};

use super::protocol::{recv_message, send_message, AgentMessage, PlatformMessage};

pub struct PlatformRelayWorker {
    socket_path: String,
    status: WorkerStatus,
}

impl PlatformRelayWorker {
    pub fn new(socket_path: impl Into<String>) -> Self {
        Self {
            socket_path: socket_path.into(),
            status: WorkerStatus::NotStarted,
        }
    }

    pub fn from_env() -> Self {
        Self::new(super::protocol::resolve_socket_path())
    }
}

#[async_trait]
impl Worker for PlatformRelayWorker {
    fn name(&self) -> &str {
        "platform_relay"
    }

    async fn start(&mut self, ctx: WorkerContext) -> Result<()> {
        // Remove stale socket file from a previous run.
        let _ = std::fs::remove_file(&self.socket_path);

        let listener = UnixListener::bind(&self.socket_path)?;
        info!(socket = %self.socket_path, "Platform relay UDS server listening");

        self.status = WorkerStatus::Healthy;

        let event_tx = ctx.event_tx.clone();
        let broadcast_tx = ctx.broadcast_rx.clone();
        let mut shutdown_rx = ctx.subscribe_shutdown();

        loop {
            tokio::select! {
                accept = listener.accept() => {
                    match accept {
                        Ok((stream, _)) => {
                            let tx = event_tx.clone();
                            let bcast = broadcast_tx.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_connection(stream, tx, bcast).await {
                                    debug!(error = %e, "Platform relay connection closed");
                                }
                            });
                        }
                        Err(e) => {
                            error!(error = %e, "Platform relay accept error");
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Platform relay worker received shutdown signal");
                    break;
                }
            }
        }

        let _ = std::fs::remove_file(&self.socket_path);
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

async fn handle_connection(
    stream: UnixStream,
    event_tx: mpsc::Sender<Event>,
    broadcast_tx: broadcast::Sender<Event>,
) -> Result<()> {
    let (read_half, write_half) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(read_half);
    let write_half = Arc::new(RwLock::new(write_half));

    // Detect which platform this connection serves from the first message,
    // then subscribe to responses for that platform only.
    let mut detected_platform: Option<Platform> = None;

    // Spawn response forwarder — waits until platform is detected.
    let (platform_tx, mut platform_rx) = mpsc::channel::<Platform>(1);
    let write_clone = Arc::clone(&write_half);
    let mut bcast_rx = broadcast_tx.subscribe();

    tokio::spawn(async move {
        // Wait for the platform to be detected from the ingest loop.
        let platform = match platform_rx.recv().await {
            Some(p) => p,
            None => return,
        };

        loop {
            match bcast_rx.recv().await {
                Ok(Event::Response(resp)) if resp.platform == platform => {
                    let msg = AgentMessage::Response { event: resp };
                    let mut w = write_clone.write().await;
                    if let Err(e) = send_message(&mut *w, &msg).await {
                        debug!(error = %e, "Platform relay: failed to forward response");
                        break;
                    }
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!(missed = n, "Platform relay broadcast receiver lagged");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Ingest loop — read frames from the platform process.
    loop {
        match recv_message::<_, PlatformMessage>(&mut reader).await? {
            None => {
                debug!("Platform relay connection EOF");
                break;
            }
            Some(PlatformMessage::Ping) => {
                let mut w = write_half.write().await;
                send_message(&mut *w, &AgentMessage::Pong).await?;
            }
            Some(PlatformMessage::Ingest { event }) => {
                let platform = event.platform;

                // Detect platform from first ingest and notify the response forwarder.
                if detected_platform.is_none() {
                    detected_platform = Some(platform);
                    let _ = platform_tx.send(platform).await;
                    info!(
                        platform = %platform,
                        "Platform relay: new connection registered"
                    );
                }

                debug!(
                    platform = %platform,
                    user = %event.username,
                    content_len = event.content.len(),
                    "Platform relay: ingest event"
                );

                let _ = event_tx.send(Event::Raw(event)).await;

                // Acknowledge receipt.
                let mut w = write_half.write().await;
                send_message(&mut *w, &AgentMessage::Ack).await?;
            }
        }
    }

    Ok(())
}
