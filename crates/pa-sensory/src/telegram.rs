use anyhow::Result;
use async_trait::async_trait;
use pa_core::event::{Event, Platform, RawEvent};
use pa_core::worker::{Worker, WorkerContext, WorkerStatus};
use teloxide::prelude::*;
use tracing::{debug, error, info, warn};

use crate::platform::PlatformAdapter;

/// Telegram sensory worker powered by teloxide.
///
/// Listens to Telegram messages via long-polling and emits RawEvents
/// into the event bus. Also handles outgoing responses.
pub struct TelegramWorker {
    /// Telegram bot token
    token: String,
    /// Worker status
    status: WorkerStatus,
    /// Bot instance (stored after start for sending messages)
    bot: Option<Bot>,
}

impl TelegramWorker {
    pub fn new(token: String) -> Self {
        Self {
            token,
            status: WorkerStatus::NotStarted,
            bot: None,
        }
    }
}

#[async_trait]
impl Worker for TelegramWorker {
    fn name(&self) -> &str {
        "telegram"
    }

    async fn start(&mut self, ctx: WorkerContext) -> Result<()> {
        info!("Telegram worker starting...");

        // Validate token format early
        if self.token.is_empty()
            || self.token == "YOUR_TELEGRAM_BOT_TOKEN"
            || self.token.starts_with("your_")
        {
            warn!("Telegram token is not configured, disabling Telegram worker");
            self.status = WorkerStatus::Stopped;
            return Ok(());
        }

        let bot = Bot::new(&self.token);
        self.bot = Some(bot.clone());

        // Validate token by calling getMe — if invalid, disable gracefully
        let me = match bot.get_me().await {
            Ok(me) => me,
            Err(e) => {
                warn!(
                    error = %e,
                    "Telegram authentication failed — invalid token, disabling Telegram worker"
                );
                self.status = WorkerStatus::Stopped;
                self.bot = None;
                return Ok(()); // Graceful — don't crash
            }
        };

        let bot_username = me.username().to_string();
        info!(
            bot_name = %bot_username,
            "Telegram bot connected"
        );

        self.status = WorkerStatus::Healthy;

        let event_tx = ctx.event_tx.clone();

        // Spawn the response listener (broadcast → Telegram)
        let bot_clone = bot.clone();
        let mut broadcast_rx = ctx.subscribe_events();
        tokio::spawn(async move {
            loop {
                match broadcast_rx.recv().await {
                    Ok(Event::Response(response)) => {
                        if response.platform == Platform::Telegram {
                            let chat_id: i64 =
                                response.channel_id.parse().unwrap_or_default();

                            // Reply to the original message if we have the ID
                            let mut req = bot_clone
                                .send_message(ChatId(chat_id), &response.content);

                            if let Some(ref reply_id) = response.reply_to_message_id {
                                if let Ok(msg_id) = reply_id.parse::<i32>() {
                                    req = req.reply_parameters(
                                        teloxide::types::ReplyParameters::new(
                                            teloxide::types::MessageId(msg_id),
                                        ),
                                    );
                                }
                            }

                            match req.await {
                                Ok(_) => {
                                    info!(
                                        chat = %chat_id,
                                        "Telegram response sent successfully"
                                    );
                                }
                                Err(e) => {
                                    error!(error = %e, "Failed to send Telegram message");
                                }
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(missed = n, "Telegram broadcast receiver lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        info!("Telegram broadcast channel closed");
                        break;
                    }
                    _ => {}
                }
            }
        });

        // Set up the message handler using teloxide's Dispatcher
        // We pass bot_username to detect mentions (@botname)
        let handler = Update::filter_message().endpoint(
            move |msg: Message,
                  event_tx: tokio::sync::mpsc::Sender<Event>,
                  bot_un: String| async move {
                if let Some(text) = msg.text() {
                    let user = msg
                        .from
                        .as_ref()
                        .map(|u| u.first_name.clone())
                        .unwrap_or_else(|| "Unknown".to_string());
                    let user_id = msg
                        .from
                        .as_ref()
                        .map(|u| u.id.0.to_string())
                        .unwrap_or_default();

                    // Detect DM (private chat) or mention (@botname)
                    let is_dm = msg.chat.is_private();
                    let mention_tag = format!("@{}", bot_un);
                    let is_mention = is_dm
                        || text.contains(&mention_tag)
                        || msg
                            .entities()
                            .map(|entities| {
                                entities.iter().any(|e| {
                                    e.kind == teloxide::types::MessageEntityKind::Mention
                                        && text
                                            .get(e.offset..e.offset + e.length)
                                            .map(|s| s.eq_ignore_ascii_case(&mention_tag))
                                            .unwrap_or(false)
                                })
                            })
                            .unwrap_or(false);

                    if is_mention {
                        info!(
                            user = %user,
                            chat = %msg.chat.id,
                            content = %text,
                            dm = is_dm,
                            "[MENTION] Bot was tagged on Telegram"
                        );
                    } else {
                        debug!(
                            user = %user,
                            chat = %msg.chat.id,
                            content = %text,
                            "Telegram message received"
                        );
                    }

                    let raw_event = Event::Raw(RawEvent {
                        platform: Platform::Telegram,
                        channel_id: msg.chat.id.0.to_string(),
                        message_id: msg.id.0.to_string(),
                        user_id,
                        username: user,
                        content: text.to_string(),
                        is_mention,
                        is_dm: msg.chat.is_private(),
                        timestamp: chrono::Utc::now(),
                    });

                    if let Err(e) = event_tx.send(raw_event).await {
                        error!(error = %e, "Failed to emit Telegram raw event");
                    }
                }
                Ok::<(), anyhow::Error>(())
            },
        );

        let mut dispatcher = Dispatcher::builder(bot, handler)
            .dependencies(dptree::deps![event_tx, bot_username])
            .default_handler(|_| async {})
            .build();

        // Run dispatcher until shutdown
        let mut shutdown_rx = ctx.subscribe_shutdown();
        let shutdown_token = dispatcher.shutdown_token();

        tokio::spawn(async move {
            let _ = shutdown_rx.recv().await;
            info!("Telegram worker received shutdown signal");
            if let Err(e) = shutdown_token.shutdown() {
                tracing::error!(error = %e, "Failed to shutdown Telegram dispatcher");
            }
        });

        dispatcher.dispatch().await;

        self.status = WorkerStatus::Stopped;
        info!("Telegram worker stopped");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        info!("Telegram worker stopping...");
        self.status = WorkerStatus::Stopped;
        Ok(())
    }

    fn health_check(&self) -> WorkerStatus {
        self.status.clone()
    }
}

#[async_trait]
impl PlatformAdapter for TelegramWorker {
    async fn send_message(&self, channel_id: &str, content: &str) -> Result<()> {
        if let Some(bot) = &self.bot {
            let chat_id: i64 = channel_id.parse()?;
            bot.send_message(ChatId(chat_id), content).await?;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Telegram bot not initialized"))
        }
    }

    async fn send_reaction(
        &self,
        _channel_id: &str,
        _message_id: &str,
        _emoji: &str,
    ) -> Result<()> {
        warn!("Telegram reactions not yet implemented");
        Ok(())
    }

    async fn send_reply(
        &self,
        channel_id: &str,
        message_id: &str,
        content: &str,
    ) -> Result<()> {
        if let Some(bot) = &self.bot {
            let chat_id: i64 = channel_id.parse()?;
            let msg_id: i32 = message_id.parse()?;
            bot.send_message(ChatId(chat_id), content)
                .reply_parameters(teloxide::types::ReplyParameters::new(
                    teloxide::types::MessageId(msg_id),
                ))
                .await?;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Telegram bot not initialized"))
        }
    }
}
