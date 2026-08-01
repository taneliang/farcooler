//! The agent view's host side: one normalized event model, and the adapters
//! that produce it.
//!
//! Clients never see a vendor protocol. That is the entire point — the UI is
//! written once against `event::AgentEvent`, and a new agent is a new adapter
//! rather than a new screen.

pub mod acp;
pub mod activity_source;
pub mod event;
pub mod fs_guard;
pub mod link;
pub mod ring;
pub mod session;
