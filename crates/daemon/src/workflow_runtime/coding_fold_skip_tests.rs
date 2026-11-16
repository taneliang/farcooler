//! The fold skips commits whose change is already in its tree (ov-1283), so a ticket stacked on another
//! lands without a manual rebase.
use std::path::Path;
use super::{Binding, Runtime};
use super::coding_tests::{admitted, configured};
use super::tests::enqueue;

/// A scratch repository: `main` holds one file, and the returned closure runs git in it.
fn stacked_repository(dir: &Path) -> impl Fn(&[&str]) -> String + '_ {
    let g = move |args: &[&str]| -> String {
        let out = std::process::Command::new("/usr/bin/git").current_dir(dir)
            .args(["-c", "user.name=T", "-c", "user.email=t@localhost", "-c", "commit.gpgsign=false"]).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    g(&["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("shared.txt"), "one\ntwo\nthree\nfour\nfive\nsix\nseven\n").unwrap();
    g(&["add", "."]); g(&["commit", "-qm", "base"]);
    g
}

fn edit(dir: &Path, from: &str, to: &str) {
    let text = std::fs::read_to_string(dir.join("shared.txt")).unwrap();
    std::fs::write(dir.join("shared.txt"), text.replacen(from, to, 1)).unwrap();
}

async fn binding() -> (tempfile::TempDir, Runtime, Binding) {
    let (dir, mut runtime) = configured(false, "pass").await;
    let definition = runtime.configured_definition().unwrap();
    enqueue(&mut runtime, "w", "one", definition);
    let binding = admitted(&runtime).await;
    (dir, runtime, binding)
}

fn member(item: &str, candidate: &str) -> farcooler_workflow::Member {
    farcooler_workflow::Member { item: item.into(), candidate: candidate.into() }
}

/// B is stacked on A, whose second commit touches the line its first one did. A folds first; B's list
/// carries A's commits again, which are skipped, and the tree equals a manual rebase of B onto A.
#[tokio::test]
async fn a_stacked_ticket_folds_clean_after_the_ticket_it_is_stacked_on() {
    let (_dir, runtime, binding) = binding().await;
    let repo = tempfile::tempdir().unwrap();
    let g = stacked_repository(repo.path());
    let base = g(&["rev-parse", "HEAD"]);
    g(&["checkout", "-qb", "a"]);
    edit(repo.path(), "two", "two-a1"); g(&["commit", "-qam", "a1"]);
    edit(repo.path(), "two-a1", "two-a2"); g(&["commit", "-qam", "a2"]);
    g(&["checkout", "-qb", "b"]);
    edit(repo.path(), "six", "six-b"); g(&["commit", "-qam", "b1"]);
    let (a, b) = (g(&["rev-parse", "a"]), g(&["rev-parse", "b"]));
    g(&["checkout", "-q", "-b", "manual", &a]);
    g(&["cherry-pick", &b]);
    let expected = g(&["rev-parse", "HEAD^{tree}"]);
    g(&["checkout", "-q", "--detach", &base]);
    assert!(runtime.pick_member(&binding, repo.path(), &base, &member("a", &a)).await.unwrap().is_none());
    assert!(runtime.pick_member(&binding, repo.path(), &base, &member("b", &b)).await.unwrap().is_none(), "A's commits are skipped");
    assert_eq!(g(&["rev-parse", "HEAD^{tree}"]), expected);
}

/// A's commits already sit on the base as copies under other shas; B, stacked on A's originals, folds clean.
#[tokio::test]
async fn a_ticket_stacked_on_commits_already_on_the_base_as_copies_folds_clean() {
    let (_dir, runtime, binding) = binding().await;
    let repo = tempfile::tempdir().unwrap();
    let g = stacked_repository(repo.path());
    let start = g(&["rev-parse", "HEAD"]);
    g(&["checkout", "-qb", "a"]);
    edit(repo.path(), "two", "two-a1"); g(&["commit", "-qam", "a1"]);
    edit(repo.path(), "two-a1", "two-a2"); g(&["commit", "-qam", "a2"]);
    g(&["checkout", "-qb", "b"]);
    edit(repo.path(), "six", "six-b"); g(&["commit", "-qam", "b1"]);
    let b = g(&["rev-parse", "b"]);
    g(&["checkout", "-q", "-B", "main", &start]);
    edit(repo.path(), "four", "four-main"); g(&["commit", "-qam", "unrelated"]);
    g(&["cherry-pick", "a~1", "a"]);
    let base = g(&["rev-parse", "HEAD"]);
    g(&["checkout", "-q", "--detach", &base]);
    assert!(runtime.pick_member(&binding, repo.path(), &base, &member("b", &b)).await.unwrap().is_none());
    assert_eq!(g(&["rev-list", "--count", &format!("{base}..HEAD")]), "1", "only B's own commit is added");
    assert_eq!(std::fs::read_to_string(repo.path().join("shared.txt")).unwrap(),
        "one\ntwo-a2\nthree\nfour-main\nfive\nsix-b\nseven\n");
}

/// A commit that truly conflicts is still the member's conflict, with the file named.
#[tokio::test]
async fn a_commit_that_truly_conflicts_still_fails_the_fold_naming_the_file() {
    let (_dir, runtime, binding) = binding().await;
    let repo = tempfile::tempdir().unwrap();
    let g = stacked_repository(repo.path());
    g(&["checkout", "-qb", "t"]);
    edit(repo.path(), "two", "two-t"); g(&["commit", "-qam", "t1"]);
    let t = g(&["rev-parse", "t"]);
    g(&["checkout", "-q", "main"]);
    edit(repo.path(), "two", "two-main"); g(&["commit", "-qam", "main moved"]);
    let base = g(&["rev-parse", "HEAD"]);
    let conflict = runtime.pick_member(&binding, repo.path(), &base, &member("t", &t)).await.unwrap().unwrap();
    assert_eq!(conflict["files"], serde_json::json!(["shared.txt"]));
    assert_eq!(g(&["status", "--porcelain"]), "", "the aborted pick leaves the tree settled");
}
