//! A barrier for the watcher tests: everything written before it has arrived.
//!
//! `wait_until_quiet` alone assumes fseventsd delivers within its half second
//! of quiet, and on a loaded Mac it does not always: one run in 83 failed that
//! way during ov-545. So on macOS this writes a sentinel file into the
//! worktree root and into its git directory — they are separate streams — and
//! waits until the backend has delivered both. A stream delivers in event
//! order, so every earlier write on it has been delivered by then. Only then
//! does it drain, and wait for quiet as before.
//!
//! The sentinels stay behind in the scratch repository: removing one would be
//! another write to wait for.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use super::TreeWatcher;
use super::tests::wait_until_quiet;
use uuid::Uuid;

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// Wait until every write made under `worktree` before this call has been
/// delivered, then drain what it caused.
pub(super) fn settle(watcher: &TreeWatcher, worktree: &Path) {
    // Twice, with quiet between: a negative check after this (the index lock
    // a probe takes) fails on a late echo of the fixture's own writes, and
    // FSEvents can report one after the first sentinel's round (ov-656). The
    // second round is delivered only after whatever the first quiet missed.
    barrier(watcher, worktree);
    wait_until_quiet(watcher, Duration::from_secs(10));
    barrier(watcher, worktree);
    wait_until_quiet(watcher, Duration::from_secs(10));
}

/// Write a sentinel into the worktree and its git directory and wait for both.
fn barrier(watcher: &TreeWatcher, worktree: &Path) {
    #[cfg(target_os = "macos")]
    {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let name = format!("fc-settle-{}-{n}", std::process::id());
        let git_dir = crate::review::git_dir(worktree);
        for dir in [worktree, git_dir.as_path()] {
            let sentinel = dir.join(&name);
            std::fs::write(&sentinel, "").unwrap();
            let real = std::fs::canonicalize(&sentinel).unwrap();
            wait_delivered(watcher, &real, Duration::from_secs(60));
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (watcher, worktree, &NEXT, Ordering::Relaxed);
}

/// Poll the backend's raw deliveries until `path` is among them.
///
/// Every few seconds it writes `path` again, as it is. A busy fseventsd can
/// drop a single event outright (ov-656: nothing at all delivered for 30 s to
/// a stream that was live a moment before), and waiting longer for a lost
/// event waits for nothing. A stream delivers in order, so a later write that
/// arrives vouches for the earlier ones just as the first would have.
///
/// Panics at `deadline` naming the path and the last few deliveries, so a
/// failure says which stream lagged rather than that something did.
#[cfg(target_os = "macos")]
pub(super) fn wait_delivered(watcher: &TreeWatcher, path: &Path, deadline: Duration) {
    const POKE_EVERY: Duration = Duration::from_secs(3);
    let start = std::time::Instant::now();
    let mut poked = start;
    loop {
        {
            let delivered = watcher.delivered.lock().unwrap();
            if delivered.iter().any(|p| p == path) {
                return;
            }
            if start.elapsed() > deadline {
                let tail: Vec<_> = delivered.iter().rev().take(10).collect();
                panic!("{} was never delivered; the last deliveries were {tail:?}", path.display());
            }
        }
        if poked.elapsed() >= POKE_EVERY {
            if let Ok(content) = std::fs::read(path) {
                let _ = std::fs::write(path, content);
            }
            poked = std::time::Instant::now();
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// `wait_for`, for a stimulus that can be made again.
///
/// FSEvents is a machine-wide service and a busy one drops or sits on a
/// single event for tens of seconds (ov-656: a full parallel run of the
/// lib tests failed a different one of these each time, and the same
/// test passes alone in under a second). Waiting longer for a lost event
/// waits for nothing, so `again` repeats the stimulus every few seconds
/// until the ceiling. The ceiling is generous because a pass returns at
/// the first event: it costs nothing unless the machine is struggling.
pub(super) fn wait_for_retrying(
    watcher: &TreeWatcher,
    id: Uuid,
    ceiling: Duration,
    mut again: impl FnMut(),
) -> bool {
    const EVERY: Duration = Duration::from_secs(4);
    let start = Instant::now();
    let mut last = start;
    loop {
        if watcher.drain_worktrees().contains_key(&id) {
            return true;
        }
        if start.elapsed() > ceiling {
            return false;
        }
        if last.elapsed() >= EVERY {
            again();
            last = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
