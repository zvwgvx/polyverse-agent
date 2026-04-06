use std::sync::Arc;

use anyhow::Result;
use kernel::biology::BiologyState;
use kernel::event::Event;
use kernel::state::{AgentState, StateError};
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{debug, info, warn};

pub struct Coordinator {
    state: AgentState,

    pub biology: Arc<RwLock<BiologyState>>,

    broadcast_tx: broadcast::Sender<Event>,
}

impl Coordinator {
    pub fn new(broadcast_tx: broadcast::Sender<Event>) -> Self {
        Self {
            state: AgentState::Initializing,
            biology: Arc::new(RwLock::new(BiologyState::new())),
            broadcast_tx,
        }
    }

    pub fn state(&self) -> AgentState {
        self.state
    }

    pub fn biology_state(&self) -> Arc<RwLock<BiologyState>> {
        Arc::clone(&self.biology)
    }

    pub fn transition(&mut self, next: AgentState) -> Result<(), StateError> {
        let new_state = self.state.transition_to(next)?;
        info!(from = %self.state, to = %new_state, "State transition");
        self.state = new_state;
        Ok(())
    }

    pub async fn run(
        &mut self,
        mut event_rx: mpsc::Receiver<Event>,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) -> Result<()> {
        self.transition(AgentState::Idle)?;
        info!(state = %self.state, "Coordinator started");

        loop {
            tokio::select! {
                Some(event) = event_rx.recv() => {
                    self.handle_event(event).await;
                }

                _ = shutdown_rx.recv() => {
                    info!("Coordinator received shutdown signal");
                    let _ = self.transition(AgentState::ShuttingDown);
                    break;
                }
            }
        }

        info!("Coordinator stopped");
        Ok(())
    }

    async fn handle_event(&mut self, event: Event) {
        match &event {
            Event::Raw(raw) => {
                info!(
                    platform = %raw.platform,
                    user = %raw.username,
                    is_mention = raw.is_mention,
                    is_dm = raw.is_dm,
                    content_len = raw.content.len(),
                    content = %raw.content,
                    "Coordinator received raw event"
                );

                if self.state == AgentState::Idle {
                    let _ = self.transition(AgentState::Processing);
                }

                if let Err(e) = self.broadcast_tx.send(event) {
                    warn!(error = %e, "No subscribers for broadcast event");
                }

                if self.state == AgentState::Processing {
                    let _ = self.transition(AgentState::Idle);
                }
            }

            Event::Intent(intent) => {
                debug!(
                    intent = ?intent.intent,
                    sentiment = ?intent.sentiment,
                    needs_cloud = intent.needs_cloud,
                    "Received intent classification"
                );
                let _ = self.broadcast_tx.send(event);
            }

            Event::Response(response) => {
                info!(
                    platform = %response.platform,
                    source = ?response.source,
                    content_len = response.content.len(),
                    "Broadcasting response to sensory workers"
                );
                if let Err(e) = self.broadcast_tx.send(event) {
                    warn!(error = %e, "No subscribers for response broadcast");
                }

                if self.state == AgentState::WaitingForCloud {
                    let _ = self.transition(AgentState::Idle);
                }
            }

            Event::Biology(bio_event) => {
                debug!(kind = ?bio_event.kind, "Biology event");
                let mut bio = self.biology.write().await;
                match &bio_event.kind {
                    kernel::event::BiologyEventKind::EnergyChanged { delta, .. } => {
                        if *delta > 0.0 {
                            bio.recover_energy(*delta);
                        } else {
                            bio.drain_energy(delta.abs());
                        }
                    }
                    kernel::event::BiologyEventKind::SleepStarted => {
                        bio.is_sleeping = true;
                    }
                    kernel::event::BiologyEventKind::SleepEnded => {
                        bio.is_sleeping = false;
                    }
                    _ => {}
                }
            }

            Event::BotTurnCompletion(_) => {
                let _ = self.broadcast_tx.send(event);
            }

            Event::System(sys) => {
                debug!(event = ?sys, "System event");
            }
        }
    }
}
