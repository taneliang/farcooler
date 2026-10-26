//! A released device does nothing (ov-712, ov-758): a device whose member
//! was removed is nobody's, so it isn't the owner either, and every call
//! from it is refused `not_yours`, its reads included (the owner, 25 Oct: a
//! removed person no longer sees the team's plan, board or Needs You).
//! Through `Rpc`, where a client reaches it.

use std::sync::Arc;

use farcooler_protocol::method::Method;
use farcooler_protocol::v1::{self as pb, Scope, response};

use super::*;
use crate::rpc::RpcFactory;
use crate::service::Service;
use crate::test_support::ScratchDir;
use crate::wire::id_bytes;

/// A runner with Bob, a member whose phone is `phone-b`, Carol, who was one
/// and whose `phone-c` is nobody's now, and a card on the board.
struct Runner {
    _dir: ScratchDir,
    svc: Arc<Service>,
    repo: Uuid,
    workspace: Uuid,
    task: Uuid,
}

async fn runner() -> Runner {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    svc.store.set_member("carol", "Carol", &["phone-c".to_string()]).unwrap();
    svc.store.remove_member("carol").unwrap();
    let task = svc.store.create_task(workspace, "Export: invoices as PDF", farcooler_store::models::Actor::Manager).unwrap().id;
    Runner { _dir: dir, svc, repo, workspace, task }
}

const RELEASED: Option<&str> = Some("phone-c");
const BOB: Option<&str> = Some("phone-b");
const OWNERS_PHONE: Option<&str> = Some("phone-o");
const LOCAL: Option<&str> = None;
const NOT_YOURS: &str = "not-yours:not_yours";

fn peer(client_id: Option<&str>) -> Peer {
    Peer { client_id: client_id.map(str::to_string), scope: Scope::Control }
}

fn factory(svc: &Arc<Service>, client_id: Option<&str>) -> RpcFactory {
    RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer(client_id))
}

/// The refusal as `<code word>:<what>`, or `Ok` for a result.
async fn call(factory: &RpcFactory, method: &str, payload: Option<request::Payload>) -> std::result::Result<(), String> {
    use farcooler_transport::Handler as _;
    let req = Request { method: method.into(), payload, ..Default::default() };
    match factory.handle(req).await.outcome {
        Some(response::Outcome::Result(_)) => Ok(()),
        Some(response::Outcome::Error(e)) => Err(format!("{}:{}", farcooler_core::error::word_for(e.code), e.what)),
        other => panic!("no outcome: {other:?}"),
    }
}

fn new_card(r: &Runner, actor: &str) -> request::Payload {
    request::Payload::TaskCreate(pb::TaskCreate {
        repository_id: id_bytes(r.repo),
        workspace_id: Some(id_bytes(r.workspace)),
        title: "Export: receipts as PDF".into(),
        actor: actor.into(),
        ..Default::default()
    })
}

fn moved(r: &Runner, actor: &str) -> request::Payload {
    request::Payload::TaskSetStatus(pb::TaskSetStatus {
        task_id: id_bytes(r.task),
        status: pb::TaskStatus::InProgress as i32,
        actor: actor.into(),
    })
}

fn note(r: &Runner, actor: &str) -> request::Payload {
    request::Payload::TaskNoteAppend(pb::TaskNoteAppend {
        task_id: id_bytes(r.task),
        kind: pb::TaskNoteKind::Comment as i32,
        body: "Receipts round the totals row differently.".into(),
        actor: actor.into(),
        ..Default::default()
    })
}

fn message(r: &Runner, actor: &str) -> request::Payload {
    request::Payload::MessageSend(pb::MessageSend {
        to: "orchestrator".into(),
        text: "Ship it Friday".into(),
        actor: actor.into(),
        workspace_id: id_bytes(r.workspace),
        ..Default::default()
    })
}

/// A released phone files no card, moves none, writes no note and sends no
/// message, saying `user` or nothing: each is refused `not_yours`, and the
/// board is as it was. Before ov-712 each was filed as `user`, which reads
/// as the owner.
#[tokio::test]
async fn a_released_device_writes_nothing_on_the_board() {
    let r = runner().await;
    let released = factory(&r.svc, RELEASED);
    let cards = || r.svc.store.list_tasks(farcooler_store::TaskScope::Workspace(r.workspace), None).unwrap().len();
    let before = cards();
    let status = r.svc.store.get_task(r.task).unwrap().status;
    for actor in ["user", ""] {
        assert_eq!(call(&released, "task.create", Some(new_card(&r, actor))).await, Err(NOT_YOURS.into()), "a card as {actor:?}");
        assert_eq!(call(&released, "task.set_status", Some(moved(&r, actor))).await, Err(NOT_YOURS.into()), "a move as {actor:?}");
        assert_eq!(call(&released, "task.note", Some(note(&r, actor))).await, Err(NOT_YOURS.into()), "a note as {actor:?}");
        assert_eq!(call(&released, "message.send", Some(message(&r, actor))).await, Err(NOT_YOURS.into()), "a message as {actor:?}");
    }
    assert_eq!(cards(), before, "no card filed");
    assert_eq!(r.svc.store.get_task(r.task).unwrap().status, status, "not moved");
    assert!(r.svc.store.notes_for(r.task, None).unwrap().iter().all(|n| n.kind != farcooler_store::models::NoteKind::Comment), "no note");
}

/// Every method but `member.list` is refused a released device at the gate,
/// whatever it carries, at any scope (ov-758). Before, the 27
/// `Ruling::Reads` methods passed. `member.list` answers it only that it
/// was released (`a_released_device_learns_only_that_it_was`).
#[tokio::test]
async fn a_released_device_is_refused_every_call() {
    let r = runner().await;
    for scope in [Scope::Read, Scope::Control, Scope::HostAdmin] {
        for method in Method::ALL {
            let req = Request { method: method.name().into(), ..Default::default() };
            let gated = gate(&r.svc, &Peer { client_id: RELEASED.map(str::to_string), scope }, None, &req);
            if *method == Method::MemberList {
                assert!(gated.is_ok(), "{gated:?}");
            } else {
                assert!(matches!(gated, Err(DomainError::NotYours { what: "not_yours" })), "{} at {scope:?}: {gated:?}", method.name());
            }
        }
    }
}

/// `member.list` tells a released device that it was, so its app can say
/// so, and nothing else: no member, not the runner's Claude account, not
/// the login switches. Bob's phone still reads them all.
#[tokio::test]
async fn a_released_device_learns_only_that_it_was() {
    use farcooler_transport::Handler as _;
    let r = runner().await;
    for scope in [Scope::Read, Scope::Control, Scope::HostAdmin] {
        let released = RpcFactory::new(
            r.svc.clone(),
            crate::watch::Watcher::new(r.svc.clone()),
            Arc::new(tokio::sync::Notify::new()),
            Peer { client_id: RELEASED.map(str::to_string), scope },
        );
        let answer = released.handle(Request { method: "member.list".into(), ..Default::default() }).await;
        let Some(response::Outcome::Result(pb::Result { value: Some(pb::result::Value::MemberList(list)) })) = answer.outcome else {
            panic!("an answer at {scope:?}: {answer:?}")
        };
        assert_eq!(list, pb::MemberList { you: "(released)".into(), ..Default::default() }, "at {scope:?}");
    }
    let bob = factory(&r.svc, BOB).handle(Request { method: "member.list".into(), ..Default::default() }).await;
    let Some(response::Outcome::Result(pb::Result { value: Some(pb::result::Value::MemberList(list)) })) = bob.outcome else { panic!("Bob's list") };
    assert_eq!((list.you.as_str(), list.members.len()), ("bob", 1), "Bob reads the members");
}

/// A released device that is still connected hears no event: not a pane's
/// title or question, a card, the plan. The owner's phone, Bob's and the
/// local socket hear them.
#[tokio::test]
async fn a_released_device_hears_no_events() {
    use farcooler_transport::Handler as _;
    let r = runner().await;
    let events = [
        pb::Event::default(),
        pb::Event { payload: Some(pb::event::Payload::TaskChanged(Default::default())), ..Default::default() },
    ];
    for event in &events {
        assert!(!factory(&r.svc, RELEASED).admits(event), "{event:?}");
        for device in [OWNERS_PHONE, BOB, LOCAL] {
            assert!(factory(&r.svc, device).admits(event), "{device:?} {event:?}");
        }
    }
}

/// The reads a team's work is seen by, each with a real payload.
fn reads(r: &Runner) -> Vec<(&'static str, Option<request::Payload>)> {
    let ws = || id_bytes(r.workspace);
    vec![
        ("task.list", Some(request::Payload::TaskList(pb::TaskListRequest { workspace_id: Some(ws()), ..Default::default() }))),
        ("task.get", Some(request::Payload::TaskGet(pb::TaskGetRequest { task_id: id_bytes(r.task), ..Default::default() }))),
        ("plan.get", Some(request::Payload::PlanGet(pb::PlanGetRequest { workspace_id: ws(), ..Default::default() }))),
        ("needs_you.list", None),
    ]
}

/// A removed member's phone reads nothing (ov-758): not the board, a card,
/// the plan or Needs You, at Read scope as at Control.
/// The same calls answer the owner's phone and Bob's, so it's the release
/// that refuses them.
#[tokio::test]
async fn a_released_device_reads_nothing() {
    let r = runner().await;
    for scope in [Scope::Read, Scope::Control] {
        let released = RpcFactory::new(
            r.svc.clone(),
            crate::watch::Watcher::new(r.svc.clone()),
            Arc::new(tokio::sync::Notify::new()),
            Peer { client_id: RELEASED.map(str::to_string), scope },
        );
        for (method, payload) in reads(&r) {
            assert_eq!(call(&released, method, payload).await, Err(NOT_YOURS.into()), "{method} at {scope:?}");
        }
    }
    for device in [OWNERS_PHONE, BOB] {
        let rpc = factory(&r.svc, device);
        for (method, payload) in reads(&r) {
            assert_eq!(call(&rpc, method, payload).await, Ok(()), "{device:?} {method}");
        }
    }
}

/// The owner's phone, the local socket and a member's phone still write: a
/// card, a move and a note go through, and a message passes the gate.
#[tokio::test]
async fn the_owner_and_a_member_still_write() {
    let r = runner().await;
    for (device, actor) in [(OWNERS_PHONE, "user"), (LOCAL, "manager"), (BOB, "user")] {
        let rpc = factory(&r.svc, device);
        assert_eq!(call(&rpc, "task.create", Some(new_card(&r, actor))).await, Ok(()), "{device:?} files");
        assert_eq!(call(&rpc, "task.note", Some(note(&r, actor))).await, Ok(()), "{device:?} notes");
        let req = Request { method: "message.send".into(), payload: Some(message(&r, actor)), ..Default::default() };
        assert!(gate(&r.svc, &peer(device), None, &req).is_ok(), "{device:?} messages");
    }
    assert_eq!(call(&factory(&r.svc, OWNERS_PHONE), "task.set_status", Some(moved(&r, "user"))).await, Ok(()), "the owner moves");
}
