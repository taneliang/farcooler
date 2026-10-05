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

mod backup;
pub mod board_reads;
mod compat;
mod error;
pub mod landing;
pub mod lfs_pointers;
mod migrate;
pub mod pages;
pub mod plan;
pub mod plan_read;
pub mod review;
pub mod rulings;
pub mod trains;
pub mod plan_cost;
pub mod board_ci;
pub mod models;
mod store;
mod tasks;
pub mod usage;
mod wakes;
pub mod waits;
pub mod workers;
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
