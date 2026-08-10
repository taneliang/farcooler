//! Per-file diff behaviour against a real repository.
//!
//! The case worth building a fixture for is the merge commit. `git show` on a
//! merge emits a combined diff, which parses cleanly into nonsense; this checks
//! that the daemon never asks for one, and that a merge still shows something
//! useful rather than an error.

use std::path::Path;
use std::process::Command;

use farcooler_daemon::file_diff::{Selector, Unsupported, commit_files, file_diff, is_merge};
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

fn capture(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git").current_dir(repo).args(args).output().expect("git");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write(repo: &Path, path: &str, contents: &str) {
    std::fs::write(repo.join(path), contents).expect("write");
}

fn repo() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    run(p, &["init", "--initial-branch=main", "-q"]);
    run(p, &["config", "user.email", "test@example.com"]);
    run(p, &["config", "user.name", "Test"]);
    run(p, &["config", "commit.gpgsign", "false"]);
    write(p, "shared.txt", "one\ntwo\nthree\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "initial"]);
    dir
}

#[tokio::test]
async fn an_ordinary_commit_diffs_against_its_parent() {
    let dir = repo();
    let p = dir.path();
    write(p, "shared.txt", "one\nTWO\nthree\n");
    run(p, &["commit", "-qam", "change two"]);
    let sha = capture(p, &["rev-parse", "HEAD"]);

    let r = file_diff(p, &Selector::Commit { sha }, "shared.txt", 0, 0).await.expect("diff");
    assert!(r.unsupported.is_none());
    assert!(!r.first_parent_of_merge);
    assert_eq!(r.diff.hunks.len(), 1);
    let texts: Vec<&str> = r.diff.hunks[0].lines.iter().map(|l| l.text.as_str()).collect();
    assert!(texts.contains(&"two"));
    assert!(texts.contains(&"TWO"));
}

#[tokio::test]
async fn the_first_commit_diffs_against_the_empty_tree_rather_than_failing() {
    // A root commit has no parent. Without the empty-tree fallback this is an
    // error, and the first commit of every repository becomes unreviewable.
    let dir = repo();
    let p = dir.path();
    let root = capture(p, &["rev-list", "--max-parents=0", "HEAD"]);

    let r = file_diff(p, &Selector::Commit { sha: root }, "shared.txt", 0, 0).await.expect("diff");
    assert!(r.unsupported.is_none());
    assert_eq!(r.diff.hunks.len(), 1, "the root commit added the whole file");
    assert!(r.diff.hunks[0].lines.iter().all(|l| l.new_no.is_some()));
}

#[tokio::test]
async fn a_merge_commit_is_shown_by_its_first_parent_and_never_as_a_combined_diff() {
    let dir = repo();
    let p = dir.path();

    run(p, &["checkout", "-q", "-b", "side"]);
    write(p, "side.txt", "from the side\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "side work"]);

    run(p, &["checkout", "-q", "main"]);
    write(p, "shared.txt", "one\ntwo\nthree\nfour\n");
    run(p, &["commit", "-qam", "main work"]);

    run(p, &["merge", "--no-ff", "-q", "side", "-m", "merge side"]);
    let sha = capture(p, &["rev-parse", "HEAD"]);

    assert!(is_merge(p, &sha).await.expect("rev-list"), "fixture really is a merge");

    let r = file_diff(p, &Selector::Commit { sha }, "side.txt", 0, 0).await.expect("diff");
    assert!(r.first_parent_of_merge, "the client needs to say which view this is");
    assert_ne!(
        r.unsupported,
        Some(Unsupported::CombinedDiff),
        "asking for first-parent means a combined diff never reaches the parser"
    );
    // The merge brought side.txt in, so a first-parent view shows it added.
    assert_eq!(r.diff.hunks.len(), 1);
    assert!(r.diff.hunks[0].lines.iter().all(|l| l.new_no.is_some()));
}

#[tokio::test]
async fn a_binary_file_is_named_as_binary_rather_than_shown_as_unchanged() {
    let dir = repo();
    let p = dir.path();
    std::fs::write(p.join("blob.bin"), [0u8, 159, 146, 150, 0, 1, 2, 3]).expect("write");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "add a binary"]);
    let sha = capture(p, &["rev-parse", "HEAD"]);

    let r = file_diff(p, &Selector::Commit { sha }, "blob.bin", 0, 0).await.expect("diff");
    assert_eq!(r.unsupported, Some(Unsupported::Binary));
    assert!(r.diff.hunks.is_empty(), "an empty hunk list plus a reason, never a bare empty diff");
}

#[tokio::test]
async fn staged_and_unstaged_are_different_views_of_the_same_file() {
    let dir = repo();
    let p = dir.path();

    write(p, "shared.txt", "one\nSTAGED\nthree\n");
    run(p, &["add", "shared.txt"]);
    write(p, "shared.txt", "one\nSTAGED\nWORKTREE\n");

    let staged = file_diff(p, &Selector::Staged, "shared.txt", 0, 0).await.expect("staged");
    let unstaged = file_diff(p, &Selector::Unstaged, "shared.txt", 0, 0).await.expect("unstaged");

    let staged_added: Vec<String> = staged.diff.hunks[0]
        .lines
        .iter()
        .filter(|l| l.old_no.is_none())
        .map(|l| l.text.clone())
        .collect();
    let unstaged_added: Vec<String> = unstaged.diff.hunks[0]
        .lines
        .iter()
        .filter(|l| l.old_no.is_none())
        .map(|l| l.text.clone())
        .collect();

    assert!(staged_added.contains(&"STAGED".to_string()));
    assert!(unstaged_added.contains(&"WORKTREE".to_string()));
    assert!(
        !staged_added.contains(&"WORKTREE".to_string()),
        "what is staged and what is merely saved are different answers"
    );
}

#[tokio::test]
async fn a_range_diff_covers_the_whole_branch_not_just_the_last_commit() {
    let dir = repo();
    let p = dir.path();
    let base = capture(p, &["rev-parse", "HEAD"]);

    run(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "shared.txt", "one\nA\nthree\n");
    run(p, &["commit", "-qam", "first"]);
    write(p, "shared.txt", "one\nA\nB\n");
    run(p, &["commit", "-qam", "second"]);

    let r = file_diff(p, &Selector::Range { base_commit: base }, "shared.txt", 0, 0)
        .await
        .expect("diff");
    let added: Vec<String> = r.diff.hunks[0]
        .lines
        .iter()
        .filter(|l| l.old_no.is_none())
        .map(|l| l.text.clone())
        .collect();
    assert!(added.contains(&"A".to_string()));
    assert!(added.contains(&"B".to_string()), "both commits' work is in the branch view");
}

#[tokio::test]
async fn commit_files_lists_what_one_commit_touched() {
    let dir = repo();
    let p = dir.path();
    write(p, "shared.txt", "one\ntwo\nthree\nfour\n");
    write(p, "other.txt", "new file\n");
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "two files"]);
    let sha = capture(p, &["rev-parse", "HEAD"]);

    let files = commit_files(p, &sha).await.expect("files");
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.contains(&"shared.txt"));
    assert!(paths.contains(&"other.txt"));
    assert_eq!(paths.len(), 2);
}

#[tokio::test]
async fn asking_for_context_recovers_the_lines_a_diff_leaves_out() {
    // What the app's expand controls are made of. A diff prints three lines
    // either side of a change and omits the rest, and the omitted lines are
    // exactly where "is this safe?" gets settled — so there has to be a way to
    // ask for them that does not mean opening the file somewhere else.
    let dir = repo();
    let p = dir.path();

    let mut long = String::new();
    for i in 1..=60 {
        long.push_str(&format!("line {i}\n"));
    }
    write(p, "long.txt", &long);
    run(p, &["add", "."]);
    run(p, &["commit", "-q", "-m", "sixty lines"]);

    // Two changes far enough apart that git cannot join their hunks.
    let edited = long.replace("line 5\n", "LINE 5\n").replace("line 55\n", "LINE 55\n");
    write(p, "long.txt", &edited);
    run(p, &["commit", "-qam", "two edits"]);
    let sha = capture(p, &["rev-parse", "HEAD"]);

    let tight = file_diff(p, &Selector::Commit { sha: sha.clone() }, "long.txt", 0, 0)
        .await
        .expect("default context");
    assert_eq!(tight.diff.hunks.len(), 2, "two changes, two hunks, and a gap between them");
    let tight_lines: usize = tight.diff.hunks.iter().map(|h| h.lines.len()).sum();

    let wide = file_diff(p, &Selector::Commit { sha }, "long.txt", 0, 5_000)
        .await
        .expect("full context");
    assert_eq!(wide.diff.hunks.len(), 1, "enough context joins them into one");
    let wide_lines: usize = wide.diff.hunks.iter().map(|h| h.lines.len()).sum();
    assert!(
        wide_lines > tight_lines,
        "the wider diff carries the lines the tighter one left out: {wide_lines} vs {tight_lines}"
    );
    // Line 30 is in neither hunk at three lines of context and in the middle of
    // the gap the app would offer to open.
    assert!(
        wide.diff.hunks[0].lines.iter().any(|l| l.text == "line 30"),
        "a line from the gap is present once the context is asked for"
    );
}
