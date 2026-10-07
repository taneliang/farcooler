//! `needs_you_changed` reaches a connected client for every trigger spec §2.4
//! names, and a burst of changes reaches it once.
//!
//! Each client here is a second connection that only listens, as the Mac's
//! `farcooler events` and a phone's event stream do. Each trigger test is the
//! one that goes red when its `announce_needs_you` call is deleted.

#[path = "support/in_process.rs"]
mod in_process;

use std::time::Duration;

use farcooler_daemon::needs_you::Observation;
use farcooler_protocol::v1::{self as pb, AgentActivity, Scope, event, request as payload};
use farcooler_store::models::{Actor, NoteKind, TaskStatus};
use farcooler_transport::request;
use in_process::*;

/// How many `needs_you_changed` arrive on `listener` within `window`.
async fn needs_you_events(listener: &mut Link, window: Duration) -> usize {
    let mut seen = 0;
    let deadline = tokio::time::Instant::now() + window;
    while let Ok(Ok(e)) = tokio::time::timeout_at(deadline, listener.next_event()).await {
        if matches!(e.payload, Some(event::Payload::NeedsYouChanged(_))) {
            seen += 1;
        }
    }
    seen
}

/// Long enough for any debounce already running to have fired, so what
/// follows is heard on its own.
async fn quiet() {
    tokio::time::sleep(Duration::from_millis(400)).await;
}

fn id(uuid: uuid::Uuid) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(uuid.as_bytes())
}

#[tokio::test]
async fn settling_a_held_ask_announces_needs_you_changed() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let pane = a_pane(&h, repo.worktree, None);
    // The hook ingress's sink is what carries an ask into the ring, and it is
    // installed when the listener starts.
    h.service.resume_agent_listeners();
    let asks = h.service.hooks().asks().clone();
    let (ask, rx) = loop {
        let (ask, rx) = asks.hold(pane);
        let offer = farcooler_agent::event::AgentEvent::Permission {
            id: ask.clone(),
            tool_call: String::new(),
            options: vec![],
        };
        asks.offer(pane, &ask, offer);
        if h.service.agents().open_permission(pane).is_some() {
            break (ask, rx);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let hook = tokio::spawn(async move {
        let settled = rx.await.expect("an ending");
        if let Some(ack) = settled.ack {
            let _ = ack.send(());
        }
    });
    quiet().await;
    let mut listener = connect(&h).await;

    let mut answer = request("terminal.agent_answer");
    answer.payload = Some(payload::Payload::AgentAnswer(pb::AgentAnswer {
        terminal_id: id(pane),
        request_id: ask,
        option_id: "allow".into(), answers: Default::default(),
    }));
    let _ = connect(&h).await.call(answer).await;
    hook.await.unwrap();
    assert_eq!(needs_you_events(&mut listener, Duration::from_secs(1)).await, 1);
}

#[tokio::test]
async fn moving_a_task_into_needs_decision_announces_needs_you_changed() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let task = h.service.store.create_task(repo.workspace, "Pick a library", Actor::User).unwrap();
    let mut listener = connect(&h).await;

    let mut moved = request("task.set_status");
    moved.payload = Some(payload::Payload::TaskSetStatus(pb::TaskSetStatus {
        task_id: id(task.id),
        status: pb::TaskStatus::NeedsDecision as i32,
        actor: "manager".into(),
    }));
    connect(&h).await.call(moved).await.expect("task.set_status");
    assert_eq!(needs_you_events(&mut listener, Duration::from_secs(1)).await, 1);
}

#[tokio::test]
async fn answering_a_decision_announces_needs_you_changed() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let store = &h.service.store;
    let task = store.create_task(repo.workspace, "Pick a library", Actor::User).unwrap();
    store.add_note(task.id, NoteKind::Question, Actor::Manager, "Which?", serde_json::json!({})).unwrap();
    store.set_task_status(task.id, TaskStatus::NeedsDecision, Actor::Manager).unwrap();
    let mut listener = connect(&h).await;

    let mut answer = request("task.note");
    answer.payload = Some(payload::Payload::TaskNoteAppend(pb::TaskNoteAppend {
        task_id: id(task.id),
        kind: pb::TaskNoteKind::Answer as i32,
        body: "printpdf".into(),
        actor: "user".into(),
        ..Default::default()
    }));
    connect(&h).await.call(answer).await.expect("task.note");
    assert_eq!(needs_you_events(&mut listener, Duration::from_secs(1)).await, 1);
}

#[tokio::test]
async fn seeing_a_failed_turn_announces_needs_you_changed() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let pane = a_pane(&h, repo.worktree, None);
    h.watcher
        .observe_for_tests(
            pane,
            Observation {
                activity: AgentActivity::Done,
                state_since: now_millis(),
                turn_failed: true,
                command: "claude".into(),
                ..Observation::default()
            },
        )
        .await;
    let mut listener = connect(&h).await;

    let mut seen = request("terminal.seen");
    seen.target_resource_id = Some(id(pane));
    // The reply re-reads the pane through tmux, which this harness may not
    // have; the event is what is asked about.
    let _ = connect(&h).await.call(seen).await;
    assert_eq!(needs_you_events(&mut listener, Duration::from_secs(1)).await, 1);
}

#[tokio::test]
async fn ten_changes_inside_the_window_announce_once() {
    let h = start(Scope::Control).await;
    let mut listener = connect(&h).await;
    for _ in 0..10 {
        h.watcher.announce_needs_you();
    }
    assert_eq!(needs_you_events(&mut listener, Duration::from_secs(1)).await, 1, "a burst is one re-read");
    // And the window closed behind it: the next change is heard too.
    h.watcher.announce_needs_you();
    assert_eq!(needs_you_events(&mut listener, Duration::from_secs(1)).await, 1);
}

/// An item takes its task's board as its workspace, so moving the task moves
/// the item.
#[tokio::test]
async fn moving_a_task_to_another_board_announces_needs_you_changed() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let billing = h.service.store.create_workspace(repo.id, "Billing", "bil").unwrap();
    let task = h.service.store.create_task(repo.workspace, "Invoice PDF export", Actor::User).unwrap();
    let mut listener = connect(&h).await;

    let mut moved = request("task.move");
    moved.payload = Some(payload::Payload::TaskMove(pb::TaskMove {
        task_ids: vec![id(task.id)],
        workspace_id: id(billing.id),
        actor: "user".into(),
    }));
    connect(&h).await.call(moved).await.expect("task.move");
    assert_eq!(needs_you_events(&mut listener, Duration::from_secs(1)).await, 1);
}

/// An orchestrator's items are about its own terminal, so a role change
/// moves them.
#[tokio::test]
async fn setting_a_terminals_role_announces_needs_you_changed() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let pane = a_pane(&h, repo.worktree, None);
    let mut listener = connect(&h).await;

    let mut role = request("terminal.set_role");
    role.target_resource_id = Some(id(pane));
    role.payload = Some(payload::Payload::TerminalSetRole(pb::TerminalSetRole {
        role: pb::TerminalRole::Shell as i32,
    }));
    let _ = connect(&h).await.call(role).await;
    assert_eq!(needs_you_events(&mut listener, Duration::from_secs(1)).await, 1);
}
