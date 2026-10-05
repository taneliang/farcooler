//! A lane shows where its pull requests stand (ov-312), end to end: a real
//! `farcoolerd` whose `gh` prints, in the shape gh 2.101 prints, one pull
//! request for each row of the stage table (`test/fixtures/pr-stage/`), and the
//! stage `plan.get` carries for each card and each lane.
//!
//! The things only a running daemon shows: that the PR watch reads without
//! anyone asking once a lane is in review, that it asks about threads only for
//! the open pull requests of live lanes, that a `gh` that cannot answer reads
//! "unknown" and not "no pull request", that a read that fails keeps what was
//! known, and that it reads nothing while no lane is waiting on one.

use std::time::Duration;

use farcooler_protocol::v1::{self as pb, LaneState as L};

mod common;
#[path = "support/gh_pr.rs"]
mod gh_pr;
use gh_pr::*;

fn stage_of<'a>(plan: &'a pb::Plan, key: &str) -> Option<&'a pb::PrStage> {
    let id = &plan.cards.iter().find(|c| c.key == key)?.task_id;
    plan.lanes.iter().flat_map(|l| &l.cards).find(|c| c.task_id == *id)?.stage.as_ref()
}

fn lane_stage<'a>(plan: &'a pb::Plan, name: &str) -> Option<&'a pb::PrStage> {
    plan.lanes.iter().find(|l| l.name == name)?.stage.as_ref()
}

fn words(s: Option<&pb::PrStage>) -> String {
    s.map_or_else(|| "none".to_string(), |s| s.label.clone())
}

/// `n` cards, and the runner serving every one's PRs.
async fn cards_and_prs(client: &mut SocketClient, workspace: &pb::Workspace, dir: &std::path::Path, n: usize) -> Vec<pb::Task> {
    let mut cards = Vec::new();
    for i in 1..=n {
        cards.push(a_card(client, workspace, &format!("Card {i}")).await);
    }
    let keys: Vec<String> = cards.iter().map(|c| c.key.clone()).collect();
    serve(dir, &keys);
    cards
}

/// Wait for the watch to ask GitHub for the PR list once more than `before`.
async fn a_read_after(dir: &std::path::Path, before: usize) {
    let deadline = std::time::Instant::now() + BUDGET;
    while lists(dir) <= before {
        assert!(std::time::Instant::now() < deadline, "the watch never read again");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_millis(800)).await;
}

/// A board with cards 3 (two threads open) and 9 (queued at 2) in review, read.
async fn a_board_at_rest() -> (tempfile::TempDir, common::DaemonChild, SocketClient, pb::Workspace, Vec<pb::Task>) {
    let (dir, daemon, mut client, workspace) = a_runner().await;
    let cards = cards_and_prs(&mut client, &workspace, dir.path(), 13).await;
    a_lane(&mut client, &workspace, "review", &[&cards[2], &cards[8]], &[L::Review]).await;
    let read = until(&mut client, &workspace, "the watch never counted the threads or the queue", |p| {
        stage_of(p, &cards[8].key).is_some_and(|s| s.kind == pb::PrStageKind::Queued as i32)
            && stage_of(p, &cards[2].key).is_some_and(|s| s.unresolved_threads == Some(2))
    })
    .await;
    assert_eq!(words(stage_of(&read, &cards[8].key)), "Queued, position 2");
    (dir, daemon, client, workspace, cards)
}

/// `pr.refresh` for the board's repository.
async fn refresh(client: &mut SocketClient, workspace: &pb::Workspace) {
    let mut req = farcooler_transport::request("pr.refresh");
    req.payload = Some(pb::request::Payload::PrRefresh(pb::PrRefresh { repository_id: workspace.repository_id.clone() }));
    client.call(req).await.expect("pr.refresh");
}

/// Card 3's stage and card 9's, as the plan says them now.
async fn at_rest(client: &mut SocketClient, workspace: &pb::Workspace, cards: &[pb::Task]) -> (String, Option<u32>, String) {
    let read = plan(client, workspace).await;
    let three = stage_of(&read, &cards[2].key);
    (words(three), three.and_then(|s| s.unresolved_threads), words(stage_of(&read, &cards[8].key)))
}

#[tokio::test]
async fn every_row_of_the_table_reaches_the_plan_for_each_card_and_each_lane() {
    let (dir, _daemon, mut client, workspace) = a_runner().await;
    let cards = cards_and_prs(&mut client, &workspace, dir.path(), 13).await;
    let keys: Vec<String> = cards.iter().map(|c| c.key.clone()).collect();
    let c = |n: usize| &cards[n - 1];

    // Lane by lane, each moved along the arrows to where its row needs it.
    a_lane(&mut client, &workspace, "review", &[c(1), c(3), c(4), c(5), c(12)], &[L::Review]).await;
    a_lane(&mut client, &workspace, "fixing", &[c(2)], &[L::Review, L::Fixing]).await;
    a_lane(&mut client, &workspace, "landing", &[c(6), c(7), c(8), c(9)], &[L::Review, L::Landing]).await;
    a_lane(&mut client, &workspace, "closed", &[c(11)], &[L::Review]).await;
    a_lane(&mut client, &workspace, "landed", &[c(10)], &[L::Review, L::Landing, L::Landed]).await;
    a_lane(&mut client, &workspace, "building", &[c(13)], &[]).await;

    let read = until(&mut client, &workspace, "the watch never read the pull requests, or never counted the queue", |p| {
        stage_of(p, &keys[8]).is_some_and(|s| s.kind == pb::PrStageKind::Queued as i32)
            && stage_of(p, &keys[2]).is_some_and(|s| s.unresolved_threads.is_some())
            && stage_of(p, &keys[7]).is_some_and(|s| s.unresolved_threads.is_some())
    })
    .await;

    // One row each, on the card. Card 8's newest PR is a revert and card 9's a
    // fork's, and card 4 has two tries: none of that changes what they say.
    let expect = [
        (1, "Agent review"),
        (2, "Fixing"),
        (3, "Waiting on alice"),
        (4, "Waiting on a reviewer"),
        (5, "Changes requested"),
        (6, "Approved \u{b7} checks running"),
        (7, "Approved \u{b7} checks failing"),
        (8, "Approved"),
        (9, "Queued, position 2"),
        (10, "Merged"),
        (11, "Closed"),
        (12, "Waiting on alice and myorg/core"),
        (13, "Building"),
    ];
    for (n, label) in expect {
        assert_eq!(words(stage_of(&read, &keys[n - 1])), label, "card {n} ({})", keys[n - 1]);
    }
    assert_eq!(stage_of(&read, &keys[8]).unwrap().queue_position, 2);
    assert_eq!(stage_of(&read, &keys[2]).unwrap().unresolved_threads, Some(2), "the GraphQL count reaches the card");
    assert_eq!(stage_of(&read, &keys[2]).unwrap().reviewers, ["alice"]);
    assert_eq!(stage_of(&read, &keys[11]).unwrap().reviewers, ["alice", "myorg/core"]);
    assert_eq!(stage_of(&read, &keys[3]).unwrap().pr_number, 125, "the newest of card 4's two PRs");
    assert_eq!(stage_of(&read, &keys[0]).unwrap().pr_url, "https://github.example/o/r/pull/101");

    // And the lane's own: the least advanced of its cards'.
    assert_eq!(words(lane_stage(&read, "review")), "Agent review", "5 cards, the draft is furthest back");
    assert_eq!(words(lane_stage(&read, "fixing")), "Fixing");
    assert_eq!(words(lane_stage(&read, "landing")), "Approved \u{b7} checks failing", "a failing check is behind a running one");
    assert_eq!(words(lane_stage(&read, "closed")), "Closed");
    assert_eq!(words(lane_stage(&read, "landed")), "Merged");
    assert_eq!(words(lane_stage(&read, "building")), "Building");
    assert_eq!(lane_stage(&read, "review").unwrap().pr_number, 101, "it names the PR that set it");

    // Threads are asked of the open pull requests of live lanes and no others:
    // not the merged one on the landed lane, nor the closed one, nor the revert
    // or the fork's, nor the card with no PR on the building lane.
    let asked: std::collections::BTreeSet<String> =
        calls(dir.path()).into_iter().filter_map(|l| l.strip_prefix("graphql ").map(str::to_string)).collect();
    let live: std::collections::BTreeSet<String> =
        [101, 102, 103, 125, 105, 106, 107, 108, 109, 112].iter().map(|n| n.to_string()).collect();
    assert_eq!(asked, live, "one GraphQL read per open PR of a live lane");
    // Open PRs and the finished ones are listed apart.
    let listed = calls(dir.path());
    assert!(listed.iter().any(|l| l == "pr list open") && listed.iter().any(|l| l == "pr list closed"), "{listed:?}");
}

/// A `gh` that cannot answer reads "PR state unknown", which is not the silence
/// of a lane that asked and found no pull request; and a later read that
/// answers replaces it.
#[tokio::test]
async fn a_gh_that_cannot_answer_reads_unknown_and_not_no_pr() {
    let (dir, _daemon, mut client, workspace) = a_runner().await;
    std::fs::write(dir.path().join("offline"), "").unwrap();
    std::fs::write(dir.path().join("pr-list-open.json"), "[]").unwrap();
    std::fs::write(dir.path().join("pr-list-closed.json"), "[]").unwrap();
    let card = a_card(&mut client, &workspace, "Card").await;
    let building = a_card(&mut client, &workspace, "Not started").await;
    let another = a_card(&mut client, &workspace, "Another").await;
    a_lane(&mut client, &workspace, "review", &[&card], &[L::Review]).await;
    a_lane(&mut client, &workspace, "building", &[&building], &[]).await;

    // The watch has asked and been refused.
    a_read_after(dir.path(), 0).await;
    let read = plan(&mut client, &workspace).await;
    assert_eq!(words(stage_of(&read, &card.key)), "PR state unknown", "could not ask");
    assert_eq!(words(lane_stage(&read, "review")), "PR state unknown");
    assert_eq!(words(lane_stage(&read, "building")), "none", "nothing is expected of a lane still building");

    // Back online, and the list is empty: asked, and there is no pull request.
    // A lane entering review tries at once, past the repository's backoff.
    std::fs::remove_file(dir.path().join("offline")).unwrap();
    a_lane(&mut client, &workspace, "second", &[&another], &[L::Review]).await;
    let read = until(&mut client, &workspace, "a gh that answers never replaced unknown", |p| {
        lane_stage(p, "review").is_none_or(|s| s.kind != pb::PrStageKind::Unknown as i32)
    })
    .await;
    assert_eq!(words(stage_of(&read, &card.key)), "none", "asked, and no pull request: not unknown");
}

/// A read that fails keeps what was known: a network blip does not turn every
/// stage unknown, and a GraphQL call that fails does not drop the thread count
/// or the queue place it read before.
#[tokio::test]
async fn a_read_that_fails_keeps_what_was_known() {
    let (dir, _daemon, mut client, workspace, cards) = a_board_at_rest().await;
    let known = at_rest(&mut client, &workspace, &cards).await;
    assert_eq!(known, ("Waiting on alice".into(), Some(2), "Queued, position 2".into()));

    // gh logged out: the list fails. A lane entering review tries at once.
    std::fs::write(dir.path().join("offline"), "").unwrap();
    let before = lists(dir.path());
    a_lane(&mut client, &workspace, "blip", &[&cards[12]], &[L::Review]).await;
    a_read_after(dir.path(), before).await;
    assert_eq!(at_rest(&mut client, &workspace, &cards).await, known, "a failed list changes nothing");

    // The list answers and the GraphQL call fails: the counts and the place stay.
    std::fs::remove_file(dir.path().join("offline")).unwrap();
    std::fs::write(dir.path().join("graphql-fails"), "").unwrap();
    let before = lists(dir.path());
    a_lane(&mut client, &workspace, "second-blip", &[&cards[0]], &[L::Review]).await;
    a_read_after(dir.path(), before).await;
    assert_eq!(at_rest(&mut client, &workspace, &cards).await, known, "a failed count keeps the last");
}

/// The manual refresh reads the list only, so it must keep the thread counts
/// and the queue place, and a refresh that cannot reach GitHub keeps the cache.
#[tokio::test]
async fn a_manual_refresh_keeps_the_counts_and_a_failed_one_keeps_the_cache() {
    let (dir, _daemon, mut client, workspace, cards) = a_board_at_rest().await;
    let known = at_rest(&mut client, &workspace, &cards).await;
    let before = lists(dir.path());
    refresh(&mut client, &workspace).await;
    assert!(lists(dir.path()) > before, "the refresh read the list");
    assert_eq!(at_rest(&mut client, &workspace, &cards).await, known, "a refresh");

    std::fs::write(dir.path().join("offline"), "").unwrap();
    refresh(&mut client, &workspace).await;
    assert_eq!(at_rest(&mut client, &workspace, &cards).await, known, "a refresh that could not reach GitHub");
}

/// While no lane is in review, fixing or landing the watch reads nothing from
/// GitHub. Only a lane entering one of them wakes it, and a lane does so once
/// in fifteen seconds.
#[tokio::test]
async fn the_watch_reads_nothing_until_a_lane_enters_review_and_a_lane_kicks_once_in_fifteen_seconds() {
    let (dir, _daemon, mut client, workspace) = a_runner().await;
    std::fs::write(dir.path().join("pr-list-open.json"), "[]").unwrap();
    std::fs::write(dir.path().join("pr-list-closed.json"), "[]").unwrap();
    let card = a_card(&mut client, &workspace, "Card").await;
    let lane = a_lane(&mut client, &workspace, "work", &[&card], &[]).await;

    // An update that moves nothing into review wakes nothing.
    let mut update = farcooler_transport::request("lane.update");
    update.payload = Some(pb::request::Payload::LaneUpdate(pb::LaneUpdate {
        lane_id: lane.id.clone(),
        reason: Some("Still building.".into()),
        actor: "manager".into(),
        ..Default::default()
    }));
    call(&mut client, update, farcooler_protocol::capability::BOARD_PLAN).await;
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(calls(dir.path()), Vec::<String>::new(), "a building lane is not waiting on a pull request");

    let review = move_lane(&mut client, &lane, L::Review).await;
    a_read_after(dir.path(), 0).await;
    let after_one_turn = lists(dir.path());
    assert_eq!(after_one_turn, 2, "the open list and the closed one");

    // Straight on to fixing: the same lane, inside fifteen seconds. No new read.
    move_lane(&mut client, &review, L::Fixing).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(lists(dir.path()), after_one_turn, "one kick per lane in fifteen seconds");
}
