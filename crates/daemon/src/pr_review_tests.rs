//! Reading review requests: what `gh search prs` prints, the cache that holds
//! it, and the rule that a `gh` that cannot answer is not an empty list.

use super::*;

fn fixture() -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/pr-review/search.json");
    std::fs::read(path).expect("the recorded search")
}

fn request(number: u32) -> ReviewRequest {
    ReviewRequest {
        url: format!("https://github.com/acme/billing/pull/{number}"),
        number,
        repository: "acme/billing".into(),
        title: format!("PR {number}"),
        author: "bob".into(),
        created_at: 1,
    }
}

/// The recorded answer holds three PRs, one of them a draft.
#[test]
fn a_search_reads_into_requests_and_leaves_drafts_out() {
    let read = parse(&fixture()).expect("it parses");
    assert_eq!(read.iter().map(|r| r.number).collect::<Vec<_>>(), [213, 7], "the draft is not asked for yet");
    assert_eq!(read[0].repository, "acme/billing");
    assert_eq!(read[0].author, "bob");
    assert_eq!(read[0].title, "Add refund endpoint");
    assert_eq!(read[0].created_at, 1_791_106_200_000, "2026-10-04T09:30:00Z, in millis");
    assert_eq!(read[1].author, "app/dependabot", "a bot's request is a request");
}

/// "No review is wanted" is `[]`. Anything else is a `gh` that could not say,
/// and must not read as that.
#[test]
fn an_empty_list_is_an_answer_and_anything_else_is_not() {
    assert_eq!(parse(b"[]"), Some(Vec::new()));
    for unread in [&b""[..], b"gh: To get started with GitHub CLI, please run: gh auth login", b"{}", b"null", b"[{\"number\":1}]"] {
        assert_eq!(parse(unread), None, "{:?}", String::from_utf8_lossy(unread));
    }
}

/// A URL that is not https never becomes a link a row offers.
#[test]
fn a_request_without_an_https_url_is_dropped() {
    let text = br#"[{"number":1,"url":"javascript:alert(1)","title":"x","createdAt":"2026-10-04T09:30:00Z"},
                    {"number":2,"url":"https://github.com/o/r/pull/2","title":"y","createdAt":""}]"#;
    let read = parse(text).expect("parses");
    assert_eq!(read.iter().map(|r| r.number).collect::<Vec<_>>(), [2]);
    assert_eq!(read[0].created_at, 0, "an unreadable date is no date, not a failure");
}

#[test]
fn the_cache_hands_back_what_it_was_given_until_it_goes_stale() {
    let cache = Cache::default();
    assert!(cache.current(0).is_empty(), "never read is nothing, not a claim");
    assert_eq!(cache.put(1_000, vec![request(1)]), None, "the first read has no before");
    assert_eq!(cache.current(1_000 + STALE_AFTER_MS).len(), 1, "fresh up to the limit");
    assert!(cache.current(1_000 + STALE_AFTER_MS + 1).is_empty(), "past it, a logged-out gh stops being believed");
    let before = cache.put(2_000, vec![request(1), request(2)]).expect("the before is returned");
    assert_eq!((before.requests.len(), before.stale), (1, false));
    assert_eq!(cache.current(2_000).len(), 2);
}

#[test]
fn clearing_forgets_the_read_and_the_next_one_seeds_again() {
    let cache = Cache::default();
    assert!(!cache.clear(), "nothing to forget");
    cache.put(1, vec![request(1)]);
    assert!(cache.clear());
    assert!(cache.current(1).is_empty());
    assert_eq!(cache.put(2, vec![request(1)]), None, "no before, so no notice for a request already waiting");
}

#[test]
fn only_a_url_not_listed_before_is_new() {
    let before = vec![request(1), request(2)];
    let now = vec![ReviewRequest { title: "Edited".into(), ..request(2) }, request(3)];
    assert_eq!(newly_requested(&before, &now).iter().map(|r| r.number).collect::<Vec<_>>(), [3]);
    assert!(newly_requested(&before, &before).is_empty());
    assert!(newly_requested(&before, &[]).is_empty(), "a request that left is not news");
}

// ---- what a read does ----

use crate::watch::Tapped;

async fn a_runner() -> (crate::test_support::ScratchDir, Arc<Service>, Arc<Watcher>) {
    let (dir, svc, _repo) = crate::test_support::fixture().await;
    let watcher = Watcher::new(svc.clone());
    (dir, svc, watcher)
}

/// The next notice, waiting out the count notice's debounce.
async fn next(taps: &mut tokio::sync::mpsc::UnboundedReceiver<Tapped>) -> Option<Tapped> {
    tokio::time::timeout(Duration::from_secs(30), taps.recv()).await.ok().flatten()
}

/// The next review-class notice, past any count notice that comes first.
async fn next_review(taps: &mut tokio::sync::mpsc::UnboundedReceiver<Tapped>) -> Option<Tapped> {
    loop {
        let sent = next(taps).await?;
        if sent.event == Some("review") {
            return Some(sent);
        }
    }
}

/// Nothing review-class arrives for a while: only count notices, which carry
/// no event.
async fn no_review_notice(taps: &mut tokio::sync::mpsc::UnboundedReceiver<Tapped>) {
    let quiet = Duration::from_millis(400);
    while let Ok(Some(sent)) = tokio::time::timeout(quiet, taps.recv()).await {
        assert_eq!(sent.event, None, "a review notice nobody asked for: {sent:?}");
    }
}

/// The first read after a board chose pull requests tells nobody about the
/// requests already waiting; the next one tells of exactly the new one.
#[tokio::test]
async fn a_new_request_is_told_once_and_the_first_read_is_not() {
    let (_dir, svc, watcher) = a_runner().await;
    let mut taps = watcher.tap_notices();
    let mut misses = 0;

    let wait = apply(&svc, &watcher, &mut misses, Some(vec![request(1), request(2)]), 1_000).await;
    assert_eq!(wait, REFRESH);
    assert_eq!(svc.pr_requests.current(1_000).len(), 2, "the cache has them");
    no_review_notice(&mut taps).await;

    apply(&svc, &watcher, &mut misses, Some(vec![request(1), request(2), request(3)]), 2_000).await;
    let sent = next_review(&mut taps).await.expect("the new one is told");
    assert_eq!((sent.kind, sent.level), (Some("task"), Some("active")));
    assert_eq!(sent.subtitle, "A pull request wants your review");

    // The same read again is no news; two new at once is one notice that counts.
    apply(&svc, &watcher, &mut misses, Some(vec![request(1), request(2), request(3)]), 3_000).await;
    no_review_notice(&mut taps).await;
    apply(&svc, &watcher, &mut misses, Some(vec![request(1), request(4), request(5)]), 4_000).await;
    assert_eq!(next_review(&mut taps).await.expect("told").subtitle, "2 pull requests want your review");
}

/// A `gh` that cannot answer changes nothing the clients see, backs off, and
/// is not read as "no review is wanted".
#[tokio::test]
async fn a_gh_that_cannot_answer_keeps_the_last_read_and_backs_off() {
    let (_dir, svc, watcher) = a_runner().await;
    let mut taps = watcher.tap_notices();
    let mut misses = 0;
    apply(&svc, &watcher, &mut misses, Some(vec![request(1)]), 1_000).await;

    let first = apply(&svc, &watcher, &mut misses, None, 2_000).await;
    let second = apply(&svc, &watcher, &mut misses, None, 3_000).await;
    assert_eq!(first, crate::pr_watch::BACKOFF_FIRST);
    assert_eq!(second, crate::pr_watch::BACKOFF_FIRST * 2, "doubling");
    assert_eq!(svc.pr_requests.current(3_000).len(), 1, "the read before the miss still stands");
    no_review_notice(&mut taps).await;

    // An answer ends the backoff.
    assert_eq!(apply(&svc, &watcher, &mut misses, Some(vec![request(1)]), 4_000).await, REFRESH);
    assert_eq!(misses, 0);
}

/// Turning pull requests off drops the rows, and a board that chooses them
/// again seeds afresh rather than telling of every request at once.
#[tokio::test]
async fn a_runner_with_no_pull_request_board_has_no_requests() {
    let (_dir, svc, watcher) = a_runner().await;
    let mut taps = watcher.tap_notices();
    let mut misses = 3;
    apply(&svc, &watcher, &mut misses, Some(vec![request(1)]), 1_000).await;
    assert_eq!(forget(&svc, &watcher, &mut misses), IDLE);
    assert_eq!(misses, 0);
    assert!(svc.pr_requests.current(1_000).is_empty());

    apply(&svc, &watcher, &mut misses, Some(vec![request(1), request(2)]), 2_000).await;
    no_review_notice(&mut taps).await;
    assert_eq!(pull_request_tree(&svc), Tree::None, "the fixture's board has not chosen pull requests");
}

/// Choosing the mode is what turns the read on.
#[tokio::test]
async fn a_board_that_chose_pull_requests_gives_a_checkout_to_read_in() {
    let (_dir, svc, _watcher) = a_runner().await;
    let workspace = svc.store.list_workspaces(None).unwrap().remove(0);
    let patch = farcooler_store::landing::LandingPatch {
        mode: Some(LandingMode::Direct),
        ..Default::default()
    };
    let version = svc.store.get_workspace(workspace.id).unwrap().resource_version;
    svc.store.set_landing(workspace.id, version, &patch).unwrap();
    assert_eq!(pull_request_tree(&svc), Tree::None, "direct landing reads no review requests");

    let patch = farcooler_store::landing::LandingPatch { mode: Some(LandingMode::PullRequests), ..Default::default() };
    let version = svc.store.get_workspace(workspace.id).unwrap().resource_version;
    svc.store.set_landing(workspace.id, version, &patch).unwrap();
    assert!(matches!(pull_request_tree(&svc), Tree::Some(_)));
}

/// An outage that outlasts the staleness limit takes the rows off the clients'
/// lists without an event; the recovery that lists the same pull requests puts
/// them back, and must say so.
#[tokio::test]
async fn a_read_after_an_outage_moves_the_lists_though_it_lists_the_same_requests() {
    let (_dir, svc, watcher) = a_runner().await;
    let mut events = watcher.subscribe();
    let mut misses = 0;
    apply(&svc, &watcher, &mut misses, Some(vec![request(1)]), 1_000).await;
    let before = svc.pr_requests.put(1_000 + STALE_AFTER_MS + 1, vec![request(1)]).expect("a before");
    assert!(before.stale, "the previous read was past the limit");
    assert!(!svc.pr_requests.put(1_000 + STALE_AFTER_MS + 2, vec![request(1)]).expect("a before").stale);

    // Through `apply`, and drained of what the first read announced.
    heard_needs_you(&mut events).await;
    apply(&svc, &watcher, &mut misses, Some(vec![request(1)]), 5_000 + 3 * STALE_AFTER_MS).await;
    assert!(heard_needs_you(&mut events).await, "the same list after a stale one is news");
    // And a fresh one the same is not.
    apply(&svc, &watcher, &mut misses, Some(vec![request(1)]), 6_000 + 3 * STALE_AFTER_MS).await;
    assert!(!heard_needs_you(&mut events).await, "the same list straight after is not");
}

/// A needs_you_changed arrives within a moment, or none does.
async fn heard_needs_you(events: &mut tokio::sync::broadcast::Receiver<farcooler_protocol::v1::Event>) -> bool {
    use farcooler_protocol::v1::event::Payload;
    let wait = async {
        loop {
            match events.recv().await {
                Ok(e) if matches!(e.payload, Some(Payload::NeedsYouChanged(_))) => return true,
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return false,
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(3), wait).await.unwrap_or(false)
}

/// The rows leave the lists on their own when the read goes stale, so the
/// moment it does is announced, once, and the wait never runs past it.
#[tokio::test]
async fn a_read_that_goes_stale_announces_the_drop_once() {
    let (_dir, svc, watcher) = a_runner().await;
    let mut events = watcher.subscribe();
    let mut misses = 0;
    apply(&svc, &watcher, &mut misses, Some(vec![request(1)]), 1_000).await;
    heard_needs_you(&mut events).await;

    // Fresh: a miss announces nothing, and the next look is no later than the drop.
    let wait = apply(&svc, &watcher, &mut misses, None, 2_000).await;
    assert!(!heard_needs_you(&mut events).await);
    assert!(wait <= Duration::from_millis((STALE_AFTER_MS - 1_000 + 1) as u64), "{wait:?}");

    // Past the limit: told once.
    let past = 1_000 + STALE_AFTER_MS + 1;
    apply(&svc, &watcher, &mut misses, None, past).await;
    assert!(heard_needs_you(&mut events).await, "the drop is announced");
    assert!(svc.pr_requests.current(past).is_empty());
    apply(&svc, &watcher, &mut misses, None, past + 1_000).await;
    assert!(!heard_needs_you(&mut events).await, "and only once");
}

#[test]
fn an_empty_read_has_nothing_to_go_stale() {
    let cache = Cache::default();
    cache.put(0, Vec::new());
    assert!(!cache.went_stale(STALE_AFTER_MS + 1));
    assert_eq!(cache.stale_in(0), None);
}

/// A store that cannot be read says nothing about the boards: the rows stay.
#[tokio::test]
async fn a_store_that_cannot_be_read_changes_nothing() {
    let (_dir, svc, watcher) = a_runner().await;
    let mut misses = 0;
    apply(&svc, &watcher, &mut misses, Some(vec![request(1)]), now_ms()).await;
    misses = 2;
    assert_eq!(without_checkout(&svc, &watcher, &mut misses, &Tree::Unknown), IDLE);
    assert_eq!((svc.pr_requests.current(now_ms()).len(), misses), (1, 2), "rows and misses as they were");
    // Whereas a store that was read and holds no such board drops the rows.
    assert_eq!(without_checkout(&svc, &watcher, &mut misses, &Tree::None), IDLE);
    assert_eq!((svc.pr_requests.current(now_ms()).len(), misses), (0, 0));
}

/// The same drop is announced on a turn that could not read the store.
#[tokio::test]
async fn rows_that_age_out_during_an_unreadable_store_are_announced() {
    let (_dir, svc, watcher) = a_runner().await;
    let mut events = watcher.subscribe();
    let mut misses = 0;
    // A read from long enough ago that it is stale now.
    svc.pr_requests.put(now_ms() - STALE_AFTER_MS - 1_000, vec![request(1)]);
    heard_needs_you(&mut events).await;
    assert_eq!(without_checkout(&svc, &watcher, &mut misses, &Tree::Unknown), IDLE);
    assert!(heard_needs_you(&mut events).await, "the drop is announced");
    without_checkout(&svc, &watcher, &mut misses, &Tree::Unknown);
    assert!(!heard_needs_you(&mut events).await, "once");
}

/// Setting the mode over and over reads once in a floor.
#[test]
fn a_kick_is_floored_like_the_pr_watchs() {
    let mut floor = crate::pr_watch::KickFloor::new(Duration::from_secs(15));
    let now = Instant::now();
    assert!(floor.allows(uuid::Uuid::nil(), now));
    assert!(!floor.allows(uuid::Uuid::nil(), now + Duration::from_secs(1)));
    assert!(floor.allows(uuid::Uuid::nil(), now + Duration::from_secs(16)));
}
