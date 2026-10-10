//! How long a phone waits for each call, and which calls go first (ov-147).
//!
//! The control connection now carries many calls at once, so a call's wait is
//! its own business: a diff taking a minute no longer holds up a keystroke,
//! and a keystroke on a link that has gone quiet fails on its own clock rather
//! than on the ssh keepalive's three missed beats.
//!
//! Three tiers, by what a person is doing while they wait:
//!
//! - **Input** — keys, pasted text, a resize. Sent ahead of anything
//!   still queued, and given 15 seconds: a key that has had no answer by then
//!   is on a link worth telling the person about. The deadline cannot unsend
//!   it; a key already on the wire may still arrive.
//! - **Work** — a diff, a commit, a git query, a worktree or pane being made,
//!   an agent being started or steered. These run git, tmux or an agent on
//!   the runner and can honestly take minutes on a big repository; two.
//! - **Everything else** — reads of what the runner already knows, and small
//!   changes to it; 30 seconds.
//!
//! A deadline does not end the session. `SessionError::TimedOut` is not a
//! disconnect, because one slow answer is not a dead link; the keepalive and
//! the reader decide that, and when they do every waiting call fails at once.
//!
//! Calls made through `actions` — a paste, a worktree hidden or removed, a
//! root added — have no deadline, as the CLI that shares them never has.

use std::time::Duration;

use farcooler_transport::CallOptions;

/// The deadline for input.
pub const INPUT: Duration = Duration::from_secs(15);
/// The deadline for work that runs git, tmux or an agent.
pub const WORK: Duration = Duration::from_secs(120);
/// The deadline for every other call.
pub const ORDINARY: Duration = Duration::from_secs(30);

/// Keystrokes and the size they are typed into: what a person is waiting on
/// with their fingers.
const INPUT_METHODS: &[&str] = &["terminal.write", "terminal.resize"];

/// Prefixes and names of calls that do real work on the runner.
const WORK_METHODS: &[&str] = &[
    "changes.",
    "pr.refresh",
    "stack.get",
    "branch.list",
    "adapter.test",
    "worktree.create",
    "worktree.file_search",
    "terminal.create",
    "terminal.agent_",
    "workspace.start_orchestrator",
    "usage.report",
    // The first page of a terminal's agent rows reads its whole transcript
    // (ov-366). A follow is held at most 25 s, inside the ordinary 30.
    "agent.rows",
    // The first piece of a prompt's image may read the transcript back the
    // same way, and then a line of up to 8 MB (ov-454).
    "agent.image",
    // A send waits out the spacing after the last, a paste's read-back and,
    // mid-turn, the queue's confirmation: give up early and the person sends
    // again what the runner then types anyway (ov-372 review).
    "terminal.compose",
    // A clear presses ctrl+u a row at a time, reading the box back after each
    // (about a second a row), after waiting up to 10 s for a send to let go.
    "terminal.bring_draft",
];

/// How the session makes a call to `method`.
pub fn for_method(method: &str) -> CallOptions {
    if INPUT_METHODS.contains(&method) {
        return CallOptions { deadline: Some(INPUT), urgent: true };
    }
    let work = WORK_METHODS
        .iter()
        .any(|m| if m.ends_with('.') || m.ends_with('_') { method.starts_with(m) } else { method == *m });
    CallOptions { deadline: Some(if work { WORK } else { ORDINARY }), urgent: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missed_deadline_is_its_own_failure_and_not_a_dropped_link() {
        use crate::session::SessionError;
        let late: SessionError = farcooler_transport::ClientError::TimedOut {
            method: "changes.file_diff".into(),
            after: WORK,
        }
        .into();
        assert!(matches!(late, SessionError::TimedOut { .. }), "{late:?}");
        assert_eq!(late.word(), "timed_out");
        // A sentence a person can read, with no method name or Rust duration
        // in it: the FFI hands this over as `error`.
        assert_eq!(late.to_string(), "The runner took too long to answer. Try again.");
        assert!(!late.is_disconnect(), "a slow answer must not empty the session slot");
    }

    #[test]
    fn a_bring_here_clear_gets_the_work_deadline() {
        assert_eq!(for_method("terminal.bring_draft").deadline, Some(WORK));
    }

    #[test]
    fn keys_go_first_and_fail_soonest() {
        for method in INPUT_METHODS {
            let how = for_method(method);
            assert!(how.urgent, "{method}");
            assert_eq!(how.deadline, Some(INPUT), "{method}");
        }
    }

    /// A mark is a small write to what the runner already holds, so it waits
    /// like any other: not ahead of anything (an app queues them after a board
    /// read and a failed one is sent again), and not for minutes.
    #[test]
    fn marking_a_board_read_is_an_ordinary_call() {
        let how = for_method("workspace.mark_read");
        assert_eq!(how.deadline, Some(ORDINARY));
        assert!(!how.urgent);
    }

    #[test]
    fn git_and_agents_get_the_long_deadline_and_reads_the_short_one() {
        for method in ["changes.file_diff", "changes.commit_files", "pr.refresh", "terminal.agent_prompt", "terminal.compose"] {
            assert_eq!(for_method(method).deadline, Some(WORK), "{method}");
            assert!(!for_method(method).urgent, "{method}");
        }
        for method in
            ["terminal.list", "terminal.screen", "worktree.list", "task.get", "pr.refreshed", "workspace.mark_read"]
        {
            assert_eq!(for_method(method).deadline, Some(ORDINARY), "{method}");
        }
    }
}
