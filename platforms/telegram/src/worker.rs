use std::sync::Arc;
use anyhow::Result;
use base64::Engine as _;
use bytes::BytesMut;
use futures_util::StreamExt;
use kernel::event::{
    ImageAttachment, Platform, RawEvent, MAX_IMAGE_ATTACHMENTS_PER_MESSAGE,
    MAX_IMAGE_ATTACHMENT_BYTES,
};
use sensory::relay::RelayClient;
use teloxide::net::Download;
use teloxide::prelude::*;
use tracing::{debug, error, info, warn};

async fn extract_photo_attachments(bot: &Bot, msg: &Message) -> Vec<ImageAttachment> {
    let mut attachments = Vec::new();

    if let Some(photos) = msg.photo() {
        if let Some(photo) = photos.last() {
            if let Ok(file) = bot.get_file(photo.file.id.clone()).await {
                let mut stream = bot.download_file_stream(&file.path);
                let mut bytes = BytesMut::new();
                while let Some(chunk) = stream.next().await {
                    match chunk {
                        Ok(chunk) => {
                            if bytes.len() + chunk.len() > MAX_IMAGE_ATTACHMENT_BYTES {
                                return attachments;
                            }
                            bytes.extend_from_slice(&chunk);
                        }
                        Err(error) => {
                            warn!(error = %error, "Failed to download Telegram photo attachment");
                            return attachments;
                        }
                    }
                }
                attachments.push(ImageAttachment {
                    mime_type: "image/jpeg".to_string(),
                    filename: Some(format!("telegram-photo-{}.jpg", msg.id.0)),
                    source_url: None,
                    data_base64: base64::prelude::BASE64_STANDARD.encode(bytes.freeze()),
                });
            }
        }
    }

    if attachments.len() >= MAX_IMAGE_ATTACHMENTS_PER_MESSAGE {
        return attachments;
    }

    if let Some(document) = msg.document() {
        let Some(mime) = document.mime_type.as_ref().map(|value| value.to_string()) else {
            return attachments;
        };
        if !ImageAttachment::is_supported_image_mime(&mime) {
            return attachments;
        }
        if document.file.size as usize > MAX_IMAGE_ATTACHMENT_BYTES {
            return attachments;
        }
        match bot.get_file(document.file.id.clone()).await {
            Ok(file) => {
                let mut stream = bot.download_file_stream(&file.path);
                let mut bytes = BytesMut::new();
                while let Some(chunk) = stream.next().await {
                    match chunk {
                        Ok(chunk) => {
                            if bytes.len() + chunk.len() > MAX_IMAGE_ATTACHMENT_BYTES {
                                return attachments;
                            }
                            bytes.extend_from_slice(&chunk);
                        }
                        Err(error) => {
                            warn!(error = %error, "Failed to download Telegram document attachment");
                            return attachments;
                        }
                    }
                }
                attachments.push(ImageAttachment {
                    mime_type: mime,
                    filename: document.file_name.clone(),
                    source_url: None,
                    data_base64: base64::prelude::BASE64_STANDARD.encode(bytes.freeze()),
                });
            }
            Err(error) => {
                warn!(error = %error, "Failed to resolve Telegram document attachment");
            }
        }
    }

    attachments
}

pub struct TelegramWorker {
    token: String,
}

impl TelegramWorker {
    pub fn new(token: String) -> Self {
        Self { token }
    }

    pub async fn run(&mut self) -> Result<()> {
        info!("Telegram worker starting...");

        if self.token.is_empty()
            || self.token == "YOUR_TELEGRAM_BOT_TOKEN"
            || self.token.starts_with("your_")
        {
            warn!("Telegram token is not configured, exiting");
            return Ok(());
        }

        let bot = Bot::new(&self.token);
        let me = match bot.get_me().await {
            Ok(me) => me,
            Err(e) => {
                warn!(error = %e, "Telegram authentication failed — invalid token");
                return Ok(());
            }
        };

        let bot_username = me.username().to_string();
        info!(bot_name = %bot_username, "Telegram bot connected");

        let mut relay_client = RelayClient::connect_default().await?;
        let relay_client_shared = Arc::new(RelayClient::connect_default().await?);

        let bot_clone = bot.clone();
        tokio::spawn(async move {
            while let Some(response) = relay_client.recv_response().await {
                if response.platform == Platform::Telegram {
                    let chat_id: i64 = response.channel_id.parse().unwrap_or_default();
                    let req = bot_clone.send_message(ChatId(chat_id), &response.content);
                    match req.await {
                        Ok(_) => info!(chat = %chat_id, "Telegram response sent successfully"),
                        Err(e) => error!(error = %e, "Failed to send Telegram message"),
                    }
                }
            }
            warn!("Relay response stream closed");
        });

        let handler = Update::filter_message().endpoint(
            move |bot: Bot,
                  msg: Message,
                  relay: Arc<RelayClient>,
                  bot_un: String| async move {
                let text = msg.caption().or_else(|| msg.text()).unwrap_or_default().to_string();
                let has_images = msg.photo().is_some() || msg.document().is_some();
                if text.starts_with('/') && !has_images {
                    return Ok::<(), anyhow::Error>(());
                }
                if text.is_empty() && !has_images {
                    return Ok::<(), anyhow::Error>(());
                }

                let user = msg.from.as_ref().map(|u| u.first_name.clone()).unwrap_or_else(|| "Unknown".to_string());
                let user_id = msg.from.as_ref().map(|u| u.id.0.to_string()).unwrap_or_default();
                let is_dm = msg.chat.is_private();
                let mention_tag = format!("@{}", bot_un);
                let is_mention = is_dm || text.contains(&mention_tag) || msg.entities().map(|entities| {
                    entities.iter().any(|e| {
                        e.kind == teloxide::types::MessageEntityKind::Mention
                            && text.get(e.offset..e.offset + e.length).map(|s| s.eq_ignore_ascii_case(&mention_tag)).unwrap_or(false)
                    })
                }).unwrap_or(false);

                if is_mention {
                    info!(user = %user, chat = %msg.chat.id, content = %text, dm = is_dm, "[MENTION] Bot was tagged on Telegram");
                } else {
                    debug!(user = %user, chat = %msg.chat.id, content = %text, "Telegram message received");
                }

                let raw = RawEvent {
                    platform: Platform::Telegram,
                    channel_id: msg.chat.id.0.to_string(),
                    message_id: msg.id.0.to_string(),
                    user_id,
                    username: user,
                    content: text,
                    attachments: extract_photo_attachments(&bot, &msg).await,
                    is_mention,
                    is_dm: msg.chat.is_private(),
                    timestamp: chrono::Utc::now(),
                };

                if let Err(e) = relay.ingest(raw).await {
                    error!(error = %e, "Failed to ingest message to relay");
                }
                Ok::<(), anyhow::Error>(())
            },
        );

        let mut dispatcher = Dispatcher::builder(bot, handler)
            .dependencies(dptree::deps![relay_client_shared, bot_username])
            .default_handler(|_| async {})
            .build();

        dispatcher.dispatch().await;

        info!("Telegram worker stopped");
        Ok(())
    }
}
