use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use base64::Engine as _;
use kernel::event::{
    ImageAttachment, Platform, RawEvent, MAX_IMAGE_ATTACHMENTS_PER_MESSAGE,
    MAX_IMAGE_ATTACHMENT_BYTES,
};
use serenity::all::{
    Context, CreateMessage, EventHandler, GatewayIntents, Message, Ready,
};
use serenity::Client;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use sensory::relay::RelayClient;

async fn extract_image_attachments(msg: &Message) -> Vec<ImageAttachment> {
    let mut images = Vec::new();

    for attachment in msg.attachments.iter().take(MAX_IMAGE_ATTACHMENTS_PER_MESSAGE) {
        let Some(mime_type) = attachment.content_type.clone() else {
            continue;
        };
        if !ImageAttachment::is_supported_image_mime(&mime_type) {
            continue;
        }
        if attachment.size as usize > MAX_IMAGE_ATTACHMENT_BYTES {
            continue;
        }

        match attachment.download().await {
            Ok(bytes) => {
                if bytes.len() > MAX_IMAGE_ATTACHMENT_BYTES {
                    continue;
                }
                images.push(ImageAttachment {
                    mime_type,
                    filename: Some(attachment.filename.clone()),
                    source_url: Some(attachment.url.clone()),
                    data_base64: base64::prelude::BASE64_STANDARD.encode(bytes),
                });
            }
            Err(error) => {
                warn!(
                    attachment = %attachment.filename,
                    error = %error,
                    "Failed to download Discord image attachment"
                );
            }
        }
    }

    images
}

async fn build_raw_event(msg: &Message, is_mention: bool, is_dm: bool) -> RawEvent {
    let attachments = extract_image_attachments(msg).await;
    RawEvent {
        platform: Platform::Discord,
        channel_id: msg.channel_id.to_string(),
        message_id: msg.id.to_string(),
        user_id: msg.author.id.to_string(),
        username: msg.author.name.clone(),
        content: msg.content.clone(),
        attachments,
        is_mention,
        is_dm,
        timestamp: chrono::Utc::now(),
    }
}

pub struct DiscordWorker {
    token: String,
    http: Arc<RwLock<Option<Arc<serenity::http::Http>>>>,
}

impl DiscordWorker {
    pub fn new(token: String) -> Self {
        Self {
            token,
            http: Arc::new(RwLock::new(None)),
        }
    }
}

struct DiscordHandler {
    relay: Arc<RelayClient>,
    http_store: Arc<RwLock<Option<Arc<serenity::http::Http>>>>,
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
        if msg.author.bot {
            return;
        }

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

        let raw = build_raw_event(&msg, is_mention, is_dm).await;

        if let Err(e) = self.relay.ingest(raw).await {
            error!(error = %e, "Failed to ingest message to relay");
        }
    }

    async fn typing_start(&self, _ctx: Context, _event: serenity::all::TypingStartEvent) {
        // typing detection is a no-op
    }
}

impl DiscordWorker {
    pub async fn run(&mut self) -> Result<()> {
        info!("Discord worker starting...");

        if self.token.is_empty()
            || self.token == "YOUR_DISCORD_BOT_TOKEN"
            || self.token.starts_with("your_")
        {
            warn!("Discord token is not configured, exiting");
            return Ok(());
        }

        let http_store = Arc::clone(&self.http);
        let bot_user_id = Arc::new(RwLock::new(None));

        // To handle response_rx which is in RelayClient, we need to extract it
        // Or we can just spawn the listener here, but RelayClient borrows response_rx.
        // Let's modify RelayClient to return the receiver so we can spawn it separately,
        // or just loop it here.
        // Actually, we can't share RelayClient if it has mutable recv_response.
        // So we will split it manually.

        let relay_client_shared = Arc::new(RelayClient::connect_default().await?);

        let handler = DiscordHandler {
            relay: Arc::clone(&relay_client_shared),
            http_store: Arc::clone(&http_store),
            bot_user_id: Arc::clone(&bot_user_id),
        };

        let intents = GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT
            | GatewayIntents::GUILD_MESSAGE_TYPING
            | GatewayIntents::DIRECT_MESSAGE_TYPING;

        let mut client = match Client::builder(&self.token, intents)
            .event_handler(handler)
            .await
        {
            Ok(client) => client,
            Err(e) => {
                warn!(error = %e, "Failed to create Discord client");
                return Ok(());
            }
        };

        let http_clone = Arc::clone(&self.http);

        // Spawn response loop
        let mut relay_reader = RelayClient::connect_default().await?;
        tokio::spawn(async move {
            while let Some(response) = relay_reader.recv_response().await {
                if response.platform == Platform::Discord {
                    debug!(
                        channel = %response.channel_id,
                        content_len = response.content.len(),
                        "Discord received ResponseEvent from relay"
                    );
                    let http = http_clone.read().await;
                    if let Some(http) = http.as_ref() {
                        let channel_id: u64 =
                            response.channel_id.parse().unwrap_or_default();
                        let channel =
                            serenity::model::id::ChannelId::new(channel_id);

                        let mut builder =
                            CreateMessage::new().content(&response.content);

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
                            "Discord HTTP client not ready, dropping response"
                        );
                    }
                }
            }
            warn!("Relay response stream closed");
        });

        match client.start().await {
            Err(e) if e.to_string().contains("InvalidAuthentication")
                || e.to_string().contains("Authentication failed") =>
            {
                warn!(
                    error = %e,
                    "Discord authentication failed — invalid token"
                );
                return Ok(());
            }
            Err(e) => {
                error!(error = %e, "Discord client error");
                return Err(e.into());
            }
            Ok(_) => {}
        }

        Ok(())
    }
}
