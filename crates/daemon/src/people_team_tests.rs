//! A team on one runner (ov-716's rules, the owner's, and ov-736): a member
//! makes a workspace, a member admin removes a member, a member answers and
//! stops another person's lane but doesn't type into it, restart it or
//! remove it, and shared views are anyone's. And a rep changes the plan for
//! its member, filed under them. Through `gate` and `Rpc`, where a client
//! reaches them.

use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, Scope, response, result};
use farcooler_store::models::{Actor, Terminal};

use super::*;
use crate::rpc::RpcFactory;
use crate::service::Service;
use crate::test_support::ScratchDir;
use crate::wire::{id_bytes, parse_id};

/// A runner with an agent pane the owner opened, and Bob and Carol,
/// members whose phones are `phone-b` and `phone-c`, and Dan, who was one
/// and whose `phone-d` is nobody's now.
struct Runner {
    dir: ScratchDir,
    svc: Arc<Service>,
    repo: Uuid,
    workspace: Uuid,
    agent: Terminal,
}

async fn runner() -> Runner {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    crate::reconcile::repository(&svc, repo).await.unwrap();
    let worktree = svc.store.list_worktrees_for_repository(repo).unwrap().remove(0);
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let agent = svc.create_terminal(worktree.id, "Agent 2", "claude").await.expect("an agent");
    svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    svc.store.set_member("carol", "Carol", &["phone-c".to_string()]).unwrap();
    svc.store.set_member("dan", "Dan", &["phone-d".to_string()]).unwrap();
    svc.store.remove_member("dan").unwrap();
    Runner { dir, svc, repo, workspace, agent }
}

const BOB: Option<&str> = Some("phone-b");
const RELEASED: Option<&str> = Some("phone-d");
const OWNERS_PHONE: Option<&str> = Some("phone-o");
const NOT_YOURS: &str = "not-yours:not_yours";

fn peer(device: Option<&str>, scope: Scope) -> Peer {
    Peer { client_id: device.map(str::to_string), scope }
}

fn factory(svc: &Arc<Service>, device: Option<&str>, scope: Scope) -> RpcFactory {
    RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer(device, scope))
}

/// The answer, or the refusal as `<code word>:<what>`.
async fn call(rpc: &RpcFactory, req: Request) -> std::result::Result<Option<result::Value>, String> {
    use farcooler_transport::Handler as _;
    match rpc.handle(req).await.outcome {
        Some(response::Outcome::Result(r)) => Ok(r.value),
        Some(response::Outcome::Error(e)) => Err(format!("{}:{}", farcooler_core::error::word_for(e.code), e.what)),
        other => panic!("no outcome: {other:?}"),
    }
}

/// The gate's word on `req` from `device` at Control: `None` when it passes.
fn gate_says(r: &Runner, device: Option<&str>, req: &Request) -> Option<String> {
    gate(&r.svc, &peer(device, Scope::Control), None, req).err().map(|e| match e {
        DomainError::NotYours { what } => format!("not-yours:{what}"),
        other => format!("{other:?}"),
    })
}

/// Record `pane` as running on `handle`'s login, as its first launch would
/// for a pane that member opened (`Store::set_terminal_login`).
pub(super) fn on_logins_of(svc: &Service, pane: Uuid, handle: &str) {
    let login_id = svc.store.ensure_member_login(handle).unwrap();
    let login = farcooler_store::claude_logins::TerminalLogin {
        handle: handle.into(),
        config_dir: format!("/r/claude/{login_id}"),
        login_id,
        shared: false,
    };
    svc.store.set_terminal_login(pane, &login).unwrap();
}

/// Every way a client acts on `pane`, by method: typing, the keys, the
/// draft, answering, and stopping, restarting, removing or recasting it.
fn acting_on(pane: Uuid) -> Vec<Request> {
    let id = || id_bytes(pane);
    let text = vec![pb::AgentPromptBlock { content: Some(pb::agent_prompt_block::Content::Text("ship it".into())) }];
    let prompt = || Some(request::Payload::AgentPrompt(pb::AgentPrompt { terminal_id: id(), blocks: text.clone(), hold_behind_dialog: false }));
    let key = || Some(request::Payload::AgentCancel(pb::AgentCancel { terminal_id: id() }));
    let call = |method: &str, payload| Request { method: method.into(), target_resource_id: Some(id()), payload, ..Default::default() };
    vec![
        call("terminal.write", Some(request::Payload::TerminalWrite(pb::TerminalWrite { payload: b"y\r".to_vec().into() }))),
        call("terminal.paste_file", Some(request::Payload::TerminalFilePut(pb::TerminalFilePut::default()))),
        call("terminal.draft_prompt", prompt()),
        call("terminal.tell", prompt()),
        call("terminal.compose", prompt()),
        call("terminal.send_now", key()),
        call("terminal.bring_draft", Some(request::Payload::BringDraft(pb::BringDraft { terminal_id: id(), ..Default::default() }))),
        call("terminal.draft_withdraw", Some(request::Payload::DraftWithdraw(pb::DraftWithdraw { terminal_id: id(), ..Default::default() }))),
        call("terminal.restart", None),
        call("terminal.remove", None),
        call("terminal.set_role", Some(request::Payload::TerminalSetRole(pb::TerminalSetRole { role: pb::TerminalRole::Shell as i32 }))),
        // Answering and stopping, which are anyone's.
        call("terminal.agent_answer", Some(request::Payload::AgentAnswer(pb::AgentAnswer { terminal_id: id(), ..Default::default() }))),
        call("terminal.interrupt", key()),
        call("terminal.stop", None),
    ]
}

/// What a member may do on another person's lane (ov-716 rule 5).
fn answers_or_stops(req: &Request) -> bool {
    matches!(req.method.as_str(), "terminal.agent_answer" | "terminal.interrupt" | "terminal.stop")
}

/// On the owner's lanes, and another member's, a member answers a prompt
/// and presses Stop, and nothing more: typing, pasting, drafting, sending
/// what's queued, the draft, restarting, removing and recasting are refused
/// `not_yours` (ov-716 rule 5). On a lane of their own they do it all, and
/// the owner does it all on anyone's. A released phone does none.
#[tokio::test]
async fn a_member_answers_and_stops_another_persons_lane_and_nothing_more() {
    let r = runner().await;
    let pane = r.agent.id;
    for whose in ["the owner's", "Carol's"] {
        if whose == "Carol's" {
            on_logins_of(&r.svc, pane, "carol");
        }
        for req in acting_on(pane) {
            let said = gate_says(&r, BOB, &req);
            if answers_or_stops(&req) {
                assert_eq!(said, None, "{} on {whose} lane", req.method);
            } else {
                assert_eq!(said.as_deref(), Some(NOT_YOURS), "{} on {whose} lane", req.method);
            }
            assert_eq!(gate_says(&r, RELEASED, &req).as_deref(), Some(NOT_YOURS), "{}: a released phone", req.method);
        }
    }
    // Carol's own, and the owner on hers.
    for req in acting_on(pane) {
        assert_eq!(gate_says(&r, Some("phone-c"), &req), None, "{}: Carol on her own lane", req.method);
        assert_eq!(gate_says(&r, OWNERS_PHONE, &req), None, "{}: the owner on Carol's lane", req.method);
        let socket = gate(&r.svc, &peer(None, Scope::HostAdmin), None, &req);
        assert!(socket.is_ok(), "{}: the owner's socket: {socket:?}", req.method);
    }
    // Through `Rpc`, the refusal is the one an app reads.
    let typed = acting_on(pane).remove(0);
    assert_eq!(call(&factory(&r.svc, BOB, Scope::Control), typed).await.err().as_deref(), Some(NOT_YOURS));
}

/// A member makes a workspace, which only adds (ov-716 rule 2); a released
/// phone can't.
#[tokio::test]
async fn a_member_makes_a_workspace() {
    let r = runner().await;
    let create = |name: &str, prefix: &str| Request {
        method: "workspace.create".into(),
        target_resource_id: Some(id_bytes(r.repo)),
        payload: Some(request::Payload::WorkspaceCreate(pb::WorkspaceCreate { name: name.into(), task_prefix: prefix.into() })),
        ..Default::default()
    };
    assert_eq!(member_ruling(farcooler_protocol::method::Method::WorkspaceCreate), Ruling::Members);
    assert_eq!(call(&factory(&r.svc, RELEASED, Scope::Control), create("Exports", "EXP")).await.err().as_deref(), Some(NOT_YOURS));
    let made = call(&factory(&r.svc, BOB, Scope::Control), create("Billing", "BIL")).await;
    assert!(made.is_ok(), "Bob makes one: {made:?}");
    let names: Vec<String> = r.svc.store.list_workspaces(Some(r.repo)).unwrap().into_iter().map(|w| w.name).collect();
    assert!(names.iter().any(|n| n == "Billing"), "{names:?}");
    assert!(!names.iter().any(|n| n == "Exports"), "{names:?}");
}

/// A member admin removes a member (ov-716 rule 4), as the owner does; a
/// member at Control can't, by scope, and a released phone can't at all.
#[tokio::test]
async fn a_member_admin_removes_a_member() {
    let r = runner().await;
    let remove = |handle: &str| Request {
        method: "member.remove".into(),
        payload: Some(request::Payload::MemberRemove(pb::MemberRemove { handle: handle.into() })),
        ..Default::default()
    };
    assert_eq!(member_ruling(farcooler_protocol::method::Method::MemberRemove), Ruling::Members);
    assert_eq!(call(&factory(&r.svc, BOB, Scope::Control), remove("carol")).await.err().as_deref(), Some("scope-denied:"));
    assert_eq!(call(&factory(&r.svc, RELEASED, Scope::HostAdmin), remove("carol")).await.err().as_deref(), Some(NOT_YOURS));
    assert!(r.svc.store.member("carol").is_ok(), "Carol is still a member");
    let removed = call(&factory(&r.svc, BOB, Scope::HostAdmin), remove("carol")).await;
    assert!(removed.is_ok(), "Bob, a runner admin, removes Carol: {removed:?}");
    assert!(r.svc.store.member("carol").is_err(), "Carol is gone");
}

/// A member starts only their own rep, runner admin or not; the owner
/// starts anyone's (ov-716 rule 3). `reps_tests` starts them through `Rpc`.
#[test]
fn only_the_owner_starts_someone_elses_rep() {
    let bob = Person::Member("bob".into());
    assert!(may_start_rep(&bob, "bob"));
    assert!(!may_start_rep(&bob, "carol"), "not Carol's, whatever Bob's scope");
    assert!(may_start_rep(&Person::Owner, "carol"));
    assert!(!may_start_rep(&Person::Released, "bob"));
}

/// The shared views, a worktree's review base, a stack's parent, the
/// tiling and the lanes' order and hiding, are anyone's to change (ov-716
/// rule 6): a member's phone passes the gate on each, and a released one
/// doesn't.
#[tokio::test]
async fn a_member_changes_the_shared_views() {
    use farcooler_protocol::method::Method as M;
    let r = runner().await;
    let views = [
        M::ChangesSetBase,
        M::StackSetParent,
        M::LayoutSplit,
        M::LayoutMove,
        M::LayoutResize,
        M::LayoutBreak,
        M::LayoutRename,
        M::LayoutViewport,
        M::LayoutPreset,
        M::LayoutCycle,
        M::LayoutFocus,
        M::LayoutZoom,
        M::LayoutSwap,
        M::LayoutGroupSelect,
        M::WorktreeHide,
        M::WorktreeUnhide,
        M::WorktreeReorder,
    ];
    for method in views {
        let req = Request { method: method.name().into(), ..Default::default() };
        assert_eq!(member_ruling(method), Ruling::Members, "{}", method.name());
        assert_eq!(gate_says(&r, BOB, &req), None, "{}: Bob's phone", method.name());
        assert_eq!(gate_says(&r, RELEASED, &req).as_deref(), Some(NOT_YOURS), "{}: a released phone", method.name());
    }
}

/// A rep changes the plan for its member (ov-736): from its pane, a theme
/// made as the rep is filed under its member, `member:bob`, never the
/// owner's `user` or the rep's pane; as the orchestrator, another pane,
/// `user` or nobody it's refused `actor`. The same write from another process
/// is left as it says.
#[tokio::test]
async fn a_reps_plan_write_is_its_members() {
    let r = runner().await;
    r.svc.store.bind_rep(r.agent.id, r.workspace, "bob").unwrap();
    let snapshot = r.svc.inventory.refresh().await;
    let pane_pid = snapshot.claimants(r.agent.id).into_iter().next().expect("the rep's pane").stamp.pid as i32;
    let in_rep = factory(&r.svc, None, Scope::HostAdmin).with_caller_pid(Some(pane_pid));
    let person = crate::test_support::Outside::a_person(r.dir.path(), 6593);
    let elsewhere = factory(&r.svc, None, Scope::HostAdmin).with_caller_pid(Some(person.pid));
    let theme = |name: &str, actor: &str| Request {
        method: "board_theme.create".into(),
        payload: Some(request::Payload::BoardThemeCreate(pb::BoardThemeCreate {
            workspace_id: id_bytes(r.workspace),
            name: name.into(),
            outcome: "Every document leaves as a PDF.".into(),
            actor: actor.into(),
            ..Default::default()
        })),
        ..Default::default()
    };
    let made_by = |value: Option<result::Value>| {
        let Some(result::Value::BoardThemeView(view)) = value else { panic!("{value:?}") };
        let id = parse_id(&view.theme.unwrap().id).unwrap();
        let events = r.svc.store.plan_events(farcooler_store::plan::Subject::Theme(id), 0).unwrap();
        events.first().map(|e| e.actor.clone()).unwrap_or_default()
    };
    assert_eq!(call(&in_rep, theme("Exports", "manager")).await.err().as_deref(), Some("invalid-argument:actor"), "not as the orchestrator");
    let other = format!("agent:{}", Uuid::now_v7());
    assert_eq!(call(&in_rep, theme("Exports", &other)).await.err().as_deref(), Some("invalid-argument:actor"), "not as another pane");
    // Nor as `user` or nobody, which a write the stamp missed would file as
    // the owner's (review M3).
    for actor in ["user", ""] {
        assert_eq!(call(&in_rep, theme("Receipts", actor)).await.err().as_deref(), Some("invalid-argument:actor"), "as {actor:?}");
    }
    let own = format!("agent:{}", r.agent.id);
    let made = call(&in_rep, theme("Exports", &own)).await.expect("as itself");
    assert_eq!(made_by(made), "member:bob", "the rep as itself is its member");
    // The stamp files what the gate found, and nothing it didn't.
    let theme_req = theme("Credit notes", &own);
    let rep = gate_rep(&r.svc, &peer(None, Scope::HostAdmin), Some(pane_pid), &theme_req).expect("passes");
    assert_eq!(rep, Some(r.agent.id), "the gate names the rep's pane");
    let stamped = stamp_rep(&r.svc.store, rep, theme_req.clone()).unwrap();
    assert_eq!(wire_actor(&stamped), Some("member:bob"));
    assert_eq!(wire_actor(&stamp_rep(&r.svc.store, None, theme_req).unwrap()), Some(own.as_str()), "no rep found, nothing stamped");
    let made = call(&elsewhere, theme("Refunds", "manager")).await.expect("the orchestrator elsewhere");
    assert_eq!(made_by(made), Actor::Manager.to_string(), "another process is left as it says");
}

/// From a rep's pane, through `Rpc`: keeping every ruling, keeping one, and
/// starting a train are refused `rep_messenger`, as the rep itself (ov-736).
/// Only what its gate's grammar reaches is a rep's (`people_rep_plan_tests`).
#[tokio::test]
async fn a_rep_keeps_nothing_and_starts_no_train() {
    let r = runner().await;
    r.svc.store.bind_rep(r.agent.id, r.workspace, "bob").unwrap();
    let snapshot = r.svc.inventory.refresh().await;
    let pane_pid = snapshot.claimants(r.agent.id).into_iter().next().expect("the rep's pane").stamp.pid as i32;
    let in_rep = factory(&r.svc, None, Scope::HostAdmin).with_caller_pid(Some(pane_pid));
    let own = format!("agent:{}", r.agent.id);
    let new = farcooler_store::rulings::NewRuling { decision: "Ship the export".into(), why: "It's ready".into(), reversal: "Pull it".into(), theme_id: None };
    let ruling = r.svc.store.add_ruling(r.workspace, &new, &[], Actor::Manager).unwrap().id;
    let ws = || id_bytes(r.workspace);
    let calls = [
        ("ruling.keep_all", request::Payload::RulingKeepAll(pb::RulingKeepAll { workspace_id: ws(), actor: own.clone() })),
        ("ruling.set", request::Payload::RulingSet(pb::RulingSet { ruling_id: id_bytes(ruling), state: pb::BoardRulingState::Confirmed as i32, actor: own.clone(), ..Default::default() })),
        ("train.start", request::Payload::TrainStart(pb::TrainStart { workspace_id: ws(), name: "train-1".into(), base: "origin/main".into(), actor: own.clone(), ..Default::default() })),
    ];
    for (method, payload) in calls {
        let said = call(&in_rep, Request { method: method.into(), payload: Some(payload), ..Default::default() }).await;
        assert_eq!(said.err().as_deref(), Some("resource-conflict:rep_messenger"), "{method}");
    }
    assert_eq!(r.svc.store.ruling(ruling).unwrap().state, farcooler_store::rulings::RulingState::Standing, "nothing kept");
}

/// A rep moves no lane that has reached landing (rereview N1): its member's
/// phone is refused that, so the rep, through `Rpc` from its pane, is
/// refused dropping a lane the orchestrator moved to landing, which would
/// take it off its train. A lane short of landing it still drops.
#[tokio::test]
async fn a_rep_drops_no_lane_that_reached_landing() {
    use pb::LaneState as S;
    let r = runner().await;
    r.svc.store.bind_rep(r.agent.id, r.workspace, "bob").unwrap();
    let snapshot = r.svc.inventory.refresh().await;
    let pane_pid = snapshot.claimants(r.agent.id).into_iter().next().expect("the rep's pane").stamp.pid as i32;
    let in_rep = factory(&r.svc, None, Scope::HostAdmin).with_caller_pid(Some(pane_pid));
    let local = factory(&r.svc, None, Scope::HostAdmin);
    let own = format!("agent:{}", r.agent.id);
    let lane = |name: &str| {
        let task = r.svc.store.create_task(r.workspace, &format!("Export: {name}"), Actor::Manager).unwrap().id;
        Request {
            method: "lane.create".into(),
            payload: Some(request::Payload::LaneCreate(pb::LaneCreate {
                workspace_id: id_bytes(r.workspace),
                name: name.into(),
                branch: name.into(),
                cards: vec![pb::LaneCard { task_id: id_bytes(task), ..Default::default() }],
                actor: "manager".into(),
                ..Default::default()
            })),
            ..Default::default()
        }
    };
    let to = |id: &[u8], state: S, actor: &str| Request {
        method: "lane.update".into(),
        payload: Some(request::Payload::LaneUpdate(pb::LaneUpdate {
            lane_id: id.to_vec().into(),
            state: Some(state as i32),
            actor: actor.into(),
            ..Default::default()
        })),
        ..Default::default()
    };
    let Ok(Some(result::Value::Lane(landing))) = call(&local, lane("invoices")).await else { panic!("a lane") };
    let Ok(Some(result::Value::Lane(other))) = call(&local, lane("receipts")).await else { panic!("a lane") };
    for state in [S::Building, S::Review, S::Landing] {
        call(&local, to(&landing.id, state, "manager")).await.unwrap_or_else(|e| panic!("{state:?}: {e}"));
    }
    assert_eq!(call(&in_rep, to(&landing.id, S::Dropped, &own)).await.err().as_deref(), Some(NOT_YOURS), "a landing lane");
    let still = r.svc.store.lane(Uuid::from_slice(&landing.id).unwrap()).unwrap().state;
    assert_eq!(still, farcooler_store::plan::LaneState::Landing, "still landing");
    call(&in_rep, to(&other.id, S::Dropped, &own)).await.expect("a queued lane it drops");
}

/// A rep changes only its own workspace's plan (rereview N3), as it
/// comments only on its own board's cards: from its pane, a theme made on
/// Ops, a theme of Ops' changed or given cards, an Ops lane changed or given
/// cards, Ops' ranking, a ruling recorded on Ops or one of Ops' reversed,
/// are refused `rep_messenger`, and nothing on Ops moves. The same on Main,
/// its own, passes.
#[tokio::test]
async fn a_rep_changes_only_its_own_workspaces_plan() {
    use request::Payload as P;
    let r = runner().await;
    r.svc.store.bind_rep(r.agent.id, r.workspace, "bob").unwrap();
    let snapshot = r.svc.inventory.refresh().await;
    let pane_pid = snapshot.claimants(r.agent.id).into_iter().next().expect("the rep's pane").stamp.pid as i32;
    let in_rep = factory(&r.svc, None, Scope::HostAdmin).with_caller_pid(Some(pane_pid));
    let local = factory(&r.svc, None, Scope::HostAdmin);
    let own = format!("agent:{}", r.agent.id);
    let ops = r.svc.store.create_workspace(r.repo, "Ops", "ops").unwrap().id;
    let send = |method: &str, payload: P| Request { method: method.into(), payload: Some(payload), ..Default::default() };
    let theme_on = |ws: Uuid, name: &str, actor: &str| {
        send("board_theme.create", P::BoardThemeCreate(pb::BoardThemeCreate { workspace_id: id_bytes(ws), name: name.into(), outcome: "Paged.".into(), actor: actor.into(), ..Default::default() }))
    };
    let Ok(Some(result::Value::BoardThemeView(view))) = call(&local, theme_on(ops, "Alerts", "manager")).await else { panic!("an Ops theme") };
    let ops_theme = view.theme.unwrap().id;
    let lane = send("lane.create", P::LaneCreate(pb::LaneCreate { workspace_id: id_bytes(ops), name: "pager".into(), actor: "manager".into(), ..Default::default() }));
    let Ok(Some(result::Value::Lane(ops_lane))) = call(&local, lane).await else { panic!("an Ops lane") };
    let new = farcooler_store::rulings::NewRuling { decision: "Page at night".into(), why: "Outages".into(), reversal: "Don't".into(), theme_id: None };
    let ops_ruling = r.svc.store.add_ruling(ops, &new, &[], Actor::Manager).unwrap().id;
    let reaches = [
        theme_on(ops, "Escalation", &own),
        send("board_theme.update", P::BoardThemeUpdate(pb::BoardThemeUpdate { theme_id: ops_theme.clone(), next: Some("Mine now".into()), actor: own.clone(), ..Default::default() })),
        send("board_theme.cards", P::BoardThemeCards(pb::BoardThemeCards { theme_id: ops_theme.clone(), actor: own.clone(), ..Default::default() })),
        send("lane.update", P::LaneUpdate(pb::LaneUpdate { lane_id: ops_lane.id.clone(), reason: Some("Mine now".into()), actor: own.clone(), ..Default::default() })),
        send("lane.cards", P::LaneCards(pb::LaneCards { lane_id: ops_lane.id.clone(), actor: own.clone(), ..Default::default() })),
        send("plan.set", P::PlanSet(pb::PlanSet { workspace_id: id_bytes(ops), lane_ids: vec![ops_lane.id.clone()], actor: own.clone() })),
        send("ruling.add", P::RulingAdd(pb::RulingAdd { workspace_id: id_bytes(ops), decision: "Never page".into(), why: "Sleep".into(), reversal: "Page".into(), actor: own.clone(), ..Default::default() })),
        send("ruling.set", P::RulingSet(pb::RulingSet { ruling_id: id_bytes(ops_ruling), state: pb::BoardRulingState::Reversed as i32, actor: own.clone(), ..Default::default() })),
    ];
    for req in reaches {
        let method = req.method.clone();
        assert_eq!(call(&in_rep, req).await.err().as_deref(), Some("resource-conflict:rep_messenger"), "{method} on Ops");
    }
    let plan = r.svc.store.plan(ops, 0).unwrap();
    assert_eq!(plan.themes.len(), 1, "no theme made on Ops");
    assert_eq!(plan.rulings.len(), 1, "no ruling recorded on Ops");
    assert_eq!(r.svc.store.ruling(ops_ruling).unwrap().state, farcooler_store::rulings::RulingState::Standing, "nothing reversed");
    call(&in_rep, theme_on(r.workspace, "Exports", &own)).await.expect("its own board");
}

/// The pid of `r.agent`'s pane, once `r.agent` is bound as Bob's rep.
async fn bobs_rep_pid(r: &Runner) -> i32 {
    r.svc.store.bind_rep(r.agent.id, r.workspace, "bob").unwrap();
    let snapshot = r.svc.inventory.refresh().await;
    snapshot.claimants(r.agent.id).into_iter().next().expect("the rep's pane").stamp.pid as i32
}

fn charter_request(workspace: Uuid) -> Request {
    Request { method: "workspace.charter".into(), target_resource_id: Some(id_bytes(workspace)), ..Default::default() }
}

fn charter_of(value: std::result::Result<Option<result::Value>, String>) -> (bool, String) {
    let Ok(Some(result::Value::WorkspaceCharter(c))) = value else { panic!("not a charter: {value:?}") };
    (c.exists, c.text)
}

/// Every person on the runner reads a workspace's charter (ov-755): the
/// owner's phone and the local socket, a member's phone, and a member's rep
/// from its pane, at Read scope. A device whose member was removed does not
/// (ov-753), though it still reads the plan. And a workspace with no
/// charter yet answers that, not an error.
#[tokio::test]
async fn everyone_on_the_runner_but_a_removed_member_reads_the_charter() {
    let r = runner().await;
    let path = crate::workspace_home::charter_path(r.svc.root_dir(), r.workspace);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let _ = std::fs::remove_file(&path);
    let nobody = factory(&r.svc, BOB, Scope::Read);
    assert_eq!(charter_of(call(&nobody, charter_request(r.workspace)).await), (false, String::new()), "none yet");

    std::fs::write(&path, "## Who decides\nThe lead.\n").unwrap();
    for (who, device, scope) in [("the owner's phone", OWNERS_PHONE, Scope::Read), ("Bob's phone", BOB, Scope::Read), ("the local socket", None, Scope::HostAdmin)] {
        let said = charter_of(call(&factory(&r.svc, device, scope), charter_request(r.workspace)).await);
        assert_eq!(said, (true, "## Who decides\nThe lead.\n".to_string()), "{who}");
    }
    let in_rep = factory(&r.svc, None, Scope::HostAdmin).with_caller_pid(Some(bobs_rep_pid(&r).await));
    assert_eq!(charter_of(call(&in_rep, charter_request(r.workspace)).await).1, "## Who decides\nThe lead.\n", "Bob's rep");

    for scope in [Scope::Read, Scope::Control] {
        let refused = call(&factory(&r.svc, RELEASED, scope), charter_request(r.workspace)).await;
        assert_eq!(refused.err().as_deref(), Some(NOT_YOURS), "a removed member's phone at {scope:?}");
    }
    let plan = Request {
        method: "plan.get".into(),
        payload: Some(request::Payload::PlanGet(pb::PlanGetRequest { workspace_id: id_bytes(r.workspace), include_closed: false })),
        ..Default::default()
    };
    assert!(call(&factory(&r.svc, RELEASED, Scope::Read), plan).await.is_ok(), "the plan is still read: a separate card");
    assert_eq!(call(&nobody, Request { target_resource_id: None, ..charter_request(r.workspace) }).await.err().as_deref(), Some("not-found:"));
}

/// Anyone moves a theme (ov-755): one ranking, the whole team's. The owner's
/// phone, Bob's phone, and Bob's rep from its pane each write a theme's
/// `ordinal`; a released phone is refused. The rep's write is filed under
/// Bob (`stamp_rep`), and the same write sneaking a budget in is refused.
#[tokio::test]
async fn the_owner_a_member_and_a_rep_move_a_theme() {
    let r = runner().await;
    let ids: Vec<Uuid> = ["Alpha", "Beta", "Gamma"]
        .iter()
        .map(|n| r.svc.store.create_theme(r.workspace, &farcooler_store::plan::NewTheme { name: n.to_string(), outcome: String::new() }, &[], Actor::Manager).unwrap().id)
        .collect();
    let ordinal = |i: usize| r.svc.store.board_theme(ids[i]).unwrap().ordinal;
    assert_eq!((ordinal(0), ordinal(1), ordinal(2)), (1, 2, 3));
    let rep_pid = bobs_rep_pid(&r).await;
    let own = format!("agent:{}", r.agent.id);
    let mv = |i: usize, to: i64, actor: &str| Request {
        method: "board_theme.update".into(),
        payload: Some(request::Payload::BoardThemeUpdate(pb::BoardThemeUpdate {
            theme_id: id_bytes(ids[i]),
            ordinal: Some(to),
            actor: actor.into(),
            ..Default::default()
        })),
        ..Default::default()
    };

    let refused = call(&factory(&r.svc, RELEASED, Scope::Control), mv(0, 9, "user")).await;
    assert_eq!(refused.err().as_deref(), Some(NOT_YOURS), "a removed member");
    assert_eq!(ordinal(0), 1);
    call(&factory(&r.svc, OWNERS_PHONE, Scope::Control), mv(0, 4, "user")).await.expect("the owner");
    assert_eq!(ordinal(0), 4);
    call(&factory(&r.svc, BOB, Scope::Control), mv(1, 5, "user")).await.expect("a member");
    assert_eq!(ordinal(1), 5);
    let in_rep = factory(&r.svc, None, Scope::HostAdmin).with_caller_pid(Some(rep_pid));
    call(&in_rep, mv(2, 6, &own)).await.expect("a member's rep");
    assert_eq!(ordinal(2), 6);

    let req = mv(2, 7, &own);
    let found = gate_rep(&r.svc, &peer(None, Scope::HostAdmin), Some(rep_pid), &req).expect("passes");
    assert_eq!(wire_actor(&stamp_rep(&r.svc.store, found, req).unwrap()), Some("member:bob"), "filed under Bob");
    let mut greedy = mv(2, 8, &own);
    let Some(request::Payload::BoardThemeUpdate(p)) = greedy.payload.as_mut() else { panic!() };
    p.budget_tokens = Some(1_000_000);
    assert_eq!(call(&in_rep, greedy).await.err().as_deref(), Some("resource-conflict:rep_messenger"), "no budget with it");
    assert_eq!(ordinal(2), 6);
}
