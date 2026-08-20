//! What an agent writes down as it works.
//!
//! Every parser here turns one agent's private JSONL into the same small
//! vocabulary, so the daemon does not learn three formats. See
//! `docs/agent-session-logs.md` for what each agent actually writes — observed,
//! not documented, and the only thing to trust about these formats.
//!
//! None of them records that it is waiting on a PERMISSION decision. That state
//! stays with the footer-scoped screen matching, which is the one place it can
//! be seen at all.
//!
//! One narrower wait IS written down, and only claude writes it: a tool call
//! whose result can arrive from nowhere but a human. `AskUserQuestion` is that
//! tool, and an outstanding one is not an inference about a quiet log — it is
//! the agent stating which question it is holding, in its own file, before it
//! stops. See [`TurnEvent::Asked`].

pub mod claude;

pub mod codex;

pub mod cursor;

pub mod tail;

/// One thing that happened in a session.
///
/// Deliberately small. Three formats with nothing in common map onto this, and
/// anything richer would be one agent's vocabulary imposed on the others.
///
/// `TaskState` and `Subagent` are, today, claude's alone — codex writes
/// `update_plan` in one rollout of 264 and cursor writes nothing task-shaped
/// at all, so neither has anything to say here yet. That is not a gap to paper
/// over: a pane with no task list falls back to its action line and its
/// transcript, which is what codex and cursor and most claude sessions do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnEvent {
    /// The human asked for something.
    Started { at_ms: Option<i64> },
    /// Prose the agent emitted, verbatim. The only thing that belongs in a
    /// transcript: it is what the agent SENT, in its own words, with no
    /// parser's verb in front of it.
    ///
    /// Both the running narration and the closing answer, because a transcript
    /// that only ever accepted the answer was EMPTY for the whole time anybody
    /// was watching — a final answer does not exist while a turn is in
    /// progress, so a short codex turn said nothing at all until it was over,
    /// which reads exactly like an integration nobody built.
    Said {
        text: String,
        /// Whether this is the turn's closing answer rather than narration on
        /// the way there.
        ///
        /// The distinction each agent draws in its own words: claude's
        /// `stop_reason == "end_turn"`, codex's `phase == "final_answer"`.
        /// Cursor draws none, so nothing it says is marked — see its parser.
        ///
        /// It survives because the two are read DIFFERENTLY, not because it
        /// might be useful later: narration is a stream, so the window takes
        /// its tail, and an answer is a summary, so the window takes its head.
        /// See `feed::Feed::conclude`.
        conclusion: bool,
    },
    /// A tool action. NOT transcript — a row's fallback "current action" line
    /// for an agent with no task list, which is codex, cursor, and most claude
    /// sessions.
    Did { verb: String, object: String },
    /// A question only a human can answer, and the agent is holding for it.
    ///
    /// The one wait an agent writes down. Everything else in this vocabulary
    /// describes a turn that is moving; this one says it has stopped, and why —
    /// which is the sentence the whole notification path exists to send.
    ///
    /// Claude's `AskUserQuestion`, and today nothing else. Counted across every
    /// transcript on this machine: 197 calls in 28 sessions, and all 197 are
    /// closed by a `tool_result` carrying the call's own `id` — so the pair is
    /// total, and an `Asked` with no `Answered` after it is the agent waiting
    /// rather than a shape this parser failed to read. The wait is real and
    /// long: a median of 101 seconds, 10% of them over 13 minutes, and the
    /// longest 21 hours. That is a row that read `working` for 21 hours.
    ///
    /// This does NOT reopen permission prompts. Those are still written down
    /// nowhere — claude records the OUTCOME of a decision and never the pending
    /// state — and the screen remains the only place they can be seen. The
    /// distinction that matters is not "the log knows the agent is blocked", it
    /// is "the agent called a tool that no machine can answer".
    Asked {
        /// The `tool_use` id, which the closing `tool_result` names as its
        /// `tool_use_id`. The join key, so a second question asked after the
        /// first was answered does not clear itself.
        id: String,
        /// The question, in the agent's own words, for the notification to
        /// carry. Empty when the call stated none.
        question: String,
    },
    /// A tool call coming back, by the id it was made under.
    ///
    /// Emitted for every `tool_result`, not only a question's, because a result
    /// block does not name the tool it came from (see `claude::tool_result`) —
    /// so the id is the only thing that can close an [`Asked`], and the fold
    /// that holds one is the only thing that cares which ids these are.
    Answered { id: String },
    /// One task in the agent's list, as ONE line stated it.
    ///
    /// Not a tally, though the stage-3 plan asked for `{ done, total,
    /// active_form }`. No line claude writes carries a tally: `TaskCreate`
    /// states a phrase and no id (the id comes back in the result),
    /// `TaskUpdate` states an id and a status and no totals, and only
    /// `TaskList` states a whole list — asked for in 6 of the 340 sessions on
    /// this machine that use tasks at all. `parse_line` is one line at a time
    /// by contract, so the running tally is the daemon's to fold (plan task
    /// 4), out of these per-task facts. See `claude::task_state` for how the
    /// phrase and the id are joined, and why.
    ///
    /// Which halves are present is what says what happened, and a fold reads
    /// all three shapes:
    ///
    /// | `id` | `status` | the line was |
    /// | --- | --- | --- |
    /// | `None` | `None` | a create, carrying the phrase and no id yet |
    /// | `Some` | `None` | that create's result, carrying the id and nothing else |
    /// | `Some` | `Some` | a task moving, or a listed task |
    TaskState {
        /// The task's own id (`"1"`), as its create's RESULT, `TaskUpdate` and
        /// `TaskList` all name it. `None` on the create itself: claude does not
        /// assign the id until the tool result comes back.
        id: Option<String>,
        /// `TaskCreate`'s `activeForm` — already a present-tense human phrase
        /// ("Designing test matrix"), used verbatim. `None` on any line that
        /// does not create a task, and on the 34 of 289 creates that carried
        /// no `activeForm`.
        active_form: Option<String>,
        /// Where the line put the task, when it said something this parser
        /// recognizes.
        status: Option<TaskStatus>,
    },
    /// One agent this agent spawned, and whether it is still going.
    ///
    /// `id` is the `tool_use` id that spawned it, which is also the
    /// `toolUseId` in the sibling `subagents/agent-<id>.meta.json` (plan task
    /// 3), so the two can be joined without a heuristic.
    Subagent { id: String, description: String, running: bool },
    /// The turn is over, however it went.
    Ended { at_ms: Option<i64>, duration_ms: Option<i64>, outcome: TurnOutcome },
    /// The agent's own name for this session's work.
    Title(String),
    /// How many background agents are still running.
    BackgroundAgents(u32),
}

/// Where a task sits, in the agent's own words.
///
/// Claude's four observed `status` values, and no fifth: an unrecognized one
/// yields no event at all rather than a guess, because a status read wrongly
/// moves a progress count that a person is reading from a lock screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Deleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnOutcome {
    Finished,
    Aborted,
    Failed,
}

/// Claude's project directory name for a working directory.
///
/// Each character is replaced on its own and runs are NOT collapsed, so
/// `/.claude` becomes `--claude`. That double hyphen is real: it is how the
/// directories on disk are named, and collapsing would send a reader to a path
/// that does not exist. Cursor's rule is different — see `cursor_slug`.
pub fn claude_slug(cwd: &str) -> String {
    cwd.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect()
}

/// Cursor's project directory name for a working directory.
///
/// From cursor's own shipped source: replace every non-alphanumeric, collapse
/// runs, trim the ends, and if the joined path would exceed 92 characters cut
/// the slug to 84 and append `-` plus seven hex of the full path's sha256.
///
/// The hash here is FNV-1a, not sha256, even though sha2 is already a workspace
/// dependency (`crates/review`, `crates/daemon`) — just not of `core`. Adding a
/// crypto crate to this crate's dependency graph to spell a directory suffix is
/// not worth it, because nothing needs the digest to match cursor's exactly:
/// Far Cooler only ever reads a truncated cursor directory after LISTING
/// `~/.cursor/projects/`, so it can find the one whose name starts with the
/// 84-character prefix, whatever seven hex characters follow it. Matching
/// cursor's exact hash would only matter if something had to compute the path
/// and open it blind, which nothing here does.
pub fn cursor_slug(cwd: &str) -> String {
    let mut out = String::with_capacity(cwd.len());
    for c in cwd.chars() {
        let c = if c.is_ascii_alphanumeric() { c } else { '-' };
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }
    let out = out.trim_matches('-').to_string();
    // 92 is cursor's own bound on `.cursor/projects/<slug>`, so the budget for
    // the slug is what is left after that prefix.
    const JOINED_MAX: usize = 92;
    const PREFIX: usize = ".cursor/projects/".len();
    if PREFIX + out.len() <= JOINED_MAX {
        return out;
    }
    let keep = 84usize.saturating_sub(PREFIX).min(out.len());
    format!("{}-{}", &out[..keep], short_hash(cwd))
}

/// Seven hex characters, stable for one `cwd`, distinct enough to tell apart
/// two long paths that happen to share an 84-character prefix.
///
/// See the note on `cursor_slug`: this does not need to reproduce cursor's own
/// sha256 digest, only to be deterministic, because callers find the real
/// directory by listing and matching the prefix rather than by recomputing its
/// full name.
fn short_hash(input: &str) -> String {
    // FNV-1a, 64-bit. Chosen for being a few lines with no dependency, not for
    // any cryptographic property — nothing here needs one.
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for byte in input.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")[..7].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_does_not_collapse_runs() {
        // A dot directory produces TWO hyphens, because claude replaces each
        // character separately. Observed across 35 real project directories.
        assert_eq!(claude_slug("/Users/e-liang/.unixconfig"), "-Users-e-liang--unixconfig");
        assert_eq!(
            claude_slug("/Users/e-liang/Dev/overnight/.claude/worktrees/agent-session-logs"),
            "-Users-e-liang-Dev-overnight--claude-worktrees-agent-session-logs"
        );
        assert_eq!(claude_slug("/private/tmp/claude-title-clean-r4x"), "-private-tmp-claude-title-clean-r4x");
    }

    #[test]
    fn cursor_collapses_runs_and_trims() {
        // Cursor's own source: replace non-alphanumerics, collapse, trim.
        assert_eq!(cursor_slug("/Users/e-liang/.unixconfig"), "Users-e-liang-unixconfig");
        assert_eq!(cursor_slug("/Users/e-liang/Dev/Verdela"), "Users-e-liang-Dev-Verdela");
    }

    /// Cursor cuts a long path and signs it. Claude has no observed equivalent.
    #[test]
    fn a_long_path_is_cut_and_hashed_for_cursor_only() {
        let long = format!("/Users/e-liang/{}", "x".repeat(200));
        let slug = cursor_slug(&long);
        let joined = format!(".cursor/projects/{slug}");
        assert!(joined.len() <= 92, "cursor guards at 92 chars, got {}", joined.len());
        assert!(slug.len() > 8 && slug.contains('-'), "the hash suffix is kept: {slug}");
        // Claude is left alone: no truncation was ever observed, and inventing one
        // would send us looking in a directory that does not exist.
        assert!(claude_slug(&long).len() > 100);
    }

    /// The hash suffix does not need to match cursor's own sha256 — see the
    /// note on `cursor_slug` — but it does need to be deterministic, or a
    /// second lookup for the same directory would compute a different prefix
    /// than the first one wrote.
    #[test]
    fn the_hash_suffix_is_deterministic() {
        let long = format!("/Users/e-liang/{}", "y".repeat(200));
        assert_eq!(cursor_slug(&long), cursor_slug(&long));
    }

    /// Two long paths that share an 84-character prefix must not collide, or a
    /// prefix-matching lookup could not tell their directories apart.
    #[test]
    fn two_long_paths_sharing_a_prefix_hash_differently() {
        let a = format!("/Users/e-liang/{}/a-tail", "x".repeat(200));
        let b = format!("/Users/e-liang/{}/b-tail", "x".repeat(200));
        assert_ne!(cursor_slug(&a), cursor_slug(&b));
    }
}
