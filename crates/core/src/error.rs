//! One domain error enum, exhaustively matched onto stable wire codes.
//!
//! There is NO catch-all arm. Adding a variant without mapping it fails the
//! build rather than shipping a generic error to a phone at the moment the user
//! needs a specific one. `retryable` is decided once per variant beside its
//! code, never guessed at a call site, because clients act on it automatically.

use farcooler_protocol::v1::ErrorCode;

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("authentication required")]
    AuthRequired,

    #[error("scope denied: {needed} required")]
    ScopeDenied { needed: &'static str },

    #[error("protocol version incompatible")]
    VersionIncompatible,

    #[error("repository is locked by another operation")]
    RepositoryLocked,

    #[error("branch already exists")]
    BranchExists,

    #[error("worktree path already exists")]
    WorktreeExists,

    #[error("worktree has uncommitted or unpushed state")]
    DirtyWorktree,

    #[error("managed processes are still running")]
    RunningProcesses,

    #[error("replay could not cover the requested sequence")]
    OutputGap,

    #[error("client exceeded its control-channel ceiling")]
    ClientTooSlow,

    #[error("operation failed")]
    OperationFailed,

    #[error("resource version is stale")]
    ResourceConflict,

    #[error("resource not found")]
    NotFound,

    #[error("invalid argument: {what}")]
    InvalidArgument { what: &'static str },

    #[error("idempotency key reused with a different request")]
    IdempotencyMismatch,

    #[error("tmux is unavailable")]
    TmuxUnavailable,

    #[error("path is outside every allowlisted repository root")]
    PathNotAllowed,

    #[error("exact typed confirmation required")]
    ConfirmationRequired,

    /// Deliberately distinct from `RunningProcesses`: nothing is running, but
    /// removing anyway would strand records and leave worktree directories on
    /// disk that Far Cooler would no longer be allowed to clean up. A client has
    /// to tell the user to remove those first, which it cannot do if this
    /// arrives as "managed processes are still running".
    #[error("workspaces still exist under this resource")]
    WorkspacesExist,
}

impl DomainError {
    /// Exhaustive. Adding a variant without a match arm is a compile error.
    pub fn wire(&self) -> (ErrorCode, bool) {
        match self {
            DomainError::AuthRequired => (ErrorCode::AuthRequired, false),
            DomainError::ScopeDenied { .. } => (ErrorCode::ScopeDenied, false),
            DomainError::VersionIncompatible => (ErrorCode::VersionIncompatible, false),
            DomainError::RepositoryLocked => (ErrorCode::RepositoryLocked, true),
            DomainError::BranchExists => (ErrorCode::BranchExists, false),
            DomainError::WorktreeExists => (ErrorCode::WorktreeExists, false),
            DomainError::DirtyWorktree => (ErrorCode::DirtyWorktree, false),
            DomainError::RunningProcesses => (ErrorCode::RunningProcesses, false),
            DomainError::OutputGap => (ErrorCode::OutputGap, false),
            DomainError::ClientTooSlow => (ErrorCode::ClientTooSlow, true),
            DomainError::OperationFailed => (ErrorCode::OperationFailed, true),
            DomainError::ResourceConflict => (ErrorCode::ResourceConflict, false),
            DomainError::NotFound => (ErrorCode::NotFound, false),
            DomainError::InvalidArgument { .. } => (ErrorCode::InvalidArgument, false),
            DomainError::IdempotencyMismatch => (ErrorCode::IdempotencyMismatch, false),
            DomainError::TmuxUnavailable => (ErrorCode::TmuxUnavailable, true),
            DomainError::PathNotAllowed => (ErrorCode::PathNotAllowed, false),
            DomainError::ConfirmationRequired => (ErrorCode::ConfirmationRequired, false),
            DomainError::WorkspacesExist => (ErrorCode::WorkspacesExist, false),
        }
    }

    pub fn code(&self) -> ErrorCode {
        self.wire().0
    }

    pub fn retryable(&self) -> bool {
        self.wire().1
    }

    /// Redacted client-facing message.
    ///
    /// Never carries a filesystem path, terminal content, command text, or a
    /// vendor session id. The `Display` impls above are written to that rule,
    /// so this is the single place that has to hold it.
    pub fn redacted_message(&self) -> String {
        self.to_string()
    }
}

pub type Result<T> = std::result::Result<T, DomainError>;

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant, so the round-trip assertion below covers the whole enum.
    fn all_variants() -> Vec<DomainError> {
        vec![
            DomainError::AuthRequired,
            DomainError::ScopeDenied { needed: "control" },
            DomainError::VersionIncompatible,
            DomainError::RepositoryLocked,
            DomainError::BranchExists,
            DomainError::WorktreeExists,
            DomainError::DirtyWorktree,
            DomainError::RunningProcesses,
            DomainError::OutputGap,
            DomainError::ClientTooSlow,
            DomainError::OperationFailed,
            DomainError::ResourceConflict,
            DomainError::NotFound,
            DomainError::InvalidArgument { what: "columns" },
            DomainError::IdempotencyMismatch,
            DomainError::TmuxUnavailable,
            DomainError::PathNotAllowed,
            DomainError::ConfirmationRequired,
            DomainError::WorkspacesExist,
        ]
    }

    #[test]
    fn every_variant_maps_to_a_specific_code() {
        for e in all_variants() {
            assert_ne!(
                e.code(),
                ErrorCode::Unspecified,
                "{e:?} fell through to UNSPECIFIED"
            );
        }
    }

    #[test]
    fn codes_are_distinct_per_variant() {
        let mut seen = std::collections::HashSet::new();
        for e in all_variants() {
            assert!(seen.insert(e.code() as i32), "{e:?} reuses a wire code");
        }
    }

    #[test]
    fn messages_leak_nothing_sensitive() {
        for e in all_variants() {
            let m = e.redacted_message();
            assert!(!m.contains('/'), "{e:?} message contains a path separator");
            assert!(!m.contains('\\'), "{e:?} message contains a path separator");
            assert!(!m.contains("session"), "{e:?} message mentions a session id");
        }
    }

    #[test]
    fn retryable_is_stable_and_deliberate() {
        // Clients auto-retry on these; a wrong value means retrying forever.
        assert!(DomainError::RepositoryLocked.retryable());
        assert!(DomainError::TmuxUnavailable.retryable());
        assert!(!DomainError::BranchExists.retryable());
        assert!(!DomainError::DirtyWorktree.retryable());
        assert!(!DomainError::ScopeDenied { needed: "control" }.retryable());
    }
}
