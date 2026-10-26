//! The member rulings the audit added (ov-716, `reports/ov-716/matrix.md`):
//! no orchestrator made from a lane by anyone but its lead and the owner, no
//! worktree removed, daemon stopped or repository registered from a member's
//! device, no owner's login lent by one, and no `gh` spent by a released
//! device's read. Through `Rpc`, where a client reaches them.

use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, Scope, response};
use farcooler_store::models::Terminal;

use super::*;
use crate::rpc::RpcFactory;
use crate::service::Service;
use crate::test_support::ScratchDir;
use crate::wire::id_bytes;

/// A runner with Main's orchestrator seat empty, a lane's agent the owner
/// opened (on the runner's own login), Bob, a member whose phone is
/// `phone-b`, and Carol, who was one and whose `phone-c` is nobody's now.
struct Runner {
    _dir: ScratchDir,
    svc: Arc<Service>,
    repo: Uuid,
    worktree: Uuid,
    agent: Terminal,
}

async fn runner() -> Runner {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    crate::reconcile::repository(&svc, repo).await.unwrap();
    let worktree = svc.store.list_worktrees_for_repository(repo).unwrap().remove(0).id;
    svc.store.ensure_main_workspace(repo).unwrap();
    let agent = svc.create_terminal(worktree, "Agent 2", "claude").await.expect("an agent");
    svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    svc.store.set_member("carol", "Carol", &["phone-c".to_string()]).unwrap();
    svc.store.remove_member("carol").unwrap();
    Runner { _dir: dir, svc, repo, worktree, agent }
}

const BOB: Option<&str> = Some("phone-b");
const RELEASED: Option<&str> = Some("phone-c");
const OWNERS_PHONE: Option<&str> = Some("phone-o");
const NOT_LEAD: &str = "not-yours:not_lead";

fn peer(device: Option<&str>, scope: Scope) -> Peer {
    Peer { client_id: device.map(str::to_string), scope }
}

fn factory(svc: &Arc<Service>, device: Option<&str>, scope: Scope) -> RpcFactory {
    RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer(device, scope))
}

/// The refusal as `<code word>:<what>`, or `None` for a result.
async fn refused(factory: &RpcFactory, req: Request) -> Option<String> {
    use farcooler_transport::Handler as _;
    match factory.handle(req).await.outcome {
        Some(response::Outcome::Result(_)) => None,
        Some(response::Outcome::Error(e)) => Some(format!("{}:{}", farcooler_core::error::word_for(e.code), e.what)),
        other => panic!("no outcome: {other:?}"),
    }
}

fn made_orchestrator(pane: Uuid) -> Request {
    Request {
        method: "terminal.set_role".into(),
        target_resource_id: Some(id_bytes(pane)),
        payload: Some(request::Payload::TerminalSetRole(pb::TerminalSetRole { role: pb::TerminalRole::Orchestrator as i32 })),
        ..Default::default()
    }
}

/// Making a lane's pane a workspace's orchestrator starts one, so it's the
/// lead's and the owner's alone, as `workspace.start_orchestrator` is
/// (ov-637). Before ov-716 only the login check held it back, and a pane the
/// owner opened is on the owner's login, the lead's while the owner leads:
/// Bob's phone took the empty seat.
#[tokio::test]
async fn a_member_makes_no_pane_the_orchestrator() {
    let r = runner().await;
    let bob = factory(&r.svc, BOB, Scope::Control);
    assert_eq!(refused(&bob, made_orchestrator(r.agent.id)).await.as_deref(), Some(NOT_LEAD), "Bob takes no seat");
    let role = |id| r.svc.store.get_terminal(id).unwrap().role;
    assert_eq!(role(r.agent.id), farcooler_store::models::TerminalRole::Agent, "the lane stays a lane");
    // Shell or agent on a lane is its person's (ov-716 rule 5): not Bob's on
    // the owner's lane, his on his own.
    let shell = Request {
        payload: Some(request::Payload::TerminalSetRole(pb::TerminalSetRole { role: pb::TerminalRole::Shell as i32 })),
        ..made_orchestrator(r.agent.id)
    };
    assert_eq!(refused(&bob, shell.clone()).await.as_deref(), Some("not-yours:not_yours"), "the owner's lane made a shell");
    super::team_tests::on_logins_of(&r.svc, r.agent.id, "bob");
    assert_eq!(refused(&bob, shell).await, None, "his own lane made a shell");
    // The owner's phone still may.
    assert!(gate(&r.svc, &peer(OWNERS_PHONE, Scope::Control), None, &made_orchestrator(r.agent.id)).is_ok());
}

/// Destroying the owner's data, or stopping their runner, isn't a member's,
/// runner admin or not (ov-716): a worktree removed (its uncommitted files
/// with it), the daemon shut down, a repository registered on the runner.
/// The owner's phone passes the gate.
#[tokio::test]
async fn a_member_admin_removes_no_worktree_and_stops_no_runner() {
    let r = runner().await;
    let calls = [
        Request { method: "worktree.remove".into(), target_resource_id: Some(id_bytes(r.worktree)), ..Default::default() },
        Request { method: "daemon.shutdown".into(), ..Default::default() },
        Request {
            method: "repository.register".into(),
            payload: Some(request::Payload::RepositoryRegister(pb::RepositoryRegister { relative_path: "other".into() })),
            ..Default::default()
        },
    ];
    for req in &calls {
        for device in [BOB, RELEASED] {
            let gated = gate(&r.svc, &peer(device, Scope::HostAdmin), None, req);
            assert!(matches!(gated, Err(DomainError::NotYours { what: "not_yours" })), "{device:?} {}: {gated:?}", req.method);
        }
        assert!(gate(&r.svc, &peer(OWNERS_PHONE, Scope::HostAdmin), None, req).is_ok(), "the owner's {}", req.method);
        assert!(gate(&r.svc, &peer(None, Scope::HostAdmin), None, req).is_ok(), "the local {}", req.method);
    }
}

fn own_login(on: bool) -> Request {
    Request {
        method: "member.set_own_login".into(),
        payload: Some(request::Payload::MemberOwnLoginSet(pb::MemberOwnLoginSet { harness: "claude".into(), on })),
        ..Default::default()
    }
}

/// Letting members run on the owner's Claude or Codex login lends the
/// owner's account, which a member admin can't do for them (ov-716): it's
/// Claude as the owner. Requiring members' own logins is anyone's with the
/// scope, as turning the nudge off is.
#[tokio::test]
async fn a_member_admin_lends_no_owners_login() {
    let r = runner().await;
    let gated = gate(&r.svc, &peer(BOB, Scope::HostAdmin), None, &own_login(false));
    assert!(matches!(gated, Err(DomainError::NotYours { what: "not_yours" })), "lent: {gated:?}");
    assert!(gate(&r.svc, &peer(BOB, Scope::HostAdmin), None, &own_login(true)).is_ok(), "required");
    assert!(gate(&r.svc, &peer(OWNERS_PHONE, Scope::HostAdmin), None, &own_login(false)).is_ok(), "the owner lends");
}

/// A released device spends nothing of the owner's: its `stack.get` is
/// refused (ov-758) and starts no `gh` fill (ov-712 review M1). A member's
/// does, as the owner's does: members read everything (ov-637), and the
/// fill only reads, once a minute a repository at most.
#[tokio::test]
async fn a_released_device_spends_no_gh() {
    let get = |r: &Runner| Request {
        method: "stack.get".into(),
        payload: Some(request::Payload::StackGet(pb::StackGet { repository_id: id_bytes(r.repo), branch: "main".into() })),
        ..Default::default()
    };
    let r = runner().await;
    assert_eq!(refused(&factory(&r.svc, RELEASED, Scope::Read), get(&r)).await.as_deref(), Some("not-yours:not_yours"), "it reads nothing");
    assert!(r.svc.claim_pr_fill(r.repo), "no fill was started for it");

    let r = runner().await;
    assert_eq!(refused(&factory(&r.svc, BOB, Scope::Read), get(&r)).await, None);
    assert!(!r.svc.claim_pr_fill(r.repo), "Bob's read fills, as the owner's does");
}

/// Every method in the protocol's table has a member ruling, and the gate
/// keeps it (ov-716). `member_ruling` is a match on `Method` with no
/// wildcard, so a method added to the table doesn't compile until it has one;
/// this checks each ruling is what a member's device, at the highest scope,
/// and a released one get: the owner's alone is refused a member's device
/// `not_yours`, and everything else passes it (its own handler's or the pane
/// rules' checks aside). A released device is refused every one, reads
/// included (ov-758). A read is at Read scope, and Read scope is a read or
/// the caller's own.
#[tokio::test]
async fn every_method_has_a_member_ruling_the_gate_keeps() {
    use farcooler_protocol::method::Method;
    let r = runner().await;
    let not_yours = |g: &Result<()>| matches!(g, Err(DomainError::NotYours { what: "not_yours" }));
    for method in Method::ALL {
        let req = Request { method: method.name().into(), ..Default::default() };
        let member = gate(&r.svc, &peer(BOB, Scope::HostAdmin), None, &req);
        let released = gate(&r.svc, &peer(RELEASED, Scope::HostAdmin), None, &req);
        let ruling = member_ruling(*method);
        let name = method.name();
        match ruling {
            Ruling::Owners(_) => assert!(not_yours(&member), "{name}, the owner's: {member:?}"),
            _ => assert!(member.is_ok(), "{name}, {ruling:?}: {member:?}"),
        }
        assert!(not_yours(&released), "{name}, {ruling:?}, from a released device: {released:?}");
        let read_scope = crate::rpc::required_scope(name) == Some(Scope::Read);
        assert_eq!(ruling == Ruling::Reads, read_scope && ruling != Ruling::Own, "{name}: {ruling:?} at {:?}", crate::rpc::required_scope(name));
    }
}
