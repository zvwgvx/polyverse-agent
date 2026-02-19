use std::sync::Arc;

use anyhow::Result;
use pa_core::biology::BiologyState;
use pa_core::event::Event;
use pa_core::state::{AgentState, StateError};
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{debug, info, warn};

/// The Coordinator is the "brain stem" of the agent.
///
/// It sits between the event bus and the workers, responsible for:
/// - Reading events from the mpsc channel (from workers)
/// - Running state machine transitions
/// - Routing events to the appropriate handler
/// - Broadcasting events to interested workers
/// - Maintaining shared biology state
pub struct Coordinator {
    /// Current agent state (the state machine).
    state: AgentState,

    /// Shared biology state, readable by all workers.
    pub biology: Arc<RwLock<BiologyState>>,

    /// Broadcast sender for distributing events to workers.
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

    /// Get the current agent state.
    pub fn state(&self) -> AgentState {
        self.state
    }

    /// Get a reference to the shared biology state.
    pub fn biology_state(&self) -> Arc<RwLock<BiologyState>> {
        Arc::clone(&self.biology)
    }

    /// Attempt a state transition.
    pub fn transition(&mut self, next: AgentState) -> Result<(), StateError> {
        let new_state = self.state.transition_to(next)?;
        info!(from = %self.state, to = %new_state, "State transition");
        self.state = new_state;
        Ok(())
    }

    /// Run the coordinator's main event loop.
    ///
    /// This consumes events from the mpsc receiver and processes them.
    /// It runs until a shutdown signal is received or the channel closes.
    pub async fn run(
        &mut self,
        mut event_rx: mpsc::Receiver<Event>,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) -> Result<()> {
        // Transition from Initializing to Idle
        self.transition(AgentState::Idle)?;
        info!(state = %self.state, "Coordinator started");

        loop {
            tokio::select! {
                // Handle incoming events from workers
                Some(event) = event_rx.recv() => {
                    self.handle_event(event).await;
                }

                // Handle shutdown signal
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

    /// Process a single event.
    async fn handle_event(&mut self, event: Event) {
        match &event {
            Event::Raw(raw) => {
                debug!(
                    platform = %raw.platform,
                    user = %raw.username,
                    content = %raw.content,
                    "Received raw event"
                );

                // Transition to Processing if we're Idle
                if self.state == AgentState::Idle {
                    let _ = self.transition(AgentState::Processing);
                }

                // Broadcast raw event to cognitive workers (SLM will pick it up)
                if let Err(e) = self.broadcast_tx.send(event) {
                    warn!(error = %e, "No subscribers for broadcast event");
                }

                // Transition back to Idle after processing
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
                // Forward to appropriate handler (Cloud or local response)
                let _ = self.broadcast_tx.send(event);
            }

            Event::Response(response) => {
                debug!(
                    platform = %response.platform,
                    source = ?response.source,
                    "Routing response to sensory worker"
                );
                // Broadcast response — the appropriate sensory worker will pick it up
                let _ = self.broadcast_tx.send(event);

                // Back to idle
                if self.state == AgentState::WaitingForCloud {
                    let _ = self.transition(AgentState::Idle);
                }
            }

            Event::Biology(bio_event) => {
                debug!(kind = ?bio_event.kind, "Biology event");
                // Update shared biology state
                let mut bio = self.biology.write().await;
                match &bio_event.kind {
                    pa_core::event::BiologyEventKind::EnergyChanged { delta, .. } => {
                        if *delta > 0.0 {
                            bio.recover_energy(*delta);
                        } else {
                            bio.drain_energy(delta.abs());
                        }
                    }
                    pa_core::event::BiologyEventKind::SleepStarted => {
                        bio.is_sleeping = true;
                    }
                    pa_core::event::BiologyEventKind::SleepEnded => {
                        bio.is_sleeping = false;
                    }
                    _ => {}
                }
            }

            Event::System(sys) => {
                debug!(event = ?sys, "System event");
                // System events are logged, no routing needed
            }
        }
    }
}
