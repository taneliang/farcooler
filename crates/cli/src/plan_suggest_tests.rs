//! "Suggested reviewers" in `farcooler plan` (ov-580): whom to ask for each
//! live lane's pull request, and nothing from a runner that sends none.

use super::*;

const NOW: i64 = 1_800_000_000_000;

fn suggestion(lane: u8, pr: u32, who: &[(&str, &str)]) -> pb::LaneReviewers {
    pb::LaneReviewers {
        lane_id: vec![lane].into(),
        pr,
        reviewers: who.iter().map(|(r, _)| r.to_string()).collect(),
        sources: who.iter().map(|(_, s)| s.to_string()).collect(),
    }
}

fn plan_with(suggested: Vec<pb::LaneReviewers>) -> pb::Plan {
    let lane = |id: u8, name: &str| pb::Lane { id: vec![id].into(), name: name.into(), state: pb::LaneState::Review as i32, ..Default::default() };
    pb::Plan { now_ms: NOW, lanes: vec![lane(1, "mac-frost"), lane(2, "phones")], suggested_reviewers: suggested, ..Default::default() }
}

#[test]
fn each_lane_s_pull_request_says_whom_to_ask_and_where_each_name_came_from() {
    let plan = plan_with(vec![
        suggestion(1, 31, &[("alice", "CODEOWNERS"), ("bob", "recent commits")]),
        suggestion(2, 32, &[("carol", "the reviewer map")]),
    ]);
    let text = overview(&plan, NOW);
    assert!(
        text.contains(
            "Suggested reviewers\n  mac-frost, PR 31: alice (CODEOWNERS), bob (recent commits)\n  phones, PR 32: carol (the reviewer map)"
        ),
        "{text}"
    );
}

#[test]
fn a_runner_that_sends_none_adds_no_section_and_a_lane_the_plan_lacks_is_skipped() {
    assert!(!overview(&plan_with(vec![]), NOW).contains("Suggested"));
    let stray = plan_with(vec![suggestion(9, 40, &[("alice", "CODEOWNERS")])]);
    assert!(!overview(&stray, NOW).contains("Suggested"), "no heading over nothing");
}

#[test]
fn the_json_carries_them_only_when_there_are_some() {
    let keys = Keys::of_plan(&plan_with(vec![]));
    let json = plan_json(&plan_with(vec![suggestion(1, 31, &[("alice", "CODEOWNERS")])]), &keys);
    assert_eq!(json["suggested_reviewers"][0]["reviewers"][0], "alice");
    assert_eq!((json["suggested_reviewers"][0]["pr"].as_u64(), json["suggested_reviewers"][0]["sources"][0].as_str()), (Some(31), Some("CODEOWNERS")));
    assert!(plan_json(&plan_with(vec![]), &keys).get("suggested_reviewers").is_none());
}
