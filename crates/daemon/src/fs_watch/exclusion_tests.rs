//! What the macOS streams are never told, proven at the backend.
//!
//! The routes already dropped `target/` before this, so a test that only
//! read `drain_worktrees` would pass with or without exclusions. These read
//! `TreeWatcher::delivered` — every path FSEvents handed over, before routing —
//! which is the cost the exclusions exist to remove.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use uuid::Uuid;

use super::TreeWatcher;
use super::settle::{settle, wait_delivered, wait_for_retrying};
use super::tests::{repo, wait_until_quiet};

/// Everything delivered so far, emptied.
fn take_delivered(watcher: &TreeWatcher) -> Vec<PathBuf> {
    std::mem::take(&mut *watcher.delivered.lock().unwrap())
}

/// Delivered paths strictly UNDER `dir`. FSEvents still reports an excluded
/// directory's own creation and removal, and the product relies on that.
fn under(delivered: &[PathBuf], dir: &Path) -> Vec<PathBuf> {
    delivered.iter().filter(|p| p.starts_with(dir) && p.as_path() != dir).cloned().collect()
}

/// Commit a `.gitignore` and one tracked source file.
async fn seed(root: &Path, ignore: &str) {
    std::fs::write(root.join(".gitignore"), ignore).unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();
    crate::git::git(root, &["add", "-A"]).await.unwrap();
    crate::git::git(root, &["commit", "-q", "-m", "base"]).await.unwrap();
}

/// A build's worth of writes, creates and deletes under `dir`.
fn churn(dir: &Path) {
    for i in 0..40 {
        let d = dir.join(format!("debug/incremental/unit{i}"));
        std::fs::create_dir_all(&d).unwrap();
        for j in 0..5 {
            let f = d.join(format!("o{j}.o"));
            std::fs::write(&f, "x").unwrap();
            std::fs::write(&f, "xy").unwrap();
        }
        std::fs::remove_file(d.join("o0.o")).unwrap();
    }
}

/// An edit to a tracked file, waited for by its own path in the raw
/// deliveries: one stream delivers in event order, so everything written
/// before it has been delivered by then. Waiting on any routed event instead
/// could return on an excluded directory's own creation, which is delivered.
fn barrier(watcher: &TreeWatcher, file: &Path, n: u32) {
    std::fs::write(file, format!("fn main() {{ let _ = {n}; }}")).unwrap();
    let real = std::fs::canonicalize(file).unwrap();
    wait_delivered(watcher, &real, Duration::from_secs(60));
    wait_until_quiet(watcher, Duration::from_secs(10));
}

#[tokio::test]
async fn build_output_is_never_delivered_even_when_the_build_creates_it() {
    let (_dir, root) = repo("fc-fsx-target").await;
    seed(&root, "target/\n").await;
    // An ignored directory that exists at registration, and `target/`, which
    // does not: the first build in a fresh worktree creates it afterwards.
    std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
    std::fs::write(root.join(".gitignore"), "target/\nnode_modules/\n").unwrap();

    let watcher = TreeWatcher::start();
    let id = Uuid::now_v7();
    watcher.sync_worktrees(&[(id, root.clone())], &HashSet::new()).await;
    assert!(watcher.covers_worktree(id));
    settle(&watcher, &root);
    take_delivered(&watcher);

    let real = std::fs::canonicalize(&root).unwrap();
    churn(&root.join("target"));
    churn(&root.join("node_modules/pkg"));
    barrier(&watcher, &root.join("src/main.rs"), 1);

    let delivered = take_delivered(&watcher);
    assert!(
        delivered.iter().any(|p| p.ends_with("src/main.rs")),
        "the barrier itself was delivered: {delivered:?}"
    );
    let leaked: Vec<PathBuf> = under(&delivered, &real.join("target"))
        .into_iter()
        .chain(under(&delivered, &real.join("node_modules")))
        .collect();
    assert!(leaked.is_empty(), "build output reached the daemon: {} paths, e.g. {:?}", leaked.len(), leaked.first());
}

#[tokio::test]
async fn tracked_edits_and_new_untracked_source_still_surface() {
    let (_dir, root) = repo("fc-fsx-source").await;
    seed(&root, "target/\n").await;
    std::fs::create_dir_all(root.join("target/debug")).unwrap();

    let watcher = TreeWatcher::start();
    let id = Uuid::now_v7();
    watcher.sync_worktrees(&[(id, root.clone())], &HashSet::new()).await;
    settle(&watcher, &root);

    let edit = |n: u32| std::fs::write(root.join("src/main.rs"), format!("fn main() {{ let x = {n}; }}")).unwrap();
    edit(1);
    let mut n = 1;
    let seen = wait_for_retrying(&watcher, id, Duration::from_secs(60), || {
        n += 1;
        edit(n);
    });
    assert!(seen, "a tracked edit must surface");
    settle(&watcher, &root);

    let fresh = || std::fs::write(root.join("src/fresh.rs"), "pub fn f() {}").unwrap();
    fresh();
    assert!(
        wait_for_retrying(&watcher, id, Duration::from_secs(60), fresh),
        "a new untracked source file must surface"
    );
    settle(&watcher, &root);

    let readme = || std::fs::write(root.join("README.md"), "hello").unwrap();
    readme();
    assert!(
        wait_for_retrying(&watcher, id, Duration::from_secs(60), readme),
        "a new file at the root must surface"
    );
}

#[tokio::test]
async fn a_nested_worktree_is_never_delivered_to_its_parent() {
    let (_dir, root) = repo("fc-fsx-nested").await;
    // The real layout: the main checkout ignores `.claude/worktrees`, so git
    // lists it as one ignored directory. And a nested repository nobody
    // ignored, which git lists as one untracked `vendor/lib/`.
    seed(&root, ".claude/worktrees/\n").await;
    let nested = root.join(".claude/worktrees/lane");
    crate::git::git(&root, &["worktree", "add", "-q", "-b", "lane", nested.to_str().unwrap()])
        .await
        .unwrap();
    let vendored = root.join("vendor/lib");
    std::fs::create_dir_all(&vendored).unwrap();
    crate::git::git(&vendored, &["init", "-q", "."]).await.unwrap();

    let parent = TreeWatcher::start();
    let main_id = Uuid::now_v7();
    parent.sync_worktrees(&[(main_id, root.clone())], &HashSet::new()).await;
    // The nested worktree is a worktree in its own right, with its own watch.
    let own = TreeWatcher::start();
    let lane_id = Uuid::now_v7();
    own.sync_worktrees(&[(lane_id, nested.clone())], &HashSet::new()).await;
    settle(&parent, &root);
    settle(&own, &nested);
    take_delivered(&parent);

    churn(&nested.join("target"));
    std::fs::write(nested.join("src/main.rs"), "fn main() { let lane = 1; }").unwrap();
    churn(&vendored.join("out"));
    barrier(&parent, &root.join("src/main.rs"), 2);

    let mut n = 0;
    let seen = wait_for_retrying(&own, lane_id, Duration::from_secs(60), || {
        n += 1;
        std::fs::write(nested.join("src/main.rs"), format!("fn main() {{ let lane = {}; }}", n + 1)).unwrap();
    });
    assert!(seen, "the nested worktree's own watch still sees its edit");
    let delivered = take_delivered(&parent);
    let real = std::fs::canonicalize(&root).unwrap();
    let leaked: Vec<PathBuf> = under(&delivered, &real.join(".claude/worktrees/lane"))
        .into_iter()
        .chain(under(&delivered, &real.join("vendor/lib")))
        .collect();
    assert!(
        leaked.is_empty(),
        "a nested checkout reached its parent's watch: {} paths, e.g. {:?}",
        leaked.len(),
        leaked.first()
    );
}

#[tokio::test]
async fn an_ignored_directory_made_after_registration_is_left_out_after_the_rewalk() {
    let (_dir, root) = repo("fc-fsx-rewalk").await;
    // Not a `BUILD_OUTPUT` name, so nothing probes for it ahead of time: it is
    // left out only because the re-walk its creation asks for finds it.
    seed(&root, "generated/\n").await;

    let watcher = TreeWatcher::start();
    let id = Uuid::now_v7();
    watcher.sync_worktrees(&[(id, root.clone())], &HashSet::new()).await;
    settle(&watcher, &root);

    let generated = root.join("generated");
    std::fs::create_dir_all(&generated).unwrap();
    let seen = wait_for_retrying(&watcher, id, Duration::from_secs(60), || {
        // Gone and made again: a second creation to deliver.
        let _ = std::fs::remove_dir_all(&generated);
        std::fs::create_dir_all(&generated).unwrap();
    });
    assert!(seen, "its creation must surface");
    assert!(watcher.needs_rewalk(id, &HashSet::from([generated.clone()])));
    watcher.sync_worktrees(&[(id, root.clone())], &HashSet::from([id])).await;
    assert!(watcher.covers_worktree(id), "the re-walk keeps the worktree covered");
    settle(&watcher, &root);
    take_delivered(&watcher);

    churn(&generated);
    barrier(&watcher, &root.join("src/main.rs"), 3);
    let real = std::fs::canonicalize(&root).unwrap();
    let leaked = under(&take_delivered(&watcher), &real.join("generated"));
    assert!(leaked.is_empty(), "an ignored directory found by a re-walk reached the daemon: {} paths", leaked.len());

    // And the restarted stream still delivers what it should.
    let later = || std::fs::write(root.join("src/later.rs"), "pub fn later() {}").unwrap();
    later();
    assert!(
        wait_for_retrying(&watcher, id, Duration::from_secs(60), later),
        "the restarted watch still sees source"
    );
}

#[tokio::test]
async fn a_git_directory_delivers_head_and_index_but_not_its_objects() {
    let (_dir, root) = repo("fc-fsx-gitdir").await;
    seed(&root, "").await;

    let watcher = TreeWatcher::start();
    let id = Uuid::now_v7();
    watcher.sync_worktrees(&[(id, root.clone())], &HashSet::new()).await;
    settle(&watcher, &root);
    take_delivered(&watcher);

    let git_dir = root.join(".git");
    for i in 0..60 {
        let d = git_dir.join(format!("objects/{i:02x}"));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("loose"), "x").unwrap();
    }
    // What a commit does to the index: a lock, renamed over it.
    std::fs::write(git_dir.join("index.lock"), "").unwrap();
    std::fs::rename(git_dir.join("index.lock"), git_dir.join("index")).unwrap();
    let index = std::fs::canonicalize(git_dir.join("index")).unwrap();
    wait_delivered(&watcher, &index, Duration::from_secs(60));
    let swap = || {
        std::fs::write(git_dir.join("index.lock"), "").unwrap();
        std::fs::rename(git_dir.join("index.lock"), git_dir.join("index")).unwrap();
    };
    assert!(
        wait_for_retrying(&watcher, id, Duration::from_secs(60), swap),
        "the index moving still surfaces"
    );

    // From the raw record: the git directory's stream is non-recursive, so
    // what got past FSEvents is dropped again before `delivered` — the cost
    // the exclusion saves is the part only the raw record shows.
    let objects = std::fs::canonicalize(git_dir.join("objects")).unwrap();
    let raw = super::fsevents::RAW.lock().unwrap().clone();
    let leaked = under(&raw, &objects);
    assert!(leaked.is_empty(), "loose objects reached the daemon: {} paths, e.g. {:?}", leaked.len(), leaked.first());
}

#[test]
fn dropping_a_stream_frees_what_it_was_handed() {
    use super::backend::Sink;
    use super::fsevents::{Queue, Stream};
    let dir = tempfile::tempdir().unwrap();
    let queue = std::sync::Arc::new(Queue::new().unwrap());
    let sink: Sink = std::sync::Arc::new(|_: &[PathBuf]| {});
    let stream = Stream::start(&queue, dir.path(), &[], true, std::sync::Arc::clone(&sink), None).unwrap();
    assert_eq!(std::sync::Arc::strong_count(&sink), 2, "the stream's context holds the sink");
    drop(stream);
    // FSEvents may let go of the stream's last reference a moment after
    // `FSEventStreamRelease` returns, on its own queue.
    let start = std::time::Instant::now();
    while std::sync::Arc::strong_count(&sink) != 1 {
        assert!(start.elapsed() < Duration::from_secs(60), "releasing the stream never freed its context");
        std::thread::sleep(Duration::from_millis(10));
    }
}

type Record = std::sync::Arc<std::sync::Mutex<Vec<(std::time::Instant, PathBuf)>>>;

/// A sink that keeps every path it is handed, with when, and what it fills.
fn recorder() -> (super::backend::Sink, Record) {
    let seen: Record = Default::default();
    let into = std::sync::Arc::clone(&seen);
    let sink = move |paths: &[PathBuf]| {
        let now = std::time::Instant::now();
        into.lock().unwrap().extend(paths.iter().map(|p| (now, p.clone())));
    };
    (std::sync::Arc::new(sink), seen)
}

/// When `seen` first held a path named `name`, waiting up to `limit`.
fn arrives(seen: &Record, name: &str, limit: Duration) -> Option<std::time::Instant> {
    let start = std::time::Instant::now();
    loop {
        if let Some((at, _)) = seen.lock().unwrap().iter().find(|(_, p)| p.ends_with(name)) {
            return Some(*at);
        }
        if start.elapsed() > limit {
            return None;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A busy fseventsd loses writes outright, from every stream at once and
/// from its own history. Measured under full parallel daemon runs (ov-666):
/// - a single write delivered in about 90 ms, or not in 300 s;
/// - a replay from before a lost write did not have it either;
/// - in a spell, up to 105 of 150 writes 100 ms apart reached no stream,
///   while the rest arrived in under a second;
/// - two streams on one directory lost exactly the same writes, every time:
///   not one of about 1,000 paired deliveries reached one and not the other.
///
/// So waiting longer cannot tell a write the replacement lost from one
/// FSEvents never delivered. A witness can: a stream of its own on the same
/// directory, never replaced. What the witness saw and the backend never
/// did, the replacement lost; what neither saw, FSEvents lost, and the
/// attempt is made again.
#[test]
fn a_replaced_stream_hands_over_what_its_latency_window_held() {
    use super::backend::Backend;
    use super::fsevents::{Queue, Stream};
    // One deadline for the whole test, well inside nextest's 600 s kill, so
    // a failure is always this test's own message.
    let deadline = std::time::Instant::now() + Duration::from_secs(300);
    let left = |limit: Duration| limit.min(deadline.saturating_duration_since(std::time::Instant::now()));
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let (sink, seen) = recorder();
    let mut backend = Backend::new(sink).unwrap();
    backend.watch(&root, true, &[]).unwrap();
    let (sink, witnessed) = recorder();
    let queue = std::sync::Arc::new(Queue::new().unwrap());
    let _witness = Stream::start(&queue, &root, &[], true, sink, None).unwrap();
    std::thread::sleep(Duration::from_millis(300));

    // Attempts that tested nothing: the write in the window was delivered
    // before the replacement, or FSEvents never delivered it at all.
    let (mut early, mut lost) = (0, 0);
    let mut attempt = 0;
    while std::time::Instant::now() < deadline {
        attempt += 1;
        // The first write after a quiet spell is delivered at once and opens
        // the latency window; the second waits in it. A lost first write is
        // made again every 3 s: a spell lost five in a row over 20 s. Any
        // delivery opens the window as well.
        let first = format!("first-{attempt}");
        let delivered = (0..20).any(|_| {
            std::fs::write(root.join(&first), "1").unwrap();
            arrives(&seen, &first, left(Duration::from_secs(3))).is_some()
        });
        assert!(delivered, "{first} was never delivered, though it was written every 3 s for a minute");
        let second = format!("second-{attempt}");
        std::fs::write(root.join(&second), "2").unwrap();
        let wrote = std::time::Instant::now();
        // Give fseventsd time to put the write in the old stream's window,
        // which stays open for a quarter second after `first` was delivered.
        // Replacing straight after the write let the new stream catch it
        // itself more often: a replacement that discarded the window failed
        // this test in 1 of 20 runs that way, and in 4 of 20 with the wait.
        std::thread::sleep(Duration::from_millis(150));
        if arrives(&seen, &second, Duration::ZERO).is_some() {
            // The window had already closed and delivered it: nothing was
            // held for the replacement to hand over.
            early += 1;
            continue;
        }
        // A different exclusion each time, or the backend has nothing to
        // replace.
        backend.reconfigure(&root, &[root.join(format!("target-{attempt}"))]);

        if arrives(&seen, &second, left(Duration::from_secs(10))).is_some() {
            return;
        }
        if let Some(witness) = arrives(&witnessed, &second, left(Duration::from_secs(10))) {
            // Late is not lost: a replacement's replay waits its turn at a
            // busy fseventsd. Only never is a failure.
            assert!(
                arrives(&seen, &second, left(Duration::from_secs(60))).is_some(),
                "{second}, held in the old stream's window, was lost when it was replaced: the witness \
                 stream delivered it {:?} after the write, and the backend never did",
                witness - wrote
            );
            return;
        }
        lost += 1;
        eprintln!("attempt {attempt}: FSEvents never delivered {second} to any stream; trying again");
    }
    panic!(
        "the handover was never tested in 300 s: of {attempt} attempts, {early} delivered the write \
         before the replacement and {lost} lost it from every stream"
    );
}

/// The other half of the handover: the replaced stream is what delivers a
/// write registered while its replacement was starting, which neither the
/// replay nor the new live feed carries, and fseventsd can hand it over 3 to
/// 9 s late under load (ov-671). A fixed 2 s retirement lost it. Here
/// fseventsd is held "behind" on purpose, so this fails every time the old
/// stream is stopped on a clock rather than on fseventsd's progress.
#[test]
fn a_replaced_stream_runs_until_fseventsd_is_past_its_replacements_start() {
    use super::backend::{Backend, RETIRED};
    use std::sync::atomic::Ordering;
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let (sink, _) = recorder();
    let mut backend = Backend::new(sink).unwrap();
    backend.watch(&root, true, &[]).unwrap();
    let retired = || RETIRED.lock().unwrap().iter().any(|p| p == &root);

    backend.seam.behind.store(true, Ordering::Release);
    backend.reconfigure(&root, &[root.join("target")]);
    // Longer than the 2 s that lost a write handed over 2.9 s late.
    std::thread::sleep(Duration::from_secs(5));
    assert!(
        !retired(),
        "the replaced stream stopped while fseventsd had not got past its replacement's start: a write \
         registered during that start, which only the old stream is handed, would be lost"
    );

    // Caught up: the beacon's write is an event past the start, even in a
    // directory nobody touches. Under the 60 s cap, so stopping on the cap
    // alone fails.
    backend.seam.behind.store(false, Ordering::Release);
    let start = std::time::Instant::now();
    while !retired() {
        assert!(
            start.elapsed() < Duration::from_secs(50),
            "the replaced stream was still running 50 s after fseventsd caught up: nothing moved the \
             queue past its replacement's start, not even the beacon"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The replay half of the handover, without FSEvents' timing: a replacement
/// starts from the last event the old stream delivered, so its replay covers
/// a write the old stream was still holding. The end-to-end test above can
/// pass with the replay broken whenever the retiring stream delivers the
/// write itself; this cannot.
#[test]
fn a_replacement_replays_from_the_last_event_the_old_stream_delivered() {
    use super::backend::{Backend, STARTS};
    use super::fsevents::IDS;
    let deadline = std::time::Instant::now() + Duration::from_secs(300);
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let (sink, seen) = recorder();
    let mut backend = Backend::new(sink).unwrap();
    backend.watch(&root, true, &[]).unwrap();
    std::thread::sleep(Duration::from_millis(300));

    let mut attempt = 0;
    while std::time::Instant::now() < deadline {
        attempt += 1;
        let first = format!("first-{attempt}");
        let delivered = (0..20).any(|_| {
            std::fs::write(root.join(&first), "1").unwrap();
            arrives(&seen, &first, Duration::from_secs(3)).is_some()
        });
        assert!(delivered, "{first} was never delivered, though it was written every 3 s for a minute");
        let second = root.join(format!("second-{attempt}"));
        std::fs::write(&second, "2").unwrap();
        backend.reconfigure(&root, &[root.join(format!("target-{attempt}"))]);
        let since = STARTS.lock().unwrap().iter().rev().find(|(path, _)| path == &root).map(|(_, since)| *since);
        let since = since.expect("the replacement was never started").expect("the replacement started from now");
        // Anything the old stream delivered up to the handoff, its record
        // landing a moment after the callback.
        std::thread::sleep(Duration::from_millis(500));
        let ids = IDS.lock().unwrap().clone();
        let mine = |p: &PathBuf| p.starts_with(&root);
        if ids.iter().any(|(p, id)| p == &second && *id <= since) {
            // Delivered before the handoff: nothing was held to replay.
            continue;
        }
        let last = ids.iter().filter(|(p, _)| mine(p) && p != &second).map(|(_, id)| *id).max().unwrap();
        assert!(
            since <= last,
            "the replacement replays from event {since}, after {last}, the last one the old stream \
             delivered: a write it still held would be in neither the replay nor the old stream"
        );
        return;
    }
    panic!("the replay was never tested in 300 s: in all {attempt} attempts the write was delivered before the replacement");
}
