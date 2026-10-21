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

fn backend_calls(watcher: &TreeWatcher) -> Vec<(&'static str, std::thread::ThreadId)> {
    let backend = watcher.watcher.lock().unwrap_or_else(|e| e.into_inner());
    backend.as_ref().expect("the platform backend builds").seam.calls.clone()
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

/// Every call into the backend ran somewhere other than the runtime's
/// thread: the first watches, a restart with new exclusions after a re-walk
/// finds build output, and the teardown of a worktree that went away.
#[tokio::test]
async fn watches_start_and_stop_off_the_runtime_thread() {
    let (_dir, root) = repo("off-runtime").await;
    std::fs::write(root.join(".gitignore"), "generated/\n").unwrap();
    std::fs::write(root.join("a.txt"), "a").unwrap();
    crate::git::git(&root, &["add", "-A"]).await.unwrap();
    crate::git::git(&root, &["commit", "-q", "-m", "base"]).await.unwrap();
    let watcher = TreeWatcher::start();
    let id = Uuid::now_v7();

    watcher.sync_worktrees(&[(id, root.clone())], &HashSet::new()).await;
    assert!(watcher.covers_worktree(id));
    // An ignored directory appears and a re-walk finds it: on the backends
    // with exclusions, the tree's stream restarts to leave it out.
    std::fs::create_dir_all(root.join("generated")).unwrap();
    std::fs::write(root.join("generated/out.o"), "x").unwrap();
    watcher.sync_worktrees(&[(id, root.clone())], &HashSet::from([id])).await;
    assert!(watcher.covers_worktree(id));
    // The worktree leaves the fleet: its streams stop.
    watcher.sync_worktrees(&[], &HashSet::new()).await;
    assert!(!watcher.covers_worktree(id));

    let runtime = std::thread::current().id();
    let calls = backend_calls(&watcher);
    let made = |name: &str| calls.iter().filter(|(call, _)| *call == name).count();
    assert!(made("watch") >= 2 && made("unwatch") >= 2, "{calls:?}");
    if cfg!(target_os = "macos") {
        assert!(made("reconfigure") >= 1, "the re-walk never restarted the stream: {calls:?}");
    }
    let inline: Vec<&str> = calls.iter().filter(|(_, thread)| *thread == runtime).map(|(call, _)| *call).collect();
    assert!(inline.is_empty(), "backend calls on the runtime's own thread: {inline:?}");
}

/// A backend that panics unwinds the pass that called it, and the flag that
/// keeps that pass single-flight is cleared anyway, so the next one starts.
#[tokio::test]
async fn a_panicking_backend_leaves_the_next_pass_free_to_start() {
    let (_dir, root) = repo("panicking").await;
    let watcher = Arc::new(TreeWatcher::start());
    watcher.watcher.lock().unwrap().as_mut().expect("the platform backend builds").seam.panics = true;
    let probing = Arc::new(std::sync::atomic::AtomicBool::new(true));

    // The shape of `Watcher::spawn_change_set_probe`.
    let pass = {
        let (watcher, probing) = (Arc::clone(&watcher), Arc::clone(&probing));
        tokio::spawn(async move {
            let _clear = super::ClearOnDrop(&probing);
            watcher.sync_worktrees(&[(Uuid::now_v7(), root)], &HashSet::new()).await;
        })
    };
    let unwound = pass.await.expect_err("the backend's panic reaches the pass");
    assert!(unwound.is_panic());
    assert!(!probing.load(std::sync::atomic::Ordering::SeqCst), "the pass's flag is still set");
}
