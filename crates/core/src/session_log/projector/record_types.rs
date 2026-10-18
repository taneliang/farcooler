//! Which of claude's record types the fold reads silently, and what it does
//! with one nothing reads (ov-621).

use std::hash::{Hash, Hasher};

use super::fold::Projection;
use super::rows::GapReason;

/// Record types claude writes that carry nothing a row is made of. Anything
/// else unrecognized becomes a `Gap`, so a new record type is seen, not lost.
const SILENT_TYPES: &[&str] = &[
    "frame-link",
    "artifact-autoreact-ledger",
    "artifact-comment-monitor",
    "mode",
    "permission-mode",
    "atis-latch",
    "last-prompt",
    "cost-state",
    "file-history-snapshot",
    "file-history-delta",
    "ai-title",
    "agent-name",
    "custom-title",
    "bridge-session",
    "pr-link",
    "summary",
    "tag",
];

/// claude 2.1.300's prompt-cache bookkeeping, `api-request`,
/// `api-request-blob` (a request's deduplicated messages, `message.role` and
/// all, never a turn) and `api-request-shape`, and any sibling it adds.
const SILENT_PREFIX: &str = "api-request";

/// The most unknown types one session logs: past these, a transcript
/// inventing types can't fill the log or this set.
const MOST_LOGGED: usize = 32;

/// Whether a record of type `kind` is read silently.
pub(super) fn silent(kind: &str) -> bool {
    SILENT_TYPES.contains(&kind) || kind.starts_with(SILENT_PREFIX)
}

/// `kind` as a log may name it: a plain word of at most 64 characters, or
/// nothing of it at all.
fn loggable(kind: &str) -> &str {
    let plain = !kind.is_empty() && kind.len() <= 64 && kind.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if plain { kind } else { "(not a plain name)" }
}

impl Projection {
    /// A record of a type nothing reads: a `Gap`, so it's seen, and its type
    /// logged the first time this session meets it, by name only.
    pub(super) fn unknown_type(&mut self, kind: &str) {
        // Kept as a hash: a type can be as long as a line.
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        kind.hash(&mut hasher);
        if self.unknown_logged.len() < MOST_LOGGED && self.unknown_logged.insert(hasher.finish()) {
            tracing::warn!(record_type = loggable(kind), "a session record of a type the projector doesn't read");
        }
        self.gap(GapReason::Unknown(kind.to_string()));
    }
}
