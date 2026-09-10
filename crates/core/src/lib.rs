//! Domain core: resource models, state derivation, preconditions, errors.
//!
//! `core` never depends on `tmux`, `store`, or `transport`. It defines the
//! `RuntimeInventory` trait that `tmux` implements, so crate dependencies point
//! one way and the derivation rule stays testable with no tmux present.

pub mod activity;
pub mod base64;
pub mod config;
pub mod derive;
pub mod error;
pub mod feed;
pub mod inventory;
pub mod names;
pub mod ports;
pub mod preconditions;
pub mod programs;
pub mod redact;
pub mod replay;
pub mod session_log;
pub mod shell;
pub mod theme;
pub mod title;
pub mod trace;
pub mod validate;

pub use error::{DomainError, Result};

/// Schema version stamped onto every managed tmux object.
pub const SCHEMA_VERSION: u32 = 1;

/// tmux user-option keys. Names, indexes, and PIDs never establish identity.
pub mod tags {
    pub const DAEMON_ID: &str = "@farcooler_daemon_id";
    pub const WORKSPACE_ID: &str = "@farcooler_workspace_id";
    pub const TERMINAL_ID: &str = "@farcooler_terminal_id";
    pub const SCHEMA_VERSION: &str = "@farcooler_schema_version";
}

/// The environment a dispatched pane carries, so a program running inside one
/// can say who it is and what it is working on without being told.
///
/// Two names, one contract, and it has two ends: the daemon exports them when
/// it launches a pane, and the CLI reads them when a command inside that pane
/// does not name an actor or a task on its own. They live here rather than as
/// a literal at each end because a contract spelled twice is a contract that
/// can drift, and the failure it drifts into is silent — a write filed under
/// the wrong name, which nothing downstream can tell from a right one.
pub mod pane_env {
    /// Who this pane is: `user`, `manager`, or `agent:<terminal id>` — the
    /// vocabulary `farcooler_store::models::Actor` parses, because it IS what
    /// parses it.
    ///
    /// `farcooler_daemon::service` exports it as `agent:<terminal id>` for
    /// every pane it launches to run an agent. A pane running a person's own
    /// shell is deliberately left without it: `user` is the honest answer for
    /// a pane somebody types into, and telling a person's write from an
    /// agent's is the only thing this field is for.
    pub const ACTOR: &str = "FARCOOLER_ACTOR";

    /// The ticket this pane is working, as a task key.
    ///
    /// **Nothing sets this yet.** There is no dispatch on this runner that
    /// knows which task a pane was opened for — a terminal record has a
    /// workspace, not a task — so the daemon has nothing honest to put here
    /// and puts nothing. The CLI reads it, so the day something does know, one
    /// export is the whole wiring.
    pub const TASK: &str = "FARCOOLER_TASK";
}
