use std::collections::HashMap;

use chrono::{DateTime, Utc};
use pa_core::get_agent_profile;
use serde::Serialize;
use tracing::{debug, info};

use crate::types::{ConversationKey, MemoryMessage};

struct Session {
    messages: Vec<MemoryMessage>,
    last_active: DateTime<Utc>,
    started_at: DateTime<Utc>,
    already_ingested: bool,
}

impl Session {
    fn new() -> Self {
        let now = Utc::now();
        Self {
            messages: Vec::new(),
            last_active: now,
            started_at: now,
            already_ingested: false,
        }
    }

    fn is_expired(&self, now: DateTime<Utc>, base_timeout_secs: i64) -> bool {
        let adaptive_secs = base_timeout_secs
            + (self.messages.len() as i64 * 60)
                .min(90 * 60 - base_timeout_secs);

        let elapsed = now - self.last_active;
        elapsed.num_seconds() > adaptive_secs
    }
}

#[derive(Debug, Clone)]
pub struct ShortTermConfig {
    pub base_timeout_secs: i64,
    pub max_prompt_messages: usize,
}

impl Default for ShortTermConfig {
    fn default() -> Self {
        Self {
            base_timeout_secs: 20 * 60,
            max_prompt_messages: 20,
        }
    }
}

pub struct ShortTermMemory {
    sessions: HashMap<ConversationKey, Session>,
    config: ShortTermConfig,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActiveSessionSnapshot {
    pub conversation: String,
    pub platform: String,
    pub channel_id: String,
    pub message_count: usize,
    pub started_at: DateTime<Utc>,
    pub last_active: DateTime<Utc>,
    pub participants: Vec<String>,
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

    pub fn push(&mut self, msg: MemoryMessage) -> Option<Vec<MemoryMessage>> {
        let key = ConversationKey::new(msg.platform, msg.channel_id.clone());
        let now = msg.timestamp;
        let mut expired_messages = None;

        if let Some(session) = self.sessions.get(&key) {
            if session.is_expired(now, self.config.base_timeout_secs) {
                let old = self.sessions.remove(&key).unwrap();
                if old.already_ingested {
                    info!(
                        conversation = %key,
                        messages = old.messages.len(),
                        "Session expired, flushed to store (boot-loaded, skipping RAG)"
                    );
                } else {
                    info!(
                        conversation = %key,
                        messages = old.messages.len(),
                        "Session expired, starting new session"
                    );
                    expired_messages = Some(old.messages);
                }
            }
        }

        let session = self.sessions.entry(key).or_insert_with(Session::new);
        session.last_active = now;
        session.messages.push(msg);

        expired_messages
    }

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

        for session in self.sessions.values_mut() {
            session.messages.sort_by_key(|m| m.timestamp);
        }
    }

    pub fn mark_all_persisted(&mut self) {
        for session in self.sessions.values_mut() {
            session.already_ingested = true;
        }
    }

    pub fn get_context_for_prompt(&self, key: &ConversationKey) -> Vec<&MemoryMessage> {
        let session = match self.sessions.get(key) {
            Some(s) => s,
            None => return Vec::new(),
        };

        if session.messages.is_empty() {
            return Vec::new();
        }

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

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let limit = self.config.max_prompt_messages.min(total);
        let mut selected_indices: Vec<usize> =
            scored.iter().take(limit).map(|(i, _)| *i).collect();

        selected_indices.sort();

        selected_indices
            .iter()
            .map(|i| &session.messages[*i])
            .collect()
    }

    pub fn format_context(&self, key: &ConversationKey) -> Option<String> {
        let messages = self.get_context_for_prompt(key);
        if messages.is_empty() {
            return None;
        }
        let profile = get_agent_profile();

        let formatted: Vec<String> = messages
            .iter()
            .map(|msg| {
                let name = if msg.is_bot_response {
                    profile.display_name.as_str()
                } else {
                    &msg.username
                };
                format!("{}: {}", name, msg.content)
            })
            .collect();

        Some(formatted.join("\n"))
    }

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
        selected.sort();

        selected
            .iter()
            .map(|i| {
                let msg = &session.messages[*i];
                let clean = strip_mention_tags(&msg.content);

                if msg.is_bot_response {
                    (
                        "assistant".to_string(),
                        String::new(),
                        clean,
                    )
                } else {
                    (
                        "user".to_string(),
                        msg.username.clone(),
                        clean,
                    )
                }
            })
            .collect()
    }

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
                if session.already_ingested {
                    debug!(
                        conversation = %key,
                        messages = session.messages.len(),
                        "Dropping boot-loaded expired session (already ingested)"
                    );
                    continue;
                }
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

    pub fn active_session_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn total_messages(&self) -> usize {
        self.sessions.values().map(|s| s.messages.len()).sum()
    }

    pub fn active_sessions_snapshot(&self) -> Vec<ActiveSessionSnapshot> {
        let mut snapshots: Vec<ActiveSessionSnapshot> = self
            .sessions
            .iter()
            .map(|(key, session)| {
                let mut participants: Vec<String> = session
                    .messages
                    .iter()
                    .filter(|msg| !msg.is_bot_response)
                    .map(|msg| msg.username.clone())
                    .collect();
                participants.sort();
                participants.dedup();

                ActiveSessionSnapshot {
                    conversation: key.to_string(),
                    platform: key.platform.to_string(),
                    channel_id: key.channel_id.clone(),
                    message_count: session.messages.len(),
                    started_at: session.started_at,
                    last_active: session.last_active,
                    participants,
                }
            })
            .collect();

        snapshots.sort_by(|a, b| b.last_active.cmp(&a.last_active));
        snapshots
    }

    pub fn recent_messages(&self, limit: usize) -> Vec<MemoryMessage> {
        let mut messages: Vec<MemoryMessage> = self
            .sessions
            .values()
            .flat_map(|session| session.messages.iter().cloned())
            .collect();
        messages.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        messages.truncate(limit);
        messages
    }

    fn prompt_score(
        msg: &MemoryMessage,
        index: usize,
        total: usize,
        _now: DateTime<Utc>,
    ) -> f32 {
        let recency = if total <= 1 {
            1.0
        } else {
            0.1 + 0.9 * (index as f32 / (total - 1) as f32)
        };

        let importance = msg.importance;

        let bot_bonus = if msg.is_bot_response { 0.2 } else { 0.0 };

        let mention_bonus = if msg.is_mention { 0.15 } else { 0.0 };

        let score = (recency * 0.5) + (importance * 0.3) + bot_bonus + mention_bonus;

        score.min(1.0)
    }
}

impl Default for ShortTermMemory {
    fn default() -> Self {
        Self::new()
    }
}

fn strip_mention_tags(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let chars: Vec<char> = content.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<' && i + 1 < chars.len() && chars[i + 1] == '@' {
            if let Some(end) = chars[i..].iter().position(|&c| c == '>') {
                i += end + 1;
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
        let msg2 = make_msg("ch1", "Bob", "hey agent", true);
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
        let profile = get_agent_profile();
        let mut mem = ShortTermMemory::new();

        mem.push(make_msg("ch1", "Alice", "xin chào", false));
        let mut bot_msg = make_msg("ch1", &profile.display_name, "chào nha", false);
        bot_msg.is_bot_response = true;
        bot_msg.username = profile.display_name.clone();
        mem.push(bot_msg);

        let key = ConversationKey::new(Platform::Discord, "ch1".to_string());
        let formatted = mem.format_context(&key).unwrap();
        assert!(formatted.contains("Alice: xin chào"));
        assert!(formatted.contains(&format!("{}: chào nha", profile.display_name)));
    }

    #[test]
    fn test_max_prompt_messages() {
        let config = ShortTermConfig {
            max_prompt_messages: 3,
            ..Default::default()
        };
        let mut mem = ShortTermMemory::with_config(config);

        for i in 0..10 {
            mem.push(make_msg("ch1", "User", &format!("msg {}", i), false));
        }

        let key = ConversationKey::new(Platform::Discord, "ch1".to_string());
        let context = mem.get_context_for_prompt(&key);
        assert_eq!(context.len(), 3);
    }
}
