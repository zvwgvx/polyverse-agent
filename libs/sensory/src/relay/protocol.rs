//! Wire protocol for the UDS platform relay.
//!
//! Each message is a length-prefixed JSON frame:
//!   [u32 LE length][JSON bytes]
//!
//! Platform process → agent: `PlatformMessage::Ingest`
//! Agent → platform process: `AgentMessage::Response`

use kernel::event::{RawEvent, ResponseEvent};
use serde::{Deserialize, Serialize};

/// Messages sent from a platform process to the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlatformMessage {
    /// A new inbound message from a user on the platform.
    Ingest { event: RawEvent },
    /// Keepalive ping.
    Ping,
}

/// Messages sent from the agent back to a platform process.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentMessage {
    /// A response to send back to the user on the platform.
    Response { event: ResponseEvent },
    /// Keepalive pong.
    Pong,
    /// Acknowledge receipt of an ingest message.
    Ack,
}

/// Default UDS socket path.
pub const DEFAULT_RELAY_SOCKET: &str = "/tmp/polyverse-agent-relay.sock";

/// Env var to override the socket path.
pub const RELAY_SOCKET_ENV: &str = "PLATFORM_RELAY_SOCKET";

pub fn resolve_socket_path() -> String {
    std::env::var(RELAY_SOCKET_ENV).unwrap_or_else(|_| DEFAULT_RELAY_SOCKET.to_string())
}

/// Read a length-prefixed frame from an async reader.
/// Returns `None` on clean EOF.
pub async fn read_frame<R>(reader: &mut R) -> anyhow::Result<Option<Vec<u8>>>
where
    R: tokio::io::AsyncReadExt + Unpin,
{
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    if len == 0 {
        return Ok(Some(vec![]));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(Some(buf))
}

/// Write a length-prefixed frame to an async writer.
pub async fn write_frame<W>(writer: &mut W, data: &[u8]) -> anyhow::Result<()>
where
    W: tokio::io::AsyncWriteExt + Unpin,
{
    let len = data.len() as u32;
    writer.write_all(&len.to_le_bytes()).await?;
    writer.write_all(data).await?;
    writer.flush().await?;
    Ok(())
}

/// Serialize a message and write it as a frame.
pub async fn send_message<W, T>(writer: &mut W, msg: &T) -> anyhow::Result<()>
where
    W: tokio::io::AsyncWriteExt + Unpin,
    T: Serialize,
{
    let bytes = serde_json::to_vec(msg)?;
    write_frame(writer, &bytes).await
}

/// Read a frame and deserialize it.
/// Returns `None` on clean EOF.
pub async fn recv_message<R, T>(reader: &mut R) -> anyhow::Result<Option<T>>
where
    R: tokio::io::AsyncReadExt + Unpin,
    T: for<'de> Deserialize<'de>,
{
    match read_frame(reader).await? {
        None => Ok(None),
        Some(bytes) => {
            let msg = serde_json::from_slice(&bytes)?;
            Ok(Some(msg))
        }
    }
}
