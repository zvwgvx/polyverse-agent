//! # pa-core
//!
//! Core types, traits, and state machine for the Polyverse Agent.
//!
//! This crate defines the foundational abstractions used by all other crates
//! in the workspace. It has no platform-specific dependencies.

pub mod biology;
pub mod event;
pub mod state;
pub mod worker;

// Re-exports for convenience
pub use biology::{BiologyState, Mood};
pub use event::{Event, Platform, RawEvent, ResponseEvent};
pub use state::AgentState;
pub use worker::{Worker, WorkerContext, WorkerStatus};
