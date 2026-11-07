//! Boundary mapping from rusqlite's error surface onto the domain's exhaustive
//! enum.
//!
//! `DomainError` has no catch-all variant by design (see `farcooler_core::error`),
//! so every rusqlite failure has to land on the closest honest existing meaning
//! instead of inventing a new one here. `ResourceConflict` for the two callers
//! below is raised directly by `Store` methods that already know the precise
//! reason (a stale expected version); this module only handles what rusqlite
//! itself reports.

use farcooler_core::DomainError;
use rusqlite::ErrorCode;

/// What the triggers that keep an owned card to the engine raise (`engine_cutover`, `workflow_ownership`).
const ENGINE_FENCES: [&str; 4] = ["owned by workflow engine", "legacy task execution authority retired", "legacy task-bound terminal",
    "retain migrated"];

/// Map a low-level rusqlite error onto the domain's stable vocabulary.
///
/// `QueryReturnedNoRows` is the one exact match (`NotFound`). A UNIQUE
/// constraint violation means the caller raced another writer for the same
/// identity (for example two roots registered at the same path), which reads
/// to the caller exactly like a version conflict: retry against the current
/// state. Everything else is an operational failure: the query was well
/// formed, something beneath it was not.
pub(crate) fn map_err(err: rusqlite::Error) -> DomainError {
    match &err {
        rusqlite::Error::QueryReturnedNoRows => DomainError::NotFound,
        // A fence that keeps a card to the engine says so, in the words `refuse_if_owned` uses. Retrying
        // against current state, which is what a conflict tells the caller, would never succeed (ov-962).
        rusqlite::Error::SqliteFailure(e, Some(message)) if e.code == ErrorCode::ConstraintViolation && ENGINE_FENCES.iter().any(|f| message.contains(f)) => {
            DomainError::Conflict { what: "engine_owned" }
        }
        rusqlite::Error::SqliteFailure(e, _) if e.code == ErrorCode::ConstraintViolation => {
            DomainError::ResourceConflict
        }
        _ => {
            tracing::warn!(error = %err, "sqlite operation failed");
            DomainError::OperationFailed
        }
    }
}
