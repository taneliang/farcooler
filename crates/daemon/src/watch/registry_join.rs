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

/// Whether the pane's foreground processes ran an agent this tick, for its
/// projector's judgment of a claude it never saw (`judge_agent`). Said only
/// of a plain terminal pane whose processes were read: a chat-mode pane's
/// foreground is the shim's, a client-drawn pane's is not an agent's, and a
/// pane not read says nothing.
pub(super) fn feed_foreground(terminal: Uuid, read: bool, mode: farcooler_store::models::PaneMode, running_agent: &str) {
    if read && mode == farcooler_store::models::PaneMode::Terminal {
        crate::session_projectors::global().note_foreground(terminal, running_agent.is_empty());
    }
}

/// A pane that has exited or been lost: no process is left to read, so its
/// projector's claude is judged by the registry alone (`judge_agent`).
/// `contested` is a lost pane that other panes claim (tmux reports Lost for
/// two live panes on one terminal): its claude may be live, so a missing
/// registry file does not say it ended. Only the registry decides while it
/// is contested: the pane's foreground is read for Running panes alone, so
/// its bare ticks never advance here.
pub(super) fn feed_gone(registry: &Registry, terminal: Uuid, contested: bool) {
    crate::session_projectors::global().judge_agent(terminal, None, !contested, |session| registry.session_ended(session));
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
    // Claude has left the pane, and every agent it ran with it, once the
    // process the registry named here is gone (ov-612).
    // A claude no tick has seen run (the daemon restarted since) may have
    // ended meanwhile (ov-623).
    // Judged by the earliest live process on the session: an older claude
    // running it too launched what predates this one (ov-625).
    let judged = entry.clone().map(|mut e| {
        e.started = registry.earliest_live_start(&e.session_id).or(e.started);
        e
    });
    projectors.judge_agent(terminal, judged.as_ref(), false, |session| registry.session_ended(session));
    projectors.watch_agent(terminal, entry.clone(), |last| registry.still_running(last));
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
    use farcooler_store::models::PaneMode;

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
    ///
    /// The registry is unwatched and re-read by its clock (`advance`), not by
    /// an FSEvents delivery: alone on an idle Mac the rewrite reached the
    /// cache 0.3 to 2.3 s late, and in 1 of 11 runs not within 5 s (ov-668).
    /// What is under test is the follow, not the watch.
    #[tokio::test]
    async fn the_watcher_joins_and_follows_a_pane_by_the_registry() {
        let (_dir, svc, _repo) = crate::test_support::fixture().await;
        let config = tempfile::tempdir().unwrap();
        session(config.path(), "s-one");
        let registry: &'static Registry =
            Box::leak(Box::new(Registry::unwatched(config.path().to_path_buf(), Box::new(fake::Alive(vec![(4242, None)])))));
        let _ = svc.hooks().clone().with_registry(registry);
        let watcher = super::super::Watcher::new(svc.clone());
        let id = uuid::Uuid::now_v7();
        let (reading, _) = watcher.turn_from_log(id, pane(), 10_000, true, false).await;
        assert!(reading.turn.is_some_and(|t| t.running), "joined by the registry: {reading:?}");

        session(config.path(), "s-two");
        // The periodic re-read (`REREAD_EVERY`) that stands in for a lost
        // or late event.
        registry.advance(std::time::Duration::from_secs(30));
        let (_, events) = watcher.turn_from_log(id, pane(), 11_000, true, false).await;
        assert!(!events.is_empty(), "the pane followed its pid to the new session's log");
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

    const LAUNCHES: &str = include_str!("../../../core/fixtures/session-logs/claude-subagents.jsonl");

    fn running(id: uuid::Uuid) -> usize {
        let rows = crate::session_projectors::global().page(id, None, 500).expect("opened");
        rows.iter()
            .filter(|r| matches!(&r.kind, farcooler_core::session_log::projector::RowKind::Subagent(s)
                if s.status == farcooler_core::session_log::projector::SubagentState::Running))
            .count()
    }

    /// A claude whose process can be ended by the test.
    struct Mortal(std::sync::Arc<std::sync::atomic::AtomicBool>);

    impl crate::claude_registry::Processes for Mortal {
        fn started(&self, pid: i32) -> Option<i64> {
            // 4343 is the claude a respawn starts in 4242's place.
            ((pid == 4242 && self.0.load(std::sync::atomic::Ordering::SeqCst)) || pid == 4343).then_some(fake::STARTED)
        }
        fn tty(&self, _: i32) -> Option<u64> {
            None
        }
    }

    /// Claude at 4242 in the registry, its transcript the background
    /// launch fixture; and a switch that ends its process.
    fn launched(config: &std::path::Path) -> (Registry, std::sync::Arc<std::sync::atomic::AtomicBool>) {
        let session = "c0ffee00-0000-4000-8000-000000000453";
        fake::write(config, 4242, session, "/nonexistent/fc-registry-join");
        let project = config.join("projects/-nonexistent-fc-registry-join");
        std::fs::create_dir_all(&project).unwrap();
        // After the fake processes' start, as a claude's own launches are.
        std::fs::write(project.join(format!("{session}.jsonl")), LAUNCHES.replace("2026-08-10", "2026-10-05")).unwrap();
        let alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        (Registry::new(config.to_path_buf(), Box::new(Mortal(alive.clone()))), alive)
    }

    /// A live claude whose screen reads as nothing for a tick (a dialog, its
    /// editor in front, a capture mid-redraw) keeps its background agent
    /// running (ov-612 review): what decides is claude's process.
    #[test]
    fn a_claude_whose_screen_misreads_keeps_its_agent_running() {
        crate::session_projectors::set_shadowing(true);
        let config = tempfile::tempdir().unwrap();
        let (registry, _alive) = launched(config.path());
        let id = uuid::Uuid::now_v7();
        feed_projector(&registry, id, &pane());
        let before = running(id);
        // Not claude by its screen, and its editor in front.
        feed_projector(&registry, id, &PaneJoin { preset: None, ..pane() });
        feed_projector(&registry, id, &PaneJoin { preset: Some("vim".into()), pid: Some(5151), ..pane() });
        let after = running(id);
        crate::session_projectors::global().forget(id);
        assert_eq!(before, 1, "launched in the background");
        assert_eq!(after, 1, "claude is still running");
    }

    /// Claude's process ended and left a shell in its pane (ov-612): the
    /// background agent it launched stops running on the pane's next tick,
    /// even with claude's last frame still on the screen.
    #[test]
    fn an_exited_claude_leaves_no_agent_running() {
        crate::session_projectors::set_shadowing(true);
        let config = tempfile::tempdir().unwrap();
        let (registry, alive) = launched(config.path());
        let id = uuid::Uuid::now_v7();
        feed_projector(&registry, id, &pane());
        let before = running(id);
        alive.store(false, std::sync::atomic::Ordering::SeqCst);
        // The shell's screen still shows claude's banner.
        feed_projector(&registry, id, &PaneJoin { pid: Some(6161), ..pane() });
        let after = running(id);
        crate::session_projectors::global().forget(id);
        assert_eq!(before, 1, "launched in the background");
        assert_eq!(after, 0, "claude exited");
    }

    /// A respawn replaces claude between two ticks (ov-612 re-review): the
    /// old process ends and a new one resumes the same session, and no tick
    /// in between shows no claude. The old one's background agents still
    /// stop running.
    #[test]
    fn a_claude_replaced_between_ticks_leaves_no_agent_running() {
        crate::session_projectors::set_shadowing(true);
        let config = tempfile::tempdir().unwrap();
        let (registry, alive) = launched(config.path());
        let id = uuid::Uuid::now_v7();
        feed_projector(&registry, id, &pane());
        let before = running(id);
        alive.store(false, std::sync::atomic::Ordering::SeqCst);
        std::fs::remove_file(config.path().join("sessions/4242.json")).unwrap();
        fake::write(config.path(), 4343, "c0ffee00-0000-4000-8000-000000000453", "/nonexistent/fc-registry-join");
        let respawned = PaneJoin { pid: Some(4343), ..pane() };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        // The registry's read of the new file can trail the write.
        while registry.by_pid(4343).is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        feed_projector(&registry, id, &respawned);
        let after = running(id);
        crate::session_projectors::global().forget(id);
        assert_eq!(before, 1, "launched in the background");
        assert_eq!(after, 0, "the claude that launched it is gone");
    }

    /// The launch fixture's transcript, where `launched` put it.
    fn transcript(config: &std::path::Path) -> PathBuf {
        config.join("projects/-nonexistent-fc-registry-join/c0ffee00-0000-4000-8000-000000000453.jsonl")
    }

    /// The daemon restarts: the projector is gone, and a new one is read from
    /// the transcript on disk, as a client's first read of the rows does.
    fn restarted(id: uuid::Uuid, config: &std::path::Path) {
        crate::session_projectors::global().forget(id);
        crate::session_projectors::global().open(id, transcript(config));
    }

    /// A shell where claude was, its process table read this tick.
    fn shell_tick(registry: &Registry, id: uuid::Uuid, bare: bool) {
        feed_foreground(id, true, PaneMode::Terminal, if bare { "" } else { "claude" });
        feed_projector(registry, id, &PaneJoin { pid: Some(6161), ..pane() });
    }

    /// A claude that crashed while the daemon was down (ov-623) left its
    /// registry file: the new projector finds its process gone, and no
    /// background agent it launched is left running.
    #[test]
    fn a_projector_restarted_after_claude_crashed_shows_no_agent_running() {
        crate::session_projectors::set_shadowing(true);
        let config = tempfile::tempdir().unwrap();
        let (registry, alive) = launched(config.path());
        let id = uuid::Uuid::now_v7();
        feed_projector(&registry, id, &pane());
        alive.store(false, std::sync::atomic::Ordering::SeqCst);
        restarted(id, config.path());
        let before = running(id);
        shell_tick(&registry, id, false);
        let after = running(id);
        crate::session_projectors::global().forget(id);
        assert_eq!(before, 1, "the transcript alone says it is running");
        assert_eq!(after, 0, "its claude's process is gone");
    }

    /// A claude that exited cleanly removed its registry file (ov-623): with
    /// nothing to read, the pane's processes decide, and only once they have
    /// held no agent for several ticks running.
    #[test]
    fn a_projector_restarted_after_claude_exited_cleanly_shows_no_agent_running() {
        crate::session_projectors::set_shadowing(true);
        let config = tempfile::tempdir().unwrap();
        let (_old, alive) = launched(config.path());
        alive.store(false, std::sync::atomic::Ordering::SeqCst);
        std::fs::remove_file(config.path().join("sessions/4242.json")).unwrap();
        // The restarted daemon's registry has never seen the file.
        let registry = Registry::new(config.path().to_path_buf(), Box::new(Mortal(alive)));
        let id = uuid::Uuid::now_v7();
        restarted(id, config.path());
        shell_tick(&registry, id, true);
        shell_tick(&registry, id, true);
        let early = running(id);
        // One tick that finds claude in the foreground starts the count over.
        shell_tick(&registry, id, false);
        shell_tick(&registry, id, true);
        shell_tick(&registry, id, true);
        let reset = running(id);
        shell_tick(&registry, id, true);
        let after = running(id);
        crate::session_projectors::global().forget(id);
        assert_eq!(early, 1, "two ticks are not enough to call it an exit");
        assert_eq!(reset, 1, "a tick that found claude starts the count over");
        assert_eq!(after, 0, "the pane's processes have held no claude for three ticks");
    }

    /// A claude with no registry file that still runs (an older one) keeps
    /// its agents: the pane's processes say claude is in the foreground.
    #[test]
    fn a_projector_restarted_with_an_unregistered_claude_running_keeps_its_agent() {
        crate::session_projectors::set_shadowing(true);
        let config = tempfile::tempdir().unwrap();
        let (_old, alive) = launched(config.path());
        std::fs::remove_file(config.path().join("sessions/4242.json")).unwrap();
        let registry = Registry::new(config.path().to_path_buf(), Box::new(Mortal(alive)));
        let id = uuid::Uuid::now_v7();
        restarted(id, config.path());
        for _ in 0..5 {
            shell_tick(&registry, id, false);
        }
        let after = running(id);
        crate::session_projectors::global().forget(id);
        assert_eq!(after, 1, "claude is in the foreground");
    }

    /// A live claude's agents stay running across a daemon restart (ov-623),
    /// and on the ticks after it.
    #[test]
    fn a_projector_restarted_with_claude_alive_keeps_its_agent_running() {
        crate::session_projectors::set_shadowing(true);
        let config = tempfile::tempdir().unwrap();
        let (registry, _alive) = launched(config.path());
        let id = uuid::Uuid::now_v7();
        feed_projector(&registry, id, &pane());
        restarted(id, config.path());
        for _ in 0..5 {
            feed_foreground(id, true, PaneMode::Terminal, "claude");
            feed_projector(&registry, id, &pane());
        }
        let after = running(id);
        crate::session_projectors::global().forget(id);
        assert_eq!(after, 1, "claude is still running");
    }

    /// A chat-mode pane runs claude under the shim (ov-623 review): its
    /// foreground is never claude's, and it has no registry file, so only the
    /// pane's mode keeps its live agents.
    #[test]
    fn a_chat_mode_shim_pane_keeps_its_live_agent() {
        crate::session_projectors::set_shadowing(true);
        let config = tempfile::tempdir().unwrap();
        let (_old, alive) = launched(config.path());
        std::fs::remove_file(config.path().join("sessions/4242.json")).unwrap();
        let registry = Registry::new(config.path().to_path_buf(), Box::new(Mortal(alive)));
        let id = uuid::Uuid::now_v7();
        restarted(id, config.path());
        for _ in 0..5 {
            feed_foreground(id, true, PaneMode::Agent, "");
            feed_projector(&registry, id, &PaneJoin { pid: Some(6161), ..pane() });
        }
        let after = running(id);
        crate::session_projectors::global().forget(id);
        assert_eq!(after, 1, "the shim's claude is live");
    }

    /// The same session resumed before the daemon was back (ov-623 review):
    /// the first tick sees the new process, and what the old one launched
    /// before the new one started is gone.
    #[test]
    fn a_session_resumed_before_the_daemon_returned_stops_the_old_agents() {
        crate::session_projectors::set_shadowing(true);
        let config = tempfile::tempdir().unwrap();
        let (_old, alive) = launched(config.path());
        alive.store(false, std::sync::atomic::Ordering::SeqCst);
        std::fs::remove_file(config.path().join("sessions/4242.json")).unwrap();
        fake::write(config.path(), 4343, "c0ffee00-0000-4000-8000-000000000453", "/nonexistent/fc-registry-join");
        // Launched the day before the resumed process started.
        let file = transcript(config.path());
        std::fs::write(&file, std::fs::read_to_string(&file).unwrap().replace("2026-10-05", "2026-10-03")).unwrap();
        let registry = Registry::new(config.path().to_path_buf(), Box::new(Mortal(alive)));
        let id = uuid::Uuid::now_v7();
        restarted(id, config.path());
        let before = running(id);
        feed_projector(&registry, id, &PaneJoin { pid: Some(4343), ..pane() });
        let after = running(id);
        crate::session_projectors::global().forget(id);
        assert_eq!(before, 1, "the transcript alone says it is running");
        assert_eq!(after, 0, "launched by a process that is gone");
    }

    /// A retained exited pane has no process to read (ov-623 review): its
    /// claude left no registry file, and its agents stop all the same.
    #[test]
    fn an_exited_pane_s_agent_is_not_left_running() {
        crate::session_projectors::set_shadowing(true);
        let config = tempfile::tempdir().unwrap();
        let (_old, alive) = launched(config.path());
        alive.store(false, std::sync::atomic::Ordering::SeqCst);
        std::fs::remove_file(config.path().join("sessions/4242.json")).unwrap();
        let registry = Registry::new(config.path().to_path_buf(), Box::new(Mortal(alive)));
        let id = uuid::Uuid::now_v7();
        restarted(id, config.path());
        let before = running(id);
        feed_gone(&registry, id, false);
        let after = running(id);
        crate::session_projectors::global().forget(id);
        assert_eq!(before, 1, "the transcript alone says it is running");
        assert_eq!(after, 0, "its pane is gone");
    }

    /// Two claudes run one session (ov-625): an older one outside the pane
    /// and a newer one in it. What the older one launched is launched before
    /// the pane's process started, and is still its own: it stays running.
    #[test]
    fn two_live_claudes_on_one_session_both_keep_their_agents() {
        struct Two;
        impl crate::claude_registry::Processes for Two {
            fn started(&self, pid: i32) -> Option<i64> {
                // 4242 started two days before 4343.
                Some(if pid == 4242 { fake::STARTED - 2 * 86_400 } else { fake::STARTED })
            }
            fn tty(&self, _: i32) -> Option<u64> {
                None
            }
        }
        crate::session_projectors::set_shadowing(true);
        let config = tempfile::tempdir().unwrap();
        let session = "c0ffee00-0000-4000-8000-000000000453";
        fake::write(config.path(), 4242, session, "/nonexistent/fc-registry-join");
        let older = config.path().join("sessions/4242.json");
        std::fs::write(&older, std::fs::read_to_string(&older).unwrap().replace("Sun Oct  4", "Fri Oct  2")).unwrap();
        fake::write(config.path(), 4343, session, "/nonexistent/fc-registry-join");
        let project = config.path().join("projects/-nonexistent-fc-registry-join");
        std::fs::create_dir_all(&project).unwrap();
        // After the older claude started, before the pane's did.
        std::fs::write(project.join(format!("{session}.jsonl")), LAUNCHES.replace("2026-08-10", "2026-10-03")).unwrap();
        let registry = Registry::new(config.path().to_path_buf(), Box::new(Two));
        let id = uuid::Uuid::now_v7();
        restarted(id, config.path());
        let before = running(id);
        feed_projector(&registry, id, &PaneJoin { pid: Some(4343), ..pane() });
        let after = running(id);
        crate::session_projectors::global().forget(id);
        assert_eq!(before, 1, "the transcript alone says it is running");
        assert_eq!(after, 1, "the older claude is live, and launched it");
    }

    /// A lost pane that two panes claim has a live claude with no registry
    /// file (ov-625): "no file means ended" does not apply to it, while a
    /// pane nobody claims is gone.
    #[test]
    fn a_contested_lost_pane_keeps_its_unregistered_claudes_agent() {
        crate::session_projectors::set_shadowing(true);
        let config = tempfile::tempdir().unwrap();
        let (_old, alive) = launched(config.path());
        std::fs::remove_file(config.path().join("sessions/4242.json")).unwrap();
        let registry = Registry::new(config.path().to_path_buf(), Box::new(Mortal(alive)));
        let contested = uuid::Uuid::now_v7();
        let unclaimed = uuid::Uuid::now_v7();
        restarted(contested, config.path());
        restarted(unclaimed, config.path());
        feed_gone(&registry, contested, true);
        feed_gone(&registry, unclaimed, false);
        let (kept, stopped) = (running(contested), running(unclaimed));
        crate::session_projectors::global().forget(contested);
        crate::session_projectors::global().forget(unclaimed);
        assert_eq!(kept, 1, "two panes claim the terminal: nothing says claude ended");
        assert_eq!(stopped, 0, "no pane claims it: claude died with it");
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
