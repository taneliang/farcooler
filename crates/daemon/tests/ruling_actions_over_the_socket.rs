//! The owner's marks on a ruling (ov-333) as a client meets them: over a real
//! socket, through the real dispatch table and scope check, and read back in
//! `plan.get`. Keep is one `ruling.set`, Keep All is `ruling.keep_all`, and a
//! reversal is the orchestrator's mark with its commit.

#[path = "support/in_process.rs"]
mod in_process;

use std::time::Duration;

use farcooler_protocol::capability::{BOARD_PLAN, BOARD_RULING_ACTIONS, BOARD_RULINGS};
use farcooler_protocol::v1::{self as pb, ErrorCode, Scope, event, request as payload, result};
use farcooler_store::models::Actor;
use farcooler_store::rulings::NewRuling;
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

fn open(h: &Harness, workspace: Uuid, decision: &str) -> Uuid {
    let new = NewRuling { decision: decision.into(), why: "Why.".into(), reversal: "Cheap.".into(), theme_id: None };
    h.service.store.add_ruling(workspace, &new, &[], Actor::Manager).unwrap().id
}

fn set(ruling: Uuid, state: pb::BoardRulingState, actor: &str, sha: Option<&str>) -> payload::Payload {
    payload::Payload::RulingSet(pb::RulingSet {
        ruling_id: id(ruling),
        state: state as i32,
        note: None,
        actor: actor.into(),
        sha: sha.map(str::to_string),
    })
}

fn keep_all(workspace: Uuid, actor: &str) -> payload::Payload {
    payload::Payload::RulingKeepAll(pb::RulingKeepAll { workspace_id: id(workspace), actor: actor.into() })
}

async fn plan(link: &mut Link, workspace: Uuid) -> pb::Plan {
    let p = payload::Payload::PlanGet(pb::PlanGetRequest { workspace_id: id(workspace), include_closed: false });
    match call(link, "plan.get", BOARD_PLAN, p).await.expect("plan.get") {
        result::Value::Plan(p) => p,
        other => panic!("wrong result: {other:?}"),
    }
}

async fn plan_events(listener: &mut Link, window: Duration) -> usize {
    let mut plans = 0;
    let deadline = tokio::time::Instant::now() + window;
    while let Ok(Ok(e)) = tokio::time::timeout_at(deadline, listener.next_event()).await {
        if matches!(e.payload, Some(event::Payload::PlanChanged(_))) {
            plans += 1;
        }
    }
    plans
}

fn refused(r: Result<result::Value, ClientError>) -> String {
    match r {
        Err(ClientError::Daemon { code, what, .. }) => {
            assert_eq!(code, ErrorCode::InvalidArgument as i32);
            what
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// Keep All keeps every open ruling in one request, as the owner, and
/// announces `plan_changed` once however many it kept; the plan then reads
/// them kept, with who and when, and nothing is open.
#[tokio::test]
async fn keep_all_keeps_every_open_ruling_and_announces_once() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let ids: Vec<Uuid> = ["A", "B", "C"].iter().map(|d| open(&h, repo.workspace, d)).collect();
    let mut a = connect(&h).await;
    let mut listener = connect(&h).await;

    let kept = match call(&mut a, "ruling.keep_all", BOARD_RULING_ACTIONS, keep_all(repo.workspace, "user")).await.unwrap() {
        result::Value::RulingsKept(k) => k.rulings,
        other => panic!("wrong result: {other:?}"),
    };
    assert_eq!(kept.len(), 3);
    let read = plan(&mut a, repo.workspace).await;
    assert!(read.rulings.iter().all(|r| r.state == pb::BoardRulingState::Confirmed as i32 && r.settled_by.as_deref() == Some("user")));
    assert!(read.rulings.iter().all(|r| ids.iter().any(|i| id(*i) == r.id)));
    assert_eq!(plan_events(&mut listener, Duration::from_millis(400)).await, 1, "one announcement for the lot");

    let again = match call(&mut a, "ruling.keep_all", BOARD_RULING_ACTIONS, keep_all(repo.workspace, "user")).await.unwrap() {
        result::Value::RulingsKept(k) => k.rulings,
        other => panic!("wrong result: {other:?}"),
    };
    assert!(again.is_empty(), "nothing left open");
    assert_eq!(plan_events(&mut listener, Duration::from_millis(300)).await, 0, "and nothing announced");
}

/// The orchestrator never keeps in bulk for the owner, and a refusal writes
/// nothing.
#[tokio::test]
async fn the_orchestrator_cannot_keep_all() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    open(&h, repo.workspace, "A");
    let mut a = connect(&h).await;
    for actor in ["manager", "agent:00000000-0000-0000-0000-000000000001"] {
        let said = refused(call(&mut a, "ruling.keep_all", BOARD_RULING_ACTIONS, keep_all(repo.workspace, actor)).await);
        assert_eq!(said, "actor", "{actor}");
    }
    assert_eq!(plan(&mut a, repo.workspace).await.rulings[0].state, pb::BoardRulingState::Standing as i32);
}

/// A reversal is marked with its commit, read back in the plan with it; a
/// malformed commit is refused by name; a commit on any other move is too.
#[tokio::test]
async fn a_reversal_carries_its_commit_through_the_plan() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let r = open(&h, repo.workspace, "A");
    let mut a = connect(&h).await;
    let rev = pb::BoardRulingState::Reversed;
    assert_eq!(refused(call(&mut a, "ruling.set", BOARD_RULINGS, set(r, rev, "manager", Some("nope"))).await), "reversed_sha");
    let kept = pb::BoardRulingState::Confirmed;
    assert_eq!(refused(call(&mut a, "ruling.set", BOARD_RULINGS, set(r, kept, "user", Some("abcd1234"))).await), "reversed_sha");
    let done = match call(&mut a, "ruling.set", BOARD_RULINGS, set(r, rev, "manager", Some("6E7E5618"))).await.unwrap() {
        result::Value::BoardRuling(b) => b,
        other => panic!("wrong result: {other:?}"),
    };
    assert_eq!((done.reversed_sha.as_deref(), done.settled_by.as_deref()), (Some("6e7e5618"), Some("manager")));
    let read = plan(&mut a, repo.workspace).await;
    assert_eq!(read.rulings[0].reversed_sha.as_deref(), Some("6e7e5618"));
}

/// A read client can't keep, one ruling or all.
#[tokio::test]
async fn a_read_client_cannot_keep() {
    let h = start(Scope::Read).await;
    let repo = a_repository(&h);
    let r = open(&h, repo.workspace, "A");
    let mut link = connect(&h).await;
    for (method, p) in [
        ("ruling.keep_all", keep_all(repo.workspace, "user")),
        ("ruling.set", set(r, pb::BoardRulingState::Confirmed, "user", None)),
    ] {
        match call(&mut link, method, BOARD_RULING_ACTIONS, p).await {
            Err(ClientError::Daemon { code, .. }) => assert_eq!(code, ErrorCode::ScopeDenied as i32, "{method}"),
            other => panic!("{method}: expected a scope denial, got {other:?}"),
        }
    }
}

/// An open ruling asks nothing of the owner (ov-333): it adds nothing to Needs
/// You and wakes nothing, however many there are, and keeping them changes
/// nothing there either. The only news is `plan_changed`.
#[tokio::test]
async fn open_rulings_never_reach_needs_you_or_a_notification() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let mut a = connect(&h).await;
    let mut listener = connect(&h).await;
    let needs = |value: result::Value| match value {
        result::Value::NeedsYouList(l) => l.items.len(),
        other => panic!("wrong result: {other:?}"),
    };
    let before = needs(a.call(request("needs_you.list")).await.unwrap().value.unwrap());
    for decision in ["A", "B", "C"] {
        open(&h, repo.workspace, decision);
    }
    // Written through the store, so the daemon's own watchers see them as it
    // would a CLI's write: announce one the way the socket does.
    let added = payload::Payload::RulingAdd(pb::RulingAdd {
        workspace_id: id(repo.workspace),
        decision: "D".into(),
        why: "Why.".into(),
        reversal: "Cheap.".into(),
        task_ids: vec![],
        theme_id: None,
        actor: "manager".into(),
    });
    call(&mut a, "ruling.add", BOARD_RULINGS, added).await.expect("ruling.add");
    call(&mut a, "ruling.keep_all", BOARD_RULING_ACTIONS, keep_all(repo.workspace, "user")).await.expect("keep all");
    assert_eq!(needs(a.call(request("needs_you.list")).await.unwrap().value.unwrap()), before, "rulings are not Needs You");
    let mut other = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(400);
    while let Ok(Ok(e)) = tokio::time::timeout_at(deadline, listener.next_event()).await {
        if !matches!(e.payload, Some(event::Payload::PlanChanged(_))) {
            other += 1;
        }
    }
    assert_eq!(other, 0, "nothing but plan_changed is announced for a ruling");
}

/// Keeping is the owner's mark on every path: a `ruling.set` to confirmed from
/// the orchestrator, or from an agent, is refused by name and writes nothing;
/// the owner's goes through, and reversing stays the orchestrator's.
#[tokio::test]
async fn only_the_owner_can_set_a_ruling_kept() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let r = open(&h, repo.workspace, "A");
    let mut a = connect(&h).await;
    let kept = pb::BoardRulingState::Confirmed;
    for actor in ["manager", "agent:00000000-0000-0000-0000-000000000001"] {
        let said = refused(call(&mut a, "ruling.set", BOARD_RULINGS, set(r, kept, actor, None)).await);
        assert_eq!(said, "actor", "{actor}");
    }
    assert_eq!(plan(&mut a, repo.workspace).await.rulings[0].state, pb::BoardRulingState::Standing as i32);
    call(&mut a, "ruling.set", BOARD_RULINGS, set(r, kept, "user", None)).await.expect("the owner keeps");
    let reversed = pb::BoardRulingState::Reversed;
    call(&mut a, "ruling.set", BOARD_RULINGS, set(r, reversed, "manager", None)).await.expect("the orchestrator reverses, with no commit");
}

/// Past Decisions and `plan ruling reverse` see every settled ruling when the
/// read asks for all (`include_closed`), however many there are.
#[tokio::test]
async fn a_read_for_all_sees_more_than_a_hundred_settled_rulings() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    for i in 0..103 {
        let id = open(&h, repo.workspace, &format!("R{i}"));
        h.service.store.set_ruling(id, farcooler_store::rulings::RulingState::Confirmed, None, Actor::User).unwrap();
    }
    let mut a = connect(&h).await;
    let all = payload::Payload::PlanGet(pb::PlanGetRequest { workspace_id: id(repo.workspace), include_closed: true });
    let read = match call(&mut a, "plan.get", BOARD_PLAN, all).await.unwrap() {
        result::Value::Plan(p) => p,
        other => panic!("wrong result: {other:?}"),
    };
    assert_eq!(read.rulings.len(), 103, "include_closed is all of them");
    assert_eq!(plan(&mut a, repo.workspace).await.rulings.len(), 100, "the ordinary read stays capped");
}
