//! A ticket shows where its pull request stands (ov-312, ov-872), end to end:
//! a real `farcoolerd` whose `gh` prints, in the shape gh 2.101 prints, one
//! pull request for each row of the stage table (`test/fixtures/pr-stage/`),
//! and the stage `plan.get` carries for each ticket on the line.
//!
//! The things only a running daemon shows: that the PR watch reads without
//! anyone asking once a ticket is in review, that it asks about threads only
//! for the open pull requests of live tickets, that a `gh` that cannot answer
//! reads "unknown" and not "no pull request", that a read that fails keeps
//! what was known, and that it reads nothing while no ticket is waiting on one.

use std::time::Duration;

use farcooler_protocol::v1::{self as pb, TicketStage as S};

mod common;
#[path = "support/gh_pr.rs"]
mod gh_pr;
use gh_pr::*;

fn stage_of<'a>(plan: &'a pb::Plan, key: &str) -> Option<&'a pb::PrStage> {
    let id = &plan.cards.iter().find(|c| c.key == key)?.task_id;
    plan.lines.iter().find(|l| l.task_id == *id)?.pr_stage.as_ref()
}

/// `card` on the line, on a branch of its own, moved through `path`.
async fn a_line(client: &mut SocketClient, card: &pb::Task, path: &[S]) -> pb::TicketLine {
    a_ticket(client, card, &format!("ticket/{}", card.key), path).await
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

/// Wait for the watch to ask GitHub for the PR list once more than `before`,
/// and for the turn's second list: a slow `gh` under load answered the
/// first more than the 800 ms below after it was asked (ov-668).
async fn a_read_after(dir: &std::path::Path, before: usize) {
    let deadline = std::time::Instant::now() + BUDGET;
    while lists(dir) <= before || listing(dir) {
        assert!(std::time::Instant::now() < deadline, "the watch never read again");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_millis(800)).await;
}

/// A board with cards 3 (two threads open) and 9 (queued at 2) in review, read.
async fn a_board_at_rest() -> (tempfile::TempDir, common::DaemonChild, SocketClient, pb::Workspace, Vec<pb::Task>) {
    let (dir, daemon, mut client, workspace) = a_runner().await;
    let cards = cards_and_prs(&mut client, &workspace, dir.path(), 13).await;
    for card in [&cards[2], &cards[8]] {
        a_line(&mut client, card, &[S::Building, S::Reviewing]).await;
    }
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
async fn every_row_of_the_table_reaches_the_plan_for_each_ticket() {
    let (dir, _daemon, mut client, workspace) = a_runner().await;
    let cards = cards_and_prs(&mut client, &workspace, dir.path(), 13).await;
    let keys: Vec<String> = cards.iter().map(|c| c.key.clone()).collect();
    let c = |n: usize| &cards[n - 1];

    // Ticket by ticket, each moved along the arrows to where its row needs it.
    for n in [1, 3, 4, 5, 11, 12] {
        a_line(&mut client, c(n), &[S::Building, S::Reviewing]).await;
    }
    a_line(&mut client, c(2), &[S::Building, S::Reviewing]).await;
    a_fix(&mut client, &workspace, c(2)).await;
    for n in [6, 7, 8, 9] {
        a_line(&mut client, c(n), &[S::Building, S::Reviewing, S::Landing]).await;
    }
    a_line(&mut client, c(10), &[S::Building, S::Reviewing, S::Landed]).await;
    a_line(&mut client, c(13), &[S::Building]).await;

    let read = until(&mut client, &workspace, "the watch never read the pull requests, or never counted the queue", |p| {
        stage_of(p, &keys[8]).is_some_and(|s| s.kind == pb::PrStageKind::Queued as i32)
            && stage_of(p, &keys[2]).is_some_and(|s| s.unresolved_threads.is_some())
            && stage_of(p, &keys[7]).is_some_and(|s| s.unresolved_threads.is_some())
    })
    .await;

    // One row each, on the ticket. Card 8's newest PR is a revert and card 9's a
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
    assert_eq!(stage_of(&read, &keys[2]).unwrap().unresolved_threads, Some(2), "the GraphQL count reaches the ticket");
    assert_eq!(stage_of(&read, &keys[2]).unwrap().reviewers, ["alice"]);
    assert_eq!(stage_of(&read, &keys[11]).unwrap().reviewers, ["alice", "myorg/core"]);
    assert_eq!(stage_of(&read, &keys[3]).unwrap().pr_number, 125, "the newest of card 4's two PRs");
    assert_eq!(stage_of(&read, &keys[0]).unwrap().pr_url, "https://github.example/o/r/pull/101");

    // Threads are asked of the open pull requests of live tickets and no
    // others: not the merged one of the landed ticket, nor the closed one, nor
    // the revert or the fork's, nor the building ticket with no PR.
    let asked: std::collections::BTreeSet<String> =
        calls(dir.path()).into_iter().filter_map(|l| l.strip_prefix("graphql ").map(str::to_string)).collect();
    let live: std::collections::BTreeSet<String> =
        [101, 102, 103, 125, 105, 106, 107, 108, 109, 112].iter().map(|n| n.to_string()).collect();
    assert_eq!(asked, live, "one GraphQL read per open PR of a live ticket");
    // Open PRs and the finished ones are listed apart.
    let listed = calls(dir.path());
    assert!(listed.iter().any(|l| l == "pr list open") && listed.iter().any(|l| l == "pr list closed"), "{listed:?}");
}

/// A `gh` that cannot answer reads "PR state unknown", which is not the silence
/// of a ticket that asked and found no pull request; and a later read that
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
    a_line(&mut client, &card, &[S::Building, S::Reviewing]).await;
    a_line(&mut client, &building, &[S::Building]).await;

    // The watch has asked and been refused.
    a_read_after(dir.path(), 0).await;
    let read = plan(&mut client, &workspace).await;
    assert_eq!(words(stage_of(&read, &card.key)), "PR state unknown", "could not ask");
    assert_eq!(words(stage_of(&read, &building.key)), "none", "nothing is expected of a ticket still building");

    // Back online, and the list is empty: asked, and there is no pull request.
    // A ticket entering review tries at once, past the repository's backoff.
    std::fs::remove_file(dir.path().join("offline")).unwrap();
    a_line(&mut client, &another, &[S::Building, S::Reviewing]).await;
    let read = until(&mut client, &workspace, "a gh that answers never replaced unknown", |p| {
        stage_of(p, &card.key).is_none_or(|s| s.kind != pb::PrStageKind::Unknown as i32)
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

    // gh logged out: the list fails. A ticket entering review tries at once.
    std::fs::write(dir.path().join("offline"), "").unwrap();
    let before = lists(dir.path());
    a_line(&mut client, &cards[12], &[S::Building, S::Reviewing]).await;
    a_read_after(dir.path(), before).await;
    assert_eq!(at_rest(&mut client, &workspace, &cards).await, known, "a failed list changes nothing");

    // The list answers and the GraphQL call fails: the counts and the place stay.
    std::fs::remove_file(dir.path().join("offline")).unwrap();
    std::fs::write(dir.path().join("graphql-fails"), "").unwrap();
    let before = lists(dir.path());
    a_line(&mut client, &cards[0], &[S::Building, S::Reviewing]).await;
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

/// While no ticket is in review, fixing or landing the watch reads nothing
/// from GitHub. Only a ticket entering one of them wakes it, and a ticket does
/// so once in fifteen seconds.
#[tokio::test]
async fn the_watch_reads_nothing_until_a_ticket_enters_review_and_a_ticket_kicks_once_in_fifteen_seconds() {
    let (dir, _daemon, mut client, workspace) = a_runner().await;
    std::fs::write(dir.path().join("pr-list-open.json"), "[]").unwrap();
    std::fs::write(dir.path().join("pr-list-closed.json"), "[]").unwrap();
    let card = a_card(&mut client, &workspace, "Card").await;
    a_line(&mut client, &card, &[S::Building]).await;

    // An update that moves nothing into review wakes nothing.
    set_line(&mut client, pb::TicketLineSet { task_id: card.id.clone(), reason: Some("Still building.".into()), ..Default::default() })
        .await;
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(calls(dir.path()), Vec::<String>::new(), "a building ticket is not waiting on a pull request");

    move_ticket(&mut client, &card, S::Reviewing).await;
    a_read_after(dir.path(), 0).await;
    let after_one_turn = lists(dir.path());
    assert_eq!(after_one_turn, 2, "the open list and the closed one");

    // Straight on to landing: the same ticket, inside fifteen seconds. No new
    // read.
    move_ticket(&mut client, &card, S::Landing).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(lists(dir.path()), after_one_turn, "one kick per ticket in fifteen seconds");
}

/// An update that names the stage a ticket is already in is not a ticket
/// entering one, even once its floor is up: a one-second floor here
/// (`TEST_KICK_FLOOR_MS`), so that's waited out in a second, not fifteen.
/// A real entry first proves the floor is up: were the daemon still on its
/// fifteen seconds, it would read nothing, and the test says so.
#[tokio::test]
async fn an_update_that_leaves_the_state_alone_does_not_wake_the_watch() {
    let floor = Duration::from_secs(1);
    let ms = floor.as_millis().to_string();
    let (dir, _daemon, mut client, workspace) = a_runner_with(&[(farcooler_daemon::pr_watch::TEST_KICK_FLOOR_MS, &ms)]).await;
    std::fs::write(dir.path().join("pr-list-open.json"), "[]").unwrap();
    std::fs::write(dir.path().join("pr-list-closed.json"), "[]").unwrap();
    let card = a_card(&mut client, &workspace, "Card").await;
    a_line(&mut client, &card, &[S::Building, S::Reviewing]).await;
    a_read_after(dir.path(), 0).await;
    let reads = lists(dir.path());

    tokio::time::sleep(floor + Duration::from_secs(2)).await;
    move_ticket(&mut client, &card, S::Landing).await;
    read_within(dir.path(), reads, "Reviewing to Landing, past the floor, read nothing: is the floor still fifteen seconds?").await;
    let reads = lists(dir.path());

    tokio::time::sleep(floor + Duration::from_secs(2)).await;
    move_ticket(&mut client, &card, S::Landing).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(lists(dir.path()), reads, "Landing to Landing is not an entry");
}

/// Wait for a read past `before`, within ten seconds: a kick's read comes in
/// well under that, and the watch's own next read is a minute away.
async fn read_within(dir: &std::path::Path, before: usize, why: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while lists(dir) <= before || listing(dir) {
        assert!(std::time::Instant::now() < deadline, "{why}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // Both lists, open and closed, are in before it's counted again.
    tokio::time::sleep(Duration::from_millis(800)).await;
}
