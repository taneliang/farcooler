//! One domain error enum, exhaustively matched onto stable wire codes.
//!
//! There is NO catch-all arm. Adding a variant without mapping it fails the
//! build rather than shipping a generic error to a phone at the moment the user
//! needs a specific one. `retryable` is decided once per variant beside its
//! code, never guessed at a call site, because clients act on it automatically.

use farcooler_protocol::v1::ErrorCode;

#[derive(Debug, Clone, thiserror::Error)]
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

    /// `ResourceConflict`'s code, naming which conflict in `Error.what`.
    ///
    /// For the refusals a client words differently: `terminal.agent_answer`'s
    /// `not_held` (someone already answered the ask) and `not_delivered` (the
    /// answer was taken but never reached the agent). Before this both were
    /// the same bare conflict, so no client could tell them apart.
    #[error("{}", sentence(what).unwrap_or("resource version is stale"))]
    Conflict { what: &'static str },

    #[error("resource not found")]
    NotFound,

    #[error("invalid argument: {what}")]
    InvalidArgument { what: &'static str },

    #[error("idempotency key reused with a different request")]
    IdempotencyMismatch,

    #[error("tmux is unavailable")]
    TmuxUnavailable,

    /// tmux is there, and a command to it ran past its deadline.
    ///
    /// Distinct from `TmuxUnavailable`, which every client words as "install
    /// tmux": the wrong advice for a runner that answered slowly. Opening a
    /// pane on a loaded machine is the usual cause (ov-176). Retryable.
    #[error("tmux did not answer in time")]
    TmuxTimedOut,

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
    /// worktree" and "this runner cannot do that yet" call for opposite
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
    #[error("worktrees still exist under this resource")]
    WorktreesExist,

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
    /// finished dialing and which will be there a second later.
    #[error("no agent is connected to this pane")]
    AgentNotConnected,

    /// A prompt, or any other message for the agent, sent to a pane whose
    /// shim has said the agent in it stopped or never started
    /// (`Terminal.agent_failure`).
    ///
    /// Split from `AgentNotConnected` (ov-174), which is retryable because
    /// its usual cause is a shim still dialing that will be there a second
    /// later. This one will not be: the adapter is gone, and nothing short of
    /// restarting the pane brings it back. Sent as that code, the apps
    /// retried it, and the phones said "Couldn't reach this runner" about a
    /// runner that had answered.
    ///
    /// The `Display` text is the plain sentence, because it is also
    /// `Error.message`, which is what an app too old to know this code shows
    /// under its own generic failure.
    #[error("The agent stopped. Restart it, then try again.")]
    AgentStopped,

    /// The database on this runner was written by a newer Far Cooler than
    /// this one, at a schema this build doesn't know.
    ///
    /// Refused rather than opened, because a schema this build has never seen
    /// can carry triggers and constraints its code would trip over, or worse,
    /// quietly work around. Raised only when the store is opened, so it ends a
    /// daemon's start, and the daemon then answers every connection with it
    /// (`farcooler_daemon::refusal`) instead of exiting into a restart loop.
    ///
    /// The `Display` text is the sentence itself, not log prose: this is the
    /// one error whose whole audience is a person deciding what to install,
    /// and it reaches them through the CLI, the Mac's runner status and a
    /// phone's connect screen, all of which show a runner's own words.
    #[error("This runner's data was written by a newer Far Cooler. Update Far Cooler to use it.")]
    NewerData,
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
            DomainError::Conflict { .. } => (ErrorCode::ResourceConflict, false),
            DomainError::NotFound => (ErrorCode::NotFound, false),
            DomainError::InvalidArgument { .. } => (ErrorCode::InvalidArgument, false),
            DomainError::IdempotencyMismatch => (ErrorCode::IdempotencyMismatch, false),
            DomainError::TmuxUnavailable => (ErrorCode::TmuxUnavailable, true),
            // Retryable: a loaded machine is slow, not broken.
            DomainError::TmuxTimedOut => (ErrorCode::TmuxTimedOut, true),
            DomainError::PathNotAllowed => (ErrorCode::PathNotAllowed, false),
            DomainError::SensitiveRoot => (ErrorCode::SensitiveRoot, false),
            DomainError::ConfirmationRequired => (ErrorCode::ConfirmationRequired, false),
            DomainError::WorktreesExist => (ErrorCode::WorktreesExist, false),
            // Retryable: a shim that has not finished dialing will be there.
            DomainError::AgentNotConnected => (ErrorCode::AgentNotConnected, true),
            // Never retryable: only a restart brings the agent back.
            DomainError::AgentStopped => (ErrorCode::AgentStopped, false),
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
            // The handshake's own code, naming which in `what`, the way
            // `Conflict` shares `ResourceConflict`'s: the two sides disagree
            // about versions, and only installing something fixes it.
            DomainError::NewerData => (ErrorCode::VersionIncompatible, false),
        }
    }

    /// WHICH argument this refusal is about, or `""` when the code says it all.
    ///
    /// Exhaustive, like `wire()` above and for the same reason: a variant
    /// added with a detail field and no arm here would cross the wire carrying
    /// nothing, which is the silence this exists to end.
    ///
    /// **A word to switch on, never text to show.** It is this daemon's own
    /// vocabulary — mostly the field name a caller got wrong, occasionally a
    /// phrase written for whoever is reading a log — and a client maps the
    /// words it knows onto its own sentences, exactly as it does with `code`.
    /// Nothing a caller sent can appear in it: `&'static str` means every
    /// value is a literal in this source.
    ///
    /// `ScopeDenied` and `CapabilityUnsupported` deliberately answer `""`
    /// though both carry a `needed`. For those two the CODE is already the
    /// whole meaning, their `needed` is read from a different table, and
    /// `CapabilityUnsupported`'s fallback in `Rpc::handle` is the prose "a
    /// newer Far Cooler" — putting that in a field documented as a machine
    /// word would invite the first client that trusted the documentation to
    /// print it.
    pub fn what(&self) -> &'static str {
        match self {
            DomainError::InvalidArgument { what } | DomainError::Conflict { what } => what,
            DomainError::NewerData => "newer_data",
            DomainError::AuthRequired
            | DomainError::ScopeDenied { .. }
            | DomainError::VersionIncompatible
            | DomainError::RepositoryLocked
            | DomainError::BranchExists
            | DomainError::WorktreeExists
            | DomainError::DirtyWorktree
            | DomainError::RunningProcesses
            | DomainError::OutputGap
            | DomainError::ClientTooSlow
            | DomainError::OperationFailed
            | DomainError::ResourceConflict
            | DomainError::NotFound
            | DomainError::IdempotencyMismatch
            | DomainError::TmuxUnavailable
            | DomainError::TmuxTimedOut
            | DomainError::PathNotAllowed
            | DomainError::SensitiveRoot
            | DomainError::ConfirmationRequired
            | DomainError::WorktreesExist
            | DomainError::AgentNotConnected
            | DomainError::AgentStopped
            | DomainError::BaseUnresolvable
            | DomainError::DiffTooLarge
            | DomainError::DiffUnsupported
            | DomainError::PrStateUnavailable
            | DomainError::AttachmentLimit
            | DomainError::DispatchUnknown
            | DomainError::CapabilityUnsupported { .. } => "",
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
        match self {
            DomainError::InvalidArgument { what } => match sentence(what) {
                Some(said) => said.to_string(),
                None => self.to_string(),
            },
            _ => self.to_string(),
        }
    }
}

/// The sentence for an argument word, where the runner has one a client can
/// say as it is.
///
/// Written for the workspace refusals (`crates/store/src/workspaces.rs`),
/// which reach a person from three apps and a CLI. The rule that a client
/// owns its sentences still stands: `Error.what` still carries the word, and
/// a client that has its own sentence for one uses it. This is what
/// `Error.message` carries instead of `invalid argument: task_prefix_taken`,
/// which is a Rust error and not a sentence, so a client that shows the
/// message shows something a person can act on.
///
/// Each word has its own sentence, because two refusals that read alike are
/// two different fixes a person can't tell apart. `None` for every other
/// word, whose message is unchanged.
pub fn sentence(what: &str) -> Option<&'static str> {
    SENTENCES.iter().find(|(word, _)| *word == what).map(|(_, said)| *said)
}

/// Every word `sentence` has a sentence for.
///
/// So a client can check it has its own line for each, from the table itself
/// rather than from a copy of it: a copy is what let a new word ship with no
/// line in the CLI while the CLI's test, reading its copy, stayed green.
pub fn sentence_words() -> impl Iterator<Item = &'static str> {
    SENTENCES.iter().map(|(word, _)| *word)
}

const SENTENCES: &[(&str, &str)] = &[
    ("task_prefix", "A prefix is a letter followed by up to seven letters or digits."),
    ("task_prefix_taken", "That prefix is already used by another workspace."),
    ("name", "A workspace needs a name."),
    ("main_workspace", "Main can't be deleted."),
    (
        "workspace_not_empty",
        "Move this workspace's tasks and worktrees to another workspace, then close the terminals left in it.",
    ),
    ("other_repository", "That workspace is in a different repository."),
    ("main_checkout", "The repository's main checkout always belongs to Main."),
    ("orchestrator_taken", "This workspace already has an orchestrator running."),
    ("orchestrator_home", "This workspace's folder couldn't be made, so its orchestrator wasn't started."),
    ("workspace", "This terminal doesn't belong to a workspace yet."),
    ("role", "Choose a role: shell, agent or orchestrator."),
    ("task_ids", "Name at least one task to move."),
    ("handoff_task", "That task isn't on this workspace's board."),
    // `terminal.agent_answer`'s two conflicts (`DomainError::Conflict`).
    ("not_held", "Someone already answered this."),
    ("not_delivered", "The answer didn't reach the agent. Try again."),
    // `terminal.dismiss_lost` on a terminal that is not lost (any more): it was
    // restarted or dismissed since the app drew it.
    ("not_lost", "That terminal isn't lost anymore."),
];

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
        ErrorCode::TmuxTimedOut => "tmux-timed-out",
        ErrorCode::PathNotAllowed => "path-not-allowed",
        ErrorCode::ConfirmationRequired => "confirmation-required",
        // The word keeps the old spelling: the apps match on it.
        ErrorCode::WorktreesExist => "workspaces-exist",
        ErrorCode::SensitiveRoot => "sensitive-root",
        ErrorCode::BaseUnresolvable => "base-unresolvable",
        ErrorCode::DiffTooLarge => "diff-too-large",
        ErrorCode::DiffUnsupported => "diff-unsupported",
        ErrorCode::PrStateUnavailable => "pr-state-unavailable",
        ErrorCode::AttachmentLimit => "attachment-limit",
        ErrorCode::DispatchUnknown => "dispatch-unknown",
        ErrorCode::CapabilityUnsupported => "capability-unsupported",
        ErrorCode::AgentNotConnected => "agent-not-connected",
        ErrorCode::AgentStopped => "agent-stopped",
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
            DomainError::TmuxTimedOut,
            DomainError::PathNotAllowed,
            DomainError::SensitiveRoot,
            DomainError::ConfirmationRequired,
            DomainError::WorktreesExist,
            DomainError::BaseUnresolvable,
            DomainError::DiffTooLarge,
            DomainError::DiffUnsupported,
            DomainError::PrStateUnavailable,
            DomainError::AttachmentLimit,
            DomainError::DispatchUnknown,
            DomainError::CapabilityUnsupported { needed: "changes" },
            DomainError::AgentNotConnected,
            DomainError::AgentStopped,
            DomainError::Conflict { what: "not_held" },
            DomainError::NewerData,
        ]
    }

    /// A named conflict is a conflict on the wire, and says which one.
    #[test]
    fn a_named_conflict_keeps_the_conflict_code_and_says_which() {
        for what in ["not_held", "not_delivered"] {
            let e = DomainError::Conflict { what };
            assert_eq!(e.wire(), DomainError::ResourceConflict.wire());
            assert_eq!(e.what(), what);
            let m = e.redacted_message();
            assert!(!m.contains('/') && !m.contains(what), "{m}");
        }
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
        // `Conflict` is `ResourceConflict`'s code on purpose, naming which in
        // `what`; `a_named_conflict_keeps_the_conflict_code_and_says_which`.
        // `NewerData` is `VersionIncompatible`'s the same way.
        for e in all_variants()
            .into_iter()
            .filter(|e| !matches!(e, DomainError::Conflict { .. } | DomainError::NewerData))
        {
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
            // `Conflict` shares `resource-conflict` by design, and `NewerData`
            // `version-incompatible`; see above.
            if matches!(e, DomainError::Conflict { .. } | DomainError::NewerData) {
                continue;
            }
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

    /// The detail crosses, and it is the variant's own.
    ///
    /// The silence this ends: `wire()` matches `InvalidArgument { .. }`, so a
    /// cycle, a blocker naming no task, a bad actor and an over-long title all
    /// reached a client as the bare word `invalid-argument` and a caller had to
    /// guess which producer had fired from the call it had just made.
    #[test]
    fn an_invalid_argument_names_which_argument() {
        assert_eq!(DomainError::InvalidArgument { what: "blocked_by" }.what(), "blocked_by");
        assert_eq!(DomainError::InvalidArgument { what: "cycle" }.what(), "cycle");
    }

    /// Every other variant answers nothing, and says nothing by accident.
    ///
    /// A code that is its own whole answer must not start carrying a second
    /// word a client could switch on -- and `ScopeDenied` and
    /// `CapabilityUnsupported` are the two that would be easiest to wire up by
    /// reflex, which is why they are named here rather than left to the sweep.
    #[test]
    fn every_other_refusal_carries_no_argument() {
        for e in all_variants() {
            if matches!(
                e,
                DomainError::InvalidArgument { .. } | DomainError::Conflict { .. } | DomainError::NewerData
            ) {
                continue;
            }
            assert_eq!(e.what(), "", "{e:?} started carrying an argument");
        }
        assert_eq!(DomainError::ScopeDenied { needed: "control" }.what(), "");
        assert_eq!(DomainError::CapabilityUnsupported { needed: "tasks" }.what(), "");
    }

    /// Every argument word this runner can actually send, read out of source.
    ///
    /// `all_variants()` cannot answer this and it is worth saying why, because
    /// the first version of the test below used it and could not fail. That
    /// list holds ONE `InvalidArgument`, built here, carrying `"columns"` --
    /// a word nothing emits. The hundred-odd real literals live in `validate`,
    /// in the daemon's routes and in the store, and a sweep that never looked
    /// at them proved a property of a seven-character fixture.
    ///
    /// A walk of the source tree, which is unusual and deliberate: this crate
    /// defines the contract, so the check that every producer keeps it belongs
    /// beside the contract rather than copied into each crate that has one.
    /// `crates/` is found from this crate's own manifest directory.
    ///
    /// Crude on purpose, in the same spirit as `rpc.rs`'s scan of its own
    /// method literals: a parser here would be a second thing that can be
    /// wrong. It finds each field initializer that opens a string and reads to
    /// the next quote, so a literal holding an escaped quote would be read
    /// short -- which is why the sweep refuses a backslash outright rather
    /// than trusting that none appears.
    ///
    /// The needle is assembled at runtime rather than written out, and that is
    /// not a flourish: spelled as a literal it appears in this file, the sweep
    /// finds ITSELF, and the first run failed on its own doc comment. Nothing
    /// in this function may contain the sequence it looks for.
    #[cfg(test)]
    fn argument_words_in_source() -> Vec<(String, String)> {
        let needle = format!("{}: {}", "what", '"');
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("this crate sits under crates/")
            .to_path_buf();

        let mut found = Vec::new();
        let mut stack = vec![crates];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else { continue };
                let mut rest = text.as_str();
                while let Some((_, after)) = rest.split_once(needle.as_str()) {
                    let Some((word, tail)) = after.split_once('"') else { break };
                    found.push((word.to_string(), path.display().to_string()));
                    rest = tail;
                }
            }
        }
        found
    }

    /// The sweep can see the tree, so the test below can fail.
    ///
    /// A walk that found nothing -- a moved crate, a packaged build, a broken
    /// path -- passes every assertion in it vacuously, which is exactly the
    /// shape of the test this replaced. So the reach is asserted first, and
    /// against the three crates that certainly hold producers.
    #[test]
    fn the_sweep_reaches_the_crates_that_produce_these_words() {
        let found = argument_words_in_source();
        assert!(found.len() > 50, "the sweep found only {} literals", found.len());
        for crate_dir in ["crates/core", "crates/daemon", "crates/store"] {
            assert!(
                found.iter().any(|(_, file)| file.contains(crate_dir)),
                "{crate_dir} produces these and the sweep never read it"
            );
        }
    }

    /// What crosses is one short line, and never a path.
    ///
    /// The field is documented as something to switch on rather than show, and
    /// this is what keeps it honest across every producer in the tree. It does
    /// NOT claim they are all single words -- a handful are phrases written
    /// for whoever is reading a daemon log, which is why the contract says a
    /// client maps the words it knows and writes its own sentence for the
    /// rest. What it does pin is that none of them is a paragraph, and that no
    /// path or session id can appear in one.
    ///
    /// Eighty characters because that is a line. The longest today is
    /// sixty-one; the room above it is for a producer that has something more
    /// to say, not for prose.
    #[test]
    fn every_argument_word_is_one_short_line_and_names_no_path() {
        for (word, file) in argument_words_in_source() {
            assert!(!word.is_empty(), "an empty argument word in {file}");
            assert!(!word.contains('/'), "{word:?} names a path, in {file}");
            assert!(!word.contains('\\'), "{word:?} carries an escape, in {file}");
            assert!(!word.contains('\n'), "{word:?} is more than a line, in {file}");
            assert!(
                !word.contains("session"),
                "{word:?} could name a session id, in {file}"
            );
            assert!(
                word.len() <= 80,
                "{word:?} is {} characters, which is prose and not a word, in {file}",
                word.len()
            );
        }
    }

    /// Every workspace refusal reads as its own sentence, and none of them as
    /// a Rust error.
    #[test]
    fn a_workspace_refusal_is_a_sentence_of_its_own() {
        let words = [
            "task_prefix",
            "task_prefix_taken",
            "name",
            "main_workspace",
            "workspace_not_empty",
            "other_repository",
            "main_checkout",
            "orchestrator_taken",
            "orchestrator_home",
            "workspace",
            "role",
            "task_ids",
        ];
        let mut seen = std::collections::HashSet::new();
        for what in words {
            let message = DomainError::InvalidArgument { what }.redacted_message();
            assert_eq!(Some(message.as_str()), sentence(what), "{what} is sent as its sentence");
            assert!(!message.contains("invalid argument"), "{what} reads as a Rust error: {message}");
            assert!(!message.contains('_'), "{what} puts a machine word on a screen: {message}");
            assert!(message.ends_with('.'), "{what} is a sentence: {message}");
            assert!(seen.insert(message.clone()), "{what} reads the same as another refusal");
            // The word still crosses for a client that has its own sentence.
            assert_eq!(DomainError::InvalidArgument { what }.what(), what);
        }
        // A word with no sentence here is left exactly as it was.
        assert_eq!(
            DomainError::InvalidArgument { what: "blocked_by" }.redacted_message(),
            "invalid argument: blocked_by"
        );
    }

    /// A tmux that answered slowly is not a tmux that is missing: the two
    /// reach a client as different words, because every client tells the
    /// second to install tmux (ov-176).
    #[test]
    fn a_slow_tmux_is_not_a_missing_one() {
        assert_ne!(DomainError::TmuxTimedOut.code(), DomainError::TmuxUnavailable.code());
        assert_eq!(word(DomainError::TmuxTimedOut.code()), "tmux-timed-out");
    }

    /// A pane whose agent stopped is not a pane whose shim is still dialing
    /// (ov-174): its own word, never retryable, and a sentence an older app
    /// can show as it is.
    #[test]
    fn a_stopped_agent_is_not_a_connecting_one() {
        assert_ne!(DomainError::AgentStopped.code(), DomainError::AgentNotConnected.code());
        assert_eq!(word(DomainError::AgentStopped.code()), "agent-stopped");
        assert!(!DomainError::AgentStopped.retryable());
        assert!(DomainError::AgentNotConnected.retryable());
        assert_eq!(DomainError::AgentStopped.redacted_message(), "The agent stopped. Restart it, then try again.");
    }

    #[test]
    fn retryable_is_stable_and_deliberate() {
        // Clients auto-retry on these; a wrong value means retrying forever.
        assert!(DomainError::RepositoryLocked.retryable());
        assert!(DomainError::TmuxUnavailable.retryable());
        assert!(DomainError::TmuxTimedOut.retryable());
        assert!(!DomainError::BranchExists.retryable());
        assert!(!DomainError::DirtyWorktree.retryable());
        assert!(!DomainError::ScopeDenied { needed: "control" }.retryable());
    }
}
