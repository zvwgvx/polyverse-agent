use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use tracing::{debug, info};

use crate::types::MemoryMessage;

/// SQLite-based persistent storage for raw messages.
/// Acts as a backup for short-term memory and source for future RAG.
pub struct MemoryStore {
    conn: Connection,
}

impl MemoryStore {
    /// Open or create the SQLite database at the given path.
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open memory database: {}", path))?;

        let store = Self { conn };
        store.init_tables()?;

        info!(path = %path, "Memory store opened");
        Ok(store)
    }

    /// Open an in-memory database (for testing).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .context("Failed to open in-memory database")?;

        let store = Self { conn };
        store.init_tables()?;
        Ok(store)
    }

    /// Create tables if they don't exist.
    fn init_tables(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                platform TEXT NOT NULL,
                channel_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                username TEXT NOT NULL,
                content TEXT NOT NULL,
                is_mention BOOLEAN DEFAULT 0,
                is_bot_response BOOLEAN DEFAULT 0,
                reply_to_user TEXT,
                importance REAL DEFAULT 0.5,
                created_at TEXT NOT NULL,
                accessed_at TEXT,
                access_count INTEGER DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_messages_channel
                ON messages(platform, channel_id);
            CREATE INDEX IF NOT EXISTS idx_messages_time
                ON messages(created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_messages_importance
                ON messages(importance DESC);
            ",
        )?;
        Ok(())
    }

    /// Insert a message into the store.
    pub fn insert(&self, msg: &MemoryMessage) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO messages
                (id, platform, channel_id, user_id, username, content,
                 is_mention, is_bot_response, reply_to_user, importance, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                msg.id,
                format!("{}", msg.platform),
                msg.channel_id,
                msg.user_id,
                msg.username,
                msg.content,
                msg.is_mention,
                msg.is_bot_response,
                msg.reply_to_user,
                msg.importance,
                msg.timestamp.to_rfc3339(),
            ],
        )?;
        debug!(id = %msg.id, "Message persisted to store");
        Ok(())
    }

    /// Insert a batch of messages.
    pub fn insert_batch(&self, messages: &[MemoryMessage]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for msg in messages {
            tx.execute(
                "INSERT OR IGNORE INTO messages
                    (id, platform, channel_id, user_id, username, content,
                     is_mention, is_bot_response, reply_to_user, importance, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    msg.id,
                    format!("{}", msg.platform),
                    msg.channel_id,
                    msg.user_id,
                    msg.username,
                    msg.content,
                    msg.is_mention,
                    msg.is_bot_response,
                    msg.reply_to_user,
                    msg.importance,
                    msg.timestamp.to_rfc3339(),
                ],
            )?;
        }
        tx.commit()?;
        debug!(count = messages.len(), "Batch persisted to store");
        Ok(())
    }

    /// Get recent messages for a channel (for restoring short-term on startup).
    pub fn get_recent(
        &self,
        platform: &str,
        channel_id: &str,
        limit: usize,
    ) -> Result<Vec<MemoryMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, platform, channel_id, user_id, username, content,
                    is_mention, is_bot_response, importance, created_at
             FROM messages
             WHERE platform = ?1 AND channel_id = ?2
             ORDER BY created_at DESC
             LIMIT ?3",
        )?;

        let rows = stmt.query_map(params![platform, channel_id, limit as i64], |row| {
            let platform_str: String = row.get(1)?;
            let platform = match platform_str.as_str() {
                "Discord" => pa_core::event::Platform::Discord,
                "Telegram" => pa_core::event::Platform::Telegram,
                _ => pa_core::event::Platform::Cli,
            };

            let timestamp_str: String = row.get(9)?;
            let timestamp = chrono::DateTime::parse_from_rfc3339(&timestamp_str)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now());

            Ok(MemoryMessage {
                id: row.get(0)?,
                platform,
                channel_id: row.get(2)?,
                user_id: row.get(3)?,
                username: row.get(4)?,
                content: row.get(5)?,
                is_mention: row.get(6)?,
                is_bot_response: row.get(7)?,
                reply_to_user: None, // Not stored in this query for now
                importance: row.get(8)?,
                timestamp,
            })
        })?;

        let mut messages: Vec<MemoryMessage> = rows.filter_map(|r| r.ok()).collect();
        // Reverse to get chronological order
        messages.reverse();
        Ok(messages)
    }

    /// Get total message count.
    pub fn message_count(&self) -> Result<i64> {
        let count: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::MemoryMessage;
    use pa_core::event::Platform;

    fn make_msg(id: &str, channel: &str, content: &str) -> MemoryMessage {
        MemoryMessage {
            id: id.to_string(),
            platform: Platform::Discord,
            channel_id: channel.to_string(),
            user_id: "u1".to_string(),
            username: "TestUser".to_string(),
            content: content.to_string(),
            is_mention: false,
            is_bot_response: false,
            reply_to_user: None,
            timestamp: chrono::Utc::now(),
            importance: 0.5,
        }
    }

    #[test]
    fn test_insert_and_retrieve() {
        let store = MemoryStore::open_in_memory().unwrap();

        let msg = make_msg("m1", "ch1", "hello world");
        store.insert(&msg).unwrap();

        let messages = store.get_recent("Discord", "ch1", 10).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "hello world");
    }

    #[test]
    fn test_batch_insert() {
        let store = MemoryStore::open_in_memory().unwrap();

        let msgs = vec![
            make_msg("m1", "ch1", "msg 1"),
            make_msg("m2", "ch1", "msg 2"),
            make_msg("m3", "ch1", "msg 3"),
        ];
        store.insert_batch(&msgs).unwrap();

        assert_eq!(store.message_count().unwrap(), 3);
    }

    #[test]
    fn test_duplicate_insert_ignored() {
        let store = MemoryStore::open_in_memory().unwrap();

        let msg = make_msg("m1", "ch1", "hello");
        store.insert(&msg).unwrap();
        store.insert(&msg).unwrap(); // duplicate

        assert_eq!(store.message_count().unwrap(), 1);
    }
}
