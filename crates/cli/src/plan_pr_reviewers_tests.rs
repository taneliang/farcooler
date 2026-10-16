//! What whom-to-ask is read from (ov-580), in a real repository: CODEOWNERS
//! from the base and never the branch, history by noreply login, the branch's
//! own committers, and paths that are only paths.

use std::path::Path;
use std::process::Command;

use farcooler_core::reviewers::{self, Evidence};
use farcooler_core::reviewers_git::gather;

fn git(dir: &Path, email: &str, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", &format!("user.email={email}"), "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn commit(dir: &Path, email: &str, file: &str, text: &str) {
    let path = dir.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    git(dir, email, &["add", file]);
    git(dir, email, &["commit", "-q", "-m", file]);
}

/// `main`: CODEOWNERS naming alice for crates/, bob's two commits and carol's
/// one on crates/store, a work-email commit, and a bot's. `lane`: dave's
/// commit, which also edits CODEOWNERS to name dave.
fn scratch() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    git(d, "x@example.com", &["init", "-q", "-b", "main"]);
    commit(d, "1+alice@users.noreply.github.com", ".github/CODEOWNERS", "crates/ @alice\n");
    commit(d, "2+bob@users.noreply.github.com", "crates/store/a.rs", "1\n");
    commit(d, "bob@users.noreply.github.com", "crates/store/a.rs", "2\n");
    commit(d, "3+carol@users.noreply.github.com", "crates/store/b.rs", "1\n");
    commit(d, "someone@corp.example", "crates/store/c.rs", "1\n");
    commit(d, "49+dependabot[bot]@users.noreply.github.com", "crates/store/d.rs", "1\n");
    git(d, "x@example.com", &["checkout", "-q", "-b", "lane"]);
    commit(d, "4+dave@users.noreply.github.com", ".github/CODEOWNERS", "crates/ @dave\n");
    commit(d, "4+dave@users.noreply.github.com", "crates/store/a.rs", "3\n");
    dir
}

fn paths(p: &[&str]) -> Vec<String> {
    p.iter().map(|s| s.to_string()).collect()
}

#[test]
fn codeowners_come_from_the_base_so_a_branch_cannot_name_itself() {
    let repo = scratch();
    let got = gather(Some(repo.path()), "main", "lane", &paths(&["crates/store/a.rs"]));
    assert_eq!(got.codeowners.as_deref(), Some("crates/ @alice\n"), "not the branch's edit naming dave");
    assert_eq!(got.map, None);
}

#[test]
fn history_counts_logins_over_the_touched_files_as_of_the_base() {
    let repo = scratch();
    let got = gather(Some(repo.path()), "main", "lane", &paths(&["crates/store/a.rs", "crates/store/b.rs", "crates/store/c.rs", "crates/store/d.rs"]));
    let mut seen = got.history.clone();
    seen.sort();
    // Bob twice, carol once; a work address and a bot are nobody; dave's commit is the branch's, after the base.
    assert_eq!(seen, [("bob".to_string(), 2), ("carol".to_string(), 1)]);
}

#[test]
fn the_branch_s_own_committers_are_found_and_are_not_suggested() {
    let repo = scratch();
    let got = gather(Some(repo.path()), "main", "lane", &paths(&["crates/store/a.rs"]));
    assert_eq!(got.committers, ["dave"]);
    let authors = vec!["e-liang".to_string()].into_iter().chain(got.committers.clone()).collect::<Vec<_>>();
    let p = paths(&["crates/store/a.rs"]);
    let asked = reviewers::suggest(&Evidence {
        authors: &authors,
        paths: &p,
        codeowners: got.codeowners.as_deref(),
        history: &got.history,
        map: None,
    });
    let names: Vec<&str> = asked.iter().map(|s| s.reviewer.as_str()).collect();
    assert_eq!(names, ["alice", "bob"], "dave wrote the branch; carol never touched a.rs");
}

#[test]
fn the_reviewer_map_is_read_from_the_base_when_the_repository_keeps_one() {
    let repo = scratch();
    git(repo.path(), "x@example.com", &["checkout", "-q", "main"]);
    commit(repo.path(), "x@example.com", ".farcooler/reviewers", "docs/ @gina\n");
    let got = gather(Some(repo.path()), "main", "", &paths(&["docs/x.md"]));
    assert_eq!(got.map.as_deref(), Some("docs/ @gina\n"));
}

#[test]
fn a_hostile_path_or_ref_is_only_a_name() {
    let repo = scratch();
    // A pathspec-magic name matches no file: it is a file name, not a pathspec for everything.
    let got = gather(Some(repo.path()), "main", "lane", &paths(&[":(top)crates", ":/", "--output=/tmp/x"]));
    assert!(got.history.is_empty(), "{got:?}");
    // A ref that git would read as an option is not asked about.
    assert_eq!(gather(Some(repo.path()), "--output=/tmp/x", "lane", &paths(&["a"])), Default::default());
    assert!(gather(Some(repo.path()), "main", "-p", &paths(&["crates/store/a.rs"])).committers.is_empty());
    // And a directory that is no repository answers nothing, rather than failing.
    let elsewhere = tempfile::tempdir().unwrap();
    assert_eq!(gather(Some(elsewhere.path()), "main", "lane", &paths(&["a"])).history, []);
}
