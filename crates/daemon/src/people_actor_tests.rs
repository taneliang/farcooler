//! A member's device writes as its person, never as the owner, the
//! orchestrator or a pane (ov-706): `gate`'s rule on every write whose
//! payload names an `actor`, through `Rpc` where a client reaches it.

use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, Scope, response};
use farcooler_store::rulings::NewRuling;

use super::*;
use crate::rpc::RpcFactory;
use crate::service::Service;
use crate::test_support::ScratchDir;
use crate::wire::id_bytes;

/// A runner with Bob, a member whose phone is `phone-b`, and Carol, who was
/// one and whose `phone-c` is nobody's now.
struct Runner {
    _dir: ScratchDir,
    svc: Arc<Service>,
    repo: Uuid,
    workspace: Uuid,
}

async fn runner() -> Runner {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    svc.store.set_member("carol", "Carol", &["phone-c".to_string()]).unwrap();
    svc.store.remove_member("carol").unwrap();
    Runner { _dir: dir, svc, repo, workspace }
}

/// Bob's phone, the owner's own phone (a device bound to nobody), and the
/// runner's local socket, where the orchestrator's CLI calls from.
const BOB: Option<&str> = Some("phone-b");
const OWNERS_PHONE: Option<&str> = Some("phone-o");
const LOCAL: Option<&str> = None;

fn peer(client_id: Option<&str>) -> Peer {
    Peer { client_id: client_id.map(str::to_string), scope: Scope::Control }
}

fn factory(svc: &Arc<Service>, client_id: Option<&str>) -> RpcFactory {
    RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer(client_id))
}

/// The refusal as `<code word>:<what>`, or `Ok` for a result.
async fn call(factory: &RpcFactory, method: &str, payload: request::Payload) -> std::result::Result<(), String> {
    use farcooler_transport::Handler as _;
    let req = Request { method: method.into(), payload: Some(payload), ..Default::default() };
    match factory.handle(req).await.outcome {
        Some(response::Outcome::Result(_)) => Ok(()),
        Some(response::Outcome::Error(e)) => Err(format!("{}:{}", farcooler_core::error::word_for(e.code), e.what)),
        other => panic!("no outcome: {other:?}"),
    }
}

const ACTOR: &str = "invalid-argument:actor";
const NOT_YOURS: &str = "not-yours:not_yours";

/// Every write whose payload carries `actor`, claiming `actor`. Ids are left
/// empty: `gate` answers before a handler reads one.
fn every_write(actor: &str) -> Vec<Request> {
    use request::Payload as P;
    let a = || actor.to_string();
    let call = |method: &str, payload: P| Request { method: method.into(), payload: Some(payload), ..Default::default() };
    vec![
        call("terminal.agent_answer", P::AgentAnswer(pb::AgentAnswer { actor: a(), ..Default::default() })),
        call("task.create", P::TaskCreate(pb::TaskCreate { actor: a(), ..Default::default() })),
        call("task.update", P::TaskUpdate(pb::TaskUpdate { actor: a(), ..Default::default() })),
        call("task.set_status", P::TaskSetStatus(pb::TaskSetStatus { actor: a(), ..Default::default() })),
        call("task.note", P::TaskNoteAppend(pb::TaskNoteAppend { actor: a(), ..Default::default() })),
        call("task.block", P::TaskBlockSet(pb::TaskBlockSet { actor: a(), ..Default::default() })),
        call("task.set_wait", P::TaskSetWait(pb::TaskSetWait { actor: a(), ..Default::default() })),
        call("task.set_line", P::TaskSetLine(pb::TaskSetLine { actor: a(), ..Default::default() })),
        call("task.worker", P::TaskWorkerSet(pb::TaskWorkerSet { actor: a(), ..Default::default() })),
        call("task.move", P::TaskMove(pb::TaskMove { actor: a(), ..Default::default() })),
        call("board_theme.create", P::BoardThemeCreate(pb::BoardThemeCreate { actor: a(), ..Default::default() })),
        call("board_theme.update", P::BoardThemeUpdate(pb::BoardThemeUpdate { actor: a(), ..Default::default() })),
        call("board_theme.cards", P::BoardThemeCards(pb::BoardThemeCards { actor: a(), ..Default::default() })),
        call("lane.create", P::LaneCreate(pb::LaneCreate { actor: a(), ..Default::default() })),
        call("lane.update", P::LaneUpdate(pb::LaneUpdate { actor: a(), ..Default::default() })),
        call("lane.cards", P::LaneCards(pb::LaneCards { actor: a(), ..Default::default() })),
        call("lane.agent", P::LaneAgentSet(pb::LaneAgentSet { actor: a(), ..Default::default() })),
        call("plan.set", P::PlanSet(pb::PlanSet { actor: a(), ..Default::default() })),
        call("ruling.add", P::RulingAdd(pb::RulingAdd { actor: a(), ..Default::default() })),
        call("ruling.set", P::RulingSet(pb::RulingSet { actor: a(), ..Default::default() })),
        call("ruling.keep_all", P::RulingKeepAll(pb::RulingKeepAll { actor: a(), ..Default::default() })),
        call("message.send", P::MessageSend(pb::MessageSend { actor: a(), ..Default::default() })),
        call("train.start", P::TrainStart(pb::TrainStart { actor: a(), ..Default::default() })),
        call("train.set", P::TrainSet(pb::TrainSet { actor: a(), ..Default::default() })),
        call("page.set", P::PageSet(pb::PageSet { actor: a(), ..Default::default() })),
        call("page.remove", P::PageRemove(pb::PageRemove { actor: a(), ..Default::default() })),
    ]
}

/// The request payloads in the proto that carry `string actor`, by message
/// name, so a write added later with one fails here until `every_write`,
/// and `wire_actor` with it, names it.
fn proto_writes_with_an_actor() -> Vec<String> {
    let proto = include_str!("../../../proto/farcooler.proto");
    let request = proto.split("\nmessage Request {").nth(1).expect("a Request message");
    let oneof = request.split("oneof payload {").nth(1).expect("a payload oneof");
    let oneof = &oneof[..oneof.find("\n  }").expect("the oneof's end")];
    let carries_actor = |name: &str| {
        let Some(body) = proto.split(&format!("\nmessage {name} {{")).nth(1) else { return false };
        body[..body.find("\n}").unwrap_or(body.len())].lines().any(|l| l.trim_start().starts_with("string actor ="))
    };
    oneof
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|ty| ty.chars().next().is_some_and(char::is_uppercase))
        .filter(|ty| carries_actor(ty))
        .map(str::to_string)
        .collect()
}

#[test]
fn every_write_that_names_an_actor_is_listed() {
    let in_proto = proto_writes_with_an_actor();
    assert_eq!(in_proto.len(), 26, "the proto's writes with an actor: {in_proto:?}");
    assert_eq!(every_write("user").len(), in_proto.len(), "every_write misses one of {in_proto:?}");
    for req in every_write("manager") {
        assert_eq!(wire_actor(&req), Some("manager"), "{} isn't read by wire_actor", req.method);
    }
}

/// A member's device, and a released one, claiming the orchestrator or a
/// pane is refused on every write; the owner's devices and the local socket
/// are not.
#[tokio::test]
async fn a_member_claims_no_one_else_on_any_write() {
    let r = runner().await;
    let pane = format!("agent:{}", Uuid::now_v7());
    for claimed in ["manager", pane.as_str()] {
        for req in every_write(claimed) {
            for device in [BOB, Some("phone-c")] {
                let refused = gate(&r.svc, &peer(device), None, &req).err();
                assert!(matches!(refused, Some(DomainError::InvalidArgument { what: "actor" })), "{device:?} as {claimed} on {}: {refused:?}", req.method);
            }
            for device in [OWNERS_PHONE, LOCAL] {
                let passed = gate(&r.svc, &peer(device), None, &req);
                assert!(passed.is_ok(), "{device:?} as {claimed} on {}: {passed:?}", req.method);
            }
        }
    }
    // Bob as himself passes the gate on every write: his handler answers.
    for req in every_write("user").into_iter().chain(every_write("")) {
        let keeps = matches!(req.method.as_str(), "ruling.set" | "ruling.keep_all");
        if !keeps {
            let passed = gate(&r.svc, &peer(BOB), None, &req);
            assert!(passed.is_ok(), "Bob as himself on {}: {passed:?}", req.method);
        }
    }
}

fn ruling(r: &Runner) -> Uuid {
    let new = NewRuling { decision: "Ship the export".into(), why: "It's ready".into(), reversal: "Pull it".into(), theme_id: None };
    r.svc.store.add_ruling(r.workspace, &new, &[], farcooler_store::models::Actor::Manager).unwrap().id
}

fn keep(ruling: Uuid, actor: &str) -> request::Payload {
    request::Payload::RulingSet(pb::RulingSet {
        ruling_id: id_bytes(ruling),
        state: pb::BoardRulingState::Confirmed as i32,
        actor: actor.into(),
        ..Default::default()
    })
}

/// Keeping a ruling is the owner's mark: Bob's phone can't make it, by name
/// or by leaving the actor out, one at a time or all at once. The owner's
/// phone and the local socket still do.
#[tokio::test]
async fn a_member_never_keeps_a_ruling_as_the_owner() {
    let r = runner().await;
    let keep_all = |actor: &str| {
        request::Payload::RulingKeepAll(pb::RulingKeepAll { workspace_id: id_bytes(r.workspace), actor: actor.into() })
    };
    let bob = factory(&r.svc, BOB);
    let first = ruling(&r);
    for actor in ["user", ""] {
        assert_eq!(call(&bob, "ruling.set", keep(first, actor)).await, Err(NOT_YOURS.into()), "Bob keeps as {actor:?}");
        assert_eq!(call(&bob, "ruling.keep_all", keep_all(actor)).await, Err(NOT_YOURS.into()), "Bob keeps all as {actor:?}");
    }
    assert_eq!(call(&bob, "ruling.set", keep(first, "manager")).await, Err(ACTOR.into()));
    assert_eq!(r.svc.store.ruling(first).unwrap().state, farcooler_store::rulings::RulingState::Standing, "nothing was kept");
    // Bob may still record a ruling, as himself.
    let add = request::Payload::RulingAdd(pb::RulingAdd {
        workspace_id: id_bytes(r.workspace),
        decision: "Hold the export".into(),
        why: "Legal hasn't read it".into(),
        reversal: "Ship it".into(),
        actor: "user".into(),
        ..Default::default()
    });
    assert_eq!(call(&bob, "ruling.add", add).await, Ok(()));

    assert_eq!(call(&factory(&r.svc, OWNERS_PHONE), "ruling.set", keep(first, "user")).await, Ok(()), "the owner's phone keeps one");
    let second = ruling(&r);
    assert_eq!(call(&factory(&r.svc, LOCAL), "ruling.set", keep(second, "user")).await, Ok(()), "the owner's Mac keeps one");
    ruling(&r);
    assert_eq!(call(&factory(&r.svc, OWNERS_PHONE), "ruling.keep_all", keep_all("user")).await, Ok(()), "the owner keeps all");
}

/// A plan write: Bob's phone can't order the lanes as the orchestrator, and
/// can as himself; the orchestrator's local CLI and the owner's phone are
/// as before.
#[tokio::test]
async fn a_member_writes_the_plan_only_as_himself() {
    let r = runner().await;
    let plan = |actor: &str| request::Payload::PlanSet(pb::PlanSet { workspace_id: id_bytes(r.workspace), lane_ids: vec![], actor: actor.into() });
    assert_eq!(call(&factory(&r.svc, BOB), "plan.set", plan("manager")).await, Err(ACTOR.into()));
    assert_eq!(call(&factory(&r.svc, BOB), "plan.set", plan("user")).await, Ok(()));
    assert_eq!(call(&factory(&r.svc, LOCAL), "plan.set", plan("manager")).await, Ok(()), "the orchestrator");
    assert_eq!(call(&factory(&r.svc, OWNERS_PHONE), "plan.set", plan("user")).await, Ok(()));
}

/// A task write: the same for a card made.
#[tokio::test]
async fn a_member_makes_a_card_only_as_himself() {
    let r = runner().await;
    let card = |actor: &str| {
        request::Payload::TaskCreate(pb::TaskCreate {
            repository_id: id_bytes(r.repo),
            workspace_id: Some(id_bytes(r.workspace)),
            title: "Export: invoices as PDF".into(),
            actor: actor.into(),
            ..Default::default()
        })
    };
    assert_eq!(call(&factory(&r.svc, BOB), "task.create", card("manager")).await, Err(ACTOR.into()));
    assert_eq!(call(&factory(&r.svc, BOB), "task.create", card(&format!("agent:{}", Uuid::now_v7()))).await, Err(ACTOR.into()));
    assert_eq!(call(&factory(&r.svc, BOB), "task.create", card("user")).await, Ok(()));
    assert_eq!(call(&factory(&r.svc, LOCAL), "task.create", card("manager")).await, Ok(()), "the orchestrator");
    assert_eq!(call(&factory(&r.svc, OWNERS_PHONE), "task.create", card("user")).await, Ok(()));
}

/// A page write: the same for a page set.
#[tokio::test]
async fn a_member_sets_a_page_only_as_himself() {
    let r = runner().await;
    let page = |actor: &str| {
        request::Payload::PageSet(pb::PageSet {
            workspace_id: id_bytes(r.workspace),
            slot: "notes".into(),
            doc_json: r#"{"v":1,"title":"Notes","summary":"s","blocks":[{"type":"heading","text":"Lanes"}]}"#.into(),
            actor: actor.into(),
            ..Default::default()
        })
    };
    assert_eq!(call(&factory(&r.svc, BOB), "page.set", page("manager")).await, Err(ACTOR.into()));
    assert_eq!(call(&factory(&r.svc, BOB), "page.set", page("user")).await, Ok(()));
    assert_eq!(call(&factory(&r.svc, LOCAL), "page.set", page("manager")).await, Ok(()), "the orchestrator");
    assert_eq!(call(&factory(&r.svc, OWNERS_PHONE), "page.set", page("user")).await, Ok(()));
}
