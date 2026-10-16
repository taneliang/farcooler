//! When a pull request is nudged, and in what words (ov-580): every rule of
//! `due` on its own, the quiet hours, who may be addressed, and the owner who
//! is the author.

use super::*;
use crate::pr_timing::PrTiming;
use crate::stack::{CheckState, PrReview, PrStatus};

const HOUR: i64 = 3_600_000;
const READY: i64 = 1_000 * HOUR;
const NOON: u32 = 12;
/// A timing read made two hours after ready, and good for ten minutes.
const READ: i64 = READY + 2 * HOUR;
const FRESH: i64 = 10 * 60_000;

/// A PR the owner opened, ready at `READY`, asking `asks` (all users), whose
/// timing was read at `READ`.
fn pr(asks: &[&str]) -> PrInfo {
    PrInfo {
        head_ref: "ov-1/work".into(),
        base_ref: "main".into(),
        title: "ov-1: work".into(),
        is_fork: false,
        author: "owner".into(),
        review: PrReview {
            requested: asks.iter().map(|a| a.to_string()).collect(),
            requested_users: asks.iter().map(|a| a.to_string()).collect(),
            ..Default::default()
        },
        timing: PrTiming { ready: Moment::At(READY), first_review: Moment::NotYet, read_at: READ, ..Default::default() },
        status: PrStatus {
            number: 7,
            url: "https://github.example/o/r/pull/7".into(),
            state: PrState::Open,
            checks: CheckState::Unknown,
            review_decision: ReviewDecision::Unknown,
            head_oid: "abc".into(),
            merged_at: None,
            fetched_at: 0,
        },
    }
}

/// The moment `age` after ready, read fresh, at `hour`, posting as `owner`.
fn at(age: i64, hour: u32, after_hours: u32) -> When<'static> {
    When { owner: "owner", now_ms: READY + age, hour, after_hours, timing_fresh_ms: FRESH * 1_000_000 }
}

fn names(v: Option<Vec<String>>) -> Vec<String> {
    v.unwrap_or_default()
}

#[test]
fn a_ready_unreviewed_pull_request_that_asks_alice_is_nudged_after_the_wait() {
    let p = pr(&["alice"]);
    assert_eq!(due(&p, &at(24 * HOUR, NOON, 24)), Some(vec!["alice".to_string()]), "exactly at the wait");
    assert_eq!(due(&p, &at(24 * HOUR - 1, NOON, 24)), None, "a millisecond early");
    assert_eq!(due(&p, &at(24 * HOUR, NOON, 48)), None, "the wait is the workspace's to set");
    assert_eq!(due(&p, &at(48 * HOUR, NOON, 48)).map(|v| v.len()), Some(1));
}

#[test]
fn nobody_is_nudged_between_ten_at_night_and_eight_in_the_morning() {
    let p = pr(&["alice"]);
    for hour in [22, 23, 0, 1, 7] {
        assert!(quiet(hour), "{hour}");
        assert_eq!(due(&p, &at(30 * HOUR, hour, 24)), None, "{hour}:00");
    }
    for hour in [8, 9, 12, 21] {
        assert!(!quiet(hour), "{hour}");
        assert!(due(&p, &at(30 * HOUR, hour, 24)).is_some(), "{hour}:00");
    }
}

#[test]
fn only_a_pull_request_nobody_has_started_on_is_nudged() {
    let late = at(30 * HOUR, NOON, 24);
    let mut reviewed = pr(&["alice"]);
    reviewed.timing.first_review = Moment::At(READY + HOUR);
    let mut unread = pr(&["alice"]);
    unread.timing.first_review = Moment::Unknown;
    let mut not_ready = pr(&["alice"]);
    not_ready.timing.ready = Moment::NotYet;
    let mut ready_unknown = pr(&["alice"]);
    ready_unknown.timing.ready = Moment::Unknown;
    let mut draft = pr(&["alice"]);
    draft.status.state = PrState::Draft;
    let mut merged = pr(&["alice"]);
    merged.status.state = PrState::Merged;
    let mut closed = pr(&["alice"]);
    closed.status.state = PrState::Closed;
    let mut fork = pr(&["alice"]);
    fork.is_fork = true;
    for (why, p) in [
        ("reviewed", reviewed),
        ("reviews not read: unknown is not none", unread),
        ("a draft is not ready", draft),
        ("not ready yet", not_ready),
        ("ready time unknown", ready_unknown),
        ("merged", merged),
        ("closed", closed),
        ("a fork's", fork),
    ] {
        assert_eq!(due(&p, &late), None, "{why}");
    }
    assert_eq!(due(&pr(&[]), &late), None, "it asks nobody");
}

#[test]
fn the_fresh_pr_list_overrules_a_timing_read_that_says_nobody_reviewed() {
    // The timing read is two hours after ready and says NotYet; the list, just now, disagrees.
    let when = at(30 * HOUR, NOON, 24);
    let mut commented = pr(&["alice"]);
    commented.review.others_reviewed = true;
    assert_eq!(due(&commented, &when), None, "someone but the author has a review standing");
    for decision in [ReviewDecision::Approved, ReviewDecision::ChangesRequested] {
        let mut decided = pr(&["alice"]);
        decided.status.review_decision = decision;
        assert_eq!(due(&decided, &when), None, "{decision:?}");
    }
    let mut required = pr(&["alice"]);
    required.status.review_decision = ReviewDecision::ReviewRequired;
    assert!(due(&required, &when).is_some(), "required and not yet given is the very case");
}

#[test]
fn a_timing_read_older_than_two_intervals_is_not_taken_for_nobody_reviewing() {
    let mut now = at(30 * HOUR, NOON, 24);
    now.timing_fresh_ms = 10 * 60_000;
    let mut p = pr(&["alice"]);
    // Read just now: fine. Read twenty minutes ago, with the same NotYet in it: not.
    p.timing.read_at = now.now_ms - 60_000;
    assert!(due(&p, &now).is_some());
    p.timing.read_at = now.now_ms - 20 * 60_000;
    assert_eq!(due(&p, &now), None, "stale");
    p.timing.read_at = 0;
    assert_eq!(due(&p, &now), None, "never read");
}

#[test]
fn only_the_owners_own_pull_request_is_nudged_in_the_owners_name() {
    let when = at(30 * HOUR, NOON, 24);
    let mut teammates = pr(&["alice"]);
    teammates.author = "dave".into();
    assert_eq!(due(&teammates, &when), None, "a teammate's PR that a card key matched");
    let mut unknown = pr(&["alice"]);
    unknown.author = String::new();
    assert_eq!(due(&unknown, &when), None, "an author GitHub didn't name");
    let nobody = When { owner: "", ..when };
    assert_eq!(due(&pr(&["alice"]), &nobody), None, "gh couldn't say who it posts as");
    let mut shouting = pr(&["alice"]);
    shouting.author = "OWNER".into();
    assert!(due(&shouting, &when).is_some(), "logins have no case");
}

#[test]
fn a_team_a_bot_and_a_login_that_is_not_one_are_never_addressed() {
    let when = at(30 * HOUR, NOON, 24);
    let mut mixed = pr(&["alice"]);
    // Asked: a team with a bare slug, Copilot, and alice, but only alice is typed a user.
    mixed.review.requested = vec!["core".into(), "Copilot".into(), "alice".into()];
    assert_eq!(names(due(&mixed, &when)), ["alice"]);
    let mut only_others = pr(&[]);
    only_others.review.requested = vec!["core".into(), "Copilot".into()];
    assert_eq!(due(&only_others, &when), None);
    for bad in ["al ice", "@alice", "alice\nclosing keyword", "$(rm -rf)", "", "-flag", &"a".repeat(40), "acme/core"] {
        assert_eq!(due(&pr(&[bad]), &when), None, "{bad:?}");
    }
    assert_eq!(names(due(&pr(&["alice", "ALICE", "bob"]), &when)), ["alice", "bob"], "once each");
}

#[test]
fn gh_types_each_request_and_only_a_user_is_a_person() {
    let list = r#"[{"number":7,"url":"https://github.example/o/r/pull/7","title":"t","state":"OPEN","headRefName":"b","isDraft":false,
        "author":{"login":"owner"},
        "reviewRequests":[
          {"__typename":"Team","slug":"core","name":"Core"},
          {"__typename":"Bot","login":"copilot-pull-request-reviewer"},
          {"__typename":"User","login":"alice"},
          {"login":"untyped"}],
        "latestReviews":[{"author":{"login":"owner"},"state":"COMMENTED"}]}]"#;
    let p = crate::stack::parse_prs(list.as_bytes()).unwrap().remove(0);
    assert_eq!(p.review.requested_users, ["alice"], "a bare team slug, a bot and an untyped request are not people");
    assert_eq!(p.review.requested.len(), 4, "they are still asked, as the plan reads them");
    assert!(!p.review.others_reviewed, "the author's own review is not someone else's");
    let other = list.replace(r#""author":{"login":"owner"},"state""#, r#""author":{"login":"Hal"},"state""#);
    assert!(crate::stack::parse_prs(other.as_bytes()).unwrap().remove(0).review.others_reviewed);
}

#[test]
fn the_owner_who_is_the_author_is_never_nudged_to_review_their_own_pull_request() {
    let when = at(30 * HOUR, NOON, 24);
    assert_eq!(due(&pr(&["owner"]), &when), None);
    assert_eq!(due(&pr(&["Owner"]), &when), None, "whatever the case");
    assert_eq!(names(due(&pr(&["owner", "alice"]), &when)), ["alice"]);
}

#[test]
fn the_words_are_plain_and_polite_for_one_two_or_three() {
    let n = |v: &[&str]| message(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    assert_eq!(n(&["alice"]), "@alice, a gentle nudge: this is ready for your review whenever you have a moment.");
    assert_eq!(n(&["alice", "bob"]), "@alice and @bob, a gentle nudge: this is ready for your review whenever you have a moment.");
    assert_eq!(
        n(&["alice", "bob", "carol"]),
        "@alice, @bob and @carol, a gentle nudge: this is ready for your review whenever you have a moment."
    );
}

#[test]
fn a_pull_request_is_its_repository_on_github_and_a_number() {
    assert_eq!(repo_key("https://github.com/Acme/Overnight/pull/31").as_deref(), Some("github.com/acme/overnight"));
    assert_eq!(repo_key("https://ghe.example/o/r/pull/7?x=1").as_deref(), Some("ghe.example/o/r"));
    for bad in ["", "http://github.com/o/r/pull/7", "https://github.com/o/pull/7", "https://github.com/o/r/issues/7", "https://github.com/o/r/pull/x", "https://github.com/o/r /pull/7", "-c"] {
        assert_eq!(repo_key(bad), None, "{bad:?}");
    }
}

#[test]
fn who_opened_a_pull_request_is_read_from_gh_and_empty_when_it_says_nothing() {
    let list = |author: &str| {
        format!(
            r#"[{{"number":7,"url":"https://github.example/o/r/pull/7","title":"t","state":"OPEN","headRefName":"b","isDraft":false{author}}}]"#
        )
    };
    let with = crate::stack::parse_prs(list(r#","author":{"login":"owner"}"#).as_bytes()).unwrap().remove(0);
    assert_eq!(with.author, "owner");
    let without = crate::stack::parse_prs(list("").as_bytes()).unwrap().remove(0);
    assert_eq!(without.author, "", "unknown, not guessed");
}
