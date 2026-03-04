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
use crate::episodic::{EpisodicStore, MemoryEvent};
use crate::embedder::MemoryEmbedder;
use crate::compressor::SemanticCompressor;

pub struct MemoryWorker {
    pub short_term: Arc<Mutex<ShortTermMemory>>,
    pub episodic: Option<Arc<EpisodicStore>>,
    pub embedder: Option<Arc<MemoryEmbedder>>,
    pub compressor: Option<Arc<SemanticCompressor>>,
    db_path: String,
}

unsafe impl Sync for MemoryWorker {}

impl MemoryWorker {
    pub fn new(db_path: &str) -> Self {
        Self {
            short_term: Arc::new(Mutex::new(ShortTermMemory::new())),
            episodic: None,
            embedder: None,
            compressor: None,
            db_path: db_path.to_string(),
        }
    }

    pub fn with_episodic(mut self, episodic: Arc<EpisodicStore>) -> Self {
        self.episodic = Some(episodic);
        self
    }

    pub fn with_embedder(mut self, embedder: Arc<MemoryEmbedder>) -> Self {
        self.embedder = Some(embedder);
        self
    }

    pub fn with_compressor(mut self, compressor: Arc<SemanticCompressor>) -> Self {
        self.compressor = Some(compressor);
        self
    }

    pub fn short_term_handle(&self) -> Arc<Mutex<ShortTermMemory>> {
        Arc::clone(&self.short_term)
    }

    fn ingest_session(
        messages: Vec<MemoryMessage>,
        compressor: Arc<SemanticCompressor>,
        embedder: Arc<MemoryEmbedder>,
        episodic: Arc<EpisodicStore>,
    ) {
        if messages.len() < 3 {
            debug!(count = messages.len(), "Session too short, ignoring semantic compression.");
            return;
        }
        
        tokio::spawn(async move {
            let session_id = uuid::Uuid::new_v4().to_string();
            
            let base_persona = tokio::fs::read_to_string("instruct.txt")
                .await
                .unwrap_or_else(|_| "Mày là Ryuuko.".to_string());

            let mut formatted_msgs = Vec::new();
            for msg in &messages {
                let speaker = if msg.is_bot_response { "Ryuuko" } else { &msg.username };
                formatted_msgs.push(format!("[{}]: {}", speaker, msg.content));
            }
            
            let chat_log_doc = format!(
                "=== BẮT ĐẦU LOG CHAT ===\n{}\n=== KẾT THÚC LOG CHAT ===",
                formatted_msgs.join("\n")
            );

            match compressor.compress(&base_persona, &chat_log_doc).await {
                Ok(Some(compression)) => {
                    info!(
                        session_id = %session_id,
                        fact = %compression.fact,
                        importance = compression.importance,
                        "Memory semantic compression successful"
                    );

                    match embedder.embed_single(compression.fact.clone()).await {
                        Ok(vector) => {
                            let timestamp = messages.last().unwrap().timestamp.timestamp();
                            let target_username = messages.iter().find(|m| !m.is_bot_response).map(|m| m.username.clone()).unwrap_or_else(|| "unknown".to_string());
                            let metadata = serde_json::json!({
                                "username": target_username,
                                "message_count": messages.len(),
                                "first_message_timestamp": messages.first().unwrap().timestamp.timestamp(),
                            }).to_string();

                            let event = MemoryEvent {
                                id: session_id.clone(),
                                vector,
                                content: compression.fact,
                                timestamp,
                                importance: compression.importance,
                                metadata,
                            };

                            if let Err(e) = episodic.insert(vec![event]).await {
                                error!(error = %e, "Failed to insert event into EpisodicStore");
                            } else {
                                info!(session_id = %session_id, "Memory event successfully ingested into EpisodicStore");
                            }
                        }
                        Err(e) => {
                            error!(error = %e, "Failed to embed memory fact");
                        }
                    }
                }
                Ok(None) => debug!("Semantic compression deemed session trivial; no event to ingest."),
                Err(e) => error!(error = %e, "Semantic compression API failed"),
            }
        });
    }
}

#[async_trait]
impl Worker for MemoryWorker {
    fn name(&self) -> &str {
        "memory"
    }

    async fn start(&mut self, ctx: WorkerContext) -> Result<()> {
        info!("Memory worker starting...");

        if let Some(parent) = std::path::Path::new(&self.db_path).parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let store = MemoryStore::open(&self.db_path)?;
        let msg_count = store.message_count().unwrap_or(0);
        info!(
            path = %self.db_path,
            existing_messages = msg_count,
            "Memory store opened"
        );

        if let Ok(recent) = store.get_recent_all(500) {
            let mut stm = self.short_term.lock().await;
            stm.load_history(recent);
            stm.mark_all_persisted();
            info!("Loaded recent history into short-term memory");
        }

        let episodic = Arc::clone(self.episodic.as_ref().expect("EpisodicStore not initialized"));
        let embedder = Arc::clone(self.embedder.as_ref().expect("MemoryEmbedder not initialized"));
        let compressor = self.compressor.clone();

        let mut broadcast_rx = ctx.subscribe_events();
        let mut shutdown_rx = ctx.subscribe_shutdown();
        let short_term = Arc::clone(&self.short_term);

        let mut flush_interval = tokio::time::interval(tokio::time::Duration::from_secs(60));

        info!("Memory worker ready");

        loop {
            tokio::select! {
                event = broadcast_rx.recv() => {
                    match event {
                        Ok(Event::Raw(raw)) => {
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

                            let expired = {
                                let mut stm = short_term.lock().await;
                                stm.push(msg.clone())
                            };

                            if let Err(e) = store.insert(&msg) {
                                error!(error = %e, "Failed to persist message");
                            }

                            if let Some(expired_msgs) = expired {
                                if let Some(comp) = &compressor {
                                    Self::ingest_session(
                                        expired_msgs,
                                        Arc::clone(comp),
                                        Arc::clone(&embedder),
                                        Arc::clone(&episodic),
                                    );
                                }
                            }
                        }
                        Ok(Event::BotTurnCompletion(complete)) => {
                            let msg = MemoryMessage::bot_response(
                                complete.platform,
                                complete.channel_id.clone(),
                                complete.content.clone(),
                                complete.reply_to_message_id.clone(),
                                complete.reply_to_user.clone(),
                            );
                            debug!(
                                channel = %msg.channel_id,
                                reply_to = ?msg.reply_to_user,
                                "Recording full bot turn to memory"
                            );

                            let mut stm = short_term.lock().await;
                            stm.push(msg.clone());

                            if let Err(e) = store.insert(&msg) {
                                error!(error = %e, "Failed to persist bot turn completion");
                            }
                        }
                        Ok(_) => {}
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
                    let expired = {
                        let mut stm = short_term.lock().await;
                        stm.flush_expired()
                    };

                    for (key, messages) in expired {
                        info!(
                            conversation = %key,
                            messages = messages.len(),
                                "Session expired, flushed to store"
                            );
                        if let Some(comp) = &compressor {
                            Self::ingest_session(
                                messages,
                                Arc::clone(comp),
                                Arc::clone(&embedder),
                                Arc::clone(&episodic),
                            );
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
