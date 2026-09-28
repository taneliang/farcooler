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
