//! `answered_for::stamp` (ov-737), through `task.note` as a device reaches
//! it: the call site is what a member's answer goes through.

use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, Request, Scope, request, response};
use farcooler_store::models::{Actor, NoteKind, TaskStatus};
use farcooler_transport::{Handler as _, Peer};
use uuid::Uuid;

use crate::rpc::RpcFactory;
use crate::service::Service;
use crate::test_support::ScratchDir;
use crate::wire::id_bytes;

struct Runner {
    _dir: ScratchDir,
    svc: Arc<Service>,
    workspace: Uuid,
}

/// Alice and Bob on their own phones; the owner's is `phone-o`, bound to nobody.
async fn runner() -> Runner {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    svc.store.set_member("alice", "Alice Chen", &["phone-a".to_string()]).unwrap();
    svc.store.set_member("bob", "Bob Okafor", &["phone-b".to_string()]).unwrap();
    Runner { _dir: dir, svc, workspace }
}

/// A decision waiting on an answer, on Bob's desk when `desk` names him.
fn decision(r: &Runner, desk: Option<&str>) -> Uuid {
    let task = r.svc.store.create_task(r.workspace, "Pick a PDF library", Actor::User).unwrap();
    r.svc.store.add_note(task.id, NoteKind::Question, Actor::Manager, "Which one?", serde_json::json!({})).unwrap();
    r.svc.store.set_task_status(task.id, TaskStatus::NeedsDecision, Actor::Manager).unwrap();
    if let Some(handle) = desk {
        r.svc.store.set_rep_desk(r.workspace, handle, task.id).unwrap();
    }
    task.id
}

/// The answer a device at `client` sends, with `extra` as the request's own.
async fn answer(r: &Runner, client: Option<&str>, task: Uuid, extra: &str) {
    let factory = RpcFactory::new(
        r.svc.clone(),
        crate::watch::Watcher::new(r.svc.clone()),
        Arc::new(tokio::sync::Notify::new()),
        Peer { client_id: client.map(str::to_string), scope: Scope::Control },
    );
    let note = pb::TaskNoteAppend {
        task_id: id_bytes(task),
        kind: pb::TaskNoteKind::Answer as i32,
        body: "pdfkit".into(),
        extra_json: extra.into(),
        actor: "user".into(),
        ..Default::default()
    };
    let req = Request { method: "task.note".into(), payload: Some(request::Payload::TaskNoteAppend(note)), ..Default::default() };
    match factory.handle(req).await.outcome {
        Some(response::Outcome::Result(_)) => {}
        other => panic!("not answered: {other:?}"),
    }
}

/// The answer's actor word and its `for`.
fn recorded(r: &Runner, task: Uuid) -> (String, Option<serde_json::Value>) {
    let notes = r.svc.store.notes_for(task, Some(NoteKind::Answer)).unwrap();
    let note = notes.last().expect("an answer");
    (note.actor.to_string(), note.extra.get("for").cloned())
}

#[tokio::test]
async fn a_member_answering_anothers_decision_records_whose_it_was() {
    let r = runner().await;
    let task = decision(&r, Some("bob"));
    answer(&r, Some("phone-a"), task, "").await;
    let (actor, whose) = recorded(&r, task);
    assert_eq!(actor, "member:alice");
    assert_eq!(whose, Some(serde_json::json!({ "handle": "bob", "name": "Bob Okafor" })));
}

#[tokio::test]
async fn the_owner_answering_a_members_decision_records_it_too() {
    let r = runner().await;
    let task = decision(&r, Some("bob"));
    answer(&r, None, task, "").await;
    let (actor, whose) = recorded(&r, task);
    assert_eq!(actor, "user");
    assert_eq!(whose, Some(serde_json::json!({ "handle": "bob", "name": "Bob Okafor" })));
}

#[tokio::test]
async fn nobodys_decision_records_no_one() {
    let r = runner().await;
    let task = decision(&r, None);
    answer(&r, Some("phone-a"), task, "").await;
    assert_eq!(recorded(&r, task).1, None, "anyone answers nobody's with nothing to say");
}

#[tokio::test]
async fn your_own_decision_records_no_one_else() {
    let r = runner().await;
    let task = decision(&r, Some("bob"));
    answer(&r, Some("phone-b"), task, "").await;
    assert_eq!(recorded(&r, task).1, None);
}

#[tokio::test]
async fn a_caller_cannot_say_whose_it_was() {
    let r = runner().await;
    let task = decision(&r, None);
    answer(&r, Some("phone-a"), task, r#"{"for": {"handle": "bob", "name": "Bob Okafor"}}"#).await;
    assert_eq!(recorded(&r, task).1, None, "a forged `for` is dropped");
}

#[tokio::test]
async fn a_solo_runner_records_none() {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    let r = Runner { _dir: dir, workspace: svc.store.ensure_main_workspace(repo).unwrap().id, svc };
    let task = decision(&r, None);
    answer(&r, None, task, "").await;
    assert_eq!(recorded(&r, task).1, None);
}
