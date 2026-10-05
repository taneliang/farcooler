//! The watch's pace, and the two numbers it reads that `gh pr list` has no
//! field for. What it does with them, end to end, is
//! `tests/a_lane_shows_its_pr_stage.rs`.

use super::*;

/// A minute while any lane is in review, fixing or landing, and the
/// look-for-one pace otherwise.
#[test]
fn a_lane_in_review_reads_every_minute_and_none_reads_at_the_idle_pace() {
    assert_eq!(cadence(true), Duration::from_secs(60));
    assert_eq!(cadence(false), IDLE);
    assert!(IDLE > ACTIVE, "idle is the slower pace");
}

/// Which lane states count as waiting on a pull request.
#[test]
fn only_review_fixing_and_landing_are_active() {
    use crate::pr_stage::Phase;
    for (phase, active) in [
        (Phase::Queued, false),
        (Phase::Building, false),
        (Phase::Review, true),
        (Phase::Fixing, true),
        (Phase::Landing, true),
        (Phase::Landed, false),
        (Phase::Dropped, false),
    ] {
        assert_eq!(phase.is_active(), active, "{phase:?}");
    }
}

/// What gh printed for the daemon's query on a PR in the queue and one with
/// two open threads (`test/fixtures/pr-stage/extras-*.json`).
#[test]
fn the_thread_count_and_the_queue_place_come_from_one_answer() {
    let fixture = |name: &str| {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/pr-stage").join(name);
        std::fs::read(path).unwrap()
    };
    assert_eq!(
        parse_extras(&fixture("extras-103.json")),
        Some(Extras { unresolved_threads: 2, queue_position: None })
    );
    assert_eq!(
        parse_extras(&fixture("extras-109.json")),
        Some(Extras { unresolved_threads: 0, queue_position: Some(2) })
    );
}

/// What isn't that JSON is not zero threads.
#[test]
fn what_gh_says_on_failure_is_not_zero_threads() {
    assert_eq!(parse_extras(b"gh: could not resolve to a PullRequest"), None);
    assert_eq!(parse_extras(b""), None);
    assert_eq!(parse_extras(b"{}"), None);
}

/// A repository whose gh is logged out or rate limited waits five minutes,
/// doubling to thirty, and a read that answers clears it.
#[test]
fn a_gh_that_fails_is_left_alone_for_five_minutes_doubling_to_thirty() {
    let mins = |n: u32| backoff_after(n).as_secs() / 60;
    assert_eq!([mins(1), mins(2), mins(3), mins(4), mins(5), mins(40)], [5, 10, 20, 30, 30, 30]);

    let (repo, other, t0) = (Uuid::from_u128(1), Uuid::from_u128(2), Instant::now());
    let mut memo = Memo::default();
    assert!(memo.due(repo, t0, false), "never missed");
    memo.missed(repo, t0);
    assert!(!memo.due(repo, t0 + Duration::from_secs(60), false), "a minute on, still backing off");
    assert!(!memo.due(repo, t0 + Duration::from_secs(299), false));
    assert!(memo.due(repo, t0 + Duration::from_secs(300), false), "five minutes on");
    assert!(memo.due(other, t0, false), "another repository is its own");
    assert!(memo.due(repo, t0 + Duration::from_secs(60), true), "a lane entering review tries once");
    memo.missed(repo, t0 + Duration::from_secs(300));
    assert!(!memo.due(repo, t0 + Duration::from_secs(300 + 599), false), "the second miss waits ten");
    memo.answered(repo);
    assert!(memo.due(repo, t0 + Duration::from_secs(301), false), "an answer clears it");
}

/// A lane's kicks are at least 15 s apart; another lane's are its own.
#[test]
fn a_lane_may_kick_the_watch_once_in_fifteen_seconds() {
    let (a, b, t0) = (Uuid::from_u128(1), Uuid::from_u128(2), Instant::now());
    let mut floor = KickFloor::default();
    assert!(floor.allows(a, t0), "the first");
    assert!(!floor.allows(a, t0 + Duration::from_secs(5)), "five seconds on");
    assert!(!floor.allows(a, t0 + Duration::from_secs(14)));
    assert!(floor.allows(b, t0 + Duration::from_secs(5)), "another lane");
    assert!(floor.allows(a, t0 + Duration::from_secs(15)), "fifteen seconds on");
    assert!(!floor.allows(a, t0 + Duration::from_secs(20)), "the clock restarts at a kick that counted");
}

fn read_with_extras(number: u32, threads: Option<u32>, queue: Option<u32>) -> crate::stack::PrInfo {
    let mut pr = crate::pr_stage::fixture_prs().into_iter().find(|p| p.status.number == 100 + number).unwrap();
    pr.review.unresolved_threads = threads;
    pr.review.queue_position = queue;
    pr
}

/// A read of GitHub that failed keeps what was known: a blip, or a manual
/// refresh with no network, never turns every stage unknown.
#[test]
fn a_failed_read_keeps_the_last_answer() {
    let known = Some(vec![read_with_extras(3, Some(2), None)]);
    assert_eq!(crate::stack::merge_read(known.clone(), None), known);
    assert_eq!(crate::stack::merge_read(None, None), None, "and with nothing known there is nothing to keep");
}

/// A read that answered, but counted no threads (`gh pr list` has none, so a
/// refresh and the background fill read none), keeps the watch's counts and
/// queue place for an open PR. One that counted its own replaces them.
#[test]
fn a_read_that_counted_nothing_keeps_the_threads_and_the_queue_place() {
    let before = Some(vec![read_with_extras(3, Some(2), None), read_with_extras(9, Some(0), Some(2))]);
    let refreshed = Some(vec![read_with_extras(3, None, None), read_with_extras(9, None, None)]);
    let merged = crate::stack::merge_read(before.clone(), refreshed).unwrap();
    assert_eq!(merged[0].review.unresolved_threads, Some(2));
    assert_eq!((merged[1].review.unresolved_threads, merged[1].review.queue_position), (Some(0), Some(2)));

    let counted = Some(vec![read_with_extras(3, Some(0), None), read_with_extras(9, Some(0), None)]);
    let merged = crate::stack::merge_read(before.clone(), counted).unwrap();
    assert_eq!(merged[0].review.unresolved_threads, Some(0), "a new count wins");
    assert_eq!(merged[1].review.queue_position, None, "and a PR that left the queue has left it");

    // A merged PR's threads are history, and are not carried onto it.
    let mut done = read_with_extras(10, None, None);
    done.status.state = crate::stack::PrState::Merged;
    let before = Some(vec![read_with_extras(10, Some(3), None)]);
    assert_eq!(crate::stack::merge_read(before, Some(vec![done])).unwrap()[0].review.unresolved_threads, None);
}

/// The cache itself keeps them, through the paths the watch does not own.
#[tokio::test]
async fn the_cache_keeps_the_last_answer_and_the_counts_through_a_refresh() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    svc.pr_cache_put(repo, Some(vec![read_with_extras(3, Some(2), None)]));

    svc.pr_cache_put(repo, Some(vec![read_with_extras(3, None, None)]));
    assert_eq!(svc.pr_cache_get(repo).unwrap()[0].review.unresolved_threads, Some(2), "a refresh");

    svc.pr_cache_put(repo, None);
    assert!(svc.pr_answer_is_known(repo), "a refresh gh could not answer");
    assert_eq!(svc.pr_cache_get(repo).unwrap()[0].review.unresolved_threads, Some(2));
}
