use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use kernel::get_agent_profile;
use kernel::event::Event;
use kernel::worker::Worker;
use kernel::prompt_registry::{get_prompt_or, render_prompt_or};
use kernel::WorkerContext;
use tokio::sync::{broadcast, mpsc, Mutex, Semaphore};
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
    ingest_limiter: Arc<Semaphore>,
    db_path: String,
}

unsafe impl Sync for MemoryWorker {}

const MEMORY_WRITE_CHANNEL_CAPACITY: usize = 1_024;
const MEMORY_WRITE_BATCH_SIZE: usize = 64;
const MEMORY_WRITE_FLUSH_INTERVAL_MS: u64 = 200;

impl MemoryWorker {
    pub fn new(db_path: &str) -> Self {
        let ingest_permits = std::thread::available_parallelism()
            .map(|value| value.get().clamp(1, 4))
            .unwrap_or(2);
        Self {
            short_term: Arc::new(Mutex::new(ShortTermMemory::new())),
            episodic: None,
            embedder: None,
            compressor: None,
            ingest_limiter: Arc::new(Semaphore::new(ingest_permits)),
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
        ingest_limiter: Arc<Semaphore>,
    ) {
        if messages.len() < 3 {
            debug!(count = messages.len(), "Session too short, ignoring semantic compression.");
            return;
        }
        
        tokio::spawn(async move {
            let permit = match ingest_limiter.acquire_owned().await {
                Ok(permit) => permit,
                Err(err) => {
                    error!(error = %err, "Failed to acquire ingestion permit");
                    return;
                }
            };
            let session_id = uuid::Uuid::new_v4().to_string();
            let profile = get_agent_profile();
            let fallback_persona = format!("You are {}.", profile.display_name);

            let base_persona = get_prompt_or("persona.base", fallback_persona.as_str());

            let mut formatted_msgs = Vec::new();
            for msg in &messages {
                let speaker = if msg.is_bot_response {
                    profile.display_name.as_str()
                } else {
                    &msg.username
                };
                formatted_msgs.push(format!("[{}]: {}", speaker, msg.content));
            }

            let joined_log = formatted_msgs.join("\n");
            let chat_log_doc = render_prompt_or(
                "memory.chatlog.wrapper",
                &[("chat_log", joined_log.as_str())],
                "=== CHAT LOG START ===\n{{chat_log}}\n=== CHAT LOG END ===\n",
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
            drop(permit);
        });
    }

    async fn persist_message(
        writer_tx: &mpsc::Sender<MemoryMessage>,
        writer_store: &Arc<std::sync::Mutex<MemoryStore>>,
        msg: MemoryMessage,
        context: &'static str,
    ) {
        match writer_tx.try_send(msg.clone()) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(pending)) => {
                if let Err(err) = writer_tx.send(pending).await {
                    warn!(error = %err, action = context, "Memory write queue unavailable, falling back to direct persist");
                    Self::flush_persist_batch(Arc::clone(writer_store), vec![err.0], context).await;
                }
            }
            Err(mpsc::error::TrySendError::Closed(pending)) => {
                warn!(action = context, "Memory write queue closed, falling back to direct persist");
                Self::flush_persist_batch(Arc::clone(writer_store), vec![pending], context).await;
            }
        }
    }

    async fn flush_persist_batch(
        store: Arc<std::sync::Mutex<MemoryStore>>,
        batch: Vec<MemoryMessage>,
        context: &'static str,
    ) {
        if batch.is_empty() {
            return;
        }

        let batch_size = batch.len();
        match tokio::task::spawn_blocking(move || {
            let store = store
                .lock()
                .map_err(|_| anyhow::anyhow!("memory store mutex poisoned"))?;
            store.insert_batch(&batch)
        })
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                error!(error = %err, action = context, count = batch_size, "Memory batch persist failed");
            }
            Err(err) => {
                error!(error = %err, action = context, count = batch_size, "Memory batch persist task failed");
            }
        }
    }

    async fn run_persist_writer(
        store: Arc<std::sync::Mutex<MemoryStore>>,
        mut writer_rx: mpsc::Receiver<MemoryMessage>,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) {
        let mut buffer = Vec::with_capacity(MEMORY_WRITE_BATCH_SIZE);
        let mut flush_interval =
            tokio::time::interval(tokio::time::Duration::from_millis(MEMORY_WRITE_FLUSH_INTERVAL_MS));
        flush_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                maybe_msg = writer_rx.recv() => {
                    match maybe_msg {
                        Some(msg) => {
                            buffer.push(msg);
                            while buffer.len() < MEMORY_WRITE_BATCH_SIZE {
                                match writer_rx.try_recv() {
                                    Ok(msg) => buffer.push(msg),
                                    Err(mpsc::error::TryRecvError::Empty) => break,
                                    Err(mpsc::error::TryRecvError::Disconnected) => break,
                                }
                            }

                            if buffer.len() >= MEMORY_WRITE_BATCH_SIZE {
                                let batch = std::mem::take(&mut buffer);
                                Self::flush_persist_batch(Arc::clone(&store), batch, "writer_flush").await;
                            }
                        }
                        None => {
                            let batch = std::mem::take(&mut buffer);
                            Self::flush_persist_batch(Arc::clone(&store), batch, "writer_drain").await;
                            break;
                        }
                    }
                }
                _ = flush_interval.tick(), if !buffer.is_empty() => {
                    let batch = std::mem::take(&mut buffer);
                    Self::flush_persist_batch(Arc::clone(&store), batch, "writer_interval").await;
                }
                _ = shutdown_rx.recv() => {
                    while let Ok(msg) = writer_rx.try_recv() {
                        buffer.push(msg);
                        if buffer.len() >= MEMORY_WRITE_BATCH_SIZE {
                            let batch = std::mem::take(&mut buffer);
                            Self::flush_persist_batch(Arc::clone(&store), batch, "writer_shutdown").await;
                        }
                    }

                    let batch = std::mem::take(&mut buffer);
                    Self::flush_persist_batch(Arc::clone(&store), batch, "writer_shutdown").await;
                    break;
                }
            }
        }
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
        let writer_store = Arc::new(std::sync::Mutex::new(store));
        let (writer_tx, writer_rx) = mpsc::channel(MEMORY_WRITE_CHANNEL_CAPACITY);
        let writer_handle = tokio::spawn(Self::run_persist_writer(
            Arc::clone(&writer_store),
            writer_rx,
            ctx.subscribe_shutdown(),
        ));

        let episodic = Arc::clone(self.episodic.as_ref().expect("EpisodicStore not initialized"));
        let embedder = Arc::clone(self.embedder.as_ref().expect("MemoryEmbedder not initialized"));
        let compressor = self.compressor.clone();
        let ingest_limiter = Arc::clone(&self.ingest_limiter);

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

                            Self::persist_message(&writer_tx, &writer_store, msg.clone(), "raw_message").await;

                            if let Some(expired_msgs) = expired {
                                if let Some(comp) = &compressor {
                                    Self::ingest_session(
                                        expired_msgs,
                                        Arc::clone(comp),
                                        Arc::clone(&embedder),
                                        Arc::clone(&episodic),
                                        Arc::clone(&ingest_limiter),
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

                            Self::persist_message(&writer_tx, &writer_store, msg.clone(), "bot_turn").await;
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
                                Arc::clone(&ingest_limiter),
                            );
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Memory worker received shutdown signal");
                    break;
                }
            }
        }

        drop(writer_tx);
        let _ = writer_handle.await;
        info!("Memory worker stopped");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        info!("Memory worker stopping");
        Ok(())
    }

    fn health_check(&self) -> kernel::worker::WorkerStatus {
        kernel::worker::WorkerStatus::Healthy
    }
}
