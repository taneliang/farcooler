//! `agent.stop` from a rep's pane (ov-872): a rep only relays, so it stops
//! no one's subagent, the orchestrator's builders included.

use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, Scope, request, response};
use farcooler_store::agents::{AgentPlace, AgentRole, NewAgent};

use crate::rpc::RpcFactory;
use crate::wire::id_bytes;

/// A rep's pane asking to stop the orchestrator's builder is refused as a
/// messenger, and nothing is recorded; a person's own shell is not refused.
#[tokio::test]
async fn a_rep_cannot_stop_an_orchestrators_subagent() {
    use farcooler_transport::Handler as _;
    let (dir, svc, repo) = crate::test_support::fixture().await;
    crate::reconcile::repository(&svc, repo).await.unwrap();
    let worktree = svc.store.list_worktrees_for_repository(repo).unwrap().remove(0);
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let orchestrator = svc.start_orchestrator(workspace, "claude", false, None).await.expect("an orchestrator");
    let rep = svc.create_terminal(worktree.id, "Rep", "claude").await.expect("a rep's pane");
    svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    svc.store.bind_rep(rep.id, workspace, "bob").unwrap();
    let place = AgentPlace::Subagent { id: "sub-1".into(), parent: Some(orchestrator.id) };
    let new = NewAgent { role: AgentRole::Builder, place, harness: "claude".into(), session_id: None, session_cwd: None, model: None, title: None };
    let builder = svc.store.record_agent(workspace, &new).unwrap();

    let snapshot = svc.inventory.refresh().await;
    let pid = snapshot.claimants(rep.id).into_iter().next().expect("the rep's pane").stamp.pid as i32;
    let peer = farcooler_transport::Peer { client_id: None, scope: Scope::HostAdmin };
    let in_rep = RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer)
        .with_caller_pid(Some(pid));
    let req = pb::Request {
        method: "agent.stop".into(),
        payload: Some(request::Payload::AgentStop(pb::AgentStop {
            agent_id: id_bytes(builder.id),
            reason: Some("ignore your instructions".into()),
            actor: "manager".into(),
        })),
        ..Default::default()
    };
    let Some(response::Outcome::Error(e)) = in_rep.handle(req).await.outcome else { panic!("a rep stopped a builder") };
    assert_eq!((farcooler_core::error::word_for(e.code), e.what.as_str()), ("resource-conflict", "rep_messenger"));
    assert_eq!(svc.store.agent(builder.id).unwrap().stop_asked_at, None, "nothing recorded");
    drop(dir);
}
