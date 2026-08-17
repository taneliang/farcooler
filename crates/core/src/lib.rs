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
