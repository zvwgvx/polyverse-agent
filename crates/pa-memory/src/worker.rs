use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use pa_core::event::Event;
use pa_core::worker::Worker;
use pa_core::WorkerContext;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::short_term::ShortTermMemory;
use crate::store::MemoryStore;
use crate::types::MemoryMessage;

/// Memory worker: integrates short-term (RAM) and SQLite store.
///
/// Listens for ALL events on broadcast:
/// - RawEvent → normalize to MemoryMessage → push into short-term + persist
/// - ResponseEvent → record Ryuuko's response in short-term + persist
///
/// Also provides context retrieval for the LLM worker via shared handle.
pub struct MemoryWorker {
    /// Short-term memory (shared with LLM worker for context retrieval)
    pub short_term: Arc<Mutex<ShortTermMemory>>,
    /// Database path
    db_path: String,
}

// Safety: MemoryStore is NOT held in the struct (it lives inside start()).
// Only Arc<Mutex<ShortTermMemory>> and String are held — both Send+Sync.
unsafe impl Sync for MemoryWorker {}

impl MemoryWorker {
    pub fn new(db_path: &str) -> Self {
        Self {
            short_term: Arc::new(Mutex::new(ShortTermMemory::new())),
            db_path: db_path.to_string(),
        }
    }

    /// Get a shared reference to short-term memory.
    /// Used by LLM worker to retrieve conversation context.
    pub fn short_term_handle(&self) -> Arc<Mutex<ShortTermMemory>> {
        Arc::clone(&self.short_term)
    }
}

#[async_trait]
impl Worker for MemoryWorker {
    fn name(&self) -> &str {
        "memory"
    }

    async fn start(&mut self, ctx: WorkerContext) -> Result<()> {
        info!("Memory worker starting...");

        // Create data directory if it doesn't exist
        if let Some(parent) = std::path::Path::new(&self.db_path).parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }

        // Open SQLite store (lives inside start — not in struct)
        let store = MemoryStore::open(&self.db_path)?;
        let msg_count = store.message_count().unwrap_or(0);
        info!(
            path = %self.db_path,
            existing_messages = msg_count,
            "Memory store opened"
        );

        let mut broadcast_rx = ctx.subscribe_events();
        let mut shutdown_rx = ctx.subscribe_shutdown();
        let short_term = Arc::clone(&self.short_term);

        // Periodic flush timer (check for expired sessions every 60s)
        let mut flush_interval = tokio::time::interval(tokio::time::Duration::from_secs(60));

        info!("Memory worker ready");

        loop {
            tokio::select! {
                event = broadcast_rx.recv() => {
                    match event {
                        Ok(Event::Raw(raw)) => {
                            // Only store messages directed at Ryuuko (DM, tag, reply)
                            if !raw.is_mention {
                                debug!(
                                    user = %raw.username,
                                    channel = %raw.channel_id,
                                    "Skipping non-mention message (not stored in memory)"
                                );
                                continue;
                            }

                            let msg = MemoryMessage::from_raw(&raw);
                            debug!(
                                user = %msg.username,
                                channel = %msg.channel_id,
                                importance = msg.importance,
                                "Recording mention/DM to memory"
                            );

                            // Push to short-term, check for expired session
                            let expired = {
                                let mut stm = short_term.lock().await;
                                stm.push(msg.clone())
                            };

                            // Persist current message
                            if let Err(e) = store.insert(&msg) {
                                error!(error = %e, "Failed to persist message");
                            }

                            // Persist expired session messages (batch)
                            if let Some(expired_msgs) = expired {
                                if let Err(e) = store.insert_batch(&expired_msgs) {
                                    error!(error = %e, "Failed to persist expired session");
                                }
                            }
                        }
                        Ok(Event::Response(response)) => {
                            // Record Ryuuko's responses too
                            let msg = MemoryMessage::bot_response(
                                response.platform,
                                response.channel_id.clone(),
                                response.content.clone(),
                                response.reply_to_message_id.clone(),
                                response.reply_to_user.clone(),
                            );
                            debug!(
                                channel = %msg.channel_id,
                                reply_to = ?msg.reply_to_user,
                                "Recording bot response to memory"
                            );

                            let mut stm = short_term.lock().await;
                            stm.push(msg.clone());

                            if let Err(e) = store.insert(&msg) {
                                error!(error = %e, "Failed to persist bot response");
                            }
                        }
                        Ok(_) => {} // Ignore other events
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            warn!(missed = n, "Memory broadcast receiver lagged");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            info!("Memory broadcast channel closed");
                            break;
                        }
                    }
                }
                _ = flush_interval.tick() => {
                    // Periodically flush expired sessions
                    let expired = {
                        let mut stm = short_term.lock().await;
                        stm.flush_expired()
                    };

                    for (key, messages) in &expired {
                        info!(
                            conversation = %key,
                            messages = messages.len(),
                            "Session expired, flushed to store"
                        );
                        if let Err(e) = store.insert_batch(messages) {
                            error!(error = %e, "Failed to persist flushed session");
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Memory worker received shutdown signal");
                    info!("Memory worker stopped");
                    break;
                }
            }
        }

        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        info!("Memory worker stopping");
        Ok(())
    }

    fn health_check(&self) -> pa_core::worker::WorkerStatus {
        pa_core::worker::WorkerStatus::Healthy
    }
}
