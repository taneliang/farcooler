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
#[cfg(target_os = "macos")]
use std::path::PathBuf;
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
        let git_dir = crate::review::git_dir(worktree);
        // One recursive witness covers both: the git directory is inside.
        let witness = Witness::on(worktree, true, &git_dir);
        for dir in [worktree, git_dir.as_path()] {
            // A fresh name every attempt: a lost sentinel is written again
            // under a new one, and any attempt that arrives vouches for the
            // earlier ones on its stream.
            await_delivered(watcher, &witness, |attempt| {
                let sentinel = dir.join(format!("fc-settle-{}-{n}-{attempt}", std::process::id()));
                std::fs::write(&sentinel, "").unwrap();
                std::fs::canonicalize(&sentinel).unwrap()
            });
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (watcher, worktree, &NEXT, Ordering::Relaxed);
}

/// Wait until the backend's raw deliveries hold `path`, writing it again as
/// it is every few seconds. For a path a test cannot rename, such as the
/// index. Fails and retries exactly as `await_delivered` does.
#[cfg(target_os = "macos")]
pub(super) fn wait_delivered(watcher: &TreeWatcher, path: &Path) {
    let witness = Witness::on(path.parent().unwrap(), false, path.parent().unwrap());
    await_delivered(watcher, &witness, |attempt| {
        if attempt > 0 {
            if let Ok(content) = std::fs::read(path) {
                let _ = std::fs::write(path, content);
            }
        }
        path.to_path_buf()
    });
}

/// How long the whole wait may take, across every attempt.
#[cfg(target_os = "macos")]
const DEADLINE: Duration = Duration::from_secs(180);
/// A busy fseventsd loses a write outright, from every stream at once (ov-666
/// measured a spell of 105 lost of 150), so `stimulus` is made again this
/// often, under a new name where the caller can.
#[cfg(target_os = "macos")]
const ATTEMPT_EVERY: Duration = Duration::from_secs(3);
/// How long the backend may trail a witness that has the write. Ten seconds
/// was too short: under a full parallel run (ov-684) the witness had a
/// sentinel and the backend's own stream delivered none of seven attempts for
/// over ten, then caught up. A routing failure is permanent, so a long grace
/// costs only the red case.
#[cfg(target_os = "macos")]
const BACKEND_GRACE: Duration = Duration::from_secs(45);

/// A stream of its own, never replaced, on the directory under test.
#[cfg(target_os = "macos")]
pub(super) struct Witness {
    seen: std::sync::Arc<std::sync::Mutex<Vec<PathBuf>>>,
    _stream: super::fsevents::Stream,
}

#[cfg(target_os = "macos")]
impl Witness {
    /// A git directory's `objects` is left out, as the backend leaves it:
    /// the witness's own deliveries land in the raw record
    /// (`fsevents::RAW`), where a test proves those objects never arrive.
    pub(super) fn on(dir: &Path, recursive: bool, git_dir: &Path) -> Witness {
        let exclude = [std::fs::canonicalize(git_dir).unwrap().join("objects")];
        let seen: std::sync::Arc<std::sync::Mutex<Vec<PathBuf>>> = Default::default();
        let into = std::sync::Arc::clone(&seen);
        let sink = move |paths: &[PathBuf]| into.lock().unwrap().extend(paths.iter().cloned());
        let queue = std::sync::Arc::new(super::fsevents::Queue::new().unwrap());
        let _stream = super::fsevents::Stream::start(&queue, dir, &exclude, recursive, std::sync::Arc::new(sink), None).unwrap();
        Witness { seen, _stream }
    }
}

/// An earlier attempt can arrive while a later one is still on its way, and
/// the later one's echo, landing after the caller's quiet, reads as the
/// fixture moving (ov-684: the probe-lock test saw `fc-settle-...-1` and
/// `-2`). So when several were written, give every one a while to arrive. A
/// lost one never will, which is why this is bounded.
#[cfg(target_os = "macos")]
fn catch_up(watcher: &TreeWatcher, sent: &[PathBuf]) {
    let start = Instant::now();
    while sent.len() > 1 && start.elapsed() < Duration::from_secs(20) {
        let delivered = watcher.delivered.lock().unwrap();
        if sent.iter().all(|p| delivered.contains(p)) {
            return;
        }
        drop(delivered);
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Call `stimulus(attempt)` (0, 1, 2, ...) to write something and return the
/// real path it will be reported under, every few seconds, until the backend
/// has delivered one of those paths or the deadline passes.
///
/// A busy fseventsd drops a write from every stream at once, so waiting
/// longer for a lost write waits for nothing and a lost write is not a
/// failure: it is tried again. What fails is the one thing the witness can
/// tell apart: a write the witness stream saw and the backend still has not
/// delivered `BACKEND_GRACE` later, a routing failure. A stream delivers in
/// order, so a delivered attempt vouches for every earlier write on it.
///
/// Panics at `DEADLINE` naming the attempts and the last few deliveries.
#[cfg(target_os = "macos")]
pub(super) fn await_delivered(watcher: &TreeWatcher, witness: &Witness, mut stimulus: impl FnMut(u32) -> PathBuf) {
    let start = Instant::now();
    let mut sent: Vec<PathBuf> = Vec::new();
    let mut attempts = 0;
    let mut next_attempt = start;
    let mut witnessed: Option<Instant> = None;
    loop {
        if Instant::now() >= next_attempt {
            let path = stimulus(attempts);
            attempts += 1;
            if !sent.contains(&path) {
                sent.push(path);
            }
            next_attempt = Instant::now() + ATTEMPT_EVERY;
        }
        {
            let delivered = watcher.delivered.lock().unwrap();
            if delivered.iter().any(|p| sent.contains(p)) {
                drop(delivered);
                catch_up(watcher, &sent);
                return;
            }
            if start.elapsed() > DEADLINE {
                let tail: Vec<_> = delivered.iter().rev().take(10).collect();
                panic!(
                    "{} was never delivered in {} attempts, to the backend or to a witness stream, in {DEADLINE:?}; \
                     the last deliveries were {tail:?}",
                    sent[0].display(),
                    attempts
                );
            }
        }
        if witnessed.is_none() && witness.seen.lock().unwrap().iter().any(|p| sent.contains(p)) {
            witnessed = Some(Instant::now());
        }
        if let Some(at) = witnessed {
            if at.elapsed() > BACKEND_GRACE {
                let tail: Vec<_> = watcher.delivered.lock().unwrap().iter().rev().take(10).cloned().collect();
                panic!(
                    "a witness stream saw {} (one of {attempts} attempts) {BACKEND_GRACE:?} ago and the backend never \
                     delivered it: routed away. The last deliveries were {tail:?}",
                    sent[0].display()
                );
            }
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

/// A sentinel the stream truly never gets still fails, naming the routing,
/// rather than being retried until the deadline: the witness has it and the
/// backend, watching another directory, does not.
#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[tokio::test]
    #[should_panic(expected = "routed away")]
    async fn a_sentinel_routed_away_fails_instead_of_retrying() {
        let (_dir, root) = super::super::tests::repo("fc-settle-routed").await;
        let watcher = TreeWatcher::start();
        watcher.sync_worktrees(&[(Uuid::now_v7(), root)], &HashSet::new()).await;
        let elsewhere = tempfile::tempdir().unwrap();
        let elsewhere = std::fs::canonicalize(elsewhere.path()).unwrap();
        let witness = Witness::on(&elsewhere, true, &elsewhere);
        await_delivered(&watcher, &witness, |attempt| {
            let sentinel = elsewhere.join(format!("fc-settle-away-{attempt}"));
            std::fs::write(&sentinel, "").unwrap();
            std::fs::canonicalize(&sentinel).unwrap()
        });
    }
}
