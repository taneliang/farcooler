//! Every member sees the plan and changes it (the owner, 24 Oct, ov-736,
//! reversing ov-637): a member's device reads every part of the plan the
//! owner's does, at Read and at Control, and makes every plan-layer write,
//! filed under their name. A released device makes none, `not_yours`.
//! Through `Rpc`, where a client reaches it.

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

/// `method` with `payload`: refused `not_yours` to a released phone,
/// saying `user` or nothing, then done by `by`: Bob's phone or the owner's,
/// as `user`, or the orchestrator's CLI. Its answer.
async fn written(r: &Runner, method: &str, payload: impl Fn(&str) -> request::Payload, by: Option<&str>) -> result::Value {
    for actor in ["user", ""] {
        let refused = call(&factory(&r.svc, RELEASED, Scope::Control), method, payload(actor)).await;
        assert_eq!(refused.err().as_deref(), Some(NOT_YOURS), "a released phone as {actor:?} on {method}");
    }
    let actor = match by {
        LOCAL => "manager",
        _ => "user",
    };
    match call(&factory(&r.svc, by, Scope::Control), method, payload(actor)).await {
        Ok(Some(value)) => value,
        other => panic!("{by:?} as {actor:?} on {method}: {other:?}"),
    }
}


/// What `write_the_plan` made.
struct Written {
    theme: Uuid,
    task: Uuid,
}

/// Every plan-layer write, each refused to a released phone first: a theme
/// with its story, Next line and ask of the owner; a ticket on the line with
/// its branch and reason, ranked in the plan; an agent recorded on it, its
/// work ended and the agent ended; a ruling kept and one kept with the rest; a
/// landing started and moved on; a page set and removed. Bob makes most of
/// them; the owner's phone and the orchestrator make the rest, as before,
/// and the orchestrator's record (an agent, a landing's state) is theirs
/// alone (`a_member_writes_none_of_the_orchestrators_record`).
async fn write_the_plan(r: &Runner) -> Written {
    use request::Payload as P;
    let ws = || id_bytes(r.workspace);
    let task = r.svc.store.create_task(r.workspace, "Export: invoices as PDF", farcooler_store::models::Actor::Manager).unwrap().id;

    let created = written(r, "board_theme.create", |actor| {
        P::BoardThemeCreate(pb::BoardThemeCreate { workspace_id: ws(), name: "Exports".into(), outcome: "Every document leaves as a PDF.".into(), actor: actor.into(), ..Default::default() })
    }, BOB).await;
    let result::Value::BoardThemeView(view) = created else { panic!("{created:?}") };
    let theme = parse_id(&view.theme.unwrap().id).unwrap();
    written(r, "board_theme.update", |actor| {
        P::BoardThemeUpdate(pb::BoardThemeUpdate {
            theme_id: id_bytes(theme),
            story: Some("Invoices export; receipts and credit notes still print to paper, and the totals row rounds differently on each.".into()),
            next: Some("Receipts, once the rounding is settled.".into()),
            owner_ask: Some("Should credit notes carry the original invoice number?".into()),
            actor: actor.into(),
            ..Default::default()
        })
    }, LOCAL).await;
    written(r, "board_theme.cards", |actor| {
        P::BoardThemeCards(pb::BoardThemeCards { theme_id: id_bytes(theme), add: vec![id_bytes(task)], actor: actor.into(), ..Default::default() })
    }, BOB).await;

    let put = written(r, "line.set", |actor| {
        P::LineSet(pb::TicketLineSet { task_id: id_bytes(task), reason: Some("Invoices first: they're most of the paper.".into()), branch: Some("exports-pdf".into()), actor: actor.into(), ..Default::default() })
    }, LOCAL).await;
    let result::Value::TicketLine(line) = put else { panic!("{put:?}") };
    assert_eq!(parse_id(&line.task_id), Some(task));
    written(r, "line.set", |actor| {
        P::LineSet(pb::TicketLineSet { task_id: id_bytes(task), reason: Some("Invoices first, then receipts.".into()), actor: actor.into(), ..Default::default() })
    }, BOB).await;
    written(r, "line.order", |actor| P::LineOrder(pb::LineOrder { workspace_id: ws(), task_ids: vec![id_bytes(task)], actor: actor.into() }), BOB).await;
    let recorded = written(r, "agent.record", |actor| {
        P::AgentRecord(pb::AgentRecord {
            workspace_id: ws(),
            role: pb::AgentRole::Builder as i32,
            harness: "claude".into(),
            subagent_id: "a-1".into(),
            task_ids: vec![id_bytes(task)],
            actor: actor.into(),
            ..Default::default()
        })
    }, LOCAL).await;
    let result::Value::Agent(agent) = recorded else { panic!("{recorded:?}") };
    written(r, "agent.work", |actor| {
        P::AgentWork(pb::AgentWork { agent_id: agent.id.clone(), task_id: id_bytes(task), role: pb::WorkRole::Build as i32, verdict: Some("finished".into()), actor: actor.into() })
    }, OWNERS_PHONE).await;
    written(r, "agent.end", |actor| P::AgentEnd(pb::AgentEnd { agent_id: agent.id.clone(), verdict: "finished".into(), actor: actor.into() }), LOCAL).await;

    let add = |decision: &'static str| {
        move |actor: &str| {
            P::RulingAdd(pb::RulingAdd { workspace_id: ws(), decision: decision.into(), why: "The totals match the paper ones.".into(), reversal: "Round half up instead.".into(), theme_id: Some(id_bytes(theme)), actor: actor.into(), ..Default::default() })
        }
    };
    let added = written(r, "ruling.add", add("Round totals half to even"), BOB).await;
    let result::Value::BoardRuling(ruling) = added else { panic!("{added:?}") };
    written(r, "ruling.set", |actor| {
        P::RulingSet(pb::RulingSet { ruling_id: ruling.id.clone(), state: pb::BoardRulingState::Confirmed as i32, actor: actor.into(), ..Default::default() })
    }, BOB).await;
    written(r, "ruling.add", add("Show the currency on every line"), LOCAL).await;
    written(r, "ruling.keep_all", |actor| P::RulingKeepAll(pb::RulingKeepAll { workspace_id: ws(), actor: actor.into() }), BOB).await;
    written(r, "ruling.add", add("Keep the paper invoice for a release"), LOCAL).await;

    let started = written(r, "landing.start", |actor| {
        P::LandingStart(pb::LandingStart { workspace_id: ws(), route: pb::LandingRoute::Push as i32, base: "origin/main".into(), actor: actor.into(), ..Default::default() })
    }, LOCAL).await;
    let result::Value::Landing(landing) = started else { panic!("{started:?}") };
    written(r, "landing.set", |actor| {
        P::LandingSet(pb::LandingSet { landing_id: landing.id.clone(), state: pb::LandingState::Gating as i32, actor: actor.into(), ..Default::default() })
    }, OWNERS_PHONE).await;

    // The orchestrator's pages are the plan's too.
    let page = |actor: &str| {
        P::PageSet(pb::PageSet {
            workspace_id: ws(),
            slot: "exports".into(),
            doc_json: r#"{"v":1,"title":"Exports","summary":"Where the PDF exports stand","blocks":[{"type":"heading","text":"Invoices"}]}"#.into(),
            actor: actor.into(),
            ..Default::default()
        })
    };
    written(r, "page.set", page, LOCAL).await;
    written(r, "page.set", page, BOB).await;
    written(r, "page.remove", |actor| P::PageRemove(pb::PageRemove { workspace_id: ws(), slot: "exports".into(), actor: actor.into() }), BOB).await;
    written(r, "page.set", page, LOCAL).await;
    Written { theme, task }
}

/// Every plan-layer write is a member's to make (ov-736), and filed under
/// them: the theme, the ticket and the first ruling Bob made say
/// `member:bob`, saying `user` or nothing, never the owner's `user`. A
/// released phone makes none, and the owner's phone and the orchestrator's
/// CLI write as before.
#[tokio::test]
async fn a_member_changes_the_plan_under_their_name() {
    use pb::plan_events_request::Subject;
    let r = runner().await;
    let written = write_the_plan(&r).await;
    let bob = factory(&r.svc, BOB, Scope::Control);
    let quiet = request::Payload::LineSet(pb::TicketLineSet { task_id: id_bytes(written.task), stage: Some(pb::TicketStage::Building as i32), ..Default::default() });
    call(&bob, "line.set", quiet).await.expect("Bob saying nothing");

    for subject in [Subject::ThemeId(id_bytes(written.theme)), Subject::TaskId(id_bytes(written.task))] {
        let events = events_as(&r, OWNERS_PHONE, Scope::Control, subject.clone()).await;
        assert!(events.iter().any(|e| e.actor == "member:bob"), "{subject:?}: {events:?}");
        // The owner's phone ended the agent's work on the ticket, the
        // orchestrator's record; the theme's are Bob's and the orchestrator's
        // alone.
        if matches!(subject, Subject::ThemeId(_)) {
            assert!(events.iter().all(|e| e.actor != "user"), "{subject:?}: nothing of Bob's is the owner's: {events:?}");
        }
    }
    let ticket = events_as(&r, OWNERS_PHONE, Scope::Control, Subject::TaskId(id_bytes(written.task))).await;
    assert_eq!(ticket.last().map(|e| e.actor.as_str()), Some("member:bob"), "saying nothing is Bob too: {ticket:?}");
    let plan = plan_as(&r, OWNERS_PHONE, Scope::Control).await;
    let ruling = plan.rulings.iter().find(|ruling| ruling.decision == "Round totals half to even").expect("Bob's ruling");
    assert_eq!(ruling.actor, "member:bob");
    assert_eq!(ruling.state, pb::BoardRulingState::Confirmed as i32, "Bob kept it");
}

/// The capabilities the plan rides on (ov-268, ov-269, ov-304, ov-333, ov-872).
const PLAN_CAPABILITIES: [&str; 5] = {
    use farcooler_protocol::capability::*;
    [BOARD_PLAN, BOARD_PAGES, BOARD_RULINGS, BOARD_RULING_ACTIONS, BOARD_LINE]
};

/// Of `methods` (name, capability, scope), the plan's writes the gate lets a
/// member make: every method behind a plan capability that needs more than
/// Read, whatever it's called, that `writes_the_plan` doesn't name.
fn unrefused<'a>(methods: impl IntoIterator<Item = (&'a str, &'a str, Option<Scope>)>) -> Vec<&'a str> {
    methods
        .into_iter()
        .filter(|(_, capability, scope)| PLAN_CAPABILITIES.contains(capability) && *scope != Some(Scope::Read))
        .filter(|(name, _, _)| !writes_the_plan(&Request { method: (*name).into(), ..Default::default() }))
        .map(|(name, _, _)| name)
        .collect()
}

/// Every plan write in the protocol is refused a member: keyed on the
/// method's capability and scope, not its name, so a plan write added under
/// any name fails here until `writes_the_plan` names it. And the 15
/// `write_the_plan` makes are those.
#[test]
fn every_plan_write_is_covered() {
    use farcooler_protocol::method::Method as M;
    let all = M::ALL.iter().map(|m| (m.name(), m.capability(), crate::rpc::required_scope(m.name())));
    assert_eq!(unrefused(all), Vec::<&str>::new());
    let mut writes: Vec<&str> = M::ALL
        .iter()
        .filter(|m| PLAN_CAPABILITIES.contains(&m.capability()) && crate::rpc::required_scope(m.name()) != Some(Scope::Read))
        .map(|m| m.name())
        .collect();
    writes.sort_unstable();
    let mut made = vec![
        "board_theme.create", "board_theme.update", "board_theme.cards", "line.set", "line.order", "agent.record",
        "agent.work", "agent.end", "ruling.add", "ruling.set", "ruling.keep_all", "landing.start", "landing.set",
        "page.set", "page.remove",
    ];
    made.sort_unstable();
    assert_eq!(writes, made, "write_the_plan makes every plan write");
}

/// The check goes red on a plan write under a name no plan method has had,
/// and not on a read behind the same capability.
#[test]
fn a_plan_write_under_a_new_name_is_caught() {
    use farcooler_protocol::capability::{BOARD_LINE, BOARD_PLAN};
    let planted = [
        ("roadmap.reorder", BOARD_PLAN, Some(Scope::Control)),
        ("roadmap.get", BOARD_PLAN, Some(Scope::Read)),
        ("integrate.now", BOARD_LINE, Some(Scope::HostAdmin)),
        ("line.order", BOARD_LINE, Some(Scope::Control)),
    ];
    assert_eq!(unrefused(planted), ["roadmap.reorder", "integrate.now"]);
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
/// themes with their stories, Next lines and asks of the owner, the line with
/// its order, the rulings and the landings, and each theme's and ticket's
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
        assert_eq!(bob.lines.len(), 1, "at {scope:?}");
        assert_eq!(bob.lines[0].task_id, id_bytes(written.task), "at {scope:?}");
        assert_eq!(bob.lines[0].plan_rank, Some(1), "at {scope:?}");
        assert_eq!(bob.lines[0].work.len(), 1, "at {scope:?}");
        assert_eq!(bob.rulings.len(), 3, "at {scope:?}");
        assert_eq!(bob.landings.len(), 1, "at {scope:?}");
        assert_eq!(bob.cards.len(), 1, "at {scope:?}");

        use pb::plan_events_request::Subject;
        for subject in [Subject::ThemeId(id_bytes(written.theme)), Subject::TaskId(id_bytes(written.task))] {
            let seen = events_as(&r, BOB, scope, subject.clone()).await;
            assert!(!seen.is_empty(), "{subject:?} at {scope:?}");
            assert_eq!(seen, events_as(&r, OWNERS_PHONE, scope, subject).await, "at {scope:?}");
        }
    }
}

/// What posts to GitHub with the runner's own `gh` login, which is the
/// owner's: marking a ticket's draft pull request ready, replying to a review
/// thread (sent or only previewed), and turning on the nudge that comments on
/// a waiting pull request. A member's device and a released one are refused
/// `not_yours`; the owner's phone and the local socket pass the gate, to be
/// answered by the handler. Turning the nudge off is anyone's.
#[tokio::test]
async fn a_member_posts_nothing_as_the_owner() {
    use request::Payload as P;
    let r = runner().await;
    let req = |method: &str, payload: P| Request { method: method.into(), payload: Some(payload), ..Default::default() };
    let nudge = |on: bool| P::WorkspaceSetSettings(pb::WorkspaceSetSettings { pr_nudge: Some(on), ..Default::default() });
    let posts = [
        req("pr.ready", P::PrReady(pb::PrReady { task_id: id_bytes(Uuid::now_v7()), pr_number: 12 })),
        req("pr.thread_reply", P::PrThreadReply(pb::PrThreadReply { send: true, ..Default::default() })),
        req("pr.thread_reply", P::PrThreadReply(pb::PrThreadReply { send: false, ..Default::default() })),
        req("workspace.set_settings", nudge(true)),
    ];
    let peer = |device: Option<&str>| Peer { client_id: device.map(str::to_string), scope: Scope::Control };
    for post in &posts {
        for device in [BOB, RELEASED] {
            let refused = gate(&r.svc, &peer(device), None, post).err();
            assert!(matches!(refused, Some(DomainError::NotYours { what: "not_yours" })), "{device:?} on {}: {refused:?}", post.method);
        }
        for device in [OWNERS_PHONE, LOCAL] {
            assert!(gate(&r.svc, &peer(device), None, post).is_ok(), "{device:?} on {}", post.method);
        }
    }
    assert!(gate(&r.svc, &peer(BOB), None, &req("workspace.set_settings", nudge(false))).is_ok(), "Bob turns the nudge off");
}

/// The plan's record of the work stays the orchestrator's and the owner's
/// (ov-736, review M2; ov-872): Bob's phone moving a ticket to landing or
/// landed, giving it a landed commit, a branch, a checkout or a stack;
/// recording an agent, setting one to work or ending one; starting a
/// landing, changing its state, commit, base, tickets or lander; or marking a
/// reversal done with its commit, is refused `not_yours`; the owner's phone
/// and the local socket pass the gate. Bob still moves a ticket to waiting,
/// dropped, building, reviewing or needs you, gives it a reason, asks for a
/// reversal, and renames a landing, which only say what the plan is.
#[tokio::test]
async fn a_member_writes_none_of_the_orchestrators_record() {
    use pb::{LandingState, TicketStage};
    use request::Payload as P;
    let r = runner().await;
    let ticket = |edit: &dyn Fn(&mut pb::TicketLineSet)| {
        let mut p = pb::TicketLineSet { task_id: id_bytes(Uuid::now_v7()), actor: "user".into(), ..Default::default() };
        edit(&mut p);
        Request { method: "line.set".into(), payload: Some(P::LineSet(p)), ..Default::default() }
    };
    let landing = |state: LandingState, sha: Option<&str>| Request {
        method: "landing.set".into(),
        payload: Some(P::LandingSet(pb::LandingSet { state: state as i32, sha: sha.map(str::to_string), actor: "user".into(), ..Default::default() })),
        ..Default::default()
    };
    let landing_with = |edit: &dyn Fn(&mut pb::LandingSet)| {
        let mut p = pb::LandingSet { actor: "user".into(), ..Default::default() };
        edit(&mut p);
        Request { method: "landing.set".into(), payload: Some(P::LandingSet(p)), ..Default::default() }
    };
    let records = [
        ("landing", ticket(&|p| p.stage = Some(TicketStage::Landing as i32))),
        ("landed", ticket(&|p| p.stage = Some(TicketStage::Landed as i32))),
        ("a landed commit", ticket(&|p| p.landed_sha = Some("abc123".into()))),
        ("a ticket's branch", ticket(&|p| p.branch = Some("elsewhere".into()))),
        ("a ticket's checkout", ticket(&|p| p.worktree_path = Some("/elsewhere".into()))),
        ("a ticket's checkout by id", ticket(&|p| p.worktree_id = Some(id_bytes(Uuid::now_v7())))),
        ("a ticket's stack", ticket(&|p| p.stack_parent = Some(id_bytes(Uuid::now_v7())))),
        ("an agent recorded", Request {
            method: "agent.record".into(),
            payload: Some(P::AgentRecord(pb::AgentRecord { harness: "claude".into(), subagent_id: "a-1".into(), actor: "user".into(), ..Default::default() })),
            ..Default::default()
        }),
        ("an agent set to work", Request {
            method: "agent.work".into(),
            payload: Some(P::AgentWork(pb::AgentWork { actor: "user".into(), ..Default::default() })),
            ..Default::default()
        }),
        ("an agent ended", Request {
            method: "agent.end".into(),
            payload: Some(P::AgentEnd(pb::AgentEnd { verdict: "finished".into(), actor: "user".into(), ..Default::default() })),
            ..Default::default()
        }),
        ("a landing green", landing(LandingState::Green, None)),
        ("a landing landed", landing(LandingState::Landed, None)),
        ("a landing pushed", landing(LandingState::Pushed, None)),
        ("a landing gating", landing(LandingState::Gating, None)),
        ("a landing's commit", landing(LandingState::Unspecified, Some("abc123"))),
        // Integration itself (fix round 2): a landing started, its tickets,
        // base or lander, and a reversal marked done.
        ("a landing started", Request {
            method: "landing.start".into(),
            payload: Some(P::LandingStart(pb::LandingStart { title: "Exports".into(), actor: "user".into(), ..Default::default() })),
            ..Default::default()
        }),
        ("a landing's base", landing_with(&|p| p.base = Some("origin/release".into()))),
        ("a ticket put on a landing", landing_with(&|p| p.add_task_ids = vec![id_bytes(Uuid::now_v7())])),
        ("a ticket taken off a landing", landing_with(&|p| p.remove_task_ids = vec![id_bytes(Uuid::now_v7())])),
        ("a landing's lander", landing_with(&|p| p.lander_id = Some(id_bytes(Uuid::now_v7())))),
        ("a reversal marked done", Request {
            method: "ruling.set".into(),
            payload: Some(P::RulingSet(pb::RulingSet {
                state: pb::BoardRulingState::Reversed as i32,
                sha: Some("abc123".into()),
                actor: "user".into(),
                ..Default::default()
            })),
            ..Default::default()
        }),
    ];
    let peer = |device: Option<&str>| Peer { client_id: device.map(str::to_string), scope: Scope::Control };
    for (what, req) in &records {
        let refused = gate(&r.svc, &peer(BOB), None, req).err();
        assert!(matches!(refused, Some(DomainError::NotYours { what: "not_yours" })), "Bob: {what}: {refused:?}");
        for device in [OWNERS_PHONE, LOCAL] {
            assert!(gate(&r.svc, &peer(device), None, req).is_ok(), "{device:?}: {what}");
        }
    }
    let plans = [
        ("waiting", ticket(&|p| p.stage = Some(TicketStage::Waiting as i32))),
        ("dropped", ticket(&|p| p.stage = Some(TicketStage::Dropped as i32))),
        ("building", ticket(&|p| p.stage = Some(TicketStage::Building as i32))),
        ("reviewing", ticket(&|p| p.stage = Some(TicketStage::Reviewing as i32))),
        ("needs you", ticket(&|p| p.stage = Some(TicketStage::NeedsYou as i32))),
        ("a reason", ticket(&|p| p.reason = Some("Invoices first.".into()))),
        ("a reversal asked for", Request {
            method: "ruling.set".into(),
            payload: Some(P::RulingSet(pb::RulingSet { state: pb::BoardRulingState::Reversed as i32, actor: "user".into(), ..Default::default() })),
            ..Default::default()
        }),
        ("a landing renamed", landing_with(&|p| p.title = Some("Exports".into()))),
    ];
    for (what, req) in &plans {
        let passed = gate(&r.svc, &peer(BOB), None, req);
        assert!(passed.is_ok(), "Bob: {what}: {passed:?}");
    }
}

/// A member moving a ticket to review starts no landing and wakes nobody to
/// land it (ov-736, review M2): only the landing stage joins the collecting
/// landing, and the departure the orchestrator is told of comes from that.
/// The owner moves the ticket to building; Bob moves it to review, and the
/// board has no landing, nothing departed, and no notice queued for anyone.
#[tokio::test]
async fn a_member_moving_a_ticket_to_review_wakes_nobody() {
    use request::Payload as P;
    let r = runner().await;
    let task = r.svc.store.create_task(r.workspace, "Export: invoices as PDF", farcooler_store::models::Actor::Manager).unwrap().id;
    let put = P::LineSet(pb::TicketLineSet { task_id: id_bytes(task), branch: Some("exports-pdf".into()), actor: "manager".into(), ..Default::default() });
    call(&factory(&r.svc, LOCAL, Scope::Control), "line.set", put).await.expect("on the line");
    let to = |stage: pb::TicketStage| {
        P::LineSet(pb::TicketLineSet { task_id: id_bytes(task), stage: Some(stage as i32), actor: "user".into(), ..Default::default() })
    };
    call(&factory(&r.svc, OWNERS_PHONE, Scope::Control), "line.set", to(pb::TicketStage::Building)).await.expect("building");
    let notes_before = r.svc.store.notes_for(task, None).unwrap().len();
    call(&factory(&r.svc, BOB, Scope::Control), "line.set", to(pb::TicketStage::Reviewing)).await.expect("Bob moves it to review");
    let plan = plan_as(&r, OWNERS_PHONE, Scope::Control).await;
    assert_eq!(plan.lines[0].stage, pb::TicketStage::Reviewing as i32);
    assert!(plan.landings.is_empty(), "no landing: {:?}", plan.landings);
    assert!(r.svc.store.departures_untold(r.workspace).unwrap().is_empty(), "nothing departed");
    let notes: Vec<String> = r.svc.store.notes_for(task, None).unwrap().iter().map(|n| n.actor.to_string()).collect();
    assert_eq!(notes.len(), notes_before, "no notice on the card: {notes:?}");
    let refused = call(&factory(&r.svc, BOB, Scope::Control), "line.set", to(pb::TicketStage::Landing)).await;
    assert_eq!(refused.err().as_deref(), Some(NOT_YOURS), "landing is the orchestrator's");
}

/// A ticket that has reached landing is integration's (ov-736, fix round 2):
/// once the orchestrator moves it to landing, Bob's phone can't move it
/// anywhere, dropping included, since taking it off its landing can send the
/// next one off; nor once it's landed. He still gives it a reason. A ticket
/// that hasn't reached landing he drops, and the owner moves the landing one.
#[tokio::test]
async fn a_member_moves_no_ticket_that_reached_landing() {
    use pb::TicketStage as S;
    use request::Payload as P;
    let r = runner().await;
    let local = factory(&r.svc, LOCAL, Scope::Control);
    let bob = factory(&r.svc, BOB, Scope::Control);
    let ticket = |name: &str| r.svc.store.create_task(r.workspace, &format!("Export: {name}"), farcooler_store::models::Actor::Manager).unwrap().id;
    let to = |task: Uuid, stage: S| {
        P::LineSet(pb::TicketLineSet { task_id: id_bytes(task), stage: Some(stage as i32), actor: "user".into(), ..Default::default() })
    };
    let landing = ticket("invoices");
    let other = ticket("receipts");
    for stage in [S::Building, S::Reviewing, S::Landing] {
        call(&local, "line.set", to(landing, stage)).await.unwrap_or_else(|e| panic!("{stage:?}: {e}"));
    }
    for stage in [S::Dropped, S::Building, S::Reviewing, S::Waiting, S::NeedsYou, S::Landed] {
        assert_eq!(call(&bob, "line.set", to(landing, stage)).await.err().as_deref(), Some(NOT_YOURS), "Bob: landing to {stage:?}");
    }
    let reason = P::LineSet(pb::TicketLineSet { task_id: id_bytes(landing), reason: Some("PDF invoices".into()), actor: "user".into(), ..Default::default() });
    call(&bob, "line.set", reason).await.expect("Bob gives a landing ticket a reason");
    call(&local, "line.set", to(landing, S::Landed)).await.expect("the orchestrator lands it");
    assert_eq!(call(&bob, "line.set", to(landing, S::Dropped)).await.err().as_deref(), Some(NOT_YOURS), "Bob: a landed ticket");
    call(&bob, "line.set", to(other, S::Building)).await.expect("Bob builds a waiting ticket");
    call(&bob, "line.set", to(other, S::Dropped)).await.expect("Bob drops a ticket short of landing");
}
