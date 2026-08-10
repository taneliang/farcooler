//! Change-set behaviour against a real git repository.
//!
//! Real git rather than fixtures of its output, because the things most likely
//! to be wrong here are what git actually prints for a merge, for a rename, and
//! for a branch whose base has commits of its own — and a fixture would encode
//! whatever the author believed on the day.

use std::path::Path;
use std::process::Command;

use farcooler_daemon::change_set::{
    BaseSource, change_set, commits_since, merge_base, working_tree, worktree_digest,
};
use tempfile::TempDir;

fn run(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write(repo: &Path, path: &str, contents: &str) {
    let full = repo.join(path);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(full, contents).expect("write");
}

/// A repository with `main` at one commit and nothing else.
fn repo() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    run(p, &["init", "--initial-branch=main", "-q"]);
    run(p, &["config", "user.email", "test@example.com"]);
    run(p, &["config", "user.name", "Test"]);
    run(p, &["config", "commit.gpgsign", "false"]);
    write(p, "README.md", "start\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "initial"]);
    dir
}

#[tokio::test]
async fn the_range_is_merge_base_to_head_and_never_the_symmetric_difference() {
    // The trap this test exists for: `git log base...HEAD` is the SYMMETRIC
    // difference and includes commits only `main` has. A reviewer would see work
    // this branch never did, attributed to it, with nothing looking wrong.
    let dir = repo();
    let p = dir.path();

    run(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "feature.txt", "mine\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "a commit the branch made"]);

    // main moves on independently.
    run(p, &["checkout", "-q", "main"]);
    write(p, "main.txt", "theirs\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "a commit ONLY main made"]);
    run(p, &["checkout", "-q", "feature"]);

    let base = merge_base(p, "main").await.expect("merge base resolves");
    let commits = commits_since(p, &base).await.expect("log");

    let subjects: Vec<&str> = commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(
        subjects,
        vec!["a commit the branch made"],
        "main's own commit must not appear in this branch's change set"
    );
}

#[tokio::test]
async fn commits_arrive_oldest_first_with_their_message_intact() {
    let dir = repo();
    let p = dir.path();
    run(p, &["checkout", "-q", "-b", "feature"]);

    for (i, subject) in ["first", "second", "third"].iter().enumerate() {
        write(p, &format!("f{i}.txt"), "x\n");
        run(p, &["add", "."]);
        run(p, &["commit", "-q", "-m", subject, "-m", "a body line"]);
    }

    let base = merge_base(p, "main").await.expect("merge base");
    let commits = commits_since(p, &base).await.expect("log");

    let subjects: Vec<&str> = commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects, vec!["first", "second", "third"], "oldest first");
    assert!(commits[0].body.contains("a body line"));
    assert!(!commits[0].sha.is_empty());
    assert!(commits[0].timestamp > 0);
}

#[tokio::test]
async fn a_multi_line_commit_body_does_not_break_the_record_split() {
    let dir = repo();
    let p = dir.path();
    run(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "x.txt", "x\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "subject", "-m", "one\n\ntwo\n\nthree"]);

    let base = merge_base(p, "main").await.expect("merge base");
    let commits = commits_since(p, &base).await.expect("log");
    assert_eq!(commits.len(), 1, "blank lines in a body are not record boundaries");
    assert!(commits[0].body.contains("three"));
}

#[tokio::test]
async fn an_unresolvable_base_is_refused_rather_than_guessed() {
    let dir = repo();
    assert!(
        merge_base(dir.path(), "no-such-branch").await.is_err(),
        "a base that does not resolve must not silently become HEAD"
    );
}

#[tokio::test]
async fn the_four_working_tree_groups_stay_apart() {
    let dir = repo();
    let p = dir.path();

    write(p, "staged.txt", "staged\n");
    run(p, &["add", "staged.txt"]);

    write(p, "README.md", "changed but not staged\n");
    write(p, "untracked.txt", "new\n");

    let wt = working_tree(p).await.expect("status");
    assert!(wt.is_dirty());
    assert!(wt.staged.iter().any(|f| f.path == "staged.txt"));
    assert!(wt.unstaged.iter().any(|f| f.path == "README.md"));
    assert!(wt.untracked.contains(&"untracked.txt".to_string()));
    assert!(wt.conflicted.is_empty());
}

#[tokio::test]
async fn a_change_set_counts_what_the_branch_changed_and_not_the_base() {
    let dir = repo();
    let p = dir.path();
    run(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "a.txt", "one\ntwo\nthree\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "add a"]);

    let cs = change_set(p, "feature", "main", BaseSource::Guessed).await.expect("change set");
    assert_eq!(cs.branch, "feature");
    assert_eq!(cs.commits.len(), 1);
    assert_eq!(cs.insertions, 3);
    assert_eq!(cs.deletions, 0);
    assert!(cs.files.iter().any(|f| f.path == "a.txt"));
    assert!(!cs.is_dirty(), "everything was committed");
}

#[tokio::test]
async fn a_rename_is_reported_as_one_file_with_both_names() {
    let dir = repo();
    let p = dir.path();
    write(p, "before.txt", "content that is long enough to be detected as a rename\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "add before"]);

    run(p, &["checkout", "-q", "-b", "feature"]);
    run(p, &["mv", "before.txt", "after.txt"]);
    run(p, &["commit", "-q", "-m", "rename"]);

    let cs = change_set(p, "feature", "main", BaseSource::Guessed).await.expect("change set");
    let renamed = cs.files.iter().find(|f| f.path == "after.txt").expect("the new name");
    assert_eq!(renamed.old_path.as_deref(), Some("before.txt"));
}

#[tokio::test]
async fn the_digest_ignores_an_mtime_bump_that_changed_no_content() {
    // Rewriting a file with identical bytes must not mark a whole review stale.
    let dir = repo();
    let p = dir.path();
    write(p, "x.txt", "same\n");

    let head = "HEADSHA";
    let before = worktree_digest(p, head).await.expect("digest");
    std::thread::sleep(std::time::Duration::from_millis(20));
    write(p, "x.txt", "same\n");
    let after = worktree_digest(p, head).await.expect("digest");

    assert_eq!(before, after, "identical content must produce an identical digest");
}

#[tokio::test]
async fn the_digest_moves_when_an_already_modified_file_is_modified_again() {
    // THE case an earlier design missed. Porcelain output alone is identical
    // across both edits below — the file is "modified" before and after — so a
    // digest built from status text would never move, and every unanchored
    // comment (most of them) would sit there claiming to be current.
    let dir = repo();
    let p = dir.path();

    write(p, "README.md", "first edit\n");
    let head = "HEADSHA";
    let once = worktree_digest(p, head).await.expect("digest");

    write(p, "README.md", "second edit\n");
    let twice = worktree_digest(p, head).await.expect("digest");

    assert_ne!(once, twice, "a second edit to an already-dirty file must move the digest");
}

#[tokio::test]
async fn the_digest_moves_when_head_moves_even_with_a_clean_tree() {
    let dir = repo();
    let p = dir.path();
    let a = worktree_digest(p, "sha-one").await.expect("digest");
    let b = worktree_digest(p, "sha-two").await.expect("digest");
    assert_ne!(a, b);
}

#[tokio::test]
async fn a_new_untracked_file_moves_the_digest() {
    let dir = repo();
    let p = dir.path();
    let before = worktree_digest(p, "head").await.expect("digest");
    write(p, "brand-new.txt", "hello\n");
    let after = worktree_digest(p, "head").await.expect("digest");
    assert_ne!(before, after, "an agent creating a file is a change worth re-reading");
}

#[tokio::test]
async fn shortstat_gives_the_three_numbers_a_sidebar_row_needs() {
    let dir = repo();
    let p = dir.path();
    run(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "a.txt", "one\ntwo\nthree\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "add a"]);
    write(p, "README.md", "changed\n");
    run(p, &["commit", "-qam", "change readme"]);

    let (files, ins, del) =
        farcooler_daemon::change_set::shortstat(p, "main").await.expect("shortstat");
    assert_eq!(files, 2);
    assert_eq!(ins, 4, "three new lines plus the readme rewrite");
    assert_eq!(del, 1, "and git says `1 deletion(-)`, singular");
}

#[tokio::test]
async fn shortstat_works_in_a_linked_worktree_which_is_the_only_kind_this_product_makes() {
    // Every workspace is a `git worktree add`, never the main checkout, so the
    // main-worktree case above is the one that never happens in production.
    let dir = repo();
    let p = dir.path();
    let linked = dir.path().parent().unwrap().join("linked-wt");

    run(p, &["worktree", "add", "-b", "feature", linked.to_str().unwrap()]);
    write(&linked, "a.txt", "one\ntwo\nthree\n");
    run(&linked, &["add", "."]);
    run(&linked, &["commit", "-q", "-m", "add a"]);

    let (files, ins, del) = farcooler_daemon::change_set::shortstat(&linked, "main")
        .await
        .expect("shortstat in a linked worktree");
    assert_eq!((files, ins, del), (1, 3, 0));

    let _ = std::fs::remove_dir_all(&linked);
}

#[tokio::test]
async fn a_repository_on_master_resolves_a_base_instead_of_showing_nothing() {
    // The bug this exists for: `main` was hardcoded, so a repository on `master`
    // resolved no merge base and review showed an empty diff with no reason —
    // "nothing changed" and "I could not work out what to compare against"
    // looked identical.
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    run(p, &["init", "--initial-branch=master", "-q"]);
    run(p, &["config", "user.email", "test@example.com"]);
    run(p, &["config", "user.name", "Test"]);
    run(p, &["config", "commit.gpgsign", "false"]);
    write(p, "README.md", "start\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "initial"]);

    let base = farcooler_daemon::change_set::guess_base(p).await;
    assert_eq!(base.as_deref(), Some("master"));
}

#[tokio::test]
async fn a_repository_on_main_still_resolves_main() {
    let dir = repo();
    let base = farcooler_daemon::change_set::guess_base(dir.path()).await;
    assert_eq!(base.as_deref(), Some("main"));
}

#[tokio::test]
async fn origin_head_wins_over_a_local_main_because_it_is_what_the_remote_calls_default() {
    let dir = repo();
    let p = dir.path();
    // A repository whose remote default is `trunk`, with a local `main` that is
    // not it. Guessing `main` here would diff against the wrong branch and look
    // entirely plausible.
    run(p, &["branch", "trunk"]);
    run(p, &["update-ref", "refs/remotes/origin/trunk", "refs/heads/trunk"]);
    run(p, &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/trunk"]);

    let base = farcooler_daemon::change_set::guess_base(p).await;
    assert_eq!(base.as_deref(), Some("origin/trunk"));
}

#[tokio::test]
async fn a_repository_with_no_main_no_master_and_no_remote_resolves_nothing() {
    // Nothing is the honest answer, and it is what lets the caller say "pick a
    // base" rather than silently diffing against a ref that is not there.
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    run(p, &["init", "--initial-branch=develop", "-q"]);
    run(p, &["config", "user.email", "test@example.com"]);
    run(p, &["config", "user.name", "Test"]);
    run(p, &["config", "commit.gpgsign", "false"]);
    write(p, "README.md", "start\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "initial"]);

    assert!(farcooler_daemon::change_set::guess_base(p).await.is_none());
}

#[tokio::test]
async fn a_change_set_against_master_counts_what_the_branch_changed() {
    // End to end on the shape that used to show nothing at all.
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    run(p, &["init", "--initial-branch=master", "-q"]);
    run(p, &["config", "user.email", "test@example.com"]);
    run(p, &["config", "user.name", "Test"]);
    run(p, &["config", "commit.gpgsign", "false"]);
    write(p, "README.md", "start\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "initial"]);

    run(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "a.txt", "one\ntwo\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "add a"]);

    let base = farcooler_daemon::change_set::guess_base(p).await.expect("a base");
    let cs = change_set(p, "feature", &base, BaseSource::Guessed).await.expect("change set");
    assert_eq!(cs.insertions, 2);
    assert_eq!(cs.commits.len(), 1);
}
