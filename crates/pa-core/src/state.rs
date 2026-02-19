use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors that can occur during state transitions.
#[derive(Debug, Error)]
pub enum StateError {
    #[error("Invalid transition from {from} to {to}")]
    InvalidTransition { from: AgentState, to: AgentState },
}

/// The top-level state of the Polyverse Agent.
///
/// This enum represents the lifecycle + operational states of the agent.
/// Transitions are validated — not every state can transition to every other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentState {
    /// Agent is booting up, workers being registered.
    Initializing,
    /// Agent is running but not actively processing any event.
    Idle,
    /// Agent is actively processing an incoming event (SLM or routing).
    Processing,
    /// Agent has delegated to Cloud LLM and is waiting for a response.
    WaitingForCloud,
    /// Agent has lost internet connectivity, running in local-only mode.
    Offline,
    /// Agent is performing memory consolidation during idle time.
    Consolidating,
    /// Agent is gracefully shutting down.
    ShuttingDown,
}

impl AgentState {
    /// Returns the set of valid states this state can transition to.
    pub fn valid_transitions(&self) -> &[AgentState] {
        use AgentState::*;
        match self {
            Initializing => &[Idle, ShuttingDown],
            Idle => &[Processing, Consolidating, Offline, ShuttingDown],
            Processing => &[Idle, WaitingForCloud, Offline, ShuttingDown],
            WaitingForCloud => &[Processing, Idle, Offline, ShuttingDown],
            Offline => &[Idle, Processing, ShuttingDown],
            Consolidating => &[Idle, Processing, Offline, ShuttingDown],
            ShuttingDown => &[], // terminal state
        }
    }

    /// Attempt to transition to a new state. Returns error if the transition is invalid.
    pub fn transition_to(&self, next: AgentState) -> Result<AgentState, StateError> {
        if self.valid_transitions().contains(&next) {
            Ok(next)
        } else {
            Err(StateError::InvalidTransition {
                from: *self,
                to: next,
            })
        }
    }

    /// Whether this is a terminal state (no further transitions possible).
    pub fn is_terminal(&self) -> bool {
        self.valid_transitions().is_empty()
    }
}

impl fmt::Display for AgentState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentState::Initializing => write!(f, "Initializing"),
            AgentState::Idle => write!(f, "Idle"),
            AgentState::Processing => write!(f, "Processing"),
            AgentState::WaitingForCloud => write!(f, "WaitingForCloud"),
            AgentState::Offline => write!(f, "Offline"),
            AgentState::Consolidating => write!(f, "Consolidating"),
            AgentState::ShuttingDown => write!(f, "ShuttingDown"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_transitions() {
        let state = AgentState::Initializing;
        assert!(state.transition_to(AgentState::Idle).is_ok());
        assert!(state.transition_to(AgentState::ShuttingDown).is_ok());
        assert!(state.transition_to(AgentState::Processing).is_err());
    }

    #[test]
    fn test_idle_transitions() {
        let state = AgentState::Idle;
        assert!(state.transition_to(AgentState::Processing).is_ok());
        assert!(state.transition_to(AgentState::Consolidating).is_ok());
        assert!(state.transition_to(AgentState::Offline).is_ok());
        assert!(state.transition_to(AgentState::Initializing).is_err());
    }

    #[test]
    fn test_shutting_down_is_terminal() {
        let state = AgentState::ShuttingDown;
        assert!(state.is_terminal());
        assert!(state.transition_to(AgentState::Idle).is_err());
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", AgentState::Idle), "Idle");
        assert_eq!(format!("{}", AgentState::WaitingForCloud), "WaitingForCloud");
    }
}
