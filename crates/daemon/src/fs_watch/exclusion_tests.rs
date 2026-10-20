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

#[test]
fn a_replaced_stream_hands_over_what_its_latency_window_held() {
    use super::backend::Backend;
    use std::sync::{Arc, Mutex};
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let seen: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
    let into = Arc::clone(&seen);
    let mut backend =
        Backend::new(Arc::new(move |paths: &[PathBuf]| into.lock().unwrap().extend_from_slice(paths))).unwrap();
    backend.watch(&root, true, &[]).unwrap();
    std::thread::sleep(Duration::from_millis(300));

    // The first write after a quiet spell is delivered at once and opens the
    // latency window; the second waits in it.
    std::fs::write(root.join("first"), "1").unwrap();
    let start = std::time::Instant::now();
    while !seen.lock().unwrap().iter().any(|p| p.ends_with("first")) {
        assert!(start.elapsed() < Duration::from_secs(60), "the first write was never delivered");
        std::thread::sleep(Duration::from_millis(5));
    }
    std::fs::write(root.join("second"), "2").unwrap();
    backend.reconfigure(&root, &[root.join("target")]);

    let start = std::time::Instant::now();
    while !seen.lock().unwrap().iter().any(|p| p.ends_with("second")) {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "a write held in the old stream's window was lost when it was replaced"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}
