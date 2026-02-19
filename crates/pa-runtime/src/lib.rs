//! # pa-runtime
//!
//! Runtime infrastructure for the Polyverse Agent.
//! Provides the event bus, supervisor, and coordinator.

pub mod coordinator;
pub mod event_bus;
pub mod supervisor;

// Re-exports
pub use coordinator::Coordinator;
pub use event_bus::EventBus;
pub use supervisor::Supervisor;
