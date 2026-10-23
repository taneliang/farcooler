//! The notice audience for a caller in a pane (ov-717), through the real
//! connection handler: a rep's pane reads as its member, whatever socket it
//! came in on, and an orchestrator on the runner's own login as the owner.

use std::sync::Arc;

use farcooler_protocol::v1::{Event, Notice, Scope, event::Payload};
use farcooler_transport::{Handler as _, Peer};
use uuid::Uuid;

use crate::service::Service;

fn notice_of(member: &str) -> Event {
    Event { payload: Some(Payload::Notice(Notice { member: member.into(), ..Notice::default() })), ..Event::default() }
}

fn connection(svc: &Arc<Service>, pid: Option<i32>) -> crate::rpc::RpcFactory {
    let peer = Peer { client_id: None, scope: Scope::Control };
    crate::rpc::RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer)
        .with_caller_pid(pid)
}

/// Alice's rep, on her own login, hears her tasks' notices and no one
/// else's; a pane on the runner's login (the owner's) hears everyone's.
/// Both came in on the local socket, which alone says "owner".
#[tokio::test]
async fn a_pane_hears_as_its_login_does() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    crate::reconcile::repository(&svc, repo).await.unwrap();
    let main = svc.store.ensure_main_workspace(repo).unwrap().id;
    svc.store.set_member("alice", "Alice Chen", &["phone-a".to_string()]).unwrap();
    svc.store.set_member_login_policy("claude", true, "-", 0).unwrap();
    let rep = svc.start_rep(main, "alice", "").await.expect("Alice's rep").terminal;
    let owners_pane: Uuid = svc.create_terminal(rep.worktree_id, "Orchestrator", "shell").await.expect("a pane").id;
    let snapshot = svc.inventory.refresh().await;
    let pid_of = |id: Uuid| snapshot.claimants(id).into_iter().next().expect("a pane").stamp.pid as i32;

    let hears = |pid: Option<i32>, member: &str| connection(&svc, pid).admits(&notice_of(member));
    assert!(hears(None, "alice") && hears(None, "bob") && hears(None, ""), "outside every pane: the owner's");
    let rep_pid = Some(pid_of(rep.id));
    assert!(hears(rep_pid, "alice"), "Alice's rep hears her task");
    assert!(!hears(rep_pid, "bob") && !hears(rep_pid, ""), "and no one else's");
    // A pane opened with no caller, as the owner's app does, is on the
    // runner's own login.
    let owner_pid = Some(pid_of(owners_pane));
    assert!(hears(owner_pid, "alice") && hears(owner_pid, "bob") && hears(owner_pid, ""), "an owner pane hears all");
}
