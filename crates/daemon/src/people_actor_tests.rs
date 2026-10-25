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

/// The request payloads in the proto that carry an `actor` field, by
/// message name: plain, `optional` or `repeated`, on the payload itself or
/// on a message it holds, however deep. A write added later with one fails
/// `every_write_that_names_an_actor_is_listed` until `every_write`, and
/// `wire_actor` with it, names it.
fn proto_writes_with_an_actor(proto: &str) -> std::collections::BTreeSet<String> {
    let request = proto.split("\nmessage Request {").nth(1).expect("a Request message");
    let oneof = request.split("oneof payload {").nth(1).expect("a payload oneof");
    let oneof = &oneof[..oneof.find("\n  }").expect("the oneof's end")];
    oneof
        .lines()
        .filter_map(field)
        .map(|(ty, _)| ty)
        .filter(|ty| carries_actor(proto, ty, &mut Default::default()))
        .collect()
}

/// A message's body in `proto`, between its braces.
fn message_body<'a>(proto: &'a str, name: &str) -> Option<&'a str> {
    let body = proto.split(&format!("\nmessage {name} {{")).nth(1)?;
    Some(&body[..body.find("\n}").unwrap_or(body.len())])
}

/// A field line's type and name: `optional string actor = 3;` is
/// ("string", "actor").
fn field(line: &str) -> Option<(String, String)> {
    let line = line.trim_start();
    let line = line.strip_prefix("optional ").or_else(|| line.strip_prefix("repeated ")).unwrap_or(line);
    let mut words = line.split_whitespace();
    let (ty, name, eq) = (words.next()?, words.next()?, words.next()?);
    (eq == "=").then(|| (ty.to_string(), name.to_string()))
}

/// Whether message `name` has an `actor` field, itself or on a message it
/// holds; `seen` stops a message that holds itself.
fn carries_actor(proto: &str, name: &str, seen: &mut std::collections::HashSet<String>) -> bool {
    if !seen.insert(name.to_string()) {
        return false;
    }
    let Some(body) = message_body(proto, name) else { return false };
    body.lines().filter_map(field).any(|(ty, field_name)| field_name == "actor" || carries_actor(proto, &ty, seen))
}

/// The payload's message name, as its variant is named after it.
fn payload_name(req: &Request) -> String {
    let debug = format!("{:?}", req.payload.as_ref().expect("a payload"));
    debug[..debug.find('(').expect("a tuple variant")].to_string()
}

#[test]
fn every_write_that_names_an_actor_is_listed() {
    let in_proto = proto_writes_with_an_actor(include_str!("../../../proto/farcooler.proto"));
    let listed: std::collections::BTreeSet<String> = every_write("user").iter().map(payload_name).collect();
    assert_eq!(listed, in_proto, "every_write and the proto's writes with an actor differ");
    for req in every_write("manager") {
        assert_eq!(wire_actor(&req), Some("manager"), "{} isn't read by wire_actor", req.method);
    }
}

/// The proto reader finds an actor however it's written: `optional`, and on
/// a message a payload holds.
#[test]
fn the_proto_reader_finds_every_way_to_write_an_actor() {
    let proto = "\nmessage Request {\n  oneof payload {\n    Plain plain = 1;\n    Maybe maybe = 2;\n    Outer outer = 3;\n    Bare bare = 4;\n  }\n}\n\
        \nmessage Plain {\n  string actor = 1;\n}\n\nmessage Maybe {\n  optional string actor = 1;\n}\n\
        \nmessage Outer {\n  repeated Inner items = 1;\n}\n\nmessage Inner {\n  string actor = 2;\n}\n\
        \nmessage Bare {\n  string title = 1;\n  Bare again = 2;\n}\n";
    let found: Vec<String> = proto_writes_with_an_actor(proto).into_iter().collect();
    assert_eq!(found, ["Maybe", "Outer", "Plain"]);
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
    // Bob as himself passes the gate on every write, the plan's included
    // (ov-736, `people_plan_tests`): his handler answers.
    for req in every_write("user").into_iter().chain(every_write("")) {
        let passed = gate(&r.svc, &peer(BOB), None, &req);
        assert!(passed.is_ok(), "Bob as himself on {}: {passed:?}", req.method);
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

/// Keeping a ruling is a person's mark (ov-736): Bob's phone keeps one, by
/// name or leaving the actor out, one at a time or all at once, and it's
/// filed under him, never as the owner's; it can't as the orchestrator. He
/// reverses one and records one too. A released phone moves none. The
/// owner's phone and the local socket still keep.
#[tokio::test]
async fn a_member_keeps_a_ruling_as_himself() {
    use farcooler_store::rulings::RulingState;
    let r = runner().await;
    let keep_all = |actor: &str| {
        request::Payload::RulingKeepAll(pb::RulingKeepAll { workspace_id: id_bytes(r.workspace), actor: actor.into() })
    };
    let bob = factory(&r.svc, BOB);
    let released = factory(&r.svc, Some("phone-c"));
    let first = ruling(&r);
    assert_eq!(call(&released, "ruling.set", keep(first, "user")).await, Err(NOT_YOURS.into()), "a released phone keeps");
    assert_eq!(call(&released, "ruling.keep_all", keep_all("user")).await, Err(NOT_YOURS.into()), "a released phone keeps all");
    assert_eq!(r.svc.store.ruling(first).unwrap().state, RulingState::Standing, "nothing was moved");
    assert_eq!(call(&bob, "ruling.set", keep(first, "manager")).await, Err(ACTOR.into()));
    for actor in ["user", ""] {
        let one = ruling(&r);
        assert_eq!(call(&bob, "ruling.set", keep(one, actor)).await, Ok(()), "Bob keeps as {actor:?}");
        let kept = r.svc.store.ruling(one).unwrap();
        assert_eq!((kept.state, kept.settled_by.as_deref()), (RulingState::Confirmed, Some("member:bob")), "kept as Bob, saying {actor:?}");
        ruling(&r);
        assert_eq!(call(&bob, "ruling.keep_all", keep_all(actor)).await, Ok(()), "Bob keeps all as {actor:?}");
    }
    // Reversing one, which is final, and recording one are his too.
    let reversed = ruling(&r);
    let reverse = request::Payload::RulingSet(pb::RulingSet { ruling_id: id_bytes(reversed), state: pb::BoardRulingState::Reversed as i32, actor: "user".into(), ..Default::default() });
    assert_eq!(call(&bob, "ruling.set", reverse).await, Ok(()), "Bob reverses one");
    assert_eq!(r.svc.store.ruling(reversed).unwrap().state, RulingState::Reversed);
    let add = request::Payload::RulingAdd(pb::RulingAdd {
        workspace_id: id_bytes(r.workspace),
        decision: "Hold the export".into(),
        why: "Legal hasn't read it".into(),
        reversal: "Ship it".into(),
        actor: "user".into(),
        ..Default::default()
    });
    assert_eq!(call(&bob, "ruling.add", add).await, Ok(()), "Bob records one");
    let plan = r.svc.store.plan(r.workspace, 0).unwrap();
    let held = plan.rulings.iter().find(|r| r.decision == "Hold the export").expect("recorded");
    assert_eq!(held.actor, "member:bob", "filed under Bob, not the owner");

    let owners = ruling(&r);
    assert_eq!(call(&factory(&r.svc, OWNERS_PHONE), "ruling.set", keep(owners, "user")).await, Ok(()), "the owner's phone keeps one");
    assert_eq!(r.svc.store.ruling(owners).unwrap().settled_by.as_deref(), Some("user"), "as the owner");
    let second = ruling(&r);
    assert_eq!(call(&factory(&r.svc, LOCAL), "ruling.set", keep(second, "user")).await, Ok(()), "the owner's Mac keeps one");
    ruling(&r);
    assert_eq!(call(&factory(&r.svc, OWNERS_PHONE), "ruling.keep_all", keep_all("user")).await, Ok(()), "the owner keeps all");
}

/// A plan write: Bob's phone can't order the lanes as the orchestrator, and
/// as himself does (ov-736); the orchestrator's local CLI and the owner's
/// phone are as before.
#[tokio::test]
async fn a_member_orders_the_lanes_as_no_one() {
    let r = runner().await;
    let plan = |actor: &str| request::Payload::PlanSet(pb::PlanSet { workspace_id: id_bytes(r.workspace), lane_ids: vec![], actor: actor.into() });
    assert_eq!(call(&factory(&r.svc, BOB), "plan.set", plan("manager")).await, Err(ACTOR.into()));
    assert_eq!(call(&factory(&r.svc, BOB), "plan.set", plan("user")).await, Ok(()), "Bob as himself");
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

/// A page write: Bob's phone can't set a page as the orchestrator, and as
/// himself does, a page being the plan's, which is everyone's (ov-736).
#[tokio::test]
async fn a_member_sets_a_page_as_no_one() {
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
    assert_eq!(call(&factory(&r.svc, BOB), "page.set", page("user")).await, Ok(()), "the plan's, everyone's (ov-736)");
    assert_eq!(call(&factory(&r.svc, LOCAL), "page.set", page("manager")).await, Ok(()), "the orchestrator");
    assert_eq!(call(&factory(&r.svc, OWNERS_PHONE), "page.set", page("user")).await, Ok(()));
}

/// Nobody claims a member on the wire (ov-707): only the runner writes
/// `member:<handle>`, from the device a write came from. Not Bob as himself,
/// not the owner, not the orchestrator's CLI.
#[tokio::test]
async fn nobody_claims_a_member_on_the_wire() {
    let r = runner().await;
    for req in every_write("member:bob") {
        for device in [BOB, OWNERS_PHONE, LOCAL] {
            let refused = gate(&r.svc, &peer(device), None, &req).err();
            assert!(matches!(refused, Some(DomainError::InvalidArgument { what: "actor" })), "{device:?} on {}: {refused:?}", req.method);
        }
    }
}

/// The actor of `task`'s notes, oldest first.
fn written_by(r: &Runner, task: Uuid) -> Vec<String> {
    r.svc.store.notes_for(task, None).unwrap().iter().map(|n| n.actor.to_string()).collect()
}

/// A member's write is filed under them (ov-707): Bob's phone saying `user`,
/// or nothing, makes a card, comments and moves it as `member:bob`. The
/// owner's phone is `user` and the orchestrator `manager`, as before.
#[tokio::test]
async fn a_members_writes_are_filed_under_them() {
    let r = runner().await;
    let card = |actor: &str, title: &str| {
        request::Payload::TaskCreate(pb::TaskCreate {
            repository_id: id_bytes(r.repo),
            workspace_id: Some(id_bytes(r.workspace)),
            title: title.into(),
            actor: actor.into(),
            ..Default::default()
        })
    };
    let comment = |task: Uuid, actor: &str| {
        request::Payload::TaskNoteAppend(pb::TaskNoteAppend {
            task_id: id_bytes(task),
            kind: pb::TaskNoteKind::Comment as i32,
            body: "The totals row is off by a cent".into(),
            actor: actor.into(),
            ..Default::default()
        })
    };
    let moved = |task: Uuid, actor: &str| {
        request::Payload::TaskSetStatus(pb::TaskSetStatus { task_id: id_bytes(task), status: pb::TaskStatus::InProgress as i32, actor: actor.into() })
    };
    let card_titled = |title: &str| r.svc.store.list_tasks(farcooler_store::TaskScope::Workspace(r.workspace), None).unwrap().into_iter().find(|t| t.title == title).unwrap().id;
    for (device, actor, title, filed) in [
        (BOB, "user", "Export: invoices as PDF", "member:bob"),
        (BOB, "", "Export: receipts as PDF", "member:bob"),
        (OWNERS_PHONE, "user", "Export: refunds as PDF", "user"),
        (LOCAL, "manager", "Export: credit notes as PDF", "manager"),
    ] {
        let rpc = factory(&r.svc, device);
        assert_eq!(call(&rpc, "task.create", card(actor, title)).await, Ok(()), "{title}");
        let task = card_titled(title);
        assert_eq!(call(&rpc, "task.note", comment(task, actor)).await, Ok(()), "{title}");
        assert_eq!(call(&rpc, "task.set_status", moved(task, actor)).await, Ok(()), "{title}");
        let written = written_by(&r, task);
        assert_eq!(written.len(), 3, "{title}: {written:?}");
        assert!(written.iter().all(|w| w == filed), "{title}: {written:?}");
    }
}

/// A member's answer is still a person's (ov-707): filed under them, and
/// queued to wake the agent waiting on it, as the owner's is.
#[tokio::test]
async fn a_members_answer_is_a_persons() {
    let r = runner().await;
    let task = r.svc.store.create_task(r.workspace, "Pick a PDF library", farcooler_store::models::Actor::Manager).unwrap();
    let answer = request::Payload::TaskNoteAppend(pb::TaskNoteAppend {
        task_id: id_bytes(task.id),
        kind: pb::TaskNoteKind::Answer as i32,
        body: "pdfkit".into(),
        actor: "user".into(),
        ..Default::default()
    });
    assert_eq!(call(&factory(&r.svc, BOB), "task.note", answer).await, Ok(()));
    let wakes = r.svc.store.pending_answer_wakes().unwrap();
    assert_eq!(wakes.len(), 1, "queued for its agent");
    assert_eq!(wakes[0].actor.to_string(), "member:bob");
    use crate::watch::answer_wake::route::Kind;
    assert!(matches!(Kind::of_wake(&wakes[0]), Kind::Prompt), "typed as Bob's turn, not an agent's message");
}
