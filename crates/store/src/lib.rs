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

mod backup;
mod error;
mod migrate;
pub mod review;
pub mod models;
mod store;

pub use models::{
    IdempotencyRecord, Repository, RepositoryRoot, Terminal, TerminalUpdate, Workspace,
};
pub use store::{IDEMPOTENCY_RETENTION_MILLIS, Store};
