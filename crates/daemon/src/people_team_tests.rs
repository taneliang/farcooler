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
/// made as the rep, as `user` or saying nothing is filed under its member,
/// `member:bob`, never the owner's `user` or the rep's pane; as the
/// orchestrator it's refused `actor`. The same write from another process
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
    let own = format!("agent:{}", r.agent.id);
    for (name, actor) in [("Exports", own.as_str()), ("Receipts", "user"), ("Credit notes", "")] {
        let made = call(&in_rep, theme(name, actor)).await.unwrap_or_else(|e| panic!("{actor:?}: {e}"));
        assert_eq!(made_by(made), "member:bob", "the rep as {actor:?}");
    }
    let made = call(&elsewhere, theme("Refunds", "manager")).await.expect("the orchestrator elsewhere");
    assert_eq!(made_by(made), Actor::Manager.to_string(), "another process is left as it says");
}
