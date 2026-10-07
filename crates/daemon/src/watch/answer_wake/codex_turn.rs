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

/// The rollout `pid` holds open, from `JOIN_KEPT` ago or found now. Also
/// what a codex pane's projector follows (`registry_join`).
pub(crate) fn rollout_of(pid: i32) -> Option<PathBuf> {
    static JOINS: Mutex<Option<HashMap<i32, Join>>> = Mutex::new(None);
    let started = Kernel.started(pid);
    {
        let joins = JOINS.lock().unwrap_or_else(|e| e.into_inner());
        // The same process (a reused pid has another start) and recent.
        if let Some((at, path, was)) = joins.as_ref().and_then(|j| j.get(&pid)) {
            if at.elapsed() < JOIN_KEPT && *was == started {
                return path.clone();
            }
        }
    }
    let path = crate::log_join::codex_rollout_of(pid);
    let mut joins = JOINS.lock().unwrap_or_else(|e| e.into_inner());
    let joins = joins.get_or_insert_with(HashMap::new);
    joins.retain(|_, (at, ..)| at.elapsed() < JOIN_KEPT);
    joins.insert(pid, (Instant::now(), path.clone(), started));
    path
}

/// `said`, for the codex process `pid`, its rollout found by what it holds
/// open. Off the executor: the join spawns `ps` and `lsof`.
pub(crate) async fn said_of(pid: i32) -> Said {
    tokio::task::spawn_blocking(move || said(rollout_of(pid).as_deref(), Kernel.started(pid))).await.unwrap_or(Said::Nothing)
}
