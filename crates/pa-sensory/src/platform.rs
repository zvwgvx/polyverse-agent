use anyhow::Result;
use async_trait::async_trait;

/// PlatformAdapter extends the Worker trait with platform-specific
/// messaging capabilities (send messages, reactions, etc.).
///
/// Every sensory worker (Discord, Telegram, CLI...) implements this
/// trait on top of the base Worker trait.
#[async_trait]
pub trait PlatformAdapter: pa_core::Worker {
    /// Send a text message to a channel/chat.
    async fn send_message(&self, channel_id: &str, content: &str) -> Result<()>;

    /// Send a reaction/emoji to a specific message.
    async fn send_reaction(
        &self,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> Result<()>;

    /// Send a reply to a specific message.
    async fn send_reply(
        &self,
        channel_id: &str,
        message_id: &str,
        content: &str,
    ) -> Result<()>;
}
