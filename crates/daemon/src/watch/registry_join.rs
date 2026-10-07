//! A claude pane's log, joined by claude's own registry before any guess
//! (ov-365). Its own file to keep `watch.rs` inside its size budget.
//!
//! The registry names the session of the process in the pane by its pid, so
//! the join needs neither the pane's title nor a count of the worktree's
//! files. And it moves when the session does: `/clear` rewrites the pid's
//! registry file with a new session, and the next tick follows it, rather
//! than waiting for the old file to look dead (`join_looks_dead`).

use std::path::PathBuf;

use uuid::Uuid;

use super::{LogFormat, PaneJoin, PaneLog};
use crate::claude_registry::Registry;

/// What the registry says this pane's log is, if it says anything.
pub(super) fn registered_log(registry: &Registry, pane: &PaneJoin) -> Option<PathBuf> {
    crate::registry_binding::registered_log(registry, pane.preset.as_deref(), pane.pid, &pane.cwd)
}

/// The pane's tick for its session projector (`session_projectors`): what its
/// files gained, and the registry's busy or idle. Opens one for a claude pane
/// the registry names only while the daemon shadows (`FARCOOLER_PROJECTOR=1`).
///
/// An open projector follows the registry too: when the registry names a
/// different file than the one it reads (a `/clear` whose `SessionStart` was
/// missed), it is moved there, as the watcher's own log join is.
pub(super) fn feed_projector(registry: &Registry, terminal: Uuid, pane: &PaneJoin) {
    let projectors = crate::session_projectors::global();
    let open = projectors.transcript(terminal);
    // Off means off (ov-372 review): a projector opened before the setting
    // went off is let go on its pane's next tick, not followed until a
    // restart. Turned on again, the pane's next `agent.rows` rebuilds it in
    // a new epoch, and a follower pages again.
    if !crate::session_projectors::shadowing() {
        if open.is_some() {
            projectors.forget(terminal);
        }
        return;
    }
    let registered = registered_log(registry, pane);
    match (&open, registered) {
        (None, None) => return,
        (None, Some(path)) => projectors.open(terminal, path),
        // The projector names its file as the filesystem does (`canonical`).
        (Some(current), Some(path)) if *current != crate::session_projectors::canonical(&path) => projectors.open(terminal, path),
        _ => {}
    }
    let claude = pane.preset.as_deref().is_some_and(|p| p.starts_with("claude"));
    let entry = pane.pid.filter(|_| claude).and_then(|pid| registry.by_pid(pid));
    projectors.tick(terminal, entry.and_then(|e| e.status));
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
    use crate::claude_registry::{Registry, fake};

    /// A config dir with pid 4242 running session `session`, whose transcript
    /// holds one prompt.
    fn session(config: &std::path::Path, session: &str) {
        fake::write(config, 4242, session, "/nonexistent/fc-registry-join");
        let project = config.join("projects/-nonexistent-fc-registry-join");
        std::fs::create_dir_all(&project).unwrap();
        let prompt = format!(r#"{{"type":"user","promptId":"{session}-p","promptSource":"typed","message":{{"role":"user","content":"hi"}}}}"#);
        std::fs::write(project.join(format!("{session}.jsonl")), format!("{prompt}\n")).unwrap();
    }

    fn pane() -> PaneJoin {
        PaneJoin { preset: Some("claude".into()), pid: Some(4242), cwd: "/nonexistent/fc-registry-join".into(), title: String::new() }
    }

    /// The call site: `turn_from_log` joins a claude pane's log through the
    /// service's registry, with nothing for the title-and-files guess to find,
    /// and follows the pid to its new session after `/clear`.
    #[tokio::test]
    async fn the_watcher_joins_and_follows_a_pane_by_the_registry() {
        let (_dir, svc, _repo) = crate::test_support::fixture().await;
        let config = tempfile::tempdir().unwrap();
        session(config.path(), "s-one");
        let registry: &'static Registry =
            Box::leak(Box::new(Registry::new(config.path().to_path_buf(), Box::new(fake::Alive(vec![(4242, None)])))));
        let _ = svc.hooks().clone().with_registry(registry);
        let watcher = super::super::Watcher::new(svc.clone());
        let id = uuid::Uuid::now_v7();
        let (reading, _) = watcher.turn_from_log(id, pane(), 10_000, true, false).await;
        assert!(reading.turn.is_some_and(|t| t.running), "joined by the registry: {reading:?}");

        session(config.path(), "s-two");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut moved = false;
        let mut now = 11_000;
        while !moved && std::time::Instant::now() < deadline {
            let (_, events) = watcher.turn_from_log(id, pane(), now, true, false).await;
            moved = !events.is_empty();
            now += 1_000;
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(moved, "the pane followed its pid to the new session's log");
    }
    use farcooler_core::session_log::tail::Tail;

    fn reading(path: &str, format: LogFormat) -> PaneLog {
        let mut log = PaneLog::new();
        log.tail = Some((Tail::new(PathBuf::from(path)), format));
        log
    }

    /// An open projector moves when the registry names another file, even
    /// with the `SessionStart` that would have moved it never seen.
    #[test]
    fn an_open_projector_follows_the_registry_to_a_new_session() {
        // With the setting off an open projector is let go (ov-372), so on.
        // Nothing else in this binary reads the flag off.
        crate::session_projectors::set_shadowing(true);
        let config = tempfile::tempdir().unwrap();
        session(config.path(), "s-one");
        let registry = Registry::new(config.path().to_path_buf(), Box::new(fake::Alive(vec![(4242, None)])));
        let id = uuid::Uuid::now_v7();
        let first = crate::session_projectors::canonical(&registered_log(&registry, &pane()).unwrap());
        crate::session_projectors::global().open(id, first.clone());
        session(config.path(), "s-two");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while crate::session_projectors::global().transcript(id).as_ref() == Some(&first) && std::time::Instant::now() < deadline {
            feed_projector(&registry, id, &pane());
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let now = crate::session_projectors::global().transcript(id).unwrap();
        crate::session_projectors::global().forget(id);
        assert!(now.ends_with("s-two.jsonl"), "{now:?}");
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
