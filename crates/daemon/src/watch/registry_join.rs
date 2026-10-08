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

/// What claude's box shows, for its pane's projector (ov-409): the prompt it
/// suggests and its generic `Try` example. Only a claude at rest has either:
/// mid-turn, a dim line is a hint, never a prediction (`composer::suggestion`).
/// Read from the screen the watcher already holds; a pane with no projector
/// open ignores it.
pub(super) fn feed_suggestion(terminal: Uuid, agent: Option<&str>, screen: &str, resting: bool) {
    if !crate::session_projectors::shadowing() {
        return;
    }
    let (suggestion, hint) = offered(agent, screen, resting);
    crate::session_projectors::global().suggest(terminal, suggestion, hint);
}

/// What `feed_suggestion` hands the projector, as the prediction and the
/// example: each only while the agent rests.
pub(super) fn offered(agent: Option<&str>, screen: &str, resting: bool) -> (Option<String>, Option<String>) {
    let read = |f: fn(&str, &str) -> Option<String>| agent.filter(|_| resting).and_then(|preset| f(preset, screen));
    (read(farcooler_core::composer::suggestion), read(farcooler_core::composer::hint))
}

/// The pane's tick for its session projector (`session_projectors`): what its
/// files gained, and the registry's busy or idle. Opens one for a claude pane
/// the registry names only while the daemon shadows (`FARCOOLER_PROJECTOR=1`).
///
/// An open projector follows the registry too: when the registry names a
/// different file than the one it reads (a `/clear` whose `SessionStart` was
/// missed), it is moved there, as the watcher's own log join is.
pub(super) fn feed_projector(registry: &Registry, terminal: Uuid, pane: &PaneJoin) {
    // codex's own (`feed_codex_projector`).
    if is_codex(pane) {
        return;
    }
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

/// A codex this daemon launched, in a pane whose foreground command reads
/// as the shell that launched it: fish runs `-c` without job control, so the
/// pane's command stays `fish`, and once codex's banner scrolls away its
/// screen names it nowhere (measured, ov-416). Named codex for the log join
/// and its projector alone, which then prove it by the rollout its process
/// holds open: a codex that has exited holds none, and nothing is joined.
pub(super) fn codex_under_a_shell(launched: &str, command: &str) -> Option<String> {
    let shell = matches!(command.rsplit('/').next().unwrap_or(command).trim_start_matches('-'), "fish" | "sh" | "bash" | "zsh");
    (launched.split(':').next() == Some("codex") && shell).then(|| "codex".to_string())
}

fn is_codex(pane: &PaneJoin) -> bool {
    pane.preset.as_deref().is_some_and(|p| p.starts_with("codex"))
}

/// A codex pane's projector (ov-378): opened on the rollout the pane's codex
/// process holds open, joined afresh (at most `JOIN_KEPT` old) rather than
/// through the watcher's own join (`log`, the fallback with no pid), which
/// moves only after its log has been quiet for half a minute; and moved
/// when that changes (`/new` starts a thread and a rollout of its own).
/// Joined so, a hook's rebind to the new thread and the next tick agree,
/// rather than taking turns (review 1, M2). Read on every tick. Its activity
/// is the rollout's own, so the tick passes none. Only while the daemon
/// shadows, as `feed_projector`.
pub(super) fn feed_codex_projector(terminal: Uuid, pane: &PaneJoin, log: &PaneLog) {
    if !is_codex(pane) {
        return;
    }
    let projectors = crate::session_projectors::global();
    let open = projectors.transcript(terminal);
    if !crate::session_projectors::shadowing() {
        if open.is_some() {
            projectors.forget(terminal);
        }
        return;
    }
    let rollout = match pane.pid {
        Some(pid) => super::answer_wake::codex_turn::rollout_of(pid),
        None => log.tail.as_ref().filter(|(_, format)| *format == LogFormat::Codex).map(|(tail, _)| tail.path().to_path_buf()),
    };
    match (&open, rollout) {
        (None, None) => return,
        (None, Some(path)) => projectors.open(terminal, path),
        (Some(current), Some(path)) if *current != crate::session_projectors::canonical(&path) => projectors.open(terminal, path),
        _ => {}
    }
    projectors.tick(terminal, None);
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

    const ROLLOUT: &str = include_str!(
        "../../../core/fixtures/session-logs/codex-tui-0.153.4/rollout-2026-10-07T12-22-14-01a117d0-fd5c-7fe3-8f0f-5f2afdf7906d.jsonl"
    );
    const NEW_ROLLOUT: &str = include_str!(
        "../../../core/fixtures/session-logs/codex-tui-0.153.4/rollout-2026-10-07T12-28-45-01a117d6-f550-7013-a939-b02d182c0321.jsonl"
    );

    /// A codex launched here, under a shell that keeps the pane's command
    /// (fish), is codex to the join; a shell pane, or claude's, is not.
    #[test]
    fn a_codex_under_fish_is_joined_as_codex() {
        assert_eq!(codex_under_a_shell("codex", "fish").as_deref(), Some("codex"));
        assert_eq!(codex_under_a_shell("codex:gpt-5", "-zsh").as_deref(), Some("codex"));
        assert_eq!(codex_under_a_shell("shell", "fish"), None);
        assert_eq!(codex_under_a_shell("claude", "fish"), None);
        assert_eq!(codex_under_a_shell("codex", "vim"), None, "something else in front");
    }

    /// A codex pane's projector opens on the rollout the watcher's join reads,
    /// its rows and its activity the rollout's own, and moves with `/new`.
    #[test]
    fn a_codex_pane_s_projector_follows_its_rollout() {
        crate::session_projectors::set_shadowing(true);
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("rollout-2026-10-07T12-22-14-01a117d0-fd5c-7fe3-8f0f-5f2afdf7906d.jsonl");
        let second = dir.path().join("rollout-2026-10-07T12-28-45-01a117d6-f550-7013-a939-b02d182c0321.jsonl");
        std::fs::write(&first, ROLLOUT).unwrap();
        std::fs::write(&second, NEW_ROLLOUT).unwrap();
        // No pid: the watcher's own join is what it follows.
        let pane = PaneJoin { preset: Some("codex".into()), pid: None, cwd: "/nonexistent".into(), title: String::new() };
        let id = uuid::Uuid::now_v7();
        let projectors = crate::session_projectors::global();

        feed_codex_projector(id, &pane, &reading(first.to_str().unwrap(), LogFormat::Codex));
        let turns = |rows: Vec<farcooler_core::session_log::projector::Row>| {
            rows.into_iter()
                .filter_map(|r| match r.kind {
                    farcooler_core::session_log::projector::RowKind::Turn(t) => Some((t.prompt, t.activity)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let rows = turns(projectors.page(id, None, 500).expect("opened"));
        assert_eq!(rows.len(), 11, "{rows:?}");
        assert_eq!(rows.last().unwrap().1, Some(farcooler_core::session_log::projector::Activity::Idle), "the rollout's own");

        feed_codex_projector(id, &pane, &reading(second.to_str().unwrap(), LogFormat::Codex));
        let now = projectors.transcript(id).unwrap();
        let rows = turns(projectors.page(id, None, 500).unwrap());
        projectors.forget(id);
        assert!(now.ends_with(second.file_name().unwrap()), "{now:?}");
        assert_eq!(rows.last().map(|r| r.0.as_str()), Some("SLOW 3"), "the new thread's turns follow: {rows:?}");

        // A claude pane's log is never a codex projector's.
        let claude = PaneJoin { preset: Some("claude".into()), ..pane };
        feed_codex_projector(id, &claude, &reading(first.to_str().unwrap(), LogFormat::Codex));
        assert!(projectors.transcript(id).is_none());
    }

    /// The call site: the watcher's own tick opens a codex pane's projector
    /// on the rollout its process holds open.
    #[tokio::test]
    async fn the_watcher_opens_a_codex_pane_s_projector_on_the_rollout_it_holds() {
        crate::session_projectors::set_shadowing(true);
        let (_dir, svc, _repo) = crate::test_support::fixture().await;
        let root = tempfile::tempdir().unwrap();
        let rollout = root.path().join(".codex/sessions/2026/10/07/rollout-2026-10-07T12-22-14-01a117d0-fd5c-7fe3-8f0f-5f2afdf7906d.jsonl");
        std::fs::create_dir_all(rollout.parent().unwrap()).unwrap();
        std::fs::write(&rollout, ROLLOUT).unwrap();
        let mut codex = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("exec 3<'{}'\nsleep 30\n", rollout.display()))
            .spawn()
            .unwrap();
        let watcher = super::super::Watcher::new(svc.clone());
        let id = uuid::Uuid::now_v7();
        let pane = || PaneJoin { preset: Some("codex".into()), pid: Some(codex.id() as i32), cwd: "/nonexistent".into(), title: String::new() };
        let projectors = crate::session_projectors::global();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut now = 10_000;
        while projectors.transcript(id).is_none() && std::time::Instant::now() < deadline {
            watcher.turn_from_log(id, pane(), now, true, false).await;
            now += 1_000;
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let _ = codex.kill();
        let _ = codex.wait();
        let opened = projectors.transcript(id);
        let rows = projectors.page(id, None, 500).unwrap_or_default().len();
        projectors.forget(id);
        assert!(opened.is_some_and(|t| t.ends_with(rollout.file_name().unwrap())), "never opened");
        assert!(rows > 20, "{rows} rows");
    }

    /// With a pid, the projector follows what the process holds, not the
    /// watcher's join, which lags after `/new` (review 1, M2); and a hook's
    /// rebind to that file is not undone by the next tick.
    #[test]
    fn a_codex_projector_follows_the_process_not_the_watcher_s_lagging_join() {
        crate::session_projectors::set_shadowing(true);
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join(".codex/sessions/2026/10/07");
        std::fs::create_dir_all(&sessions).unwrap();
        let old = sessions.join("rollout-2026-10-07T12-22-14-01a117d0-fd5c-7fe3-8f0f-5f2afdf7906d.jsonl");
        let new = sessions.join("rollout-2026-10-07T12-28-45-01a117d6-f550-7013-a939-b02d182c0321.jsonl");
        std::fs::write(&old, ROLLOUT).unwrap();
        std::fs::write(&new, NEW_ROLLOUT).unwrap();
        let mut codex = std::process::Command::new("sh").arg("-c").arg(format!("exec 3<'{}'\nsleep 30\n", new.display())).spawn().unwrap();
        let pane = PaneJoin { preset: Some("codex".into()), pid: Some(codex.id() as i32), cwd: "/nonexistent".into(), title: String::new() };
        let id = uuid::Uuid::now_v7();
        let projectors = crate::session_projectors::global();
        // The hook's rebind put it on the new thread; the watcher still reads the old.
        projectors.open(id, new.clone());
        let lagging = reading(old.to_str().unwrap(), LogFormat::Codex);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut stayed = true;
        while std::time::Instant::now() < deadline && stayed {
            feed_codex_projector(id, &pane, &lagging);
            stayed = projectors.transcript(id).is_some_and(|t| t.ends_with(new.file_name().unwrap()));
            if super::super::answer_wake::codex_turn::rollout_of(pane.pid.unwrap()).is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        feed_codex_projector(id, &pane, &lagging);
        let now = projectors.transcript(id);
        let _ = codex.kill();
        let _ = codex.wait();
        projectors.forget(id);
        assert!(stayed && now.is_some_and(|t| t.ends_with(new.file_name().unwrap())), "moved back to the old thread");
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
