//! `pr.ready` and `pr.thread_reply` (ov-317), end to end: a real `farcoolerd`
//! whose `gh` is a shim that logs what it is asked and keeps what it is sent.
//!
//! What only a running daemon shows: that a lane's draft pull request is marked
//! ready and no other is, that a reply lands on the comment the card's link
//! names or on the pull request when the link names none, as the login `gh`
//! reports, and that a link the runner cannot read to one place posts nothing.
//! The scope check is `pr_actions_need_control.rs`.

use farcooler_protocol::capability::{PR_ACTIONS, WORKSTREAMS};
use farcooler_protocol::v1::{self as pb, LaneState as L, request, result};
use farcooler_transport::{Client, request as request_for};
use std::time::{Duration, Instant};

mod common;
#[path = "support/gh_pr.rs"]
mod gh_pr;
use gh_pr::*;

fn stage_of<'a>(plan: &'a pb::Plan, key: &str) -> Option<&'a pb::PrStage> {
    let id = &plan.cards.iter().find(|c| c.key == key)?.task_id;
    plan.lanes.iter().flat_map(|l| &l.cards).find(|c| c.task_id == *id)?.stage.as_ref()
}

/// Cards `1..=n` and the shim serving their PRs (card N is PR 100 + N; 1 and 2
/// are drafts).
async fn cards(client: &mut SocketClient, workspace: &pb::Workspace, dir: &std::path::Path, n: usize) -> Vec<pb::Task> {
    let mut out = Vec::new();
    for i in 1..=n {
        out.push(a_card(client, workspace, &format!("Card {i}")).await);
    }
    let keys: Vec<String> = out.iter().map(|c| c.key.clone()).collect();
    serve(dir, &keys);
    out
}

async fn ready(client: &mut SocketClient, lane: &pb::Lane, pr_number: u32) -> Result<pb::PrReadied, farcooler_transport::ClientError> {
    let mut req = request_for("pr.ready");
    req.payload = Some(request::Payload::PrReady(pb::PrReady { lane_id: lane.id.clone(), pr_number }));
    req.required_capabilities = vec![PR_ACTIONS.into()];
    match client.call(req).await?.value {
        Some(result::Value::PrReadied(r)) => Ok(r),
        other => panic!("wrong result: {other:?}"),
    }
}

/// A lane in review over cards 1 and 2 (two drafts) and one over card 3 (ready), read.
async fn lanes(client: &mut SocketClient, workspace: &pb::Workspace, cards: &[pb::Task]) -> (pb::Lane, pb::Lane) {
    let drafts = a_lane(client, workspace, "drafts", &[&cards[0], &cards[1]], &[L::Review]).await;
    let one = a_lane(client, workspace, "one", &[&cards[2]], &[L::Review]).await;
    until(client, workspace, "the watch never read the pull requests", |p| {
        stage_of(p, &cards[0].key).is_some_and(|s| s.kind == pb::PrStageKind::AgentReview as i32)
            && stage_of(p, &cards[2].key).is_some_and(|s| s.pr_number == 103)
    })
    .await;
    (drafts, one)
}

#[tokio::test]
async fn a_lane_s_draft_pull_request_is_marked_ready_and_no_other_is() {
    let (dir, _daemon, mut client, workspace) = a_runner().await;
    let cards = cards(&mut client, &workspace, dir.path(), 3).await;
    let (drafts, one) = lanes(&mut client, &workspace, &cards).await;

    // Two drafts on one lane and no number: ambiguous, so nothing is touched.
    let refused = ready(&mut client, &drafts, 0).await.expect_err("two drafts, no number");
    assert!(format!("{refused:?}").contains("lane_pr"), "{refused:?}");
    // A number that is not the lane's own: card 3's PR is on another lane.
    let refused = ready(&mut client, &drafts, 103).await.expect_err("another lane's pull request");
    assert!(format!("{refused:?}").contains("lane_pr"), "{refused:?}");
    // A number the repository has nowhere.
    let refused = ready(&mut client, &drafts, 999).await.expect_err("no such pull request");
    assert!(format!("{refused:?}").contains("lane_pr"), "{refused:?}");
    assert!(!calls(dir.path()).iter().any(|c| c.starts_with("pr ready")), "{:?}", calls(dir.path()));

    // Naming one of its own does it.
    let done = ready(&mut client, &drafts, 102).await.expect("pr.ready");
    assert_eq!((done.number, done.already_ready), (102, false));
    assert!(done.url.ends_with("/pull/102"), "{done:?}");
    assert_eq!(calls(dir.path()).iter().filter(|c| *c == "pr ready 102").count(), 1, "{:?}", calls(dir.path()));

    // GitHub says it is ready now: asked again, nothing is run a second time.
    let again = ready(&mut client, &drafts, 102).await.expect("pr.ready again");
    assert!(again.already_ready, "{again:?}");
    assert_eq!(calls(dir.path()).iter().filter(|c| c.starts_with("pr ready")).count(), 1, "{:?}", calls(dir.path()));

    // A lane whose only pull request is not a draft: it is already ready.
    let single = ready(&mut client, &one, 0).await;
    assert!(single.is_err(), "no draft among its pull requests: {single:?}");
}

#[tokio::test]
async fn a_pull_request_that_is_closed_or_that_gh_will_not_ready_is_refused() {
    let (dir, _daemon, mut client, workspace) = a_runner().await;
    let cards = cards(&mut client, &workspace, dir.path(), 3).await;
    let (drafts, _) = lanes(&mut client, &workspace, &cards).await;

    std::fs::write(dir.path().join("closed"), "").unwrap();
    ready(&mut client, &drafts, 101).await.expect_err("closed on GitHub since the last read");
    std::fs::remove_file(dir.path().join("closed")).unwrap();
    assert!(!calls(dir.path()).iter().any(|c| c.starts_with("pr ready")), "{:?}", calls(dir.path()));

    std::fs::write(dir.path().join("ready-fails"), "").unwrap();
    ready(&mut client, &drafts, 101).await.expect_err("gh refused");
}

// ---- replies ----

const THREAD: &str = "https://github.com/acme/billing/pull/213#discussion_r987";

/// A card waiting on the owner with a question that escalates `link`.
async fn a_thread_card(client: &mut SocketClient, workspace: &pb::Workspace, link: Option<&str>) -> pb::Task {
    let card = a_card(client, workspace, "Refund endpoint").await;
    let mut note = request_for("task.note");
    let extra = link.map(|l| serde_json::json!({ "thread": { "url": l, "reviewer": "bob" } }).to_string()).unwrap_or_default();
    note.payload = Some(request::Payload::TaskNoteAppend(pb::TaskNoteAppend {
        task_id: card.id.clone(),
        kind: pb::TaskNoteKind::Question as i32,
        body: "Why not a transaction?".into(),
        extra_json: extra,
        supersedes: None,
        actor: "manager".into(),
    }));
    call(client, note, WORKSTREAMS).await;
    let mut status = request_for("task.set_status");
    status.payload = Some(request::Payload::TaskSetStatus(pb::TaskSetStatus {
        task_id: card.id.clone(),
        status: pb::TaskStatus::NeedsDecision as i32,
        actor: "manager".into(),
    }));
    call(client, status, WORKSTREAMS).await;
    card
}

async fn reply_raw(
    client: &mut SocketClient,
    task: &pb::Task,
    body: &str,
    send: bool,
    bound: Option<(bytes::Bytes, String)>,
) -> Result<pb::PrThreadReplied, farcooler_transport::ClientError> {
    let mut req = request_for("pr.thread_reply");
    let (note_id, digest) = bound.unwrap_or_default();
    req.payload =
        Some(request::Payload::PrThreadReply(pb::PrThreadReply { task_id: task.id.clone(), body: body.into(), send, note_id, digest }));
    req.required_capabilities = vec![PR_ACTIONS.into()];
    match client.call(req).await?.value {
        Some(result::Value::PrThreadReplied(r)) => Ok(r),
        other => panic!("wrong result: {other:?}"),
    }
}

/// A preview, or a send bound to a preview taken just before it, as the apps do.
async fn reply(
    client: &mut SocketClient,
    task: &pb::Task,
    body: &str,
    send: bool,
) -> Result<pb::PrThreadReplied, farcooler_transport::ClientError> {
    if !send {
        return reply_raw(client, task, body, false, None).await;
    }
    let shown = reply_raw(client, task, "", false, None).await?;
    reply_raw(client, task, body, true, Some((shown.note_id, shown.digest))).await
}

fn posted(dir: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(dir.join("posted-body")).ok()
}

fn posts(dir: &std::path::Path) -> Vec<String> {
    calls(dir).into_iter().filter(|c| c.starts_with("api post") || c.starts_with("pr comment")).collect()
}

#[tokio::test]
async fn a_preview_names_the_pull_request_and_the_login_and_posts_nothing() {
    let (dir, _daemon, mut client, workspace) = a_runner().await;
    std::fs::write(dir.path().join("login"), "e-liang\n").unwrap();
    let card = a_thread_card(&mut client, &workspace, Some(THREAD)).await;

    let preview = reply(&mut client, &card, "", false).await.expect("preview");
    assert_eq!(
        (preview.repository.as_str(), preview.number, preview.login.as_str(), preview.sent, preview.in_thread),
        ("acme/billing", 213, "e-liang", false, true)
    );
    assert!(posts(dir.path()).is_empty(), "{:?}", calls(dir.path()));
}

#[tokio::test]
async fn a_reply_lands_on_the_comment_the_link_names_with_the_text_sent() {
    let (dir, _daemon, mut client, workspace) = a_runner().await;
    let card = a_thread_card(&mut client, &workspace, Some(THREAD)).await;

    // The text starts with characters a shell or an option parser would eat.
    let text = "--body @/etc/passwd $(touch pwned) \"quoted\"\n\u{1F44D} second line";
    let sent = reply(&mut client, &card, text, true).await.expect("reply");
    assert!(sent.sent && sent.in_thread, "{sent:?}");
    assert_eq!(
        posts(dir.path()),
        ["api post github.com repos/acme/billing/pulls/213/comments/987/replies"],
        "{:?}",
        calls(dir.path())
    );
    assert_eq!(posted(dir.path()).as_deref(), Some(text), "the text arrives whole, on stdin");
    assert!(!std::path::Path::new("pwned").exists());
}

#[tokio::test]
async fn a_link_that_names_the_pull_request_only_is_answered_on_the_pull_request() {
    let (dir, _daemon, mut client, workspace) = a_runner().await;
    let card = a_thread_card(&mut client, &workspace, Some("https://github.com/acme/billing/pull/213")).await;

    let sent = reply(&mut client, &card, "Fixed in the next commit.", true).await.expect("reply");
    assert!(sent.sent && !sent.in_thread, "{sent:?}");
    assert_eq!(posts(dir.path()), ["pr comment https://github.com/acme/billing/pull/213"], "{:?}", calls(dir.path()));
    assert_eq!(posted(dir.path()).as_deref(), Some("Fixed in the next commit."));
}

#[tokio::test]
async fn a_link_the_runner_cannot_read_to_one_place_posts_nothing() {
    let (dir, _daemon, mut client, workspace) = a_runner().await;
    for (n, link) in [
        // github.com, so the card offers it, but not a pull request's page.
        "https://github.com/acme/billing/issues/5#issuecomment-1",
        "https://github.com/acme/billing/pull/213/files#diff-abc",
        "https://github.com/acme/billing/pull/0#discussion_r1",
        "https://github.com/acme/billing/pull/213#r987",
        "https://github.com/acme/../pull/213#discussion_r1",
        "https://github.com/acme/billing/pull/213#discussion_r99999999999999999999999",
    ]
    .iter()
    .enumerate()
    {
        let card = a_thread_card(&mut client, &workspace, Some(link)).await;
        let refused = reply(&mut client, &card, "Hello", true).await.expect_err(link);
        assert!(format!("{refused:?}").contains("thread_link"), "{n} {link}: {refused:?}");
    }
    assert!(posts(dir.path()).is_empty(), "{:?}", calls(dir.path()));

    // No thread on the card at all, and a card that is not waiting.
    let plain = a_thread_card(&mut client, &workspace, None).await;
    reply(&mut client, &plain, "Hello", true).await.expect_err("an ordinary question has no thread");
    let idle = a_card(&mut client, &workspace, "Idle").await;
    reply(&mut client, &idle, "Hello", true).await.expect_err("a card with no question");
    assert!(posts(dir.path()).is_empty(), "{:?}", calls(dir.path()));
}

#[tokio::test]
async fn a_reply_that_is_empty_or_too_long_or_that_gh_refuses_is_not_posted() {
    let (dir, _daemon, mut client, workspace) = a_runner().await;
    let card = a_thread_card(&mut client, &workspace, Some(THREAD)).await;

    for bad in ["".to_string(), "   \n".to_string(), "x".repeat(2001), "a\0b".to_string()] {
        let refused = reply(&mut client, &card, &bad, true).await.expect_err("a bad body");
        assert!(format!("{refused:?}").contains("reply_body"), "{refused:?}");
    }
    assert!(posts(dir.path()).is_empty(), "{:?}", calls(dir.path()));
    // The cap is in characters: 2,000 of a multi-byte one is allowed.
    reply(&mut client, &card, &"\u{e9}".repeat(2000), true).await.expect("exactly the cap");
    // That answered the card; the rest is on a fresh one.
    let card = a_thread_card(&mut client, &workspace, Some(THREAD)).await;

    // gh logged out: no login, so no preview and no post.
    std::fs::write(dir.path().join("offline"), "").unwrap();
    let before = posts(dir.path()).len();
    reply(&mut client, &card, "Hello", false).await.expect_err("no login");
    reply(&mut client, &card, "Hello", true).await.expect_err("no login");
    assert_eq!(posts(dir.path()).len(), before);
    std::fs::remove_file(dir.path().join("offline")).unwrap();

    // GitHub refusing the post: not reported as sent.
    std::fs::write(dir.path().join("post-fails"), "").unwrap();
    reply(&mut client, &card, "Hello", true).await.expect_err("GitHub refused");
}

async fn tasks_notes(client: &mut SocketClient, task: &pb::Task) -> Vec<pb::TaskNote> {
    let mut get = request_for("task.get");
    get.payload = Some(request::Payload::TaskGet(pb::TaskGetRequest { task_id: task.id.clone(), ..Default::default() }));
    let result::Value::TaskDetail(detail) = call(client, get, WORKSTREAMS).await else { panic!("wrong result") };
    detail.notes
}

async fn thread_items(client: &mut SocketClient) -> usize {
    let result::Value::NeedsYouList(list) = call(client, request_for("needs_you.list"), farcooler_protocol::capability::NEEDS_YOU).await
    else {
        panic!("wrong result")
    };
    list.items.iter().filter(|i| i.kind() == pb::NeedsYouKind::PrThread).count()
}

/// M1: a link into a repository that is not one of the board's remotes posts nothing,
/// though it is on github.com and the login can reach it.
#[tokio::test]
async fn a_link_into_another_repository_is_refused_before_anything_runs() {
    let (dir, _daemon, mut client, workspace) = a_runner().await;
    for link in [
        "https://github.com/attacker/x/pull/1#discussion_r1",
        "https://github.com/acme/billing-evil/pull/1#discussion_r1",
        "https://github.com/acme/other/pull/1",
    ] {
        let card = a_thread_card(&mut client, &workspace, Some(link)).await;
        let refused = reply(&mut client, &card, "Hello", true).await.expect_err(link);
        assert!(format!("{refused:?}").contains("thread_link"), "{link}: {refused:?}");
        let refused = reply(&mut client, &card, "", false).await.expect_err("not even a preview");
        assert!(format!("{refused:?}").contains("thread_link"), "{refused:?}");
    }
    assert!(posts(dir.path()).is_empty(), "{:?}", calls(dir.path()));
    assert!(!calls(dir.path()).iter().any(|c| c.starts_with("api user")), "gh was asked: {:?}", calls(dir.path()));
}

/// M2: the send is bound to the thread the person was shown.
#[tokio::test]
async fn a_thread_that_changes_between_the_preview_and_the_send_is_not_posted_to() {
    let (dir, _daemon, mut client, workspace) = a_runner().await;
    let card = a_thread_card(&mut client, &workspace, Some(THREAD)).await;
    let shown = reply_raw(&mut client, &card, "", false, None).await.expect("preview");

    // No preview at all, or a wrong digest, or another question's id.
    for bound in [None, Some((shown.note_id.clone(), "0".repeat(24))), Some((bytes::Bytes::from_static(&[7; 16]), shown.digest.clone()))] {
        let refused = reply_raw(&mut client, &card, "Hello", true, bound).await.expect_err("unbound");
        assert!(format!("{refused:?}").contains("thread_link"), "{refused:?}");
    }

    // The agent asks again about another thread, in the same repository, after the preview.
    let mut ask = request_for("task.note");
    ask.payload = Some(request::Payload::TaskNoteAppend(pb::TaskNoteAppend {
        task_id: card.id.clone(),
        kind: pb::TaskNoteKind::Question as i32,
        body: "And this one?".into(),
        extra_json: serde_json::json!({ "thread": { "url": "https://github.com/acme/billing/pull/214#discussion_r5" } }).to_string(),
        supersedes: None,
        actor: "manager".into(),
    }));
    call(&mut client, ask, WORKSTREAMS).await;
    let refused = reply_raw(&mut client, &card, "Hello", true, Some((shown.note_id.clone(), shown.digest.clone()))).await.expect_err("stale preview");
    assert!(format!("{refused:?}").contains("thread_link"), "{refused:?}");
    assert!(posts(dir.path()).is_empty(), "{:?}", calls(dir.path()));

    // A fresh preview names the new thread and sends.
    let fresh = reply_raw(&mut client, &card, "", false, None).await.expect("preview");
    assert_eq!(fresh.number, 214);
    assert_ne!(fresh.digest, shown.digest);
    reply_raw(&mut client, &card, "Hello", true, Some((fresh.note_id, fresh.digest))).await.expect("send");
    assert_eq!(posts(dir.path()), ["api post github.com repos/acme/billing/pulls/214/comments/5/replies"]);
}

/// M3: a reply answers the card, so the row clears and a second send is refused.
#[tokio::test]
async fn a_reply_answers_the_card_and_a_second_send_is_refused() {
    let (dir, _daemon, mut client, workspace) = a_runner().await;
    std::fs::write(dir.path().join("login"), "e-liang\n").unwrap();
    let card = a_thread_card(&mut client, &workspace, Some(THREAD)).await;
    assert_eq!(thread_items(&mut client).await, 1);
    let shown = reply_raw(&mut client, &card, "", false, None).await.expect("preview");

    let sent = reply_raw(&mut client, &card, "Fixed in the next commit.", true, Some((shown.note_id.clone(), shown.digest.clone()))).await.expect("send");
    assert!(sent.sent);
    let notes = tasks_notes(&mut client, &card).await;
    let answer = notes.iter().rev().find(|n| n.kind() == pb::TaskNoteKind::Answer).expect("an ANSWER note");
    assert_eq!(answer.body, "Replied on GitHub as e-liang: Fixed in the next commit.");
    let extra: serde_json::Value = serde_json::from_str(&answer.extra_json).unwrap();
    assert_eq!(extra["thread_reply"]["login"], "e-liang");
    assert_eq!(extra["thread_reply"]["url"], THREAD);
    assert_eq!(extra["thread_reply"]["comment"], 987);
    assert_eq!(thread_items(&mut client).await, 0, "the row clears the way every answer clears it");

    // The same preview, sent again, or a new preview: refused, and nothing more is posted.
    let again = reply_raw(&mut client, &card, "Again", true, Some((shown.note_id, shown.digest))).await.expect_err("already answered");
    assert!(format!("{again:?}").contains("already_answered"), "{again:?}");
    let again = reply_raw(&mut client, &card, "", false, None).await.expect_err("already answered");
    assert!(format!("{again:?}").contains("already_answered"), "{again:?}");
    assert_eq!(posts(dir.path()).len(), 1, "{:?}", calls(dir.path()));
}

/// L2: Mark Ready is for the lane's own pull request: on the lane's branch, or opened by this login.
#[tokio::test]
async fn a_pull_request_that_is_neither_on_the_lane_s_branch_nor_this_login_s_is_not_readied() {
    let (dir, _daemon, mut client, workspace) = a_runner().await;
    let cards = cards(&mut client, &workspace, dir.path(), 3).await;
    let (drafts, _) = lanes(&mut client, &workspace, &cards).await;
    std::fs::write(dir.path().join("head-ref"), "someone/else").unwrap();
    std::fs::write(dir.path().join("author"), "mallory").unwrap();
    let refused = ready(&mut client, &drafts, 101).await.expect_err("a stranger's branch and author");
    assert!(format!("{refused:?}").contains("lane_pr"), "{refused:?}");
    assert!(!calls(dir.path()).iter().any(|c| c.starts_with("pr ready")), "{:?}", calls(dir.path()));
    // Opened by the runner's own login from another branch: allowed.
    std::fs::write(dir.path().join("author"), "octo-owner").unwrap();
    ready(&mut client, &drafts, 101).await.expect("this login's own");
    assert_eq!(calls(dir.path()).iter().filter(|c| *c == "pr ready 101").count(), 1);
}

/// Two sends of one preview, both past the first check while `gh api user` is slow for the second:
/// the first posts and answers the card, and the second, which wakes after that, is refused.
#[tokio::test]
async fn two_sends_of_one_preview_post_once() {
    let (dir, _daemon, mut client, workspace) = a_runner().await;
    let card = a_thread_card(&mut client, &workspace, Some(THREAD)).await;
    let shown = reply_raw(&mut client, &card, "", false, None).await.expect("preview");
    let socket = dir.path().join("home").join("farcoolerd.sock");
    let mut a = Client::connect(&socket, "test-client", "0.0.0").await.expect("connect");
    let mut b = Client::connect(&socket, "test-client", "0.0.0").await.expect("connect");
    let mut watcher = Client::connect(&socket, "test-client", "0.0.0").await.expect("connect");
    std::fs::write(dir.path().join("slow"), "").unwrap();
    let bound = || Some((shown.note_id.clone(), shown.digest.clone()));

    let release = async {
        // Let the second caller's `gh` go once the first has posted and answered the card.
        let deadline = Instant::now() + BUDGET;
        while !(posts(dir.path()).len() == 1 && thread_items(&mut watcher).await == 0) {
            assert!(Instant::now() < deadline, "the first send never finished: {:?}", calls(dir.path()));
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        std::fs::write(dir.path().join("go"), "").unwrap();
    };
    let (first, second, ()) = tokio::join!(
        reply_raw(&mut a, &card, "Once", true, bound()),
        reply_raw(&mut b, &card, "Once", true, bound()),
        release
    );
    let outcomes = [first, second];
    assert_eq!(outcomes.iter().filter(|r| r.is_ok()).count(), 1, "{outcomes:?}");
    let refused = outcomes.iter().find_map(|r| r.as_ref().err()).expect("one refused");
    assert!(format!("{refused:?}").contains("already_answered"), "{refused:?}");
    assert_eq!(posts(dir.path()).len(), 1, "posted twice: {:?}", calls(dir.path()));
}

/// L3: a branch of a fork named like the lane's is not the lane's.
#[tokio::test]
async fn a_fork_branch_named_like_the_lane_s_is_not_readied() {
    let (dir, _daemon, mut client, workspace) = a_runner().await;
    let cards = cards(&mut client, &workspace, dir.path(), 3).await;
    let (drafts, _) = lanes(&mut client, &workspace, &cards).await;
    std::fs::write(dir.path().join("cross"), "").unwrap();
    std::fs::write(dir.path().join("author"), "mallory").unwrap();
    let refused = ready(&mut client, &drafts, 101).await.expect_err("a fork's branch of that name");
    assert!(format!("{refused:?}").contains("lane_pr"), "{refused:?}");
    assert!(!calls(dir.path()).iter().any(|c| c.starts_with("pr ready")), "{:?}", calls(dir.path()));
}

/// L5: owner and repository are the same in any case.
#[tokio::test]
async fn a_link_in_another_case_is_the_board_s_repository() {
    let (dir, _daemon, mut client, workspace) = a_runner().await;
    let card = a_thread_card(&mut client, &workspace, Some("https://github.com/ACME/Billing/pull/213#discussion_r987")).await;
    let sent = reply(&mut client, &card, "Hello", true).await.expect("reply");
    assert!(sent.sent && sent.repository == "ACME/Billing", "{sent:?}");
    assert_eq!(posts(dir.path()).len(), 1, "{:?}", calls(dir.path()));
}
