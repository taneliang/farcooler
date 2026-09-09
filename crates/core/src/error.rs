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

    // ---- review ----
    //
    // Six failures the review surface can produce. Each maps to a sentence a
    // person reads; none of them ever puts a Rust error in front of a user.
    #[error("the base this branch is compared against could not be resolved")]
    BaseUnresolvable,

    #[error("the diff is larger than a client may be sent at once")]
    DiffTooLarge,

    #[error("this file has no unified diff to show")]
    DiffUnsupported,

    #[error("pull request state could not be read")]
    PrStateUnavailable,

    #[error("attachment exceeds a size or count limit")]
    AttachmentLimit,

    #[error("the agent may or may not have received this dispatch")]
    DispatchUnknown,

    /// This runner's Far Cooler is older than what the client asked for.
    ///
    /// Distinct from `NotFound`, which it used to arrive as. "No such
    /// workspace" and "this runner cannot do that yet" call for opposite
    /// responses, and a newer app could not tell them apart — so it could
    /// neither dim the control nor say anything a person could act on.
    ///
    /// Names the capability, because "update it" is only useful advice if the
    /// person can tell which feature is missing.
    #[error("this runner is running an older Far Cooler that can't do this yet")]
    CapabilityUnsupported { needed: &'static str },

    #[error("path is outside every allowlisted repository root")]
    PathNotAllowed,

    /// Distinct from `PathNotAllowed`: that one is "allowlist this", this one
    /// is "this will never be allowlistable". A whole home directory or a
    /// system path reused `PathNotAllowed`'s message once, and someone trying
    /// to add their `$HOME` as a root read "outside every allowlisted root"
    /// for a path that was, from where they were sitting, obviously the root
    /// they were trying to add.
    #[error("that location is a whole home directory or a system path, and can never be allowlisted — pick a folder inside it")]
    SensitiveRoot,

    #[error("exact typed confirmation required")]
    ConfirmationRequired,

    /// Deliberately distinct from `RunningProcesses`: nothing is running, but
    /// removing anyway would strand records and leave worktree directories on
    /// disk that Far Cooler would no longer be allowed to clean up. A client has
    /// to tell the user to remove those first, which it cannot do if this
    /// arrives as "managed processes are still running".
    #[error("workspaces still exist under this resource")]
    WorkspacesExist,

    /// A prompt was sent to a pane no shim is holding the socket for.
    ///
    /// Deliberately not `OperationFailed`, and deliberately not silence, which
    /// is what this replaces: `agent_prompt` handed the message to the
    /// supervisor, the supervisor found no writer, and the RPC replied with
    /// the terminal read back as though the words had been delivered. The
    /// person watched their own message vanish with nothing anywhere saying it
    /// had.
    ///
    /// Retryable, because the usual cause is a chat whose shim has not
    /// finished dialling and which will be there a second later.
    #[error("no agent is connected to this pane")]
    AgentNotConnected,
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
            DomainError::SensitiveRoot => (ErrorCode::SensitiveRoot, false),
            DomainError::ConfirmationRequired => (ErrorCode::ConfirmationRequired, false),
            DomainError::WorkspacesExist => (ErrorCode::WorkspacesExist, false),
            // Retryable: a shim that has not finished dialling will be there.
            DomainError::AgentNotConnected => (ErrorCode::AgentNotConnected, true),
            DomainError::BaseUnresolvable => (ErrorCode::BaseUnresolvable, false),
            DomainError::DiffTooLarge => (ErrorCode::DiffTooLarge, false),
            DomainError::DiffUnsupported => (ErrorCode::DiffUnsupported, false),
            // Retryable: a captive portal clears, a rate limit expires.
            DomainError::PrStateUnavailable => (ErrorCode::PrStateUnavailable, true),
            DomainError::AttachmentLimit => (ErrorCode::AttachmentLimit, false),
            // Retryable in the sense that Send Again is the answer, though only
            // a person may decide to press it.
            DomainError::DispatchUnknown => (ErrorCode::DispatchUnknown, true),
            // Never retryable: no amount of retrying updates the other side.
            DomainError::CapabilityUnsupported { .. } => (ErrorCode::CapabilityUnsupported, false),
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

/// The word a client switches on, for a code that came off the wire.
///
/// **The runner sends a stable machine word; the app owns the sentence.** The
/// same rule `Terminal.agent_failure` follows with `not-authenticated` and
/// friends, and `TunnelError::code` follows for the tunnel — spelled the same
/// way, in kebab-case, rather than in the proto's `ERROR_CODE_SCOPE_DENIED`
/// shouting form, because these are read in Swift and Kotlin beside those
/// other words and one vocabulary is easier to hold than three.
///
/// Deliberately NOT `DomainError`'s `Display`. That text is written for
/// whoever is reading a daemon log — "resource version is stale", "invalid
/// argument: idempotency_key" — and it is the thing this word exists to keep
/// off a screen.
///
/// Exhaustive: adding a code to the proto without a word here fails the build.
pub fn word(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::Unspecified => "unspecified",
        ErrorCode::AuthRequired => "auth-required",
        ErrorCode::ScopeDenied => "scope-denied",
        ErrorCode::VersionIncompatible => "version-incompatible",
        ErrorCode::HostOffline => "host-offline",
        ErrorCode::RepositoryLocked => "repository-locked",
        ErrorCode::BranchExists => "branch-exists",
        ErrorCode::WorktreeExists => "worktree-exists",
        ErrorCode::DirtyWorktree => "dirty-worktree",
        ErrorCode::RunningProcesses => "running-processes",
        ErrorCode::OutputGap => "output-gap",
        ErrorCode::ClientTooSlow => "client-too-slow",
        ErrorCode::OperationFailed => "operation-failed",
        ErrorCode::ResourceConflict => "resource-conflict",
        ErrorCode::NotFound => "not-found",
        ErrorCode::InvalidArgument => "invalid-argument",
        ErrorCode::IdempotencyMismatch => "idempotency-mismatch",
        ErrorCode::TmuxUnavailable => "tmux-unavailable",
        ErrorCode::PathNotAllowed => "path-not-allowed",
        ErrorCode::ConfirmationRequired => "confirmation-required",
        ErrorCode::WorkspacesExist => "workspaces-exist",
        ErrorCode::SensitiveRoot => "sensitive-root",
        ErrorCode::BaseUnresolvable => "base-unresolvable",
        ErrorCode::DiffTooLarge => "diff-too-large",
        ErrorCode::DiffUnsupported => "diff-unsupported",
        ErrorCode::PrStateUnavailable => "pr-state-unavailable",
        ErrorCode::AttachmentLimit => "attachment-limit",
        ErrorCode::DispatchUnknown => "dispatch-unknown",
        ErrorCode::CapabilityUnsupported => "capability-unsupported",
        ErrorCode::AgentNotConnected => "agent-not-connected",
    }
}

/// A code number this build has no name for.
///
/// Its own word rather than `"unspecified"`, which means something else and
/// already has a code: zero is a daemon that named no reason, this is a daemon
/// that named one we are too old to read.
pub const UNRECOGNIZED_WORD: &str = "unrecognized";

/// The word for a raw wire integer, INCLUDING one this build has never heard
/// of.
///
/// Total by construction, and that is the whole point. A runner newer than the
/// client sends a number that is not in `ErrorCode` yet, and the one thing that
/// must not happen then is for the refusal to arrive carrying no code at all: a
/// client that reads an unknown code as nothing shows nothing where it owes the
/// reader a failure. That exact bug shipped in the agent-failure work and had to
/// be fixed on macOS afterwards; it is answered here so no client has to
/// remember to.
pub fn word_for(code: i32) -> &'static str {
    ErrorCode::try_from(code).map(word).unwrap_or(UNRECOGNIZED_WORD)
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
            DomainError::SensitiveRoot,
            DomainError::ConfirmationRequired,
            DomainError::WorkspacesExist,
            DomainError::BaseUnresolvable,
            DomainError::DiffTooLarge,
            DomainError::DiffUnsupported,
            DomainError::PrStateUnavailable,
            DomainError::AttachmentLimit,
            DomainError::DispatchUnknown,
            DomainError::CapabilityUnsupported { needed: "changes" },
            DomainError::AgentNotConnected,
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

    /// A word per variant, and never the two that mean "no word".
    ///
    /// `unspecified` is a daemon that named no reason and `unrecognized` is a
    /// code this build cannot read; a produced error landing on either would be
    /// a code that reaches a phone as no code at all.
    #[test]
    fn every_variant_has_a_word_of_its_own() {
        let mut seen = std::collections::HashSet::new();
        for e in all_variants() {
            let w = word(e.code());
            assert_ne!(w, "unspecified", "{e:?} has no word");
            assert_ne!(w, UNRECOGNIZED_WORD, "{e:?} has no word");
            assert!(seen.insert(w), "{e:?} reuses the word {w}");
        }
    }

    /// The words are what Swift and Kotlin switch on, so their SHAPE is part of
    /// the contract: lowercase kebab, the way `agent_failure`'s are.
    #[test]
    fn words_are_lowercase_kebab() {
        for e in all_variants() {
            let w = word(e.code());
            assert!(
                w.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "{e:?} has the word {w}, which is not lowercase kebab"
            );
        }
    }

    /// The rule an unknown code has to obey, stated where it is decided.
    ///
    /// A runner newer than this client sends a number `ErrorCode` does not
    /// have. It must come back as a word — the generic one — and never as
    /// nothing, because a client that reads it as nothing shows nothing.
    #[test]
    fn a_code_from_the_future_still_has_a_word() {
        assert_eq!(word_for(ErrorCode::TmuxUnavailable as i32), "tmux-unavailable");
        assert_eq!(word_for(0), "unspecified");
        // Well past the last code the proto declares, which is what the next
        // release of the daemon will be sending.
        assert_eq!(word_for(9_999), UNRECOGNIZED_WORD);
        assert_eq!(word_for(-1), UNRECOGNIZED_WORD);
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
