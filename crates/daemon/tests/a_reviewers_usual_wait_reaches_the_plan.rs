//! A reviewer's usual time to a first review reaches `plan.get` (ov-579), end
//! to end: a real `farcoolerd` whose shim `gh` prints the PR list and, apart
//! from it, the GraphQL timing read (`ReadyForReviewEvent` times and reviews).
//!
//! What only a running daemon shows: that every time is GitHub's (the median
//! is an exact figure), that the median is drawn from the reviewer's own
//! reviews, that a reviewer with no history is absent and not zero, and that a
//! timing read that fails changes nothing, the lane's stage included.

use farcooler_protocol::v1::{self as pb, LaneState as L};

mod common;
#[path = "support/gh_pr.rs"]
mod gh_pr;
use gh_pr::*;

const MIN: i64 = 60_000;
const T0: i64 = 1_790_000_000_000;

fn iso(ms: i64) -> String {
    let secs = ms / 1000;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// One PR as `gh pr list` prints it: no timing in it.
fn listed(number: u32, key: &str, state: &str, asked: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "number": number, "url": format!("https://github.example/o/r/pull/{number}"), "title": format!("{key}: work"),
        "state": state, "headRefName": format!("{key}/work"), "baseRefName": "main", "isDraft": false,
        "mergedAt": if state == "MERGED" { serde_json::json!(iso(T0)) } else { serde_json::Value::Null },
        "reviewDecision": "", "latestReviews": [], "mergeStateStatus": "CLEAN", "statusCheckRollup": [],
        "reviewRequests": asked.iter().map(|a| serde_json::json!({"login": a})).collect::<Vec<_>>(),
    })
}

/// The timing answer for a PR made ready at minute 0 and reviewed by `who` after `after` minutes.
fn timed(who: Option<(&str, i64)>) -> serde_json::Value {
    let reviews: Vec<_> = who
        .iter()
        .map(|(w, after)| serde_json::json!({"author": {"login": w}, "state": "COMMENTED", "submittedAt": iso(T0 + after * MIN)}))
        .collect();
    serde_json::json!({
        "createdAt": iso(T0 - 600 * MIN), "isDraft": false, "additions": 40, "deletions": 3, "author": {"login": "agent"},
        "reviews": {"nodes": reviews},
        "timelineItems": {"nodes": [{"createdAt": iso(T0)}]},
    })
}

fn serve_timing(dir: &std::path::Path, prs: &[(u32, serde_json::Value)]) {
    let repository: serde_json::Map<String, serde_json::Value> = prs.iter().map(|(n, v)| (format!("p{n}"), v.clone())).collect();
    std::fs::write(dir.join("timing.json"), serde_json::json!({"data": {"repository": repository}}).to_string()).unwrap();
}

fn wait_of<'a>(plan: &'a pb::Plan, who: &str) -> Option<&'a pb::ReviewerWait> {
    plan.review_waits.iter().find(|w| w.reviewer == who)
}

fn timing_calls(dir: &std::path::Path) -> usize {
    calls(dir).iter().filter(|l| *l == "timing").count()
}

/// A lane of four cards in review: card 1's PR waits on alice, bob and a team; cards 2 to 4 merged,
/// each first reviewed by alice 30, 40 and 50 minutes after it was ready.
async fn a_board_with_history() -> (tempfile::TempDir, common::DaemonChild, SocketClient, pb::Workspace, pb::Lane) {
    let (dir, daemon, mut client, workspace) =
        a_runner_with(&[("FARCOOLER_TEST_KICK_FLOOR_MS", "500"), ("FARCOOLER_TEST_TIMING_EVERY_MS", "500")]).await;
    let mut cards = Vec::new();
    for i in 1..=4 {
        cards.push(a_card(&mut client, &workspace, &format!("Card {i}")).await);
    }
    let k = |i: usize| cards[i].key.clone();
    let open = vec![listed(101, &k(0), "OPEN", &["alice", "bob", "myorg/core"])];
    let merged: Vec<_> = (1..4).map(|i| listed(101 + i as u32, &k(i), "MERGED", &[])).collect();
    std::fs::write(dir.path().join("pr-list-open.json"), serde_json::Value::Array(open).to_string()).unwrap();
    std::fs::write(dir.path().join("pr-list-closed.json"), serde_json::Value::Array(merged).to_string()).unwrap();
    serve_timing(
        dir.path(),
        &[(101, timed(None)), (102, timed(Some(("alice", 30)))), (103, timed(Some(("alice", 40)))), (104, timed(Some(("alice", 50))))],
    );
    let refs: Vec<&pb::Task> = cards.iter().collect();
    let lane = a_lane(&mut client, &workspace, "review", &refs, &[L::Review]).await;
    (dir, daemon, client, workspace, lane)
}

#[tokio::test]
async fn a_reviewers_usual_wait_is_githubs_exact_median_of_their_reviews() {
    let (_dir, _daemon, mut client, workspace, _lane) = a_board_with_history().await;
    let read = until(&mut client, &workspace, "alice's wait never reached the plan", |p| {
        wait_of(p, "alice").is_some_and(|w| w.samples == 3)
    })
    .await;
    let alice = wait_of(&read, "alice").unwrap();
    assert_eq!((alice.prs, alice.median_wait_ms, alice.samples), (1, Some(40 * MIN), 3));
    let bob = wait_of(&read, "bob").unwrap();
    assert_eq!((bob.prs, bob.median_wait_ms, bob.samples), (1, None, 0), "no history is absent, not zero");
    let team = wait_of(&read, "myorg/core").unwrap();
    assert_eq!((team.median_wait_ms, team.samples), (None, 0), "a team slug has no reviews of its own");
}

#[tokio::test]
async fn a_failed_timing_read_keeps_the_last_timing_and_the_stage() {
    let (dir, _daemon, mut client, workspace, lane) = a_board_with_history().await;
    let before = until(&mut client, &workspace, "alice's wait never reached the plan", |p| wait_of(p, "alice").is_some_and(|w| w.samples == 3)).await;
    let stage = before.lanes.iter().find(|l| l.name == "review").and_then(|l| l.stage.clone()).expect("a stage").label;

    // GitHub's timing now fails, and would say something else if it did not.
    std::fs::write(dir.path().join("timing-fails"), "").unwrap();
    serve_timing(dir.path(), &[(101, timed(None))]);
    let asked = timing_calls(dir.path());
    let lists = lists(dir.path());
    // Past the timing cadence set above, so the next turn reads it.
    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    move_lane(&mut client, &lane, L::Fixing).await;
    move_lane(&mut client, &lane, L::Review).await;
    let deadline = std::time::Instant::now() + BUDGET;
    while timing_calls(dir.path()) <= asked || self::lists(dir.path()) <= lists {
        assert!(std::time::Instant::now() < deadline, "the watch never asked for the timing again");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    let after = plan(&mut client, &workspace).await;
    assert_eq!(wait_of(&after, "alice"), wait_of(&before, "alice"), "the last timing stands");
    let now = after.lanes.iter().find(|l| l.name == "review").and_then(|l| l.stage.clone()).expect("a stage").label;
    assert_eq!(now, stage, "and the stage never depended on it");
}
