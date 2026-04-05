use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const MAX_IMAGE_ATTACHMENTS_PER_MESSAGE: usize = 4;
pub const MAX_IMAGE_ATTACHMENT_BYTES: usize = 5 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageAttachment {
    pub mime_type: String,
    pub filename: Option<String>,
    pub source_url: Option<String>,
    pub data_base64: String,
}

impl ImageAttachment {
    pub fn is_supported_image_mime(mime_type: &str) -> bool {
        matches!(
            mime_type,
            "image/jpeg" | "image/png" | "image/webp" | "image/gif"
        )
    }

    pub fn as_data_url(&self) -> String {
        format!("data:{};base64,{}", self.mime_type, self.data_base64)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Platform {
    Discord,
    DiscordSelfbot,
    Telegram,
    Cli,
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Platform::Discord => write!(f, "Discord"),
            Platform::DiscordSelfbot => write!(f, "DiscordSelfbot"),
            Platform::Telegram => write!(f, "Telegram"),
            Platform::Cli => write!(f, "CLI"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawEvent {
    pub platform: Platform,
    pub channel_id: String,
    pub message_id: String,
    pub user_id: String,
    pub username: String,
    pub content: String,
    #[serde(default)]
    pub attachments: Vec<ImageAttachment>,
    pub is_mention: bool,
    pub is_dm: bool,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Intent {
    Command,
    Question,
    ChitChat,
    Insult,
    Noise,
    ComplexQuery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sentiment {
    Positive,
    Neutral,
    Negative,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentEvent {
    pub source: RawEvent,
    pub intent: Intent,
    pub sentiment: Sentiment,
    pub needs_cloud: bool,
    pub confidence: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResponseSource {
    LocalSLM,
    CloudLLM,
    Template,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseEvent {
    pub platform: Platform,
    pub channel_id: String,
    pub reply_to_message_id: Option<String>,
    pub reply_to_user: Option<String>,
    pub is_dm: bool,
    pub content: String,
    pub source: ResponseSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotTurnCompletion {
    pub platform: Platform,
    pub channel_id: String,
    pub reply_to_message_id: Option<String>,
    pub reply_to_user: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BiologyEventKind {
    EnergyChanged { delta: f32, reason: String },
    MoodChanged { new_mood: String, trigger: String },
    SleepStarted,
    SleepEnded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiologyEvent {
    pub kind: BiologyEventKind,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SystemEvent {
    WorkerStarted { name: String },
    WorkerStopped { name: String },
    WorkerError { name: String, error: String },
    ShutdownRequested,
    HealthCheckRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    Raw(RawEvent),
    Intent(IntentEvent),
    Response(ResponseEvent),
    BotTurnCompletion(BotTurnCompletion),
    Biology(BiologyEvent),
    System(SystemEvent),
}

impl Event {
    pub fn is_system(&self) -> bool {
        matches!(self, Event::System(_))
    }

    pub fn is_raw(&self) -> bool {
        matches!(self, Event::Raw(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raw_event_serialization() {
        let event = RawEvent {
            platform: Platform::Discord,
            channel_id: "123".to_string(),
            message_id: "456".to_string(),
            user_id: "789".to_string(),
            username: "TestUser".to_string(),
            content: "Hello agent!".to_string(),
            attachments: vec![],
            is_mention: false,
            is_dm: false,
            timestamp: Utc::now(),
        };

        let json = serde_json::to_string(&event).unwrap();
        let deserialized: RawEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.platform, Platform::Discord);
        assert_eq!(deserialized.content, "Hello agent!");
    }

    #[test]
    fn test_event_enum_variants() {
        let raw = Event::Raw(RawEvent {
            platform: Platform::Telegram,
            channel_id: "ch1".to_string(),
            message_id: "m1".to_string(),
            user_id: "u1".to_string(),
            username: "user".to_string(),
            content: "test".to_string(),
            attachments: vec![],
            is_mention: false,
            is_dm: false,
            timestamp: Utc::now(),
        });

        assert!(raw.is_raw());
        assert!(!raw.is_system());

        let sys = Event::System(SystemEvent::ShutdownRequested);
        assert!(sys.is_system());
        assert!(!sys.is_raw());
    }
}
