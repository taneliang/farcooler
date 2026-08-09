//! The agent view's host side: one normalized event model, and the adapters
//! that produce it.
//!
//! Clients never see a vendor protocol. That is the entire point — the UI is
//! written once against `event::AgentEvent`, and a new agent is a new adapter
//! rather than a new screen.

pub mod activity_source;
pub mod chat;
pub mod dispatch;
pub mod link;
pub mod ring;

/// Re-exported so `farcooler_agent::event::AgentEvent` keeps resolving.
///
/// The vocabulary moved to `farcooler-agent-core` so a backend crate can
/// depend on it without depending on the orchestration above it. Every
/// consumer — the daemon, the CLI, the client — still spells it the way it
/// always did, because a crate split is not a reason to churn call sites.
pub use farcooler_agent_core::{event, fs_guard};

/// Re-exported so `farcooler_agent::acp::conn::AcpConnection` keeps resolving.
///
/// ACP became a backend crate alongside the native ones rather than staying
/// the one hard-wired protocol. It is still the only backend a config-added
/// adapter can use, so this is not a deprecation path.
pub use farcooler_acp as acp;

/// Re-exported so `farcooler_agent::session::AgentSession` keeps resolving.
///
/// Every function in it speaks ACP — `initialize`, `session/load`,
/// `session/new`, and the parsers that read their results — so it moved with
/// the rest of the protocol rather than staying behind as a neutral-looking
/// module that was never neutral.
pub use farcooler_acp::session;
