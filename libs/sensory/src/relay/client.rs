//! UDS relay client — used by platform processes to communicate with the agent.
//!
//! Usage:
//! ```ignore
//! let mut client = RelayClient::connect("/tmp/polyverse-agent-relay.sock").await?;
//! client.ingest(raw_event).await?;
//!
//! // In a separate task:
//! while let Some(response) = client.recv_response().await? {
//!     // send response back to the platform
//! }
//! ```

use anyhow::{Context, Result};
use kernel::event::{RawEvent, ResponseEvent};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use super::protocol::{recv_message, send_message, AgentMessage, PlatformMessage};

pub struct RelayClient {
    /// Sender for outbound events (ingest).
    ingest_tx: mpsc::Sender<RawEvent>,
    /// Receiver for inbound responses from the agent.
    response_rx: mpsc::Receiver<ResponseEvent>,
}

impl RelayClient {
    /// Connect to the agent relay socket and start background I/O tasks.
    pub async fn connect(socket_path: &str) -> Result<Self> {
        let stream = UnixStream::connect(socket_path)
            .await
            .with_context(|| format!("Failed to connect to relay socket: {}", socket_path))?;

        let (read_half, write_half) = stream.into_split();
        let mut reader = tokio::io::BufReader::new(read_half);
        let mut writer = write_half;

        let (ingest_tx, mut ingest_rx) = mpsc::channel::<RawEvent>(64);
        let (response_tx, response_rx) = mpsc::channel::<ResponseEvent>(64);

        // Writer task: drain ingest_rx and send frames to agent.
        tokio::spawn(async move {
            while let Some(event) = ingest_rx.recv().await {
                let msg = PlatformMessage::Ingest { event };
                if let Err(e) = send_message(&mut writer, &msg).await {
                    debug!(error = %e, "Relay client: write error");
                    break;
                }
            }
        });

        // Reader task: receive frames from agent and dispatch.
        tokio::spawn(async move {
            loop {
                match recv_message::<_, AgentMessage>(&mut reader).await {
                    Ok(None) => {
                        debug!("Relay client: connection closed by agent");
                        break;
                    }
                    Ok(Some(AgentMessage::Response { event })) => {
                        if response_tx.send(event).await.is_err() {
                            break;
                        }
                    }
                    Ok(Some(AgentMessage::Ack)) => {
                        debug!("Relay client: ack received");
                    }
                    Ok(Some(AgentMessage::Pong)) => {
                        debug!("Relay client: pong received");
                    }
                    Err(e) => {
                        warn!(error = %e, "Relay client: read error");
                        break;
                    }
                }
            }
        });

        Ok(Self {
            ingest_tx,
            response_rx,
        })
    }

    /// Connect using the env-configured or default socket path.
    pub async fn connect_default() -> Result<Self> {
        Self::connect(&super::protocol::resolve_socket_path()).await
    }

    /// Send a RawEvent to the agent.
    pub async fn ingest(&self, event: RawEvent) -> Result<()> {
        self.ingest_tx
            .send(event)
            .await
            .context("Relay client: ingest channel closed")?;
        Ok(())
    }

    /// Wait for the next response from the agent.
    /// Returns `None` when the connection is closed.
    pub async fn recv_response(&mut self) -> Option<ResponseEvent> {
        self.response_rx.recv().await
    }
}
