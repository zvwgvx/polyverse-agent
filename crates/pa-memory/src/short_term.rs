use std::collections::HashMap;

use chrono::{DateTime, Utc};
use tracing::{debug, info};

use crate::types::{ConversationKey, MemoryMessage};

/// A single conversation session with all its messages.
struct Session {
    /// All messages in this session (not limited by sliding window)
    messages: Vec<MemoryMessage>,
    /// When the last message was received
    last_active: DateTime<Utc>,
    /// Session start time
    started_at: DateTime<Utc>,
}

impl Session {
    fn new() -> Self {
        let now = Utc::now();
        Self {
            messages: Vec::new(),
            last_active: now,
            started_at: now,
        }
    }

    /// Check if session has timed out based on adaptive timeout.
    fn is_expired(&self, now: DateTime<Utc>, base_timeout_secs: i64) -> bool {
        // Adaptive timeout: longer conversations get more grace period
        // base + (message_count * 1 min), capped at 90 min
        let adaptive_secs = base_timeout_secs
            + (self.messages.len() as i64 * 60)
                .min(90 * 60 - base_timeout_secs);

        let elapsed = now - self.last_active;
        elapsed.num_seconds() > adaptive_secs
    }
}

/// Configuration for short-term memory.
#[derive(Debug, Clone)]
pub struct ShortTermConfig {
    /// Base session timeout in seconds (default: 30 minutes)
    pub base_timeout_secs: i64,
    /// Maximum messages to inject into prompt (token budget)
    pub max_prompt_messages: usize,
}

impl Default for ShortTermConfig {
    fn default() -> Self {
        Self {
            base_timeout_secs: 20 * 60, // 20 minutes
            max_prompt_messages: 20,
        }
    }
}

/// Short-term memory: session-based conversation context living in RAM.
///
/// Each conversation (platform + channel) has its own session.
/// Sessions expire after an adaptive timeout.
/// When building a prompt, messages are scored and the top-N are selected.
pub struct ShortTermMemory {
    /// Active sessions keyed by conversation
    sessions: HashMap<ConversationKey, Session>,
    /// Configuration
    config: ShortTermConfig,
}

impl ShortTermMemory {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            config: ShortTermConfig::default(),
        }
    }

    pub fn with_config(config: ShortTermConfig) -> Self {
        Self {
            sessions: HashMap::new(),
            config,
        }
    }

    /// Push a message into the appropriate conversation session.
    /// If the session has expired, starts a new one.
    /// Returns messages from the expired session (if any) for persistence.
    pub fn push(&mut self, msg: MemoryMessage) -> Option<Vec<MemoryMessage>> {
        let key = ConversationKey::new(msg.platform, msg.channel_id.clone());
        let now = msg.timestamp;
        let mut expired_messages = None;

        // Check if existing session has expired
        if let Some(session) = self.sessions.get(&key) {
            if session.is_expired(now, self.config.base_timeout_secs) {
                // Session expired — take the old messages for persistence
                let old = self.sessions.remove(&key).unwrap();
                info!(
                    conversation = %key,
                    messages = old.messages.len(),
                    "Session expired, starting new session"
                );
                expired_messages = Some(old.messages);
            }
        }

        // Get or create session
        let session = self.sessions.entry(key).or_insert_with(Session::new);
        session.last_active = now;
        session.messages.push(msg);

        expired_messages
    }

    /// Load conversation history from persistent store on startup.
    pub fn load_history(&mut self, messages: Vec<MemoryMessage>) {
        for msg in messages {
            let key = ConversationKey::new(msg.platform, msg.channel_id.clone());
            let session = self.sessions.entry(key).or_insert_with(Session::new);
            
            if session.messages.is_empty() || msg.timestamp > session.last_active {
                session.last_active = msg.timestamp;
            }
            if session.messages.is_empty() || msg.timestamp < session.started_at {
                session.started_at = msg.timestamp;
            }
            
            session.messages.push(msg);
        }
        
        // Ensure chronological order
        for session in self.sessions.values_mut() {
            session.messages.sort_by_key(|m| m.timestamp);
        }
    }

    /// Get conversation context formatted for LLM prompt injection.
    /// Selects the most relevant messages using scoring.
    pub fn get_context_for_prompt(&self, key: &ConversationKey) -> Vec<&MemoryMessage> {
        let session = match self.sessions.get(key) {
            Some(s) => s,
            None => return Vec::new(),
        };

        if session.messages.is_empty() {
            return Vec::new();
        }

        // Score each message for prompt inclusion
        let now = Utc::now();
        let total = session.messages.len();
        let mut scored: Vec<(usize, f32)> = session
            .messages
            .iter()
            .enumerate()
            .map(|(i, msg)| {
                let score = Self::prompt_score(msg, i, total, now);
                (i, score)
            })
            .collect();

        // Sort by score descending
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Take top N
        let limit = self.config.max_prompt_messages.min(total);
        let mut selected_indices: Vec<usize> =
            scored.iter().take(limit).map(|(i, _)| *i).collect();

        // Re-sort by chronological order for natural conversation flow
        selected_indices.sort();

        selected_indices
            .iter()
            .map(|i| &session.messages[*i])
            .collect()
    }

    /// Format context as a string for injection into LLM prompt (legacy).
    pub fn format_context(&self, key: &ConversationKey) -> Option<String> {
        let messages = self.get_context_for_prompt(key);
        if messages.is_empty() {
            return None;
        }

        let formatted: Vec<String> = messages
            .iter()
            .map(|msg| {
                let name = if msg.is_bot_response {
                    "Ryuuko"
                } else {
                    &msg.username
                };
                format!("{}: {}", name, msg.content)
            })
            .collect();

        Some(formatted.join("\n"))
    }

    /// Get conversation history as structured `(role, username, content)` triples for LLM.
    ///
    /// - Excludes current message by ID (avoids duplicate in history + user role).
    /// - User messages: content prefixed with `[username]:` AND username returned for `name` field.
    /// - Assistant messages: NO prefix in content (avoids `[Ryuuko]:` leak into output).
    /// - Returns in chronological order.
    pub fn get_history_for_prompt(
        &self,
        key: &ConversationKey,
        exclude_id: &str,
    ) -> Vec<(String, String, String)> {
        let session = match self.sessions.get(key) {
            Some(s) => s,
            None => return Vec::new(),
        };

        if session.messages.is_empty() {
            return Vec::new();
        }

        let total = session.messages.len();
        let now = Utc::now();

        // Score each message (excluding current one)
        let mut scored: Vec<(usize, f32)> = session
            .messages
            .iter()
            .enumerate()
            .filter(|(_, msg)| msg.id != exclude_id)
            .map(|(i, msg)| {
                let score = Self::prompt_score(msg, i, total, now);
                (i, score)
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let limit = self.config.max_prompt_messages.min(scored.len());
        let mut selected: Vec<usize> = scored.iter().take(limit).map(|(i, _)| *i).collect();
        selected.sort(); // chronological

        selected
            .iter()
            .map(|i| {
                let msg = &session.messages[*i];
                let clean = strip_mention_tags(&msg.content);

                if msg.is_bot_response {
                    // Assistant: NO name field — prevents model echoing "Ryuuko:" prefix
                    (
                        "assistant".to_string(),
                        String::new(), // empty = no name field on assistant
                        clean,
                    )
                } else {
                    // User: identity via name field only, NO [username]: prefix in content
                    (
                        "user".to_string(),
                        msg.username.clone(),
                        clean,
                    )
                }
            })
            .collect()
    }

    /// Flush all expired sessions. Returns expired messages for persistence.
    pub fn flush_expired(&mut self) -> Vec<(ConversationKey, Vec<MemoryMessage>)> {
        let now = Utc::now();
        let timeout = self.config.base_timeout_secs;

        let expired_keys: Vec<ConversationKey> = self
            .sessions
            .iter()
            .filter(|(_, session)| session.is_expired(now, timeout))
            .map(|(key, _)| key.clone())
            .collect();

        let mut result = Vec::new();
        for key in expired_keys {
            if let Some(session) = self.sessions.remove(&key) {
                debug!(
                    conversation = %key,
                    messages = session.messages.len(),
                    "Flushing expired session"
                );
                result.push((key, session.messages));
            }
        }

        result
    }

    /// Get the number of active sessions.
    pub fn active_session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Get total messages across all sessions.
    pub fn total_messages(&self) -> usize {
        self.sessions.values().map(|s| s.messages.len()).sum()
    }

    /// Score a message for prompt inclusion.
    /// Higher score = more likely to be included.
    fn prompt_score(
        msg: &MemoryMessage,
        index: usize,
        total: usize,
        _now: DateTime<Utc>,
    ) -> f32 {
        // Recency: recent messages score higher
        // Linear scale: last message = 1.0, first = 0.1
        let recency = if total <= 1 {
            1.0
        } else {
            0.1 + 0.9 * (index as f32 / (total - 1) as f32)
        };

        // Importance from the message itself
        let importance = msg.importance;

        // Bot responses are important for coherence
        let bot_bonus = if msg.is_bot_response { 0.2 } else { 0.0 };

        // Mentions/DMs are high priority
        let mention_bonus = if msg.is_mention { 0.15 } else { 0.0 };

        // Weighted combination
        let score = (recency * 0.5) + (importance * 0.3) + bot_bonus + mention_bonus;

        score.min(1.0)
    }
}

impl Default for ShortTermMemory {
    fn default() -> Self {
        Self::new()
    }
}

/// Strip Discord mention tags like `<@123456>` from message content.
fn strip_mention_tags(content: &str) -> String {
    // Remove <@user_id> and <@!user_id> patterns
    let mut result = String::with_capacity(content.len());
    let chars: Vec<char> = content.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<' && i + 1 < chars.len() && chars[i + 1] == '@' {
            // Find closing >
            if let Some(end) = chars[i..].iter().position(|&c| c == '>') {
                i += end + 1; // skip past the >
                // Trim any leading space after the mention
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

#[cfg(test)]
mod tests {
    use super::*;
    use pa_core::event::Platform;

    fn make_msg(channel: &str, user: &str, content: &str, is_mention: bool) -> MemoryMessage {
        MemoryMessage {
            id: uuid::Uuid::new_v4().to_string(),
            platform: Platform::Discord,
            channel_id: channel.to_string(),
            user_id: user.to_string(),
            username: user.to_string(),
            content: content.to_string(),
            is_mention,
            reply_to_user: None,
            is_bot_response: false,
            timestamp: Utc::now(),
            importance: if is_mention { 0.7 } else { 0.3 },
        }
    }

    #[test]
    fn test_push_and_context() {
        let mut mem = ShortTermMemory::new();

        let msg1 = make_msg("ch1", "Alice", "hello", false);
        let msg2 = make_msg("ch1", "Bob", "hey ryuuko", true);
        let msg3 = make_msg("ch1", "Alice", "what's up", false);

        mem.push(msg1);
        mem.push(msg2);
        mem.push(msg3);

        let key = ConversationKey::new(Platform::Discord, "ch1".to_string());
        let context = mem.get_context_for_prompt(&key);
        assert_eq!(context.len(), 3);

        assert_eq!(mem.active_session_count(), 1);
        assert_eq!(mem.total_messages(), 3);
    }

    #[test]
    fn test_separate_conversations() {
        let mut mem = ShortTermMemory::new();

        mem.push(make_msg("ch1", "Alice", "hello", false));
        mem.push(make_msg("ch2", "Bob", "hey", false));

        assert_eq!(mem.active_session_count(), 2);
    }

    #[test]
    fn test_format_context() {
        let mut mem = ShortTermMemory::new();

        mem.push(make_msg("ch1", "Alice", "xin chào", false));
        let mut bot_msg = make_msg("ch1", "Ryuuko", "chào nha", false);
        bot_msg.is_bot_response = true;
        bot_msg.username = "Ryuuko".to_string();
        mem.push(bot_msg);

        let key = ConversationKey::new(Platform::Discord, "ch1".to_string());
        let formatted = mem.format_context(&key).unwrap();
        assert!(formatted.contains("Alice: xin chào"));
        assert!(formatted.contains("Ryuuko: chào nha"));
    }

    #[test]
    fn test_max_prompt_messages() {
        let config = ShortTermConfig {
            max_prompt_messages: 3,
            ..Default::default()
        };
        let mut mem = ShortTermMemory::with_config(config);

        // Push 10 messages
        for i in 0..10 {
            mem.push(make_msg("ch1", "User", &format!("msg {}", i), false));
        }

        let key = ConversationKey::new(Platform::Discord, "ch1".to_string());
        let context = mem.get_context_for_prompt(&key);
        assert_eq!(context.len(), 3); // Limited to max_prompt_messages
    }
}
