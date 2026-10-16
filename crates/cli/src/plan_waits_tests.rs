//! "Waiting for alice: 2 PRs, about 50 minutes" (ov-579): what the plan says of
//! the reviewers the board's pull requests wait on.

use super::*;

const NOW: i64 = 1_800_000_000_000;
const MINUTE: i64 = 60_000;

fn wait(who: &str, prs: u32, median_minutes: Option<i64>) -> pb::ReviewerWait {
    pb::ReviewerWait {
        reviewer: who.into(),
        prs,
        median_wait_ms: median_minutes.map(|m| m * MINUTE),
        samples: u32::from(median_minutes.is_some()),
    }
}

fn plan_with(waits: Vec<pb::ReviewerWait>) -> pb::Plan {
    pb::Plan {
        now_ms: NOW,
        lanes: vec![pb::Lane { id: vec![1].into(), name: "review".into(), state: pb::LaneState::Review as i32, ..Default::default() }],
        review_waits: waits,
        ..Default::default()
    }
}

#[test]
fn each_reviewer_a_pr_waits_on_gets_a_line_with_their_usual_time() {
    let text = overview(&plan_with(vec![wait("alice", 2, Some(48)), wait("bob", 1, None)]), NOW);
    assert!(text.contains("Waiting on reviewers\n  Waiting for alice: 2 PRs, about 50 minutes\n  Waiting for bob: 1 PR, no history yet"), "{text}");
}

#[test]
fn a_runner_that_sends_no_waits_adds_no_section() {
    assert!(!overview(&plan_with(vec![]), NOW).contains("Waiting"));
}

#[test]
fn a_usual_time_is_said_in_the_unit_a_person_plans_by() {
    let said = |minutes: i64| overview_text::usual_wait(minutes * MINUTE);
    assert_eq!(said(0), "under 5 minutes");
    assert_eq!(said(7), "about 5 minutes");
    assert_eq!(said(48), "about 50 minutes");
    assert_eq!(said(89), "about 90 minutes");
    assert_eq!(said(90), "about 2 hours");
    assert_eq!(said(150), "about 3 hours");
    assert_eq!(said(49 * 60), "about 2 days");
}

#[test]
fn the_json_carries_the_waits_with_an_absent_median_as_null() {
    let json = plan_json(&plan_with(vec![wait("alice", 2, Some(48)), wait("bob", 1, None)]), &Keys::of_plan(&plan_with(vec![])));
    assert_eq!(json["review_waits"][0]["median_wait_ms"], 48 * MINUTE);
    assert!(plan_json(&plan_with(vec![]), &Keys::of_plan(&plan_with(vec![]))).get("review_waits").is_none());
    assert!(json["review_waits"][1]["median_wait_ms"].is_null());
}
