use std::sync::Arc;
use std::net::SocketAddr;
use async_trait::async_trait;
use futures_util::{sink::SinkExt, stream::StreamExt};
use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

use pa_core::{
    event::{Event, Platform, RawEvent},
    worker::{Worker, WorkerContext, WorkerStatus},
};
use crate::buffer::SensoryBuffer;

// ─── JSON Payload Contracts ──────────────────────────────────────

#[derive(Debug, Deserialize)]
struct WsIncomingPayload {
    #[serde(rename = "type")]
    payload_type: String, // "message"
    data: WsIncomingData,
}

#[derive(Debug, Deserialize)]
struct WsIncomingData {
    channel_id: String,
    message_id: String,
    user_id: String,
    username: String,
    content: String,
    is_mention: bool,
    is_dm: bool,
}

#[derive(Debug, Serialize)]
struct WsOutgoingPayload {
    #[serde(rename = "type")]
    payload_type: String, // "response"
    data: WsOutgoingData, // Wait, maybe it's better to structure this like the response event
}

#[derive(Debug, Serialize)]
struct WsOutgoingData {
    channel_id: String,
    content: String,
    reply_to_message_id: Option<String>,
    is_typing: bool,
}

// ─── Worker Definition ──────────────────────────────────────────

pub struct SelfbotWsWorker {
    port: u16,
    status: WorkerStatus,
    child_process: Option<tokio::process::Child>,
}

impl SelfbotWsWorker {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            status: WorkerStatus::NotStarted,
            child_process: None,
        }
    }
}

#[async_trait]
impl Worker for SelfbotWsWorker {
    fn name(&self) -> &str {
        "discord_selfbot_ws"
    }

    async fn start(&mut self, ctx: WorkerContext) -> anyhow::Result<()> {
        info!("Discord Selfbot WS worker starting on port {}...", self.port);
        self.status = WorkerStatus::Healthy;

        let node_script_path = "crates/pa-sensory/src/discord/nodejs-selfbot/index.js";
        match tokio::process::Command::new("node")
            .arg(node_script_path)
            .kill_on_drop(true)
            .spawn()
        {
            Ok(child) => {
                info!("Spawned Node.js selfbot process (PID: {:?})", child.id());
                self.child_process = Some(child);
            }
            Err(e) => {
                warn!(error = %e, "Failed to spawn Node.js selfbot process. You must run it manually or verify Node.js is installed.");
            }
        }

        let buffer = Arc::new(SensoryBuffer::new(ctx.event_tx.clone()));

        // Setup TCP Listener
        let addr = format!("127.0.0.1:{}", self.port);
        let listener = TcpListener::bind(&addr).await?;
        info!("WebSocket server listening on {}", addr);

        // Shared outgoing sender channel for the active connection
        let (bus_tx, mut _bus_rx) = tokio::sync::mpsc::channel::<Message>(100);
        let active_ws_tx = Arc::new(RwLock::new(Some(bus_tx)));

        let mut broadcast_rx = ctx.subscribe_events();

        // Handle incoming responses from the Event Bus and route to the internal channel
        let ws_tx_clone = Arc::clone(&active_ws_tx);
        tokio::spawn(async move {
            loop {
                match broadcast_rx.recv().await {
                    Ok(Event::Response(response)) => {
                        if response.platform == Platform::DiscordSelfbot {
                            debug!(
                                channel = %response.channel_id,
                                content_len = response.content.len(),
                                "Discord Selfbot received ResponseEvent"
                            );

                            let out_data = WsOutgoingData {
                                channel_id: response.channel_id,
                                content: response.content,
                                reply_to_message_id: response.reply_to_message_id,
                                is_typing: false, // For now
                            };

                            let payload = WsOutgoingPayload {
                                payload_type: "response".to_string(),
                                data: out_data,
                            };

                            if let Ok(json_str) = serde_json::to_string(&payload) {
                                let lock = ws_tx_clone.read().await;
                                if let Some(tx) = lock.as_ref() {
                                    let _ = tx.send(Message::Text(json_str.into())).await;
                                }
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(missed = n, "Discord WS broadcast receiver lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        info!("Discord WS broadcast channel closed");
                        break;
                    }
                    _ => {} // Ignore non-Response events
                }
            }
        });

        // Connection acceptor loop
        let buffer_clone = Arc::clone(&buffer);
        
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let buffer_clone2 = Arc::clone(&buffer_clone);
                let active_ws_tx_clone = Arc::clone(&active_ws_tx);
                
                tokio::spawn(async move {
                    handle_connection(stream, buffer_clone2, active_ws_tx_clone).await;
                });
            }
        });

        let mut shutdown_rx = ctx.subscribe_shutdown();
        
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    info!("Discord Selfbot WS worker received shutdown signal");
                    break;
                }
            }
        }

        Ok(())
    }

    async fn stop(&mut self) -> anyhow::Result<()> {
        info!("Discord Selfbot WS worker stopping...");
        
        if let Some(mut child) = self.child_process.take() {
            info!("Killing Node.js selfbot process...");
            let _ = child.kill().await;
            let _ = child.wait().await;
        }

        self.status = WorkerStatus::Stopped;
        Ok(())
    }

    fn health_check(&self) -> WorkerStatus {
        self.status.clone()
    }
}

async fn handle_connection(
    stream: TcpStream,
    buffer: Arc<SensoryBuffer>,
    active_ws_tx: Arc<RwLock<Option<tokio::sync::mpsc::Sender<Message>>>>,
) {
    // Tối ưu hoá cực mạnh: Tắt Nagle's algorithm (TCP_NODELAY) để giảm độ trễ gửi gói tin nhỏ xuống mức 0
    if let Err(e) = stream.set_nodelay(true) {
        warn!("Failed to set TCP_NODELAY: {}", e);
    }

    let ws_stream = match accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            error!(error = %e, "Error during websocket handshake");
            return;
        }
    };
    info!("New WebSocket client connected");

    let (mut write, mut read) = ws_stream.split();

    // Create a local channel to send messages to this connection
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Message>(100);
    
    // Register our sender in the global state
    {
        let mut lock = active_ws_tx.write().await;
        *lock = Some(tx);
    }
    
    // Spawn a task to forward messages from the global state to our websocket writer
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Err(e) = write.send(msg).await {
                error!(error = %e, "Failed to send WS message");
                break;
            }
        }
    });

    // The read loop
    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                if let Ok(payload) = serde_json::from_str::<WsIncomingPayload>(&text) {
                    if payload.payload_type == "message" {
                        let raw = RawEvent {
                            platform: Platform::DiscordSelfbot,
                            channel_id: payload.data.channel_id,
                            message_id: payload.data.message_id,
                            user_id: payload.data.user_id,
                            username: payload.data.username,
                            content: payload.data.content,
                            is_mention: payload.data.is_mention,
                            is_dm: payload.data.is_dm,
                            timestamp: chrono::Utc::now(),
                        };
                        buffer.push(raw).await;
                    }
                } else {
                    warn!("Failed to parse incoming WS JSON: {}", text);
                }
            }
            Ok(Message::Close(_)) => {
                info!("WebSocket client disconnected");
                break;
            }
            Err(e) => {
                error!(error = %e, "WebSocket error");
                break;
            }
            _ => {}
        }
    }

    // Cleanup on disconnect
    {
        let mut lock = active_ws_tx.write().await;
        *lock = None;
    }
}
