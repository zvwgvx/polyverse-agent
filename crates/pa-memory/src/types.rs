use chrono::{DateTime, Utc};
use pa_core::event::{Platform, RawEvent};
use serde::{Deserialize, Serialize};

/// A normalized message stored in memory.
/// Converted from RawEvent with additional metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryMessage {
    /// Unique message ID (from the platform)
    pub id: String,
    /// Source platform
    pub platform: Platform,
    /// Channel / chat ID
    pub channel_id: String,
    /// User who sent the message
    pub user_id: String,
    /// Display name
    pub username: String,
    /// Message text
    pub content: String,
    /// Whether this was a mention/DM (triggered LLM response)
    pub is_mention: bool,
    /// Whether this is Ryuuko's own response
    pub is_bot_response: bool,
    /// Who Ryuuko is replying to (set on bot responses)
    pub reply_to_user: Option<String>,
    /// When the message was received
    pub timestamp: DateTime<Utc>,
    /// Importance score (0.0 - 1.0)
    /// Higher = more likely to be included in prompt
    pub importance: f32,
}

/// Key to identify a conversation (platform + channel).
/// All messages in the same conversation share the same key.
#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationKey {
    pub platform: Platform,
    pub channel_id: String,
}

impl ConversationKey {
    pub fn new(platform: Platform, channel_id: String) -> Self {
        Self {
            platform,
            channel_id,
        }
    }

    pub fn from_raw(raw: &RawEvent) -> Self {
        Self {
            platform: raw.platform,
            channel_id: raw.channel_id.clone(),
        }
    }
}

impl std::fmt::Display for ConversationKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.platform, self.channel_id)
    }
}

impl MemoryMessage {
    /// Create a MemoryMessage from a RawEvent with computed importance.
    pub fn from_raw(raw: &RawEvent) -> Self {
        let importance = Self::compute_importance(raw);

        Self {
            id: raw.message_id.clone(),
            platform: raw.platform,
            channel_id: raw.channel_id.clone(),
            user_id: raw.user_id.clone(),
            username: raw.username.clone(),
            content: Self::strip_mention_tags(&raw.content),
            is_mention: raw.is_mention,
            is_bot_response: false,
            reply_to_user: None,
            timestamp: raw.timestamp,
            importance,
        }
    }

    /// Create a MemoryMessage for Ryuuko's own response.
    pub fn bot_response(
        platform: Platform,
        channel_id: String,
        content: String,
        _reply_to: Option<String>,
        reply_to_user: Option<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            platform,
            channel_id,
            user_id: "ryuuko".to_string(),
            username: "Ryuuko".to_string(),
            content,
            is_mention: false,
            is_bot_response: true,
            reply_to_user,
            timestamp: Utc::now(),
            importance: 0.6,
        }
    }

    /// Compute importance score based on message characteristics.
    fn compute_importance(raw: &RawEvent) -> f32 {
        let mut score: f32 = 0.3; // Base score for any message

        // DM / mention = more important (directed at Ryuuko)
        if raw.is_mention {
            score += 0.4;
        }

        // Longer messages tend to carry more info
        let word_count = raw.content.split_whitespace().count();
        if word_count > 10 {
            score += 0.1;
        }

        // Questions are important (contain ?)
        if raw.content.contains('?') {
            score += 0.1;
        }

        score.min(1.0)
    }

    /// Strip Discord mention tags (e.g. `<@123456>`) from message content.
    /// Returns the cleaned content, trimmed of leading/trailing whitespace.
    fn strip_mention_tags(content: &str) -> String {
        let mut result = String::with_capacity(content.len());
        let chars: Vec<char> = content.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '<' && i + 1 < chars.len() && chars[i + 1] == '@' {
                if let Some(end) = chars[i..].iter().position(|&c| c == '>') {
                    i += end + 1;
                    // Skip following space
                    while i < chars.len() && chars[i] == ' ' {
                        i += 1;
                    }
                    continue;
                }
            }
            result.push(chars[i]);
            i += 1;
        }
        result.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_message_from_raw() {
        let raw = RawEvent {
            platform: Platform::Discord,
            channel_id: "ch1".to_string(),
            message_id: "m1".to_string(),
            user_id: "u1".to_string(),
            username: "TestUser".to_string(),
            content: "Hello Ryuuko!".to_string(),
            is_mention: true,
            is_dm: false,
            timestamp: Utc::now(),
        };

        let msg = MemoryMessage::from_raw(&raw);
        assert_eq!(msg.id, "m1");
        assert!(msg.is_mention);
        assert!(!msg.is_bot_response);
        assert!(msg.importance >= 0.7); // mention bonus
    }

    #[test]
    fn test_conversation_key() {
        let key = ConversationKey::new(Platform::Discord, "ch1".to_string());
        assert_eq!(key.to_string(), "Discord:ch1");
    }

    #[test]
    fn test_bot_response() {
        let msg = MemoryMessage::bot_response(
            Platform::Telegram,
            "chat123".to_string(),
            "xin chào".to_string(),
            None,
            Some("zvwgvx".to_string()),
        );
        assert!(msg.is_bot_response);
        assert_eq!(msg.username, "Ryuuko");
    }
}
