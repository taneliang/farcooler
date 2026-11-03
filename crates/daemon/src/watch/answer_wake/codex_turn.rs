//! Whether codex is between turns, as its own rollout says (ov-378): codex's
//! counterpart to `registry_turn`, which reads claude's session registry.
//!
//! The rollout is the one the pane's codex process holds open (`log_join`'s
//! `lsof` join, the process and its descendants), so a file no live process
//! writes says nothing, as a stale registry entry says nothing for claude.
//! Its last turn boundary (`farcooler_core::session_log::codex_turn`) says
//! the rest: `task_started` last is a turn running, `task_complete` or
//! `turn_aborted` last is between turns. Hooks aren't read: codex runs them
//! only once a person has trusted them, and `!cmd` fires none
//! (`.claude/agent/reports/codex-projection/design.md`).
//!
//! The screen can say "between turns" while this says a turn runs: measured
//! on codex 0.153.4, a Tab-queued message's turn starts in the same flush
//! that ends the last, and a busy footer can scroll out of the capture. So a
//! screen that reads idle is checked against this, and the rollout wins when
//! it says a turn runs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use farcooler_core::session_log::codex_turn::{self, RolloutTurn};

use super::registry_turn::Said;
use crate::claude_registry::{Kernel, Processes};

/// How long a pid's rollout, as `lsof` found it, is used again before it is
/// looked up afresh (review 1, L2: a `ps` and an `lsof` per check, under the
/// wake pump). The file itself is read every time; only which file it is
/// waits this long, and `/new` moving to another file in that time leaves
/// the old one, closed, saying idle, which changes nothing the screen
/// doesn't already say.
const JOIN_KEPT: Duration = Duration::from_secs(3);

/// What the rollout at `rollout` says, `None` when the process holds none
/// (no turn yet: codex opens it at the first prompt). `started` is when the
/// process writing it started (seconds since the epoch): a turn begun before
/// that is a dead process's, left open when it was killed, and `codex
/// resume` writes nothing over it until the next prompt (review 1, M1,
/// measured on 0.153.4). Such a turn is over.
pub(crate) fn said(rollout: Option<&Path>, started: Option<i64>) -> Said {
    let Some(rollout) = rollout else { return Said::Nothing };
    match codex_turn::last_turn(rollout) {
        RolloutTurn::Closed => Said::Idle,
        RolloutTurn::Open { started_ms: Some(at) } if started.is_some_and(|s| at < s * 1000) => Said::Idle,
        RolloutTurn::Open { .. } | RolloutTurn::Writing => Said::NotIdle,
        RolloutTurn::Unknown => Said::Nothing,
    }
}

/// A pid's rollout as found: when, which file, and the process's start.
type Join = (Instant, Option<PathBuf>, Option<i64>);

/// The rollout `rollout_of` kept for `pid` in the last `JOIN_KEPT`, without
/// looking it up.
fn cached(pid: i32) -> Option<PathBuf> {
    let joins = joins().lock().unwrap_or_else(|e| e.into_inner());
    let (at, path, was) = joins.as_ref()?.get(&pid)?;
    (at.elapsed() < JOIN_KEPT && *was == Kernel.started(pid)).then(|| path.clone()).flatten()
}

/// The processes (pid and start) that have been seen holding a rollout:
/// codex doesn't close it short of exiting, so a later miss is a lost
/// rollout however long ago the last hit was.
static HAD: Mutex<Vec<(i32, Option<i64>)>> = Mutex::new(Vec::new());

fn had(pid: i32) -> bool {
    HAD.lock().unwrap_or_else(|e| e.into_inner()).contains(&(pid, Kernel.started(pid)))
}

fn joins() -> &'static Mutex<Option<HashMap<i32, Join>>> {
    static JOINS: Mutex<Option<HashMap<i32, Join>>> = Mutex::new(None);
    &JOINS
}

/// The rollout `pid` holds open, from `JOIN_KEPT` ago or found now. Also
/// what a codex pane's projector follows (`registry_join`).
pub(crate) fn rollout_of(pid: i32) -> Option<PathBuf> {
    let started = Kernel.started(pid);
    {
        let joins = joins().lock().unwrap_or_else(|e| e.into_inner());
        // The same process (a reused pid has another start) and recent.
        if let Some((at, path, was)) = joins.as_ref().and_then(|j| j.get(&pid)) {
            if at.elapsed() < JOIN_KEPT && *was == started {
                return path.clone();
            }
        }
    }
    let path = crate::log_join::codex_rollout_of(pid);
    remember(pid, path.clone(), started);
    path
}

/// Keep `path` as `pid`'s rollout for `JOIN_KEPT`, and drop what's older.
fn remember(pid: i32, path: Option<PathBuf>, started: Option<i64>) {
    if path.is_some() {
        let mut had = HAD.lock().unwrap_or_else(|e| e.into_inner());
        if had.len() > 1024 {
            had.clear();
        }
        if !had.contains(&(pid, started)) {
            had.push((pid, started));
        }
    }
    let mut joins = joins().lock().unwrap_or_else(|e| e.into_inner());
    let joins = joins.get_or_insert_with(HashMap::new);
    joins.retain(|_, (at, ..)| at.elapsed() < JOIN_KEPT);
    joins.insert(pid, (Instant::now(), path, started));
}

/// The rollout codex process `pid` holds open now, looked up afresh (not
/// `rollout_of`'s join, kept for seconds): the one a send's record goes to.
/// Off the executor: it spawns `ps` and `lsof`.
pub(crate) async fn rollout_now(pid: i32) -> Option<PathBuf> {
    tokio::task::spawn_blocking(move || crate::log_join::codex_rollout_of(pid)).await.ok().flatten()
}

// Own each discovery process until it exits or has been killed and reaped.
// Output is capped so a stuck writer cannot make discovery grow unbounded.
tokio::task_local! { static DISCOVERY_OWNER: std::sync::Arc<DiscoveryOwner>; }

#[derive(Default)]
pub(super) struct DiscoveryOwner {
    child: tokio::sync::Mutex<Option<tokio::process::Child>>,
}

impl DiscoveryOwner {
    pub(super) async fn scope<F: std::future::Future>(self: std::sync::Arc<Self>, future: F) -> F::Output {
        DISCOVERY_OWNER.scope(self, future).await
    }
    pub(super) async fn stop_and_reap(&self) -> bool {
        let mut slot = self.child.lock().await;
        let Some(child) = slot.as_mut() else { return true };
        if let Ok(Some(_)) = child.try_wait() { *slot = None; return true }
        let _ = child.start_kill();
        let reaped = matches!(tokio::time::timeout(Duration::from_secs(2), child.wait()).await, Ok(Ok(_)));
        if reaped { *slot = None }
        reaped
    }
}

#[cfg(test)]
tokio::task_local! { pub(super) static TEST_STALLED_DISCOVERY: std::path::PathBuf; }
#[cfg(test)]
tokio::task_local! { pub(super) static TEST_DISCOVERY_PANIC: std::path::PathBuf; }

pub(super) async fn discovery_output(program: &str, args: &[&str]) -> Option<String> {
    #[cfg(test)]
    if let Ok(path) = TEST_STALLED_DISCOVERY.try_with(Clone::clone) {
        let path = path.to_str()?;
        return discovery_output_inner("sh", &["-c", "echo $$ > \"$1\"; exec sleep 30", "sh", path]).await;
    }
    discovery_output_inner(program, args).await
}

async fn discovery_output_inner(program: &str, args: &[&str]) -> Option<String> {
    use tokio::io::AsyncReadExt;
    let owner = DISCOVERY_OWNER.try_with(Clone::clone).ok()?;
    let mut slot = owner.child.lock().await;
    if slot.is_some() { return None }
    let child = tokio::process::Command::new(program).args(args)
        .stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped()).kill_on_drop(true).spawn().ok()?;
    *slot = Some(child);
    let child = slot.as_mut()?;
    #[cfg(test)]
    if let Ok(path) = TEST_DISCOVERY_PANIC.try_with(Clone::clone) {
        std::fs::write(path, child.id().expect("spawned child has a PID").to_string()).expect("record child PID");
        panic!("injected scheduler discovery panic after spawn");
    }
    let stdout = child.stdout.take()?;
    let mut bytes = Vec::new();
    let result = tokio::time::timeout(Duration::from_secs(3), async {
        stdout.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes).await.ok()?;
        let status = child.wait().await.ok()?;
        (status.success() && bytes.len() <= 4 * 1024 * 1024).then_some(())
    }).await;
    match result {
        Ok(Some(())) => { *slot = None; String::from_utf8(bytes).ok() },
        _ => {
            let _ = child.start_kill();
            if matches!(tokio::time::timeout(Duration::from_secs(2), child.wait()).await, Ok(Ok(_))) {
                *slot = None;
            } else {
                tracing::error!(program, "discovery child could not be reaped");
            }
            None
        }
    }
}

async fn bounded_open_files(pid: i32) -> Option<Vec<(i32, PathBuf)>> {
    let ps = discovery_output("ps", &["-axo", "pid=,ppid="]).await?;
    if ps.lines().any(|line| {
        let mut fields = line.split_whitespace();
        fields.next().and_then(|p| p.parse::<i32>().ok()).is_none()
            || fields.next().and_then(|p| p.parse::<i32>().ok()).is_none()
            || fields.next().is_some()
    }) { return None }
    if !ps.lines().any(|line| line.split_whitespace().next().and_then(|p| p.parse::<i32>().ok()) == Some(pid)) { return None }
    let subtree = crate::log_join::subtree_from_ps(pid, &ps);
    let pids = subtree.iter().map(i32::to_string).collect::<Vec<_>>().join(",");
    let files = discovery_output("lsof", &["-p", &pids, "-Fn"]).await?;
    let mut owner = None;
    let mut result = Vec::new();
    for line in files.lines() {
        if let Some(p) = line.strip_prefix('p') {
            let parsed = p.parse().ok()?;
            if !subtree.contains(&parsed) { return None }
            owner = Some(parsed);
        }
        else if let Some(path) = line.strip_prefix('n') { result.push((owner?, PathBuf::from(path))); }
    }
    Some(result)
}

/// The scheduler cannot infer the active thread by mtime: Codex may retain
/// an old main file after /new. Unknown or competing candidates hold.
pub(crate) async fn unique_rollout_now(pid: i32) -> Option<PathBuf> {
    unique_rollout(pid, &bounded_open_files(pid).await?)
}

fn unique_rollout(pid: i32, files: &[(i32, PathBuf)]) -> Option<PathBuf> {
    let mut main = None;
    let mut held_by_foreground = false;
    for (holder, path) in files {
        if !crate::log_join::is_codex_rollout(path) { continue }
        match crate::log_join::rollout_is_subagent(path)? {
            true => continue,
            false => {
                if main.as_ref().is_some_and(|old| old != path) { return None }
                main = Some(path.clone());
                held_by_foreground |= *holder == pid;
            }
        }
    }
    main.filter(|_| held_by_foreground)
}

/// `said_of`, for a gate that mustn't pass a rollout it has lost (ov-428,
/// the compose path's rule): looked up afresh, and when none is held now but
/// one was known (`held`: an earlier check found it; or the join kept from
/// the last few seconds has it) the lookup missed, which is not a first
/// prompt, so a turn may run: `NotIdle`. Also whether a rollout was found,
/// for the next check's `held`.
pub(crate) async fn said_held(pid: i32, held: bool) -> (Said, bool) {
    let known = held || had(pid) || cached(pid).is_some();
    let Some(now) = rollout_now(pid).await else {
        return (if known { Said::NotIdle } else { Said::Nothing }, false);
    };
    let started = Kernel.started(pid);
    let said = tokio::task::spawn_blocking(move || {
        remember(pid, Some(now.clone()), started);
        said(Some(&now), started)
    })
    .await
    .unwrap_or(Said::Nothing);
    (said, true)
}

#[cfg(test)]
mod check_in_discovery_tests {
    use super::*;

    #[test]
    fn unique_main_requires_foreground_holder_and_rejects_competing_or_unknown_files() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions/2026/01/01");
        std::fs::create_dir_all(&sessions).unwrap();
        let old = sessions.join("rollout-old.jsonl");
        let new = sessions.join("rollout-new.jsonl");
        let unknown = sessions.join("rollout-unknown.jsonl");
        std::fs::write(&old, "{\"type\":\"session_meta\",\"payload\":{\"source\":\"cli\"}}\n").unwrap();
        std::fs::write(&new, "{\"type\":\"session_meta\",\"payload\":{\"source\":\"cli\"}}\n").unwrap();
        std::fs::write(&unknown, "partial").unwrap();
        std::fs::OpenOptions::new().write(true).open(&old).unwrap()
            .set_modified(std::time::SystemTime::now() + Duration::from_secs(60)).unwrap();
        assert_eq!(unique_rollout(42, &[(42, old.clone())]), Some(old.clone()));
        assert_eq!(unique_rollout(42, &[(42, old.clone()), (43, old.clone())]), Some(old.clone()));
        assert_eq!(unique_rollout(42, &[(43, old.clone())]), None);
        assert_eq!(unique_rollout(42, &[(42, old.clone()), (42, new.clone())]), None);
        assert_eq!(unique_rollout(42, &[(42, old.clone()), (43, new)]), None);
        assert_eq!(unique_rollout(42, &[(42, old), (42, unknown)]), None);
    }

    #[tokio::test]
    async fn timed_out_discovery_is_killed_and_reaped() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let script = format!("echo $$ > '{}'; exec sleep 30", pidfile.display());
        let owner = std::sync::Arc::new(DiscoveryOwner::default());
        assert!(owner.clone().scope(discovery_output("sh", &["-c", &script])).await.is_none());
        assert!(owner.stop_and_reap().await);
        let pid: i32 = std::fs::read_to_string(pidfile).unwrap().trim().parse().unwrap();
        // SAFETY: signal 0 checks existence without sending a signal.
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
    }

    #[tokio::test]
    async fn cancelled_discovery_stays_owned_until_reaped() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let script = format!("echo $$ > '{}'; exec sleep 30", pidfile.display());
        let owner = std::sync::Arc::new(DiscoveryOwner::default());
        let scoped = owner.clone();
        let task = tokio::spawn(async move { scoped.scope(discovery_output("sh", &["-c", &script])).await });
        let observed = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(text) = std::fs::read_to_string(&pidfile)
                    && let Some(line) = text.strip_suffix('\n')
                    && let Ok(pid) = line.parse::<i32>()
                    && pid > 0 { break pid }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await;
        let pid = match observed {
            Ok(pid) => pid,
            Err(_) => {
                task.abort();
                let _ = task.await;
                assert!(owner.stop_and_reap().await, "setup timeout must reap discovery child");
                panic!("child PID did not become ready");
            }
        };
        task.abort();
        let _ = task.await;
        let retained = owner.child.lock().await.as_ref().and_then(|child| child.id());
        if retained != u32::try_from(pid).ok() {
            assert!(owner.stop_and_reap().await, "failed custody assertion must reap discovery child");
            panic!("the owner must retain the exact child before cleanup: {retained:?} != {pid}");
        }
        assert!(owner.stop_and_reap().await);
        // SAFETY: signal 0 checks existence without delivering a signal.
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
    }
}

