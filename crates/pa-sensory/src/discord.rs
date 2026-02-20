use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use pa_core::event::{Event, Platform, RawEvent};
use pa_core::worker::{Worker, WorkerContext, WorkerStatus};
use serenity::all::{
    Context, CreateMessage, EventHandler, GatewayIntents, Message, Ready,
};
use serenity::Client;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::buffer::SensoryBuffer;
use crate::platform::PlatformAdapter;

/// Discord sensory worker powered by serenity.
///
/// Listens to Discord messages and emits RawEvents into the event bus.
/// Also listens for ResponseEvents on the broadcast channel and sends
/// replies back to Discord.
pub struct DiscordWorker {
    /// Discord bot token
    token: String,
    /// Worker status
    status: WorkerStatus,
    /// Shared context handle (for sending messages from PlatformAdapter methods)
    http: Arc<RwLock<Option<Arc<serenity::http::Http>>>>,
}

impl DiscordWorker {
    pub fn new(token: String) -> Self {
        Self {
            token,
            status: WorkerStatus::NotStarted,
            http: Arc::new(RwLock::new(None)),
        }
    }
}

/// Internal serenity event handler that bridges Discord events into our event bus.
struct DiscordHandler {
    buffer: SensoryBuffer,
    http_store: Arc<RwLock<Option<Arc<serenity::http::Http>>>>,
    /// The bot's own user ID (set on ready), used to detect mentions.
    bot_user_id: Arc<RwLock<Option<serenity::model::id::UserId>>>,
}

#[async_trait]
impl EventHandler for DiscordHandler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        info!(
            bot_name = %ready.user.name,
            bot_id = %ready.user.id,
            guilds = ready.guilds.len(),
            "Discord bot connected"
        );

        // Store the HTTP client and bot user ID
        {
            let mut http = self.http_store.write().await;
            *http = Some(Arc::clone(&ctx.http));
        }
        {
            let mut bot_id = self.bot_user_id.write().await;
            *bot_id = Some(ready.user.id);
        }
    }

    async fn message(&self, _ctx: Context, msg: Message) {
        // Ignore messages from bots (including ourselves)
        if msg.author.bot {
            return;
        }

        // Check if this is a DM (private channel) or if the bot is mentioned
        let is_dm = msg.guild_id.is_none();
        let is_mention = is_dm || {
            let bot_id = self.bot_user_id.read().await;
            if let Some(bot_id) = *bot_id {
                msg.mentions.iter().any(|u| u.id == bot_id)
            } else {
                false
            }
        };

        if is_mention {
            info!(
                user = %msg.author.name,
                channel = %msg.channel_id,
                content = %msg.content,
                dm = is_dm,
                "[MENTION] Bot was tagged on Discord"
            );
        } else {
            debug!(
                user = %msg.author.name,
                channel = %msg.channel_id,
                content = %msg.content,
                "Discord message received"
            );
        }

        let raw = RawEvent {
            platform: Platform::Discord,
            channel_id: msg.channel_id.to_string(),
            message_id: msg.id.to_string(),
            user_id: msg.author.id.to_string(),
            username: msg.author.name.clone(),
            content: msg.content.clone(),
            is_mention,
            is_dm,
            timestamp: chrono::Utc::now(),
        };

        // Push RawEvent into the Sensory Buffer for debounce & aggregation
        self.buffer.push(raw).await;
    }
}

#[async_trait]
impl Worker for DiscordWorker {
    fn name(&self) -> &str {
        "discord"
    }

    async fn start(&mut self, ctx: WorkerContext) -> Result<()> {
        info!("Discord worker starting...");

        // Validate token format early (Discord tokens have a specific structure)
        if self.token.is_empty()
            || self.token == "YOUR_DISCORD_BOT_TOKEN"
            || self.token.starts_with("your_")
        {
            warn!("Discord token is not configured, disabling Discord worker");
            self.status = WorkerStatus::Stopped;
            return Ok(());
        }

        // Instantiate sensory buffer
        let buffer = SensoryBuffer::new(ctx.event_tx.clone());

        // Prepare context and handler
        let http_store = Arc::clone(&self.http);
        let bot_user_id = Arc::new(RwLock::new(None));
        let handler = DiscordHandler {
            buffer,
            http_store,
            bot_user_id: Arc::clone(&bot_user_id),
        };

        let intents = GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT;

        let mut client = match Client::builder(&self.token, intents)
            .event_handler(handler)
            .await
        {
            Ok(client) => client,
            Err(e) => {
                warn!(error = %e, "Failed to create Discord client, disabling Discord worker");
                self.status = WorkerStatus::Stopped;
                return Ok(());
            }
        };

        self.status = WorkerStatus::Healthy;

        // Listen for ResponseEvents on the broadcast channel and send them to Discord
        let http_clone = Arc::clone(&self.http);
        let mut broadcast_rx = ctx.subscribe_events();
        tokio::spawn(async move {
            loop {
                match broadcast_rx.recv().await {
                    Ok(Event::Response(response)) => {
                        if response.platform == Platform::Discord {
                            debug!(
                                channel = %response.channel_id,
                                content_len = response.content.len(),
                                "Discord received ResponseEvent"
                            );
                            let http = http_clone.read().await;
                            if let Some(http) = http.as_ref() {
                                let channel_id: u64 =
                                    response.channel_id.parse().unwrap_or_default();
                                let channel =
                                    serenity::model::id::ChannelId::new(channel_id);

                                // Build message — reply to original if we have the ID
                                let mut builder =
                                    CreateMessage::new().content(&response.content);

                                // Reply-tag only in group channels, skip in DMs
                                if !response.is_dm {
                                    if let Some(ref reply_id) = response.reply_to_message_id {
                                        if let Ok(msg_id) = reply_id.parse::<u64>() {
                                            let msg_ref =
                                                serenity::model::id::MessageId::new(msg_id);
                                            builder = builder.reference_message(
                                                serenity::model::channel::MessageReference::from(
                                                    (channel, msg_ref),
                                                ),
                                            );
                                        }
                                    }
                                }

                                match channel.send_message(http, builder).await {
                                    Ok(_) => {
                                        info!(
                                            channel = %channel_id,
                                            "Discord response sent successfully"
                                        );
                                    }
                                    Err(e) => {
                                        error!(error = %e, "Failed to send Discord message");
                                    }
                                }
                            } else {
                                warn!(
                                    channel = %response.channel_id,
                                    "Discord HTTP client not ready (Ready event not yet received), dropping response"
                                );
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(missed = n, "Discord broadcast receiver lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        info!("Discord broadcast channel closed");
                        break;
                    }
                    _ => {} // Ignore non-Response events
                }
            }
        });

        // Run the Discord client (blocks until shutdown)
        let mut shutdown_rx = ctx.subscribe_shutdown();
        tokio::select! {
            result = client.start() => {
                match result {
                    Err(e) if e.to_string().contains("InvalidAuthentication")
                        || e.to_string().contains("Authentication failed") =>
                    {
                        warn!(
                            error = %e,
                            "Discord authentication failed — invalid token, disabling Discord worker"
                        );
                        self.status = WorkerStatus::Stopped;
                        return Ok(()); // Graceful — don't crash
                    }
                    Err(e) => {
                        error!(error = %e, "Discord client error");
                        self.status = WorkerStatus::Stopped;
                        return Err(e.into());
                    }
                    Ok(_) => {}
                }
            }
            _ = shutdown_rx.recv() => {
                info!("Discord worker received shutdown signal");
                client.shard_manager.shutdown_all().await;
            }
        }

        self.status = WorkerStatus::Stopped;
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        info!("Discord worker stopping...");
        self.status = WorkerStatus::Stopped;
        Ok(())
    }

    fn health_check(&self) -> WorkerStatus {
        self.status.clone()
    }
}

#[async_trait]
impl PlatformAdapter for DiscordWorker {
    async fn send_message(&self, channel_id: &str, content: &str) -> Result<()> {
        let http = self.http.read().await;
        if let Some(http) = http.as_ref() {
            let channel_id: u64 = channel_id.parse()?;
            let channel = serenity::model::id::ChannelId::new(channel_id);
            let builder = CreateMessage::new().content(content);
            channel.send_message(http, builder).await?;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Discord HTTP client not initialized"))
        }
    }

    async fn send_reaction(
        &self,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> Result<()> {
        let http = self.http.read().await;
        if let Some(http) = http.as_ref() {
            let channel_id: u64 = channel_id.parse()?;
            let message_id: u64 = message_id.parse()?;
            let channel = serenity::model::id::ChannelId::new(channel_id);
            let msg_id = serenity::model::id::MessageId::new(message_id);
            let reaction = serenity::model::channel::ReactionType::Unicode(emoji.to_string());
            http.create_reaction(channel, msg_id, &reaction).await?;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Discord HTTP client not initialized"))
        }
    }

    async fn send_reply(
        &self,
        channel_id: &str,
        message_id: &str,
        content: &str,
    ) -> Result<()> {
        let http = self.http.read().await;
        if let Some(http) = http.as_ref() {
            let channel_id: u64 = channel_id.parse()?;
            let message_id: u64 = message_id.parse()?;
            let channel = serenity::model::id::ChannelId::new(channel_id);
            let msg_ref = serenity::model::id::MessageId::new(message_id);
            let builder = CreateMessage::new()
                .content(content)
                .reference_message(serenity::model::channel::MessageReference::from((
                    channel, msg_ref,
                )));
            channel.send_message(http, builder).await?;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Discord HTTP client not initialized"))
        }
    }
}
