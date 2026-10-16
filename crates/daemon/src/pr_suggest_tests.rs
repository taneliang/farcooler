//! Whom to ask, read in a real repository the way the watch reads it (ov-580):
//! against `origin/<base>`, the author out, and a pull request that gets
//! nothing when nobody can be told from the author.

use std::path::Path;
use std::process::Command;

use super::*;
use crate::pr_timing::PrTiming;
use crate::stack::{CheckState, PrReview, PrStatus, ReviewDecision};

fn git(dir: &Path, email: &str, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", &format!("user.email={email}"), "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// A checkout whose `origin/main` holds CODEOWNERS naming the owner and alice
/// for crates/, and bob's commit on crates/a.rs.
fn checkout() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    git(d, "x@example.com", &["init", "-q", "-b", "main"]);
    std::fs::create_dir_all(d.join("crates")).unwrap();
    std::fs::write(d.join("CODEOWNERS"), "crates/ @owner @alice\n").unwrap();
    git(d, "x@example.com", &["add", "."]);
    git(d, "1+owner@users.noreply.github.com", &["commit", "-q", "-m", "owners"]);
    std::fs::write(d.join("crates/a.rs"), "1\n").unwrap();
    git(d, "x@example.com", &["add", "."]);
    git(d, "2+bob@users.noreply.github.com", &["commit", "-q", "-m", "a"]);
    git(d, "x@example.com", &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    dir
}

fn pr(author: &str) -> PrInfo {
    PrInfo {
        head_ref: "ov-1/work".into(),
        base_ref: "main".into(),
        title: "t".into(),
        is_fork: false,
        author: author.into(),
        review: PrReview::default(),
        timing: PrTiming::default(),
        status: PrStatus {
            number: 31,
            url: String::new(),
            state: PrState::Open,
            checks: CheckState::Unknown,
            review_decision: ReviewDecision::Unknown,
            head_oid: "abc".into(),
            merged_at: None,
            fetched_at: 0,
        },
    }
}

fn names(s: &[Suggestion]) -> Vec<&str> {
    s.iter().map(|s| s.reviewer.as_str()).collect()
}

#[tokio::test]
async fn codeowners_then_history_from_origin_main_and_never_the_author() {
    let repo = checkout();
    let got = suggest_for(repo.path(), &pr("owner"), vec!["crates/a.rs".into()]).await;
    assert_eq!(names(&got), ["alice", "bob"], "the owner opened it, so alice from CODEOWNERS and then bob from history");
    assert_eq!(got.iter().map(|s| s.source).collect::<Vec<_>>(), [Source::Codeowners, Source::History]);
    let by_alice = suggest_for(repo.path(), &pr("Alice"), vec!["crates/a.rs".into()]).await;
    assert_eq!(names(&by_alice), ["owner", "bob"], "whatever the case of the author");
}

#[tokio::test]
async fn an_author_nobody_knows_means_nobody_is_suggested() {
    let repo = checkout();
    assert!(suggest_for(repo.path(), &pr(""), vec!["crates/a.rs".into()]).await.is_empty());
}

#[tokio::test]
async fn a_directory_that_is_no_checkout_suggests_nobody_rather_than_failing() {
    let nowhere = tempfile::tempdir().unwrap();
    assert!(suggest_for(nowhere.path(), &pr("owner"), vec!["crates/a.rs".into()]).await.is_empty());
}

#[test]
fn what_was_read_is_kept_by_head_and_read_again_when_the_head_moves() {
    let repo = Uuid::from_u128(0xfeed);
    let p = pr("owner");
    assert!(stale(repo, &p), "never read");
    put(repo, 31, "abc", vec![Suggestion { reviewer: "alice".into(), source: Source::Map }]);
    assert!(!stale(repo, &p));
    assert_eq!(names(&get(repo, 31)), ["alice"]);
    let mut moved = p.clone();
    moved.status.head_oid = "def".into();
    assert!(stale(repo, &moved), "a new head, new files");
}

#[test]
fn a_pull_request_that_asks_someone_or_was_reviewed_or_is_closed_wants_none() {
    assert!(wanted(&pr("owner")));
    let mut asked = pr("owner");
    asked.review.requested = vec!["carol".into()];
    let mut reviewed = pr("owner");
    reviewed.timing.first_review = Moment::At(5);
    let mut merged = pr("owner");
    merged.status.state = PrState::Merged;
    let mut fork = pr("owner");
    fork.is_fork = true;
    for p in [asked, reviewed, merged, fork] {
        assert!(!wanted(&p));
    }
}

/// `checkout()` with its tip rewritten to carry a signature header, and a repository config that
/// would run `gpg.program` to verify it whenever `git log` is asked. The program
/// leaves `marker` behind if it ever runs.
fn planted(marker: &Path) -> tempfile::TempDir {
    let repo = checkout();
    let d = repo.path();
    let program = d.join("fake-gpg.sh");
    std::fs::write(&program, format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display())).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let raw = Command::new("git").current_dir(d).args(["cat-file", "commit", "HEAD"]).output().unwrap().stdout;
    let raw = String::from_utf8(raw).unwrap();
    let (head, message) = raw.split_once("\n\n").unwrap();
    let signed = format!("{head}\ngpgsig -----BEGIN PGP SIGNATURE-----\n \n abc\n -----END PGP SIGNATURE-----\n\n{message}");
    let mut hash = Command::new("git")
        .current_dir(d)
        .args(["hash-object", "-t", "commit", "-w", "--stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(hash.stdin.as_mut().unwrap(), signed.as_bytes()).unwrap();
    let sha = String::from_utf8(hash.wait_with_output().unwrap().stdout).unwrap();
    git(d, "x@example.com", &["update-ref", "refs/remotes/origin/main", sha.trim()]);
    git(d, "x@example.com", &["config", "log.showSignature", "true"]);
    git(d, "x@example.com", &["config", "gpg.program", &program.to_string_lossy()]);
    repo
}

#[tokio::test]
async fn the_runner_reads_git_through_its_guard_so_a_planted_gpg_program_never_runs() {
    let scratch = tempfile::tempdir().unwrap();
    let marker = scratch.path().join("ran");
    let repo = planted(&marker);
    // The plant works: the plain git on the path runs it, so the next assertion can fail.
    let _ = Command::new("git").current_dir(repo.path()).args(["log", "--format=%ae", "origin/main"]).output().unwrap();
    assert!(marker.exists(), "the plant must fire for an unguarded git, or this test proves nothing");
    std::fs::remove_file(&marker).unwrap();

    let got = suggest_for(repo.path(), &pr("owner"), vec!["crates/a.rs".into()]).await;
    assert!(!marker.exists(), "the guard pins gpg.program and log.showSignature, so nothing was executed");
    assert_eq!(names(&got), ["alice", "bob"], "and the reading still worked");
}

#[tokio::test]
async fn committers_alone_do_not_say_who_opened_it() {
    let repo = checkout();
    assert!(suggest_for(repo.path(), &pr(""), vec!["crates/a.rs".into()]).await.is_empty());
}
