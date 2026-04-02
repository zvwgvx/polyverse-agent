use kernel::event::{Event, RawEvent};
use tokio::sync::mpsc;
use tracing::debug;

#[derive(Clone)]
pub struct SensoryBuffer {
    event_tx: mpsc::Sender<Event>,
}

impl SensoryBuffer {
    pub fn new(event_tx: mpsc::Sender<Event>) -> Self {
        Self { event_tx }
    }

    pub async fn push(&self, mut raw: RawEvent) {
        raw.content = raw.content.trim().to_string();
        debug!(
            user = %raw.username,
            content_len = raw.content.len(),
            "SensoryBuffer forwarding event immediately"
        );
        let _ = self.event_tx.send(Event::Raw(raw)).await;
    }

    pub async fn typing(&self, _platform: kernel::event::Platform, _channel_id: String, _user_id: String) {
        // typing detection removed
    }
}
