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
use std::time::Duration;

use super::TreeWatcher;
use super::tests::wait_until_quiet;

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// Wait until every write made under `worktree` before this call has been
/// delivered, then drain what it caused.
pub(super) fn settle(watcher: &TreeWatcher, worktree: &Path) {
    #[cfg(target_os = "macos")]
    {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let name = format!("fc-settle-{}-{n}", std::process::id());
        let git_dir = crate::review::git_dir(worktree);
        for dir in [worktree, git_dir.as_path()] {
            let sentinel = dir.join(&name);
            std::fs::write(&sentinel, "").unwrap();
            let real = std::fs::canonicalize(&sentinel).unwrap();
            wait_delivered(watcher, &real, Duration::from_secs(30));
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (worktree, &NEXT, Ordering::Relaxed);
    wait_until_quiet(watcher, Duration::from_secs(10));
}

/// Poll the backend's raw deliveries until `path` is among them.
///
/// Panics at `deadline` naming the path and the last few deliveries, so a
/// failure says which stream lagged rather than that something did.
#[cfg(target_os = "macos")]
pub(super) fn wait_delivered(watcher: &TreeWatcher, path: &Path, deadline: Duration) {
    let start = std::time::Instant::now();
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
        std::thread::sleep(Duration::from_millis(25));
    }
}
