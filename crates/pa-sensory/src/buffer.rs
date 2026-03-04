use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use pa_core::event::{Event, Platform, RawEvent};
use tokio::sync::{mpsc, Mutex};
use tracing::debug;

pub enum BufferMsg {
    Event(RawEvent),
    Typing,
}

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

#[derive(Clone)]
#[repr(align(64))]
pub struct SensoryBuffer {
    active_sessions: Arc<Mutex<HashMap<BufferKey, mpsc::Sender<BufferMsg>>>>,
    event_tx: mpsc::Sender<Event>,
}

impl SensoryBuffer {
    pub fn new(event_tx: mpsc::Sender<Event>) -> Self {
        Self {
            active_sessions: Arc::new(Mutex::new(HashMap::new())),
            event_tx,
        }
    }

    pub async fn push(&self, raw: RawEvent) {
        let key = BufferKey::from_raw(&raw);
        let mut sessions = self.active_sessions.lock().await;

        if let Some(tx) = sessions.get(&key) {
            if let Err(e) = tx.send(BufferMsg::Event(raw.clone())).await {
                debug!(error = %e, "Failed to send message to buffer actor, it might have just died");
            } else {
                return;
            }
        }

        let (tx, rx) = mpsc::channel(100);
        sessions.insert(key.clone(), tx.clone());
        
        let sessions_clone = Arc::clone(&self.active_sessions);
        let event_tx = self.event_tx.clone();
        
        let _ = tx.send(BufferMsg::Event(raw)).await;

        tokio::spawn(async move {
            Self::debounce_actor(key, rx, event_tx, sessions_clone).await;
        });
    }

    pub async fn typing(&self, platform: Platform, channel_id: String, user_id: String) {
        let key = BufferKey {
            platform,
            channel_id,
            user_id,
        };
        
        let sessions = self.active_sessions.lock().await;
        if let Some(tx) = sessions.get(&key) {
            let _ = tx.send(BufferMsg::Typing).await;
        }
    }

    async fn debounce_actor(
        key: BufferKey,
        mut rx: mpsc::Receiver<BufferMsg>,
        event_tx: mpsc::Sender<Event>,
        sessions_map: Arc<Mutex<HashMap<BufferKey, mpsc::Sender<BufferMsg>>>>,
    ) {
        let mut aggregated: Option<RawEvent> = None;
        let mut deadline = tokio::time::Instant::now() + Duration::from_secs(3);

        loop {
            tokio::select! {
                msg_opt = rx.recv() => {
                    match msg_opt {
                        Some(BufferMsg::Event(new_msg)) => {
                            if let Some(mut existing) = aggregated.take() {
                                existing.content.push('\n');
                                existing.content.push_str(&new_msg.content);
                                existing.is_mention |= new_msg.is_mention;
                                existing.is_dm |= new_msg.is_dm;
                                existing.timestamp = chrono::Utc::now();
                                aggregated = Some(existing);
                            } else {
                                aggregated = Some(new_msg);
                            }
                            deadline = tokio::time::Instant::now() + Duration::from_secs(3);
                        }
                        Some(BufferMsg::Typing) => {
                            deadline = tokio::time::Instant::now() + Duration::from_secs(4);
                            debug!(
                                user_id = %key.user_id,
                                "User is typing... extending debounce buffer to 4 seconds"
                            );
                        }
                        None => {
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep_until(deadline) => {
                    break;
                }
            }
        }

        {
            let mut sm = sessions_map.lock().await;
            sm.remove(&key);
        }

        if let Some(mut final_msg) = aggregated {
            debug!(
                user = %final_msg.username,
                content_len = final_msg.content.len(),
                "Flushing aggregated sensory buffer"
            );
            final_msg.content = final_msg.content.trim().to_string();
            let _ = event_tx.send(Event::Raw(final_msg)).await;
        }
    }
}
