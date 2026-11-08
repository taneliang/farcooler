//! Durable SQLite storage.
//!
//! State ownership splits by durability: SQLite stores only what must outlive
//! tmux. tmux is the sole authority for whether a process is alive right now,
//! so the `terminals` table has no `state`, `is_running`, or `pid` column --
//! there is no column in which a stale "running" could ever be recorded.
//! Runtime state is derived fresh from tmux on every read by
//! `farcooler_core::derive`, never stored here.
//!
//! Schema changes are forward-only migrations within a major version, tracked
//! by `schema_version` in the `meta` table. A pre-existing database gets a
//! checksummed backup written next to it before a migration touches it.
//! A database at a NEWER schema than this build knows is refused
//! (`DomainError::NewerData`) unless the build that wrote it stamped a
//! `compatible_down_to` at or below this build's schema. See `compat`.

pub mod agents;
pub mod agent_panes;
mod backup;
pub mod claude_logins;
pub mod member_connectors;
pub mod member_roles;
pub mod member_login_policy;
pub mod board_reads;
mod member_marks;
mod compat;
mod crash_signatures;
mod error;
pub mod landing;
pub mod landings;
pub mod landing_phase;
mod lane_migrations;
mod line_migration;
pub mod collecting;
pub mod lfs_pointers;
mod migrate;
pub mod read_counts;
pub mod pages;
pub mod plan;
pub mod plan_read;
pub mod plan_agents;
pub mod review;
pub mod rulings;
pub mod ticket_lines;
pub mod ticket_work;
pub mod pr_nudges;
pub mod plan_cost;
pub mod board_ci;
pub mod models;
mod store;
mod tasks;
pub mod usage;
mod wakes;
mod messages;
mod compose_holds;
mod compose_waits;
mod check_ins;
pub mod people;
mod device_enrollers;
mod revoked_marks;
pub mod device_added_by;
pub mod access;
pub mod access_ops;
pub mod step_up;
mod owner_answers;
mod engine_manual;
mod engine_stage;
pub use compose_holds::ComposeHold;
pub use compose_waits::ComposeWait;
pub use check_ins::{EnableCheckIn, PinCheckInRollout, fixed_check_in_message, CheckInObservation, CheckInOccurrence, CheckInStatus, CheckInTargetHold, PendingCheckIn, CHECK_IN_CONFIRMATION_TIMEOUT_MS};
pub use crash_signatures::{CrashFiling, CrashRecord};
mod subagent_notes;
mod web_panes;
pub mod waits;
pub mod workers;
pub mod workflow;
pub mod workflow_effects;
pub mod engine_cutover;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
mod workspaces;

pub use models::{
    AcceptanceItem, Actor, ClaimSource, IdempotencyRecord, NoteHit, NoteKind, Repository, RepositoryRoot,
    Task, TaskBlock, TaskNote, TaskStatus, TaskUpdate, Terminal, TerminalRole, TerminalUpdate, Workspace,
    Worktree,
};
pub use compat::{DatabaseSchema, read_schema};
pub use store::{IDEMPOTENCY_RETENTION_MILLIS, Store};
pub use tasks::{TaskScope, derive_prefix};
pub use wakes::{PendingWake, WakeKind};
pub use workspaces::{Vacated, valid_prefix};

pub mod workflow_publication;
pub mod workflow_notices;
pub mod workflow_overlay;

mod workflow_ownership;
mod workflow_projection;
pub use workflow_projection::BoardMove;
impl Store {
    /// Open cards no run owns: (workspace, key, column), for the engine to track (ov-951). Only a board
    /// with a published workflow is tracked; a board without one keeps its cards as they always were (ov-962).
    pub fn untracked_tickets(&self, only: Option<uuid::Uuid>) -> farcooler_core::Result<Vec<(uuid::Uuid, String, models::TaskStatus)>> {
        let conn = self.conn();
        let mut found = workflow_projection::untracked(&conn, only).map_err(error::map_err)?;
        found.retain(|(workspace, _, _)| conn.query_row("SELECT 1 FROM workflow_bundle_selections WHERE workspace=?1",
            [workspace.to_string()], |_| Ok(())).is_ok());
        Ok(found)
    }
    /// Whether the engine holds this card as a ticket at rest: a column, no run (ov-962).
    pub fn task_is_tracked_ticket(&self, task: uuid::Uuid) -> farcooler_core::Result<bool> {
        workflow_projection::is_ticket(&self.conn(), task).map_err(error::map_err)
    }
    /// Whether the engine holds this card (a ticket at rest or a run).
    pub fn task_is_engine_owned(&self, task: uuid::Uuid) -> farcooler_core::Result<bool> {
        workflow_projection::is_owned(&self.conn(), task).map_err(error::map_err)
    }
}
