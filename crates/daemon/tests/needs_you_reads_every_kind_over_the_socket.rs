//! `needs_you.list` as a client meets it: over a real socket, through the real
//! dispatch table and scope check, against the real store.
//!
//! `needs_you.rs`'s own tests pin the assembly as a pure function. These pin
//! what those cannot: that the list a Control client reads carries every kind
//! in rank order, and that a Read client is handed the redacted shape.

#[path = "support/in_process.rs"]
mod in_process;

use farcooler_agent::event::{AgentEvent, PermissionOption};
use farcooler_daemon::needs_you::Observation;
use farcooler_protocol::v1::{AgentActivity, NeedsYouItem, NeedsYouKind, Scope, result};
use farcooler_store::models::{Actor, NoteKind, TaskStatus};
use farcooler_transport::request;
use in_process::*;

/// One of each kind: a chat ask, a blocked agent, a decision and a review.
/// Made oldest-last, so the order that comes back is the kinds' and not the
/// clock's.
async fn one_of_each(h: &Harness) {
    let repo = a_repository(h);
    let store = &h.service.store;

    let review = store.create_task(repo.workspace, "Invoice PDF export", Actor::User).unwrap();
    store.set_task_status(review.id, TaskStatus::InReview, Actor::Manager).unwrap();

    let decision = store.create_task(repo.workspace, "Pick a PDF library", Actor::User).unwrap();
    store
        .add_note(decision.id, NoteKind::Question, Actor::Manager, "Which one?", serde_json::json!({ "options": ["printpdf", "typst"] }))
        .unwrap();
    store.set_task_status(decision.id, TaskStatus::NeedsDecision, Actor::Manager).unwrap();

    let blocked = a_pane(h, repo.worktree, None);
    h.watcher
        .observe_for_tests(
            blocked,
            Observation {
                activity: AgentActivity::Blocked,
                state_since: now_millis(),
                blocked_question: Some("Allow codex to run tests?".into()),
                command: "codex".into(),
                ..Observation::default()
            },
        )
        .await;

    let asking = a_pane(h, repo.worktree, None);
    h.service.agents().record(
        asking,
        vec![AgentEvent::Permission {
            id: "chat-1".into(),
            tool_call: "t1".into(),
            options: vec![
                PermissionOption { id: "allow".into(), name: "Allow /tmp/probe/x.txt".into(), kind: "allow_once".into() },
                PermissionOption { id: "deny".into(), name: "Deny".into(), kind: "reject_once".into() },
            ],
        }],
        &|_, _| {},
    );
    h.watcher
        .observe_for_tests(
            asking,
            Observation { activity: AgentActivity::Blocked, state_since: now_millis(), command: "claude".into(), ..Observation::default() },
        )
        .await;
}

async fn needs_you(link: &mut Link) -> Vec<NeedsYouItem> {
    let result = link.call(request("needs_you.list")).await.expect("needs_you.list");
    let Some(result::Value::NeedsYouList(list)) = result.value else { panic!("wrong result: {result:?}") };
    list.items
}

#[tokio::test]
async fn every_kind_comes_back_in_rank_order() {
    let h = start(Scope::Control).await;
    one_of_each(&h).await;
    let items = needs_you(&mut connect(&h).await).await;
    let kinds: Vec<_> = items.iter().map(|i| i.kind()).collect();
    assert_eq!(
        kinds,
        [NeedsYouKind::Ask, NeedsYouKind::Blocked, NeedsYouKind::Decision, NeedsYouKind::Review],
        "{items:#?}"
    );
    assert!(items.windows(2).all(|w| w[0].rank < w[1].rank), "ranks ascend down the list");
    // What Control is owed: the ask's own buttons and id, and the question.
    assert_eq!(items[0].ask_id.as_deref(), Some("chat-1"));
    assert_eq!(items[0].question, "Allow /tmp/probe/x.txt");
    assert_eq!(items[0].actions.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["allow", "deny"]);
    assert_eq!(items[2].question, "Which one?");
    assert_eq!(items[2].task.as_ref().map(|t| t.title.as_str()), Some("Pick a PDF library"));
    assert_eq!(items[3].question, "Ready for review");
    assert!(items.iter().all(|i| i.workspace_name == "Main"), "{items:#?}");
}

#[tokio::test]
async fn a_read_scoped_client_gets_the_redacted_shape() {
    let h = start(Scope::Read).await;
    one_of_each(&h).await;
    let items = needs_you(&mut connect(&h).await).await;
    assert_eq!(items.len(), 4, "{items:#?}");
    assert!(!format!("{items:?}").contains("/tmp/probe"), "{items:#?}");
    let questions: Vec<_> = items.iter().map(|i| i.question.as_str()).collect();
    assert_eq!(
        questions,
        ["claude is asking to use a tool", "codex needs you", "Needs a decision", "Ready for review"]
    );
    for item in &items {
        assert_eq!((&item.detail, &item.ask_id, item.actions.len(), &item.worktree), (&None, &None, 0, &None), "{item:#?}");
    }
}

/// A chat's ask, answered through `terminal.agent_answer`, leaves the list.
///
/// No shim ever reports a `Resolved`, so the daemon records one when it hands
/// the answer on; before it did, every answered chat ask stayed an ask item
/// for the life of its pane. Nothing here records one by hand: the ask
/// arrives from a shim, the answer goes back to it, and the agent is left
/// Blocked, so only the answer can be what ends the item.
#[tokio::test]
async fn answering_a_chat_ask_takes_it_off_the_list() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let pane = a_pane(&h, repo.worktree, None);
    h.watcher
        .observe_for_tests(
            pane,
            Observation { activity: AgentActivity::Blocked, state_since: now_millis(), command: "claude".into(), ..Observation::default() },
        )
        .await;
    let mut shim = Shim::dial(&h, pane).await;
    shim.says(vec![AgentEvent::Permission {
        id: "chat-1".into(),
        tool_call: "t1".into(),
        options: vec![PermissionOption { id: "allow".into(), name: "Allow touch x".into(), kind: "allow_once".into() }],
    }])
    .await;
    let mut link = connect(&h).await;
    let mut listed = Vec::new();
    for _ in 0..100 {
        listed = needs_you(&mut link).await;
        if !listed.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(listed.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["ask:chat-1"]);

    let mut answer = request("terminal.agent_answer");
    answer.payload = Some(farcooler_protocol::v1::request::Payload::AgentAnswer(farcooler_protocol::v1::AgentAnswer {
        terminal_id: bytes::Bytes::copy_from_slice(pane.as_bytes()),
        request_id: "chat-1".into(),
        option_id: "allow".into(),
    }));
    // The reply re-reads the pane through tmux, which this harness may not
    // have; that the shim heard the answer is what's asked about.
    let _ = link.call(answer).await;
    assert!(matches!(
        shim.heard().await,
        farcooler_agent::link::DaemonMessage::Answer { request_id, .. } if request_id == "chat-1"
    ));
    // The agent is still Blocked here, as no sample has run since: it may be
    // a block now, and never an ask.
    let after = needs_you(&mut link).await;
    assert!(after.iter().all(|i| i.kind() != NeedsYouKind::Ask), "the answered ask is still listed: {after:#?}");
}

/// A chat pane whose shim is dialed, with the watcher holding whatever the
/// supervisor has folded its activity to, as a sample would.
async fn a_chat(h: &Harness) -> (uuid::Uuid, Shim, Link) {
    let repo = a_repository(h);
    let pane = a_pane(h, repo.worktree, None);
    let shim = Shim::dial(h, pane).await;
    (pane, shim, connect(h).await)
}

async fn as_sampled(h: &Harness, pane: uuid::Uuid) {
    let activity = h.service.agents().activity(pane);
    h.watcher
        .observe_for_tests(pane, Observation { activity, state_since: now_millis(), command: "claude".into(), ..Observation::default() })
        .await;
}

fn a_permission(id: &str) -> AgentEvent {
    AgentEvent::Permission {
        id: id.into(),
        tool_call: format!("t-{id}"),
        options: vec![PermissionOption { id: "allow".into(), name: "Allow touch x".into(), kind: "allow_once".into() }],
    }
}

/// The asks listed once the shim's events have landed and been folded.
async fn asks_after(h: &Harness, pane: uuid::Uuid, link: &mut Link, events: usize) -> Vec<String> {
    for _ in 0..100 {
        if h.service.agents().replay(pane, 0, 0).1.len() >= events {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    as_sampled(h, pane).await;
    needs_you(link).await.into_iter().filter(|i| i.kind() == NeedsYouKind::Ask).map(|i| i.id).collect()
}

/// One subagent waits on an ask while a sibling keeps working. The sibling's
/// tool call folds the pane to Working; the ask is still waiting.
#[tokio::test]
async fn an_ask_stays_listed_while_a_sibling_subagent_works() {
    let h = start(Scope::Control).await;
    let (pane, mut shim, mut link) = a_chat(&h).await;
    let sibling = AgentEvent::ToolCall {
        id: "call-2".into(),
        title: "Reading watch.rs".into(),
        kind: "read".into(),
        status: farcooler_agent::event::ToolStatus::InProgress,
        locations: vec![],
        parent: Some("task-b".into()),
        subagent: false,
    };
    shim.says(vec![a_permission("chat-1"), sibling]).await;
    assert_eq!(asks_after(&h, pane, &mut link, 2).await, ["ask:chat-1"]);
}

/// Two asks open, the later answered: the earlier is still waiting, though
/// the answer's `Resolved` folded the pane to Working.
#[tokio::test]
async fn answering_the_later_of_two_asks_leaves_the_earlier_listed() {
    let h = start(Scope::Control).await;
    let (pane, mut shim, mut link) = a_chat(&h).await;
    shim.says(vec![a_permission("chat-1"), a_permission("chat-2")]).await;
    assert_eq!(asks_after(&h, pane, &mut link, 2).await, ["ask:chat-2"]);
    let mut answer = request("terminal.agent_answer");
    answer.payload = Some(farcooler_protocol::v1::request::Payload::AgentAnswer(farcooler_protocol::v1::AgentAnswer {
        terminal_id: bytes::Bytes::copy_from_slice(pane.as_bytes()),
        request_id: "chat-2".into(),
        option_id: "allow".into(),
    }));
    let _ = link.call(answer).await;
    assert!(matches!(shim.heard().await, farcooler_agent::link::DaemonMessage::Answer { .. }));
    assert_eq!(asks_after(&h, pane, &mut link, 3).await, ["ask:chat-1"]);
}

/// A turn that ended, answered or cancelled, took its asks with it.
#[tokio::test]
async fn an_ask_ends_with_its_turn() {
    let h = start(Scope::Control).await;
    let (pane, mut shim, mut link) = a_chat(&h).await;
    shim.says(vec![
        a_permission("chat-1"),
        AgentEvent::TurnEnded { reason: farcooler_agent::event::EndReason::Cancelled },
    ])
    .await;
    assert_eq!(asks_after(&h, pane, &mut link, 2).await, Vec::<String>::new());
}
