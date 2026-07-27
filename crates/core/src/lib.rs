//! Domain core: resource models, state derivation, preconditions, errors.
//!
//! `core` never depends on `tmux`, `store`, or `transport`. It defines the
//! `RuntimeInventory` trait that `tmux` implements, so crate dependencies point
//! one way and the derivation rule stays testable with no tmux present.

pub mod derive;
pub mod error;
pub mod inventory;
pub mod preconditions;
pub mod replay;
pub mod validate;

pub use error::{DomainError, Result};

/// Schema version stamped onto every managed tmux object.
pub const SCHEMA_VERSION: u32 = 1;

/// tmux user-option keys. Names, indexes, and PIDs never establish identity.
pub mod tags {
    pub const DAEMON_ID: &str = "@overnight_daemon_id";
    pub const WORKSPACE_ID: &str = "@overnight_workspace_id";
    pub const TERMINAL_ID: &str = "@overnight_terminal_id";
    pub const SCHEMA_VERSION: &str = "@overnight_schema_version";
}
