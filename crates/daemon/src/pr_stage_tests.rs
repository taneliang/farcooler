//! The stage table, one test per row, fed what `gh pr list` prints
//! (`test/fixtures/pr-stage/pr-list.json`: one pull request for each row, in the
//! shape gh 2.101 prints, the card's key standing where `@N@` is).

use super::*;
use crate::stack::{PrInfo, parse_prs};

fn pr(n: u32) -> PrInfo {
    fixture_prs().into_iter().find(|p| p.status.number == 100 + n).expect("a PR in the fixture")
}

fn stage(n: u32, phase: Phase) -> Stage {
    derive(Some(&pr(n)), phase).expect("a stage")
}

#[test]
fn no_pr_on_a_building_lane_is_building() {
    let s = derive(None, Phase::Building).expect("a stage");
    assert_eq!((s.kind, s.label().as_str(), s.pr_number), (Kind::Building, "Building", 0));
}

/// A lane already in review with no PR contradicts "Building", so it says
/// nothing and the app shows the lane's own state.
#[test]
fn no_pr_on_a_lane_past_building_says_nothing() {
    for phase in [Phase::Review, Phase::Fixing, Phase::Landing] {
        assert_eq!(derive(None, phase), None, "{phase:?}");
    }
}

#[test]
fn a_draft_in_review_is_agent_review() {
    let s = stage(1, Phase::Review);
    assert_eq!((s.kind, s.label().as_str(), s.pr_number), (Kind::AgentReview, "Agent review", 101));
}

#[test]
fn a_draft_on_a_fixing_lane_is_fixing() {
    let s = stage(2, Phase::Fixing);
    assert_eq!((s.kind, s.label().as_str()), (Kind::Fixing, "Fixing"));
}

#[test]
fn a_draft_on_a_building_lane_is_still_building() {
    assert_eq!(stage(1, Phase::Building).kind, Kind::Building);
}

#[test]
fn a_ready_pr_waits_on_whom_it_asked() {
    let s = stage(3, Phase::Review);
    assert_eq!((s.kind, s.reviewers.as_slice(), s.label().as_str()), (Kind::WaitingOnReviewer, &["alice".to_string()][..], "Waiting on alice"));
    assert_eq!(s.pr_url, "https://github.example/o/r/pull/103");
}

#[test]
fn a_ready_pr_nobody_was_asked_to_review_waits_on_a_reviewer() {
    let s = stage(4, Phase::Review);
    assert_eq!((s.kind, s.label().as_str()), (Kind::WaitingOnReviewer, "Waiting on a reviewer"));
}

#[test]
fn several_reviewers_are_named_to_two_and_counted_after() {
    assert_eq!(stage(12, Phase::Review).label(), "Waiting on alice and myorg/core", "a user and a team, as gh names it");
    assert_eq!(names(&["a".into(), "b".into(), "c".into(), "d".into()]), "a and 3 others");
}

#[test]
fn changes_requested_is_changes_requested() {
    let s = stage(5, Phase::Fixing);
    assert_eq!((s.kind, s.label().as_str()), (Kind::ChangesRequested, "Changes requested"));
}

#[test]
fn approved_with_checks_pending_is_checks_running() {
    let s = stage(6, Phase::Landing);
    assert_eq!((s.kind, s.label().as_str()), (Kind::ApprovedChecksRunning, "Approved \u{b7} checks running"));
}

#[test]
fn approved_with_checks_failing_says_so_and_not_running() {
    let s = stage(7, Phase::Landing);
    assert_eq!((s.kind, s.label().as_str()), (Kind::ApprovedChecksFailing, "Approved \u{b7} checks failing"));
}

#[test]
fn approved_with_checks_passing_is_approved() {
    let s = stage(8, Phase::Landing);
    assert_eq!((s.kind, s.label().as_str()), (Kind::Approved, "Approved"));
}

/// Approved with no checks configured is approved, not "running": an empty
/// rollup is `CheckState::Unknown`, which is not a pending check.
#[test]
fn approved_with_no_checks_at_all_is_approved() {
    let mut p = pr(8);
    p.status.checks = crate::stack::CheckState::Unknown;
    assert_eq!(derive(Some(&p), Phase::Landing).unwrap().kind, Kind::Approved);
}

#[test]
fn a_pr_in_the_merge_queue_says_its_position() {
    let mut p = pr(9);
    p.review.queue_position = Some(2);
    let s = derive(Some(&p), Phase::Landing).unwrap();
    assert_eq!((s.kind, s.queue_position, s.label().as_str()), (Kind::Queued, 2, "Queued, position 2"));
    // Without the queue entry the same PR is approved with green checks.
    assert_eq!(stage(9, Phase::Landing).kind, Kind::Approved);
}

#[test]
fn a_merged_pr_is_merged_whatever_the_lane_says() {
    for phase in [Phase::Review, Phase::Landing, Phase::Landed] {
        assert_eq!(stage(10, phase).label(), "Merged", "{phase:?}");
    }
}

#[test]
fn a_pr_closed_without_merging_is_closed() {
    assert_eq!(stage(11, Phase::Review).label(), "Closed");
}

#[test]
fn threads_and_the_read_time_ride_on_the_stage() {
    let mut p = pr(3);
    p.review.unresolved_threads = Some(2);
    let s = derive(Some(&p), Phase::Review).unwrap();
    assert_eq!(s.unresolved_threads, Some(2));
    assert_eq!(s.read_at, p.status.fetched_at);
    assert_eq!(stage(3, Phase::Review).unresolved_threads, None, "not counted is not zero");
}

#[test]
fn unknown_is_its_own_stage_and_not_building() {
    let s = Stage::unknown();
    assert_eq!((s.kind, s.label().as_str()), (Kind::Unknown, "PR state unknown"));
    assert_ne!(s, derive(None, Phase::Building).unwrap());
}

/// No rule requires a review, so GitHub prints an empty decision; the reviews
/// still say. Changes requested wins over an approval.
#[test]
fn an_empty_decision_is_read_from_the_reviews() {
    assert_eq!(pr(14).status.review_decision, crate::stack::ReviewDecision::Approved);
    assert_eq!(stage(14, Phase::Review).label(), "Approved");
    assert_eq!(pr(15).status.review_decision, crate::stack::ReviewDecision::ChangesRequested);
    assert_eq!(stage(15, Phase::Review).label(), "Changes requested");
    // And no reviews at all stays undecided.
    assert_eq!(pr(4).status.review_decision, crate::stack::ReviewDecision::Unknown);
    // A decision GitHub gave is not second-guessed.
    assert_eq!(pr(5).status.review_decision, crate::stack::ReviewDecision::ChangesRequested);
}

/// Commit statuses carry `state` and neither `status` nor `conclusion`; read as
/// check runs they would be pending forever, and a failing one never failing.
#[test]
fn a_commit_status_counts_by_its_state() {
    assert_eq!(stage(16, Phase::Landing).kind, Kind::Approved, "success beside a green run");
    assert_eq!(stage(17, Phase::Landing).kind, Kind::ApprovedChecksRunning, "pending");
    assert_eq!(stage(18, Phase::Landing).kind, Kind::ApprovedChecksFailing, "failure");
    assert_eq!(stage(18, Phase::Landing).checks, crate::stack::CheckState::Failing);
}

#[test]
fn approved_with_merge_conflicts_says_so() {
    let s = stage(19, Phase::Landing);
    assert_eq!((s.kind, s.label().as_str()), (Kind::ApprovedConflicts, "Approved \u{b7} conflicts"));
}

fn kinds(order: &[(u32, Phase)]) -> Vec<Stage> {
    order.iter().map(|(n, phase)| stage(*n, *phase)).collect()
}

#[test]
fn a_lane_is_as_far_along_as_its_slowest_card() {
    let waiting_and_running = kinds(&[(6, Phase::Review), (3, Phase::Review)]);
    assert_eq!(rollup(&waiting_and_running).unwrap().kind, Kind::WaitingOnReviewer);
    let changes_and_approved = kinds(&[(8, Phase::Review), (5, Phase::Review)]);
    assert_eq!(rollup(&changes_and_approved).unwrap().kind, Kind::ChangesRequested);
    let the_first_of_equals = kinds(&[(3, Phase::Review), (12, Phase::Review)]);
    assert_eq!(rollup(&the_first_of_equals).unwrap().pr_number, 103);
}

#[test]
fn a_card_with_no_pr_yet_holds_its_lane_at_building() {
    let mut stages = kinds(&[(8, Phase::Review)]);
    stages.push(derive(None, Phase::Building).unwrap());
    assert_eq!(rollup(&stages).unwrap().kind, Kind::Building);
}

#[test]
fn a_closed_pr_holds_nothing_back_unless_every_one_was_closed() {
    assert_eq!(rollup(&kinds(&[(11, Phase::Review), (8, Phase::Review)])).unwrap().kind, Kind::Approved);
    assert_eq!(rollup(&kinds(&[(11, Phase::Review)])).unwrap().kind, Kind::Closed);
    assert_eq!(rollup(&[]), None);
}

#[test]
fn a_pr_we_cannot_see_holds_the_lane_below_everything() {
    let stages = vec![stage(8, Phase::Review), Stage::unknown()];
    assert_eq!(rollup(&stages).unwrap().kind, Kind::Unknown);
}

/// A state gh invented after this was written reads unknown, never open.
#[test]
fn a_state_gh_never_named_reads_unknown() {
    let text = r#"[{"number":9,"url":"u","title":"t","state":"SOMETHING_NEW","headRefName":"b","isDraft":false}]"#;
    let prs = parse_prs(text.as_bytes()).unwrap();
    assert_eq!(derive(Some(&prs[0]), Phase::Review).unwrap().kind, Kind::Unknown);
}
