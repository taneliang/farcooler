//! Rulings (ov-304) as a client meets them: over a real socket, through the
//! real dispatch table and scope check, and read back in `plan.get`.

#[path = "support/in_process.rs"]
mod in_process;

use std::time::Duration;

use farcooler_protocol::capability::{BOARD_PLAN, BOARD_RULINGS};
use farcooler_protocol::v1::{self as pb, ErrorCode, Scope, event, request as payload, result};
use farcooler_store::models::Actor;
use farcooler_transport::{ClientError, request};
use in_process::*;
use uuid::Uuid;

fn id(uuid: Uuid) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(uuid.as_bytes())
}

async fn call(link: &mut Link, method: &str, capability: &str, p: payload::Payload) -> Result<result::Value, ClientError> {
    let mut r = request(method);
    r.required_capabilities = vec![capability.into()];
    r.payload = Some(p);
    Ok(link.call(r).await?.value.expect("a value"))
}

fn add(workspace: Uuid, decision: &str, tasks: &[Uuid]) -> payload::Payload {
    payload::Payload::RulingAdd(pb::RulingAdd {
        workspace_id: id(workspace),
        decision: decision.into(),
        why: "It's the one attention color.".into(),
        reversal: "One token.".into(),
        task_ids: tasks.iter().map(|t| id(*t)).collect(),
        theme_id: None,
        actor: "manager".into(),
    })
}

fn set(ruling: &pb::BoardRuling, state: pb::BoardRulingState, note: Option<&str>) -> payload::Payload {
    payload::Payload::RulingSet(pb::RulingSet {
        ruling_id: ruling.id.clone(),
        state: state as i32,
        note: note.map(str::to_string),
        actor: "manager".into(),
    })
}

async fn ruling(link: &mut Link, p: payload::Payload, method: &str) -> pb::BoardRuling {
    match call(link, method, BOARD_RULINGS, p).await.expect(method) {
        result::Value::BoardRuling(r) => r,
        other => panic!("wrong result: {other:?}"),
    }
}

async fn plan(link: &mut Link, workspace: Uuid, all: bool) -> pb::Plan {
    let p = payload::Payload::PlanGet(pb::PlanGetRequest { workspace_id: id(workspace), include_closed: all });
    match call(link, "plan.get", BOARD_PLAN, p).await.expect("plan.get") {
        result::Value::Plan(p) => p,
        other => panic!("wrong result: {other:?}"),
    }
}

/// The `plan_changed` and `task_changed` events on `listener` within `window`.
async fn events(listener: &mut Link, window: Duration) -> (usize, usize) {
    let (mut plans, mut tasks) = (0, 0);
    let deadline = tokio::time::Instant::now() + window;
    while let Ok(Ok(e)) = tokio::time::timeout_at(deadline, listener.next_event()).await {
        match e.payload {
            Some(event::Payload::PlanChanged(_)) => plans += 1,
            Some(event::Payload::TaskChanged(_)) => tasks += 1,
            _ => {}
        }
    }
    (plans, tasks)
}

/// Added, confirmed and read back in the plan, standing first; each write
/// announces `plan_changed` once and never `task_changed`.
#[tokio::test]
async fn a_ruling_round_trips_through_the_plan_and_announces() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let task = h.service.store.create_task(repo.workspace, "Mac: the inbox", Actor::User).unwrap();
    let mut a = connect(&h).await;
    let mut listener = connect(&h).await;

    let first = ruling(&mut a, add(repo.workspace, "The inbox is amber.", &[task.id]), "ruling.add").await;
    assert_eq!((first.number, first.state), (1, pb::BoardRulingState::Standing as i32));
    assert_eq!(first.task_ids, vec![id(task.id)]);
    let second = ruling(&mut a, add(repo.workspace, "The gutter is 12 points.", &[]), "ruling.add").await;
    let confirmed = ruling(&mut a, set(&first, pb::BoardRulingState::Confirmed, Some("Yes.")), "ruling.set").await;
    assert_eq!((confirmed.note.as_str(), confirmed.settled_by.as_deref()), ("Yes.", Some("manager")));
    assert!(confirmed.settled_at.is_some());

    let read = plan(&mut a, repo.workspace, false).await;
    let numbers: Vec<u32> = read.rulings.iter().map(|r| r.number).collect();
    assert_eq!(numbers, [second.number, first.number], "standing first");
    assert_eq!(read.rulings[1].task_keys, vec![task.key.clone()], "the ruling names its card by key");
    assert!(read.cards.is_empty(), "and leaves the plan's cards alone (review 1005a F2)");

    let (plans, tasks) = events(&mut listener, Duration::from_millis(400)).await;
    assert_eq!((plans, tasks), (3, 0));
}

/// Refusals arrive as the words the CLI maps to sentences; a reversed ruling
/// takes no more moves.
#[tokio::test]
async fn refusals_name_what_was_wrong() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let mut a = connect(&h).await;
    let refused = |r: Result<result::Value, ClientError>| match r {
        Err(ClientError::Daemon { code, what, .. }) => {
            assert_eq!(code, ErrorCode::InvalidArgument as i32);
            what
        }
        other => panic!("expected a refusal, got {other:?}"),
    };
    assert_eq!(refused(call(&mut a, "ruling.add", BOARD_RULINGS, add(repo.workspace, " ", &[])).await), "decision");
    let r = ruling(&mut a, add(repo.workspace, "A", &[]), "ruling.add").await;
    ruling(&mut a, set(&r, pb::BoardRulingState::Reversed, None), "ruling.set").await;
    let again = call(&mut a, "ruling.set", BOARD_RULINGS, set(&r, pb::BoardRulingState::Confirmed, None)).await;
    assert_eq!(refused(again), "ruling_state");
    let unset = call(&mut a, "ruling.set", BOARD_RULINGS, set(&r, pb::BoardRulingState::Unspecified, None)).await;
    assert_eq!(refused(unset), "state");
}

/// A read client sees rulings in the plan and can't write one.
#[tokio::test]
async fn a_read_client_reads_rulings_and_cannot_write_them() {
    let h = start(Scope::Read).await;
    let repo = a_repository(&h);
    let new = farcooler_store::rulings::NewRuling {
        decision: "A".into(),
        why: "B".into(),
        reversal: "C".into(),
        theme_id: None,
    };
    h.service.store.add_ruling(repo.workspace, &new, &[], Actor::Manager).unwrap();
    let mut link = connect(&h).await;
    assert_eq!(plan(&mut link, repo.workspace, true).await.rulings.len(), 1);
    match call(&mut link, "ruling.add", BOARD_RULINGS, add(repo.workspace, "B", &[])).await {
        Err(ClientError::Daemon { code, .. }) => assert_eq!(code, ErrorCode::ScopeDenied as i32),
        other => panic!("expected a scope denial, got {other:?}"),
    }
}
