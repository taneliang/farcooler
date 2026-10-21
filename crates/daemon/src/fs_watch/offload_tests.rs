//! Registering and dropping watches never runs on an async worker (ov-670).
//!
//! A stream start is a round trip to fseventsd, and a busy fseventsd made one
//! take 3.5 to 11 seconds. `sync_worktrees` runs on a detached task, but a
//! detached task is still on a worker, so every connection that worker was
//! serving waited too. Both tests run on `#[tokio::test]`'s current-thread
//! runtime, where the test's own thread IS the only worker: anything that
//! blocks on it stops the whole runtime, which is what makes the stall
//! measurable rather than a matter of luck.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use uuid::Uuid;

use super::TreeWatcher;
use super::tests::repo;

const STALL: Duration = Duration::from_secs(2);

fn slowed(watcher: &TreeWatcher, stall: Option<Duration>) {
    let mut backend = watcher.watcher.lock().unwrap();
    backend.as_mut().expect("the platform backend builds").seam.stall = stall;
}

fn backend_threads(watcher: &TreeWatcher) -> Vec<std::thread::ThreadId> {
    let backend = watcher.watcher.lock().unwrap();
    backend.as_ref().expect("the platform backend builds").seam.threads.clone()
}

/// A tick every 20ms keeps ticking while a worktree's watches take two
/// seconds each to start.
#[tokio::test]
async fn a_slow_stream_start_does_not_hold_up_the_executor() {
    let (_dir, root) = repo("slow-start").await;
    let watcher = Arc::new(TreeWatcher::start());
    slowed(&watcher, Some(STALL));
    let id = Uuid::now_v7();

    let registering = {
        let watcher = Arc::clone(&watcher);
        let fleet = vec![(id, root.clone())];
        tokio::spawn(async move { watcher.sync_worktrees(&fleet, &HashSet::new()).await })
    };
    let started = Instant::now();
    let mut last = started;
    let mut worst = Duration::ZERO;
    while !registering.is_finished() {
        tokio::time::sleep(Duration::from_millis(20)).await;
        worst = worst.max(last.elapsed());
        last = Instant::now();
        assert!(started.elapsed() < Duration::from_secs(60), "registration never finished");
    }
    registering.await.unwrap();

    // The stall really ran, so a short `worst` is not a registration that
    // skipped it.
    assert!(started.elapsed() >= STALL, "registration took {:?}", started.elapsed());
    assert!(
        worst < STALL / 2,
        "the executor stood still for {worst:?} while a watch started; a tick or a connection's \
         page waits that long"
    );
    assert!(watcher.covers_worktree(id), "the slow watch still took");
}

/// Every call into the backend, the teardown of a worktree that went away
/// included, ran somewhere other than the runtime's thread.
#[tokio::test]
async fn watches_start_and_stop_off_the_runtime_thread() {
    let (_dir, root) = repo("off-runtime").await;
    let watcher = TreeWatcher::start();
    let id = Uuid::now_v7();

    watcher.sync_worktrees(&[(id, root.clone())], &HashSet::new()).await;
    assert!(watcher.covers_worktree(id));
    // The worktree leaves the fleet: its streams stop.
    watcher.sync_worktrees(&[], &HashSet::new()).await;
    assert!(!watcher.covers_worktree(id));

    let runtime = std::thread::current().id();
    let threads = backend_threads(&watcher);
    // A watch for the `.git`, one for the tree, and an unwatch for each.
    assert!(threads.len() >= 4, "the backend was called {} times", threads.len());
    assert!(
        threads.iter().all(|thread| *thread != runtime),
        "{} of {} backend calls ran on the runtime's own thread",
        threads.iter().filter(|thread| **thread == runtime).count(),
        threads.len()
    );
}
