//! How `gh pr comment` ending maps to `Posted` (ov-591): a stub `gh` of the
//! test's own, a script, so the exit, the spawn and the stdin are the real
//! ones. Whether a claim is kept or given back hangs on this mapping.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

use super::*;

const URL: &str = "https://github.example/o/r/pull/7";
/// Long enough that a slow runner's script has started and finished.
const LIMIT: Duration = Duration::from_secs(30);

/// A script in its own scratch directory, and where it records what it saw.
fn stub(name: &str, script: &str) -> (PathBuf, PathBuf) {
    let dir = farcooler_core::scratch::dir(name);
    let program = dir.join("gh");
    std::fs::write(&program, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    (program, dir)
}

#[tokio::test]
async fn a_gh_that_exits_zero_posted_and_gets_the_body_on_stdin_and_the_url_in_its_arguments() {
    let (program, dir) = stub("nudge-post-ok", "cat > \"$(dirname \"$0\")/body\"\necho \"$@\" > \"$(dirname \"$0\")/args\"");
    let mut gh = tokio::process::Command::new(&program);
    assert_eq!(comment(&mut gh, URL, "@alice, hello", LIMIT).await, Posted::Yes);
    assert_eq!(std::fs::read_to_string(dir.join("body")).unwrap(), "@alice, hello");
    assert_eq!(std::fs::read_to_string(dir.join("args")).unwrap().trim(), format!("pr comment {URL} --body-file -"));
}

#[tokio::test]
async fn a_gh_that_ran_and_failed_may_have_posted_so_it_is_ran_not_notrun() {
    let (program, _dir) = stub("nudge-post-exit1", "cat > /dev/null\nexit 1");
    assert_eq!(comment(&mut tokio::process::Command::new(&program), URL, "x", LIMIT).await, Posted::Ran);
}

#[tokio::test]
async fn a_gh_that_closes_its_stdin_before_reading_it_ran() {
    let (program, _dir) = stub("nudge-post-early", "exec 0<&-\nsleep 1\nexit 0");
    // A large body, so the write meets the closed pipe rather than its buffer.
    let body = "x".repeat(1 << 20);
    let got = comment(&mut tokio::process::Command::new(&program), URL, &body, LIMIT).await;
    assert_eq!(got, Posted::Ran, "a body that wasn't taken is not a post");
}

#[tokio::test]
async fn a_gh_that_cannot_be_started_certainly_posted_nothing() {
    let dir = farcooler_core::scratch::dir("nudge-post-missing");
    let mut gh = tokio::process::Command::new(dir.join("no-such-gh"));
    assert_eq!(comment(&mut gh, URL, "x", LIMIT).await, Posted::NotRun);
}

#[tokio::test]
async fn a_gh_that_does_not_finish_in_time_ran() {
    let (program, _dir) = stub("nudge-post-slow", "exec sleep 600");
    let started = Instant::now();
    let got = comment(&mut tokio::process::Command::new(&program), URL, "x", Duration::from_millis(200)).await;
    assert_eq!(got, Posted::Ran, "it may have posted");
    assert!(started.elapsed() < Duration::from_secs(30), "and the wait ended at the limit");
}

#[test]
fn the_owner_one_host_said_is_not_the_owner_on_another() {
    remember_owner("a.nudge-test.example", "alice");
    assert_eq!(known_owner("a.nudge-test.example").as_deref(), Some("alice"));
    assert_eq!(known_owner("b.nudge-test.example"), None, "another host's account is another login");
    remember_owner("b.nudge-test.example", "bob");
    assert_eq!(known_owner("a.nudge-test.example").as_deref(), Some("alice"));
    assert_eq!(known_owner("b.nudge-test.example").as_deref(), Some("bob"));
}
