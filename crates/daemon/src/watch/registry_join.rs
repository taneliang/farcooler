//! A claude pane's log, joined by claude's own registry before any guess
//! (ov-365). Its own file to keep `watch.rs` inside its size budget.
//!
//! The registry names the session of the process in the pane by its pid, so
//! the join needs neither the pane's title nor a count of the worktree's
//! files. And it moves when the session does: `/clear` rewrites the pid's
//! registry file with a new session, and the next tick follows it, rather
//! than waiting for the old file to look dead (`join_looks_dead`).

use std::path::PathBuf;

use super::{LogFormat, PaneJoin, PaneLog};

/// What the registry says this pane's log is, if it says anything.
pub(super) fn registered_log(pane: &PaneJoin) -> Option<PathBuf> {
    let registry = crate::claude_registry::global();
    crate::registry_binding::registered_log(registry, pane.preset.as_deref(), pane.pid, &pane.cwd)
}

/// Move a claude pane that is reading one session's log onto the one the
/// registry now names. Nothing for a pane with no log yet (the ordinary join
/// takes it), another agent's, or one already on the named file.
pub(super) fn follow_registry(log: &mut PaneLog, registered: Option<PathBuf>) {
    let Some(path) = registered else { return };
    let moved = log.tail.as_ref().is_some_and(|(tail, format)| *format == LogFormat::Claude && tail.path() != path);
    if moved {
        log.adopt(Some((path, LogFormat::Claude)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_core::session_log::tail::Tail;

    fn reading(path: &str, format: LogFormat) -> PaneLog {
        let mut log = PaneLog::new();
        log.tail = Some((Tail::new(PathBuf::from(path)), format));
        log
    }

    #[test]
    fn a_cleared_session_moves_the_pane_to_its_new_log() {
        let mut log = reading("/p/old.jsonl", LogFormat::Claude);
        follow_registry(&mut log, Some(PathBuf::from("/p/new.jsonl")));
        assert_eq!(log.tail.as_ref().map(|(t, _)| t.path().to_path_buf()), Some(PathBuf::from("/p/new.jsonl")));
    }

    #[test]
    fn the_same_log_another_agents_or_no_answer_moves_nothing() {
        let mut same = reading("/p/s.jsonl", LogFormat::Claude);
        same.asked = Some(super::super::Ask { id: "q".into(), question: String::new() });
        follow_registry(&mut same, Some(PathBuf::from("/p/s.jsonl")));
        assert!(same.asked.is_some(), "the file it is on keeps its question");
        let mut codex = reading("/p/rollout.jsonl", LogFormat::Codex);
        follow_registry(&mut codex, Some(PathBuf::from("/p/s.jsonl")));
        assert_eq!(codex.tail.as_ref().map(|(t, _)| t.path().to_path_buf()), Some(PathBuf::from("/p/rollout.jsonl")));
        let mut none = PaneLog::new();
        follow_registry(&mut none, Some(PathBuf::from("/p/s.jsonl")));
        assert!(none.tail.is_none(), "the ordinary join takes a pane with no log");
    }
}
