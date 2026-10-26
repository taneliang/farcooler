//! `status --json`'s `buildsMatch`, the field the Mac's Update Runner reads
//! (ov-352): a runner on this CLI's own commit matches in whatever spelling
//! it reports, and one on another commit never does.

use super::*;

/// This CLI's commit, however `BUILD` spells it, and whether its tree was dirty.
fn own_commit() -> (String, &'static str) {
    let build = farcooler_protocol::BUILD;
    let dirty = if build.ends_with("-dirty") { "-dirty" } else { "" };
    let commit = build.rsplit('+').next().unwrap().trim_end_matches("-dirty").to_string();
    (commit, dirty)
}

fn matches_for(daemon_version: &str) -> bool {
    let host = farcooler_protocol::v1::Host { daemon_version: daemon_version.into(), ..Default::default() };
    let counts = StatusCounts { roots: 0, repositories: 0, worktrees: 0, terminals: 0 };
    status_json(&host, &[], counts)["buildsMatch"].as_bool().unwrap()
}

#[test]
fn a_runner_on_this_commit_matches_in_every_spelling() {
    let (commit, dirty) = own_commit();
    assert!(matches_for(farcooler_protocol::BUILD));
    assert!(matches_for(&format!("{commit}{dirty}")), "a bare commit");
    assert!(matches_for(&format!("0.1.0 (canary {commit}{dirty})")), "the app's display string");
}

#[test]
fn a_runner_on_another_commit_never_matches() {
    let (commit, dirty) = own_commit();
    // Differs from this commit in its first digit, whatever that is.
    let other = format!("{}{}", if commit.starts_with('0') { '1' } else { '0' }, &commit[1..]);
    assert!(!matches_for(&format!("0.1.0+{other}{dirty}")));
    assert!(!matches_for(&format!("{other}{dirty}")));
    assert!(!matches_for(&format!("0.1.0 (canary {other}{dirty})")));
}
