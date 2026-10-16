//! What a PR's timing says, read from what GitHub's GraphQL prints
//! (`pr_timing_read`), and a reviewer's median wait: none, one and many
//! samples; unknown against none; the ready moment from the timeline.

use super::*;
use crate::pr_timing_read::{parse, query};

const T0: i64 = 1_790_000_000_000;
const MIN: i64 = 60_000;

pub(crate) fn iso(ms: i64) -> String {
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

/// One PR as the query's answer spells it. `ready` is the timeline event's
/// minute; `reviews` are `(login, state, minute)`, `None` leaving the field out.
fn answer(opened: i64, draft: bool, ready: Option<i64>, reviews: Option<&[(&str, &str, i64)]>) -> serde_json::Value {
    let mut pr = serde_json::json!({
        "createdAt": iso(T0 + opened * MIN), "isDraft": draft, "additions": 12, "deletions": 0,
        "author": {"login": "agent"},
        "timelineItems": {"nodes": ready.iter().map(|m| serde_json::json!({"createdAt": iso(T0 + m * MIN)})).collect::<Vec<_>>()},
    });
    if let Some(reviews) = reviews {
        pr["reviews"] = serde_json::json!({"nodes": reviews.iter().map(|(who, state, at)|
            serde_json::json!({"author": {"login": who}, "state": state, "submittedAt": iso(T0 + at * MIN)})).collect::<Vec<_>>()});
    }
    pr
}

fn timing(pr: serde_json::Value) -> PrTiming {
    let body = serde_json::json!({"data": {"repository": {"p7": pr}}}).to_string();
    parse(body.as_bytes()).expect("parses").remove(&7).expect("PR 7")
}

#[test]
fn ready_is_the_timeline_event_and_every_time_is_githubs() {
    let t = timing(answer(0, false, Some(30), Some(&[("alice", "APPROVED", 50)])));
    assert_eq!((t.opened, t.ready), (Moment::At(T0), Moment::At(T0 + 30 * MIN)));
    assert_eq!(t.first_by, vec![FirstReview { reviewer: "alice".into(), at: T0 + 50 * MIN }]);
}

#[test]
fn a_pr_never_a_draft_was_ready_when_opened() {
    assert_eq!(timing(answer(5, false, None, Some(&[]))).ready, Moment::At(T0 + 5 * MIN));
}

#[test]
fn a_draft_is_not_ready_yet_even_after_an_earlier_ready_event() {
    assert_eq!(timing(answer(0, true, None, Some(&[]))).ready, Moment::NotYet);
    assert_eq!(timing(answer(0, true, Some(30), Some(&[]))).ready, Moment::NotYet, "back in draft");
}

#[test]
fn the_moments_are_read_from_the_reviews() {
    let t = timing(answer(
        0,
        false,
        None,
        Some(&[("alice", "COMMENTED", 40), ("bob", "CHANGES_REQUESTED", 50), ("alice", "CHANGES_REQUESTED", 60), ("carol", "DISMISSED", 70), ("alice", "APPROVED", 90)]),
    ));
    assert_eq!((t.first_review, t.approval), (Moment::At(T0 + 40 * MIN), Moment::At(T0 + 90 * MIN)));
    assert_eq!(t.changes_requested, Some(2), "a dismissed review is not one that still asks");
    assert_eq!((t.additions, t.deletions), (Some(12), Some(0)), "zero deletions is a size, not a missing one");
    assert_eq!(t.first_by.len(), 3, "each reviewer's first review only");
}

#[test]
fn nobody_having_reviewed_is_none_and_no_review_list_is_unknown() {
    let none = timing(answer(0, false, None, Some(&[])));
    let unknown = timing(answer(0, false, None, None));
    assert_eq!((none.first_review, none.approval, none.changes_requested), (Moment::NotYet, Moment::NotYet, Some(0)));
    assert_eq!((unknown.first_review, unknown.approval, unknown.changes_requested), (Moment::Unknown, Moment::Unknown, None));
    assert_ne!(none, unknown);
}

#[test]
fn the_authors_own_and_pending_reviews_count_for_nothing() {
    let t = timing(answer(0, false, None, Some(&[("agent", "COMMENTED", 5), ("zed", "PENDING", 6)])));
    assert_eq!((t.first_review, t.changes_requested, t.first_by.len()), (Moment::NotYet, Some(0), 0));
}

#[test]
fn a_review_made_while_a_draft_is_skipped_and_the_reviewers_next_one_counts() {
    let t = timing(answer(0, false, Some(60), Some(&[("alice", "COMMENTED", 10), ("alice", "COMMENTED", 90), ("bob", "COMMENTED", 20)])));
    assert_eq!(t.first_review, Moment::At(T0 + 10 * MIN), "the PR's first review is still the first");
    assert_eq!(t.first_by, vec![FirstReview { reviewer: "alice".into(), at: T0 + 90 * MIN }], "bob only reviewed the draft");
}

#[test]
fn the_query_names_each_pr_by_alias_and_asks_only_what_it_keeps() {
    let q = query(&[7, 9]);
    assert!(q.contains("p7:pullRequest(number:7)") && q.contains("p9:pullRequest(number:9)"));
    assert!(q.contains("READY_FOR_REVIEW_EVENT") && !q.contains("body") && !q.contains("comments"));
}

#[test]
fn the_median_of_none_one_and_many() {
    assert_eq!(median(&[]), None, "no history is not zero");
    assert_eq!(median(&[7 * MIN]), Some(7 * MIN));
    assert_eq!(median(&[90 * MIN, 10 * MIN, 50 * MIN]), Some(50 * MIN), "the middle, whatever the order");
    assert_eq!(median(&[10 * MIN, 20 * MIN, 30 * MIN, 100 * MIN]), Some(25 * MIN), "the mean of the middle two");
}

/// A PR ready at T0 that `who` first reviewed `wait` minutes later.
fn reviewed(number: u32, who: &str, wait: i64) -> crate::stack::PrInfo {
    let body = serde_json::json!({"data": {"repository": {format!("p{number}"): answer(-60, false, Some(0), Some(&[(who, "COMMENTED", wait)]))}}});
    let t = parse(body.to_string().as_bytes()).unwrap().remove(&number).unwrap();
    let list = format!(
        r#"[{{"number":{number},"url":"https://github.example/o/r/pull/{number}","title":"t","state":"OPEN","headRefName":"b","isDraft":false}}]"#
    );
    let mut pr = crate::stack::parse_prs(list.as_bytes()).unwrap().remove(0);
    pr.timing = t;
    pr
}

#[test]
fn a_reviewers_wait_is_the_median_of_their_latest_ten() {
    // Ten reviews of 30 to 39 minutes, and an eleventh, older and far slower.
    let mut history: Vec<_> = (0..10).map(|i| reviewed(i + 1, "alice", 30 + i64::from(i))).collect();
    history.push(reviewed(20, "alice", 5_000));
    history[10].timing.first_by[0].at = T0 - 1;
    let waits = waits_of(&history, "alice");
    assert_eq!(waits.len(), LATEST);
    assert_eq!(median(&waits), Some(34 * MIN + MIN / 2), "30 to 39 minutes; the slow, older one fell outside the ten");
}

#[test]
fn an_undated_ready_is_no_sample() {
    let mut p = reviewed(1, "alice", 10);
    p.timing.ready = Moment::Unknown;
    assert_eq!(waits_of(&[p], "alice"), Vec::<i64>::new());
}

#[test]
fn the_waits_count_open_prs_per_reviewer_and_say_no_history_yet() {
    let ask = |mut p: crate::stack::PrInfo, who: &[&str]| {
        p.review.requested = who.iter().map(|w| w.to_string()).collect();
        p
    };
    let waiting = [
        ask(reviewed(1, "x", 1), &["alice", "bob", "myorg/core"]),
        ask(reviewed(2, "x", 1), &["alice"]),
        {
            let mut p = ask(reviewed(3, "x", 1), &["carol"]);
            p.status.state = crate::stack::PrState::Draft;
            p
        },
    ];
    let refs: Vec<&_> = waiting.iter().collect();
    let w = waits(&refs, &[reviewed(10, "Alice", 50)]);
    let by = |name: &str| w.iter().find(|w| w.reviewer == name).cloned();
    assert_eq!(by("alice").map(|w| (w.prs, w.median_ms, w.samples)), Some((2, Some(50 * MIN), 1)), "the login's case does not matter");
    assert_eq!(by("bob").map(|w| (w.prs, w.median_ms, w.samples)), Some((1, None, 0)));
    assert_eq!(by("myorg/core").map(|w| (w.prs, w.median_ms)), Some((1, None)), "a team slug has no history of its own");
    assert!(by("carol").is_none(), "a draft waits on no one yet");
    assert_eq!(w[0].reviewer, "alice", "most waiting first");
}

/// A refresh of the PR list carries the timing a PR had, and a read that failed changes nothing.
#[test]
fn a_list_refresh_keeps_the_timing_it_had() {
    let had = vec![reviewed(1, "alice", 30)];
    let fresh = vec![{
        let mut p = had[0].clone();
        p.timing = PrTiming::default();
        p
    }];
    let merged = crate::stack::merge_read(Some(had.clone()), Some(fresh)).unwrap();
    assert_eq!(merged[0].timing, had[0].timing, "the list has no timing of its own");
    assert_eq!(crate::stack::merge_read(Some(had.clone()), None).unwrap()[0].timing, had[0].timing, "and a read that failed keeps all of it");
}
