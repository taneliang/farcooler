//! Nested checkouts: left out of a watch, but never out of sight.
//!
//! git lists a nested repository or worktree as one `?? dir/` line. The
//! directory that holds it must stay watched, on every platform, or a second
//! checkout appearing beside it — or this one going — changes `git status`
//! without any event saying so.

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use uuid::Uuid;

use super::settle::{settle, wait_for_retrying};
use super::tests::repo;
use super::{TreeWatcher, tracked_directories};

async fn with_vendored_checkout(name: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let (dir, root) = repo(name).await;
    std::fs::write(root.join("a.txt"), "one").unwrap();
    crate::git::git(&root, &["add", "-A"]).await.unwrap();
    crate::git::git(&root, &["commit", "-q", "-m", "base"]).await.unwrap();
    // `vendor/` holds nothing git tracks: only the nested repository.
    let vendored = root.join("vendor/lib");
    std::fs::create_dir_all(&vendored).unwrap();
    crate::git::git(&vendored, &["init", "-q", "."]).await.unwrap();
    (dir, root)
}

#[tokio::test]
async fn a_directory_holding_only_nested_checkouts_is_still_watched() {
    let (_dir, root) = with_vendored_checkout("fc-fsn-dirs").await;
    let (dirs, nested) = tracked_directories(&root).await.unwrap();
    assert!(dirs.contains(Path::new("vendor")), "the checkout's parent must be watched: {dirs:?}");
    assert!(!dirs.contains(Path::new("vendor/lib")), "the checkout itself is not: {dirs:?}");
    assert_eq!(nested, vec![Path::new("vendor/lib").to_path_buf()]);
}

#[tokio::test]
async fn a_nested_checkout_appearing_or_going_surfaces_its_worktree() {
    let (_dir, root) = with_vendored_checkout("fc-fsn-appear").await;

    let watcher = TreeWatcher::start();
    let id = Uuid::now_v7();
    watcher.sync_worktrees(&[(id, root.clone())], &HashSet::new()).await;
    assert!(watcher.covers_worktree(id));
    settle(&watcher, &root);

    // The retries below prove the parent directory delivers, not the nested-checkout handling.
    let other = root.join("vendor/other");
    std::fs::create_dir_all(&other).unwrap();
    crate::git::git(&other, &["init", "-q", "."]).await.unwrap();
    let mut n = 0;
    assert!(
        wait_for_retrying(&watcher, id, Duration::from_secs(60), || {
            // A file made and removed beside the checkout: a new event in `vendor`.
            n += 1;
            let marker = root.join(format!("vendor/marker{n}"));
            std::fs::write(&marker, "").unwrap();
            std::fs::remove_file(&marker).unwrap();
        }),
        "a second checkout appearing beside the first must surface"
    );
    settle(&watcher, &root);

    let lib = root.join("vendor/lib");
    std::fs::remove_dir_all(&lib).unwrap();
    let seen = wait_for_retrying(&watcher, id, Duration::from_secs(60), || {
        // The removal happens once; if it was lost, a directory made and
        // removed in the same place is another one to deliver.
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::remove_dir_all(&lib).unwrap();
    });
    assert!(seen, "a nested checkout going must surface");
}

#[test]
fn a_git_that_did_not_answer_keeps_the_exclusions_it_had() {
    use std::path::PathBuf;
    let base = Path::new("/r");
    let dirs: HashSet<PathBuf> = [PathBuf::new(), PathBuf::from("src")].into_iter().collect();
    let had = vec![PathBuf::from("/r/target")];
    assert_eq!(super::kept_or(None, Some(&had), base, &dirs), had);
    assert_eq!(super::kept_or(None, None, base, &dirs), Vec::<PathBuf>::new());
    let fresh = vec![PathBuf::from("/r/node_modules")];
    assert_eq!(super::kept_or(Some(fresh.clone()), Some(&had), base, &dirs), fresh);
}

#[test]
fn reused_exclusions_never_hide_a_directory_that_is_now_wanted() {
    use std::path::PathBuf;
    // `build/` was ignored at the last walk; since then a file under it was
    // force-added, so the walk that just ran has `build` in its set.
    let base = Path::new("/r");
    let dirs: HashSet<PathBuf> = [PathBuf::new(), PathBuf::from("build")].into_iter().collect();
    let had = vec![PathBuf::from("/r/target"), PathBuf::from("/r/build"), PathBuf::from("/elsewhere/x")];
    assert_eq!(super::kept_or(None, Some(&had), base, &dirs), vec![PathBuf::from("/r/target")]);
}
