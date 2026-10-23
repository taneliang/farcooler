//! A member sees the plan and can't change it (ov-637, the owner's ruling):
//! a member's device reads every part of the plan the owner's does, at Read
//! and at Control, and every plan-layer write from it, or from a released
//! one, is refused `not_yours`. Through `Rpc`, where a client reaches it.

use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, Scope, response, result};

use super::*;
use crate::rpc::RpcFactory;
use crate::service::Service;
use crate::test_support::ScratchDir;
use crate::wire::{id_bytes, parse_id};

/// A runner with Bob, a member whose phone is `phone-b`, and Carol, who was
/// one and whose `phone-c` is nobody's now.
struct Runner {
    _dir: ScratchDir,
    svc: Arc<Service>,
    workspace: Uuid,
}

async fn runner() -> Runner {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    svc.store.set_member("carol", "Carol", &["phone-c".to_string()]).unwrap();
    svc.store.remove_member("carol").unwrap();
    Runner { _dir: dir, svc, workspace }
}

const BOB: Option<&str> = Some("phone-b");
const RELEASED: Option<&str> = Some("phone-c");
const OWNERS_PHONE: Option<&str> = Some("phone-o");
/// The runner's local socket, where the orchestrator's CLI calls from.
const LOCAL: Option<&str> = None;

fn factory(svc: &Arc<Service>, client_id: Option<&str>, scope: Scope) -> RpcFactory {
    let peer = Peer { client_id: client_id.map(str::to_string), scope };
    RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer)
}

/// The answer, or the refusal as `<code word>:<what>`.
async fn call(rpc: &RpcFactory, method: &str, payload: request::Payload) -> std::result::Result<Option<result::Value>, String> {
    use farcooler_transport::Handler as _;
    let req = Request { method: method.into(), payload: Some(payload), ..Default::default() };
    match rpc.handle(req).await.outcome {
        Some(response::Outcome::Result(r)) => Ok(r.value),
        Some(response::Outcome::Error(e)) => Err(format!("{}:{}", farcooler_core::error::word_for(e.code), e.what)),
        other => panic!("no outcome: {other:?}"),
    }
}

const NOT_YOURS: &str = "not-yours:not_yours";

/// `method` with `payload`: refused `not_yours` to Bob's phone and to a
/// released one, saying `user` or nothing, then done by `by`, the owner's
/// phone or the orchestrator's CLI. Its answer.
async fn only_the_owner(r: &Runner, method: &str, payload: impl Fn(&str) -> request::Payload, by: Option<&str>) -> result::Value {
    for device in [BOB, RELEASED] {
        for actor in ["user", ""] {
            let refused = call(&factory(&r.svc, device, Scope::Control), method, payload(actor)).await;
            assert_eq!(refused.err().as_deref(), Some(NOT_YOURS), "{device:?} as {actor:?} on {method}");
        }
    }
    let actor = if by == LOCAL { "manager" } else { "user" };
    match call(&factory(&r.svc, by, Scope::Control), method, payload(actor)).await {
        Ok(Some(value)) => value,
        other => panic!("{by:?} on {method}: {other:?}"),
    }
}

/// What `write_the_plan` made.
struct Written {
    theme: Uuid,
    lane: Uuid,
}

/// Every plan-layer write, each refused to a member first: a theme with its
/// story, Next line and ask of the owner; a lane with cards and an agent,
/// ranked in the plan; a ruling kept and one kept with the rest; a train
/// started and moved on. The owner's phone and the orchestrator take turns.
async fn write_the_plan(r: &Runner) -> Written {
    use request::Payload as P;
    let ws = || id_bytes(r.workspace);
    let task = r.svc.store.create_task(r.workspace, "Export: invoices as PDF", farcooler_store::models::Actor::Manager).unwrap().id;

    let created = only_the_owner(r, "board_theme.create", |actor| {
        P::BoardThemeCreate(pb::BoardThemeCreate { workspace_id: ws(), name: "Exports".into(), outcome: "Every document leaves as a PDF.".into(), actor: actor.into(), ..Default::default() })
    }, OWNERS_PHONE).await;
    let result::Value::BoardThemeView(view) = created else { panic!("{created:?}") };
    let theme = parse_id(&view.theme.unwrap().id).unwrap();
    only_the_owner(r, "board_theme.update", |actor| {
        P::BoardThemeUpdate(pb::BoardThemeUpdate {
            theme_id: id_bytes(theme),
            story: Some("Invoices export; receipts and credit notes still print to paper, and the totals row rounds differently on each.".into()),
            next: Some("Receipts, once the rounding is settled.".into()),
            owner_ask: Some("Should credit notes carry the original invoice number?".into()),
            actor: actor.into(),
            ..Default::default()
        })
    }, LOCAL).await;
    only_the_owner(r, "board_theme.cards", |actor| {
        P::BoardThemeCards(pb::BoardThemeCards { theme_id: id_bytes(theme), add: vec![id_bytes(task)], actor: actor.into(), ..Default::default() })
    }, OWNERS_PHONE).await;

    let created = only_the_owner(r, "lane.create", |actor| {
        P::LaneCreate(pb::LaneCreate { workspace_id: ws(), name: "exports-pdf".into(), reason: "Invoices first: they're most of the paper.".into(), branch: "exports-pdf".into(), actor: actor.into(), ..Default::default() })
    }, LOCAL).await;
    let result::Value::Lane(lane) = created else { panic!("{created:?}") };
    let lane = parse_id(&lane.id).unwrap();
    only_the_owner(r, "lane.update", |actor| {
        P::LaneUpdate(pb::LaneUpdate { lane_id: id_bytes(lane), reason: Some("Invoices first, then receipts.".into()), actor: actor.into(), ..Default::default() })
    }, OWNERS_PHONE).await;
    only_the_owner(r, "lane.cards", |actor| {
        P::LaneCards(pb::LaneCards { lane_id: id_bytes(lane), add: vec![pb::LaneCard { task_id: id_bytes(task), ..Default::default() }], actor: actor.into(), ..Default::default() })
    }, LOCAL).await;
    only_the_owner(r, "lane.agent", |actor| {
        let agent = pb::LaneAgentRecord { harness: "claude".into(), agent_id: "a-1".into(), role: pb::LaneAgentRole::Build as i32, ..Default::default() };
        P::LaneAgentSet(pb::LaneAgentSet { lane_id: id_bytes(lane), agent: Some(agent), actor: actor.into() })
    }, OWNERS_PHONE).await;
    only_the_owner(r, "plan.set", |actor| P::PlanSet(pb::PlanSet { workspace_id: ws(), lane_ids: vec![id_bytes(lane)], actor: actor.into() }), LOCAL).await;

    let add = |decision: &'static str| {
        move |actor: &str| {
            P::RulingAdd(pb::RulingAdd { workspace_id: ws(), decision: decision.into(), why: "The totals match the paper ones.".into(), reversal: "Round half up instead.".into(), theme_id: Some(id_bytes(theme)), actor: actor.into(), ..Default::default() })
        }
    };
    let added = only_the_owner(r, "ruling.add", add("Round totals half to even"), OWNERS_PHONE).await;
    let result::Value::BoardRuling(ruling) = added else { panic!("{added:?}") };
    only_the_owner(r, "ruling.set", |actor| {
        P::RulingSet(pb::RulingSet { ruling_id: ruling.id.clone(), state: pb::BoardRulingState::Confirmed as i32, actor: actor.into(), ..Default::default() })
    }, OWNERS_PHONE).await;
    only_the_owner(r, "ruling.add", add("Show the currency on every line"), LOCAL).await;
    only_the_owner(r, "ruling.keep_all", |actor| P::RulingKeepAll(pb::RulingKeepAll { workspace_id: ws(), actor: actor.into() }), OWNERS_PHONE).await;
    only_the_owner(r, "ruling.add", add("Keep the paper invoice for a release"), LOCAL).await;

    let started = only_the_owner(r, "train.start", |actor| {
        P::TrainStart(pb::TrainStart { workspace_id: ws(), name: "train-1".into(), base: "origin/main".into(), actor: actor.into(), ..Default::default() })
    }, LOCAL).await;
    let result::Value::BoardTrain(train) = started else { panic!("{started:?}") };
    only_the_owner(r, "train.set", |actor| {
        P::TrainSet(pb::TrainSet { train_id: train.id.clone(), state: pb::BoardTrainState::Gating as i32, actor: actor.into(), ..Default::default() })
    }, OWNERS_PHONE).await;
    Written { theme, lane }
}

/// Every plan-layer write is refused a member's device and a released one,
/// as itself or saying nothing, and done by the owner's phone and the
/// orchestrator's CLI as before.
#[tokio::test]
async fn a_member_changes_nothing_in_the_plan() {
    let r = runner().await;
    write_the_plan(&r).await;
}

/// The plan-layer writes `write_the_plan` covers are every one there is: a
/// method added to the layer fails here until the gate, and the test, name it.
#[test]
fn every_plan_write_is_covered() {
    use farcooler_protocol::method::Method as M;
    let covered = [
        "plan.set", "board_theme.create", "board_theme.update", "board_theme.cards", "lane.create", "lane.update", "lane.cards",
        "lane.agent", "ruling.add", "ruling.set", "ruling.keep_all", "train.start", "train.set",
    ];
    let layer = |name: &str| ["plan.", "board_theme.", "lane.", "ruling.", "train."].iter().any(|p| name.starts_with(p));
    let writes: Vec<&str> = M::ALL.iter().map(|m| m.name()).filter(|m| layer(m) && !matches!(*m, "plan.get" | "plan.events")).collect();
    let mut sorted = covered.to_vec();
    sorted.sort_unstable();
    let mut found = writes.clone();
    found.sort_unstable();
    assert_eq!(found, sorted);
    for name in covered {
        let req = Request { method: name.into(), ..Default::default() };
        assert!(writes_the_plan(&req), "{name} isn't refused a member");
    }
}

/// The plan as `device` reads it at `scope`, the runner's clock left out.
async fn plan_as(r: &Runner, device: Option<&str>, scope: Scope) -> pb::Plan {
    let get = request::Payload::PlanGet(pb::PlanGetRequest { workspace_id: id_bytes(r.workspace), include_closed: true });
    match call(&factory(&r.svc, device, scope), "plan.get", get).await {
        Ok(Some(result::Value::Plan(plan))) => pb::Plan { now_ms: 0, ..plan },
        other => panic!("{device:?} at {scope:?}: {other:?}"),
    }
}

async fn events_as(r: &Runner, device: Option<&str>, scope: Scope, subject: pb::plan_events_request::Subject) -> Vec<pb::PlanEvent> {
    let events = request::Payload::PlanEvents(pb::PlanEventsRequest { subject: Some(subject), since_ms: 0 });
    match call(&factory(&r.svc, device, scope), "plan.events", events).await {
        Ok(Some(result::Value::PlanEventList(list))) => list.events,
        other => panic!("{device:?} at {scope:?}: {other:?}"),
    }
}

/// A member's device reads the whole plan, at Read and at Control: the
/// themes with their stories, Next lines and asks of the owner, the ranking,
/// the lanes, the rulings and the trains, and each theme's and lane's
/// history. The same plan the owner's phone reads at that scope.
#[tokio::test]
async fn a_member_reads_the_plan_the_owner_does() {
    let r = runner().await;
    let written = write_the_plan(&r).await;
    for scope in [Scope::Read, Scope::Control] {
        let bob = plan_as(&r, BOB, scope).await;
        assert_eq!(bob, plan_as(&r, OWNERS_PHONE, scope).await, "at {scope:?}");
        let theme = bob.themes[0].theme.as_ref().unwrap();
        assert_eq!(theme.owner_ask, "Should credit notes carry the original invoice number?", "at {scope:?}");
        assert!(theme.story.starts_with("Invoices export;"), "at {scope:?}");
        assert_eq!(theme.next, "Receipts, once the rounding is settled.", "at {scope:?}");
        assert_eq!(bob.order, vec![id_bytes(written.lane)], "at {scope:?}");
        assert_eq!(bob.lanes.len(), 1, "at {scope:?}");
        assert_eq!(bob.lanes[0].cards.len(), 1, "at {scope:?}");
        assert_eq!(bob.rulings.len(), 3, "at {scope:?}");
        assert_eq!(bob.trains.len(), 1, "at {scope:?}");
        assert_eq!(bob.cards.len(), 1, "at {scope:?}");

        use pb::plan_events_request::Subject;
        for subject in [Subject::ThemeId(id_bytes(written.theme)), Subject::LaneId(id_bytes(written.lane))] {
            let seen = events_as(&r, BOB, scope, subject.clone()).await;
            assert!(!seen.is_empty(), "{subject:?} at {scope:?}");
            assert_eq!(seen, events_as(&r, OWNERS_PHONE, scope, subject).await, "at {scope:?}");
        }
    }
}
