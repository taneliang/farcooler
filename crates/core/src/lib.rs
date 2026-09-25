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
    /// `farcooler_daemon::service` exports it beside `ACTOR` for an agent
    /// pane opened for a task (`farcooler task dispatch`, or `terminal create
    /// --task`), on its first launch and on every relaunch after, reading the
    /// key off the terminal's record (`terminals.task_id`). A pane opened for
    /// no task gets nothing: a guessed key would file an agent's notes on
    /// somebody else's ticket.
    pub const TASK: &str = "FARCOOLER_TASK";

    /// The agents a pane can be opened FOR a task with: the three that take
    /// an initial prompt as their launch argument, so they are told the task
    /// on their first launch. Any other preset would export the key beside a
    /// program that never reads it, or a person's shell, and the board would
    /// say somebody is working a task nobody is.
    ///
    /// One list for both ends: the daemon refuses a task for anything else
    /// (`terminal.create`), and `farcooler task dispatch` refuses it before
    /// asking. The daemon's tests hold this list to the launch arms that
    /// really pass the prompt.
    pub const TASK_AGENTS: &[&str] = &["claude", "codex", "cursor"];

    /// Whether `preset` (`claude`, `codex:gpt-5`, …) is one of `TASK_AGENTS`.
    pub fn takes_a_task(preset: &str) -> bool {
        let head = preset.split_once(':').map_or(preset, |(agent, _)| agent);
        TASK_AGENTS.contains(&head)
    }
}
