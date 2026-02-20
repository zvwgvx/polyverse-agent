//! # pa-sensory
//!
//! Sensory workers (platform adapters) for the Polyverse Agent.
//! Provides Discord and Telegram integration.

pub mod buffer;
pub mod discord;
pub mod platform;
pub mod telegram;

// Re-exports
pub use discord::DiscordWorker;
pub use platform::PlatformAdapter;
pub use telegram::TelegramWorker;
