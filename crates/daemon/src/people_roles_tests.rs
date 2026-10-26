//! A member's role and the charter's edit (ov-757, slice 1 of ov-753): only
//! the owner gives a role, a member admin included in "not the owner"; the
//! owner, a workspace's lead and a Maintainer edit its charter, and nobody
//! else, a rep or a removed member's device; each change goes to the people
//! log; and with no role given, a member edits nothing they didn't before.
//! Through `gate` and `Rpc`, where a client reaches them.

use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, Scope, response, result};
use farcooler_store::member_roles::Role;
use farcooler_store::models::Terminal;

use super::*;
use crate::rpc::RpcFactory;
use crate::service::Service;
use crate::test_support::ScratchDir;
use crate::wire::id_bytes;

/// A runner with Main, an agent pane, and Bob, Carol and Erin, members whose
/// phones are `phone-b`, `phone-c` and `phone-e`, and Dan, who was one and
/// whose `phone-d` is nobody's now. Nobody has a role or leads anything.
struct Runner {
    _dir: ScratchDir,
    svc: Arc<Service>,
    workspace: Uuid,
    agent: Terminal,
}

async fn runner() -> Runner {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    crate::reconcile::repository(&svc, repo).await.unwrap();
    let worktree = svc.store.list_worktrees_for_repository(repo).unwrap().remove(0);
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let agent = svc.create_terminal(worktree.id, "Agent 2", "claude").await.expect("an agent");
    for (handle, name, phone) in [("bob", "Bob", "phone-b"), ("carol", "Carol", "phone-c"), ("erin", "Erin", "phone-e"), ("dan", "Dan", "phone-d")] {
        svc.store.set_member(handle, name, &[phone.to_string()]).unwrap();
    }
    svc.store.remove_member("dan").unwrap();
    Runner { _dir: dir, svc, workspace, agent }
}

const BOB: Option<&str> = Some("phone-b");
const CAROL: Option<&str> = Some("phone-c");
const ERIN: Option<&str> = Some("phone-e");
const RELEASED: Option<&str> = Some("phone-d");
const OWNERS_PHONE: Option<&str> = Some("phone-o");
const LOCAL: Option<&str> = None;
const NOT_YOURS: &str = "not-yours:not_yours";

fn factory(svc: &Arc<Service>, device: Option<&str>, scope: Scope) -> RpcFactory {
    let peer = Peer { client_id: device.map(str::to_string), scope };
    RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer)
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

fn set_role(handle: &str, role: &str) -> Request {
    Request {
        method: "member.set_role".into(),
        payload: Some(request::Payload::MemberSetRole(pb::MemberSetRole { handle: handle.into(), role: role.into() })),
        ..Default::default()
    }
}

fn set_charter(workspace: Uuid, text: &str, expected: &str) -> Request {
    Request {
        method: "workspace.set_charter".into(),
        target_resource_id: Some(id_bytes(workspace)),
        payload: Some(request::Payload::WorkspaceSetCharter(pb::WorkspaceSetCharter { text: text.into(), expected_sha256: expected.into() })),
        ..Default::default()
    }
}

fn read_charter(workspace: Uuid) -> Request {
    Request { method: "workspace.charter".into(), target_resource_id: Some(id_bytes(workspace)), ..Default::default() }
}

/// The charter's hash as `workspace.charter` answers it.
async fn hash_now(r: &Runner) -> String {
    match call(&factory(&r.svc, LOCAL, Scope::HostAdmin), read_charter(r.workspace)).await {
        Ok(Some(result::Value::WorkspaceCharter(c))) => c.sha256,
        other => panic!("not a charter: {other:?}"),
    }
}

fn charter_on_disk(r: &Runner) -> Option<String> {
    std::fs::read_to_string(crate::workspace_home::charter_path(r.svc.root_dir(), r.workspace)).ok()
}

/// Only the owner gives a role: from their phone or the local socket. A
/// member admin's device is refused `not_yours`, so nobody makes themselves
/// a Maintainer; a member's device below admin isn't let near it by scope;
/// a removed member's device is refused. Refused calls change nothing.
#[tokio::test]
async fn only_the_owner_gives_a_role() {
    let r = runner().await;
    let role = || r.svc.store.member_role("bob").unwrap();
    assert_eq!(call(&factory(&r.svc, BOB, Scope::HostAdmin), set_role("bob", "maintainer")).await.err().as_deref(), Some(NOT_YOURS), "Bob, a member admin, for himself");
    assert_eq!(call(&factory(&r.svc, CAROL, Scope::HostAdmin), set_role("bob", "maintainer")).await.err().as_deref(), Some(NOT_YOURS), "Carol, a member admin, for Bob");
    assert_eq!(call(&factory(&r.svc, BOB, Scope::Control), set_role("bob", "maintainer")).await.err().as_deref(), Some("scope-denied:"), "below admin");
    assert_eq!(call(&factory(&r.svc, RELEASED, Scope::HostAdmin), set_role("bob", "maintainer")).await.err().as_deref(), Some(NOT_YOURS), "a removed member's");
    assert_eq!(role(), Role::Member, "no refused call gave it");
    assert!(r.svc.store.people_events(10).unwrap().is_empty(), "nor logged it");

    let Ok(Some(result::Value::MemberList(list))) = call(&factory(&r.svc, OWNERS_PHONE, Scope::HostAdmin), set_role("bob", "maintainer")).await else {
        panic!("the owner's phone gives it")
    };
    assert_eq!(role(), Role::Maintainer);
    assert_eq!(list.members.iter().find(|m| m.handle == "bob").map(|m| m.role.as_str()), Some("maintainer"), "member.list says so");
    call(&factory(&r.svc, LOCAL, Scope::HostAdmin), set_role("bob", "member")).await.expect("the local socket takes it back");
    assert_eq!(role(), Role::Member);
    assert_eq!(call(&factory(&r.svc, LOCAL, Scope::HostAdmin), set_role("bob", "owner")).await.err().as_deref(), Some("invalid-argument:role"));
    assert_eq!(call(&factory(&r.svc, LOCAL, Scope::HostAdmin), set_role("nobody", "maintainer")).await.err().as_deref(), Some("not-found:"));
}

/// The local socket is the owner's to the gate, so a process in a pane,
/// a member's agent among them, would pass it: the handler refuses a role
/// change from inside any pane, as it refuses lending a login.
#[tokio::test]
async fn no_pane_gives_a_role() {
    let r = runner().await;
    let snapshot = r.svc.inventory.refresh().await;
    let pid = snapshot.claimants(r.agent.id).into_iter().next().expect("the agent's pane").stamp.pid as i32;
    let in_pane = factory(&r.svc, LOCAL, Scope::HostAdmin).with_caller_pid(Some(pid));
    assert_eq!(call(&in_pane, set_role("bob", "maintainer")).await.err().as_deref(), Some("resource-conflict:in_pane"));
    assert_eq!(r.svc.store.member_role("bob").unwrap(), Role::Member);
}

/// Each role change is in the people log, with who made it, from and to,
/// and `member.events` reads it to anyone on the runner at Read scope.
#[tokio::test]
async fn each_role_change_is_in_the_people_log() {
    let r = runner().await;
    let owner = factory(&r.svc, OWNERS_PHONE, Scope::HostAdmin);
    call(&owner, set_role("bob", "maintainer")).await.unwrap();
    call(&owner, set_role("bob", "maintainer")).await.unwrap();
    call(&owner, set_role("bob", "member")).await.unwrap();
    let events = Request { method: "member.events".into(), ..Default::default() };
    let Ok(Some(result::Value::PeopleEventList(log))) = call(&factory(&r.svc, CAROL, Scope::Read), events.clone()).await else {
        panic!("a member reads the log")
    };
    let roles: Vec<(String, String, String)> = log
        .events
        .iter()
        .map(|e| (e.actor.clone(), e.handle.clone(), e.detail.clone()))
        .collect();
    assert_eq!(
        roles,
        [
            ("user".into(), "bob".into(), r#"{"from":"maintainer","to":"member"}"#.into()),
            ("user".into(), "bob".into(), r#"{"from":"member","to":"maintainer"}"#.into()),
        ],
        "newest first, and the repeat changed nothing"
    );
    assert!(log.events.iter().all(|e| e.kind == "role"));
    assert_eq!(call(&factory(&r.svc, RELEASED, Scope::Read), events).await.err().as_deref(), Some(NOT_YOURS), "not a removed member's");
}

/// The owner, the workspace's lead and a Maintainer edit its charter, each
/// from their phone at Control. Every edit is given the hash it was made
/// to, and each is in the people log under who made it.
#[tokio::test]
async fn the_owner_the_lead_and_a_maintainer_edit_the_charter() {
    let r = runner().await;
    r.svc.store.set_workspace_lead(r.workspace, Some("bob")).unwrap();
    r.svc.store.set_member_role("carol", Role::Maintainer, "user").unwrap();
    for (who, device, text) in [
        ("the owner's phone", OWNERS_PHONE, "## Who decides\nThe owner.\n"),
        ("the local socket", LOCAL, "## Who decides\nThe owner, still.\n"),
        ("Bob, the lead", BOB, "## Who decides\nBob.\n"),
        ("Carol, a Maintainer", CAROL, "## Who decides\nCarol.\n"),
    ] {
        let expected = hash_now(&r).await;
        let Ok(Some(result::Value::WorkspaceCharter(c))) = call(&factory(&r.svc, device, Scope::Control), set_charter(r.workspace, text, &expected)).await else {
            panic!("{who} edits it")
        };
        assert_eq!(charter_on_disk(&r).as_deref(), Some(text), "{who}");
        assert_eq!((c.exists, c.text.as_str()), (true, text));
        assert_eq!(c.sha256, hash_now(&r).await, "{who}: the answer's hash is the file's");
    }
    let log = r.svc.store.people_events(10).unwrap();
    let charters: Vec<&str> = log.iter().filter(|e| e.kind == "charter").map(|e| e.actor.as_str()).collect();
    assert_eq!(charters, ["member:carol", "member:bob", "user", "user"]);
    assert!(log.iter().filter(|e| e.kind == "charter").all(|e| e.workspace == Some(r.workspace)));
}

/// With no role given, a member who doesn't lead edits no charter, as
/// before ov-757: Erin is refused `not_yours`, a removed member's phone
/// too, and the file is as it was. A lead edits only the workspace they
/// lead: Bob's lead elsewhere gives him nothing here.
#[tokio::test]
async fn anyone_else_is_refused_the_charter() {
    let r = runner().await;
    let path = crate::workspace_home::charter_path(r.svc.root_dir(), r.workspace);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "## Who decides\nThe owner.\n").unwrap();
    let expected = hash_now(&r).await;
    let other = r.svc.store.create_workspace(r.svc.store.get_workspace(r.workspace).unwrap().repository_id, "Ops", "OPS").unwrap().id;
    r.svc.store.set_workspace_lead(other, Some("bob")).unwrap();
    for (who, device) in [("Erin, a member", ERIN), ("Bob, who leads Ops", BOB), ("a removed member's phone", RELEASED)] {
        for scope in [Scope::Control, Scope::HostAdmin] {
            let said = call(&factory(&r.svc, device, scope), set_charter(r.workspace, "## Who decides\nMe.\n", &expected)).await;
            assert_eq!(said.err().as_deref(), Some(NOT_YOURS), "{who} at {scope:?}");
        }
    }
    let read_only = call(&factory(&r.svc, OWNERS_PHONE, Scope::Read), set_charter(r.workspace, "x", &expected)).await;
    assert_eq!(read_only.err().as_deref(), Some("scope-denied:"), "a Read device, even the owner's");
    assert_eq!(charter_on_disk(&r).as_deref(), Some("## Who decides\nThe owner.\n"), "unchanged");
    assert!(r.svc.store.people_events(10).unwrap().is_empty(), "nothing logged");
}

/// A Maintainer's rep doesn't edit the charter: it relays the ask to the
/// orchestrator (`rep_may`), whatever its member may do.
#[tokio::test]
async fn a_rep_never_edits_the_charter() {
    let r = runner().await;
    r.svc.store.set_member_role("bob", Role::Maintainer, "user").unwrap();
    r.svc.store.bind_rep(r.agent.id, r.workspace, "bob").unwrap();
    let snapshot = r.svc.inventory.refresh().await;
    let pid = snapshot.claimants(r.agent.id).into_iter().next().expect("the rep's pane").stamp.pid as i32;
    let in_rep = factory(&r.svc, LOCAL, Scope::HostAdmin).with_caller_pid(Some(pid));
    let expected = hash_now(&r).await;
    assert_eq!(call(&in_rep, set_charter(r.workspace, "## Who decides\nBob's rep.\n", &expected)).await.err().as_deref(), Some("resource-conflict:rep_messenger"));
    assert_eq!(charter_on_disk(&r), None);
}

/// An edit made to a charter that changed since it was read is refused
/// `charter_changed`, and the change made meanwhile stays. So is an edit to
/// a charter that appeared since none was read, an empty one, and one
/// written through a link.
#[tokio::test]
async fn an_edit_never_writes_over_one_made_meanwhile() {
    let r = runner().await;
    let owner = factory(&r.svc, OWNERS_PHONE, Scope::Control);
    let none = hash_now(&r).await;
    assert_eq!(none, "", "no charter yet");
    call(&owner, set_charter(r.workspace, "## First\n", &none)).await.expect("written where there was none");
    let first = hash_now(&r).await;
    std::fs::write(crate::workspace_home::charter_path(r.svc.root_dir(), r.workspace), "## Edited in the file\n").unwrap();
    assert_eq!(call(&owner, set_charter(r.workspace, "## Second\n", &first)).await.err().as_deref(), Some("resource-conflict:charter_changed"));
    assert_eq!(call(&owner, set_charter(r.workspace, "## Second\n", "")).await.err().as_deref(), Some("resource-conflict:charter_changed"), "not as if there were none");
    assert_eq!(charter_on_disk(&r).as_deref(), Some("## Edited in the file\n"));
    let now = hash_now(&r).await;
    assert_eq!(call(&owner, set_charter(r.workspace, " \n", &now)).await.err().as_deref(), Some("invalid-argument:charter_empty"));
    call(&owner, set_charter(r.workspace, "## Second\n", &now.to_uppercase())).await.expect("the hash in either case");

    let path = crate::workspace_home::charter_path(r.svc.root_dir(), r.workspace);
    let elsewhere = r.svc.root_dir().join("elsewhere.md");
    std::fs::write(&elsewhere, "not a charter").unwrap();
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &path).unwrap();
    assert_eq!(call(&owner, set_charter(r.workspace, "## Third\n", "")).await.err().as_deref(), Some("resource-conflict:charter_not_a_file"));
    assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), "not a charter", "nothing written through the link");
}
