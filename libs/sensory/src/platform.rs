use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait PlatformAdapter: kernel::Worker {
    async fn send_message(&self, channel_id: &str, content: &str) -> Result<()>;

    async fn send_reaction(
        &self,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> Result<()>;

    async fn send_reply(
        &self,
        channel_id: &str,
        message_id: &str,
        content: &str,
    ) -> Result<()>;
}
