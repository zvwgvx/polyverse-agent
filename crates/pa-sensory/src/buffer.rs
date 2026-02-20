use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use pa_core::event::{Event, Platform, RawEvent};
use tokio::sync::{mpsc, Mutex};
use tracing::debug;

/// Key to uniquely identify a typing session
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct BufferKey {
    pub platform: Platform,
    pub channel_id: String,
    pub user_id: String,
}

impl BufferKey {
    pub fn from_raw(raw: &RawEvent) -> Self {
        Self {
            platform: raw.platform,
            channel_id: raw.channel_id.clone(),
            user_id: raw.user_id.clone(),
        }
    }
}

/// The Sensory Buffer acts as a debounce layer to prevent context fragmentation.
/// It catches raw events and groups them by user/channel within a sliding 3-second window.
#[derive(Clone)]
pub struct SensoryBuffer {
    /// Maps a session to an active tokio task's sender channel
    active_sessions: Arc<Mutex<HashMap<BufferKey, mpsc::Sender<RawEvent>>>>,
    /// The global event bus transmitter to send completed aggregated events
    event_tx: mpsc::Sender<Event>,
}

impl SensoryBuffer {
    pub fn new(event_tx: mpsc::Sender<Event>) -> Self {
        Self {
            active_sessions: Arc::new(Mutex::new(HashMap::new())),
            event_tx,
        }
    }

    /// Push an incoming raw event into the buffer.
    /// If a session exists, it concatenates. Otherwise, assigns a new actor task.
    pub async fn push(&self, raw: RawEvent) {
        let key = BufferKey::from_raw(&raw);
        let mut sessions = self.active_sessions.lock().await;

        if let Some(tx) = sessions.get(&key) {
            // Give message to existing actor task
            if let Err(e) = tx.send(raw.clone()).await {
                debug!(error = %e, "Failed to send message to buffer actor, it might have just died");
                // The task closed, fall through to create a new one.
            } else {
                return; // Sent successfully
            }
        }

        // Create new session channel
        let (tx, rx) = mpsc::channel(100);
        sessions.insert(key.clone(), tx.clone());
        
        let sessions_clone = Arc::clone(&self.active_sessions);
        let event_tx = self.event_tx.clone();
        
        // Push the very first message into its own actor
        let _ = tx.send(raw).await;

        tokio::spawn(async move {
            Self::debounce_actor(key, rx, event_tx, sessions_clone).await;
        });
    }

    /// Actor loop: waits for contiguous messages and emits upon a 3-second timeout silence.
    async fn debounce_actor(
        key: BufferKey,
        mut rx: mpsc::Receiver<RawEvent>,
        event_tx: mpsc::Sender<Event>,
        sessions_map: Arc<Mutex<HashMap<BufferKey, mpsc::Sender<RawEvent>>>>,
    ) {
        let mut aggregated: Option<RawEvent> = None;

        loop {
            // Block and wait for a message for up to 3 seconds.
            match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
                Ok(Some(new_msg)) => {
                    // Received a new message before silence timeout.
                    if let Some(mut existing) = aggregated.take() {
                        // Concatenate text
                        existing.content.push('\n');
                        existing.content.push_str(&new_msg.content);
                        // Update boolean flags if the new message escalated them (e.g. they mentioned the bot later)
                        existing.is_mention |= new_msg.is_mention;
                        existing.is_dm |= new_msg.is_dm;
                        // Timestamp assumes the start of the session, but we can update to newest if needed.
                        existing.timestamp = chrono::Utc::now();
                        aggregated = Some(existing);
                    } else {
                        // First message starts the aggregation
                        aggregated = Some(new_msg);
                    }
                }
                Ok(None) => {
                    // The mpsc sender was explicitly dropped.
                    break;
                }
                Err(_) => {
                    // Timeout Elapsed! 3 seconds of typing silence.
                    break;
                }
            }
        }

        // Cleanup self from HashMap so future messages spawn a new actor
        {
            let mut sm = sessions_map.lock().await;
            sm.remove(&key);
        }

        // Fire the aggregated chunk onto the actual event bus!
        if let Some(mut final_msg) = aggregated {
            debug!(
                user = %final_msg.username,
                content_len = final_msg.content.len(),
                "Flushing aggregated sensory buffer"
            );
            // We trim trailing whitespaces
            final_msg.content = final_msg.content.trim().to_string();
            let _ = event_tx.send(Event::Raw(final_msg)).await;
        }
    }
}
