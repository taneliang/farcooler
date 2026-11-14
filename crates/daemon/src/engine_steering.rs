//! Steering capabilities exist only for the lifetime of a native effect. They never advance a
//! workflow, change its candidate, or touch landing. Watching remains a separate read-only path.
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1 as pb;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Control { Pause, Resume, Stop }
pub(crate) type Order = (Control, oneshot::Sender<Result<()>>);
#[derive(Clone)]
struct Live {
    workspace: String,
    item: String,
    tickets: Vec<String>,
    row: pb::Agent,
    send: mpsc::Sender<Order>,
}
static LIVE: LazyLock<Mutex<HashMap<Uuid, Live>>> = LazyLock::new(Default::default);

/// Dropped before the private server is reaped, so a stale id can never steer a replacement step.
pub(crate) struct Listed(Uuid);
impl Drop for Listed {
    fn drop(&mut self) { if let Ok(mut live) = LIVE.lock() { live.remove(&self.0); } }
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn publish(workspace: &str, item: &str, id: Uuid, harness: &str, model: Option<String>,
    review: bool, tickets: Vec<String>, send: mpsc::Sender<Order>) -> Listed {
    let row = pb::Agent { id: crate::wire::id_bytes(id), terminal_id: Some(crate::wire::id_bytes(id)),
        role: if review { pb::AgentRole::Reviewer } else { pb::AgentRole::Builder } as i32,
        harness: harness.into(), model: model.unwrap_or_default(), title: format!("Engine: {item}"),
        started_at: now_ms(), working: true, ..Default::default() };
    LIVE.lock().unwrap_or_else(|e| e.into_inner()).insert(id, Live { workspace:workspace.into(),item:item.into(),tickets,row,send });
    Listed(id)
}
pub(crate) fn workspace(id: Uuid) -> Option<Uuid> {
    LIVE.lock().ok()?.get(&id)?.workspace.parse().ok()
}
pub(crate) fn row(id: Uuid) -> Option<pb::Agent> { LIVE.lock().ok()?.get(&id).map(|a| a.row.clone()) }
pub(crate) fn find(workspace: Uuid, item: &str) -> Option<Uuid> {
    LIVE.lock().ok()?.iter().find(|(_, a)| a.workspace == workspace.to_string() && (a.item == item || a.tickets.iter().any(|t| t == item))).map(|(id, _)| *id)
}
pub(crate) async fn control(id: Uuid, op: Control) -> Result<()> {
    let sender = LIVE.lock().ok().and_then(|live| live.get(&id).map(|a| a.send.clone())).ok_or(DomainError::NotFound)?;
    let (reply, answer) = oneshot::channel();
    sender.send((op, reply)).await.map_err(|_| DomainError::NotFound)?;
    answer.await.map_err(|_| DomainError::OperationFailed)?
}
pub(crate) fn paused(id: Uuid, paused: bool) {
    if let Ok(mut live) = LIVE.lock() && let Some(agent) = live.get_mut(&id) {
        agent.row.working = !paused;
        agent.row.doing_now = if paused { "Paused".into() } else { String::new() };
    }
}
/// Add current native agents and the cards needed to resolve their keys. Runtime facts are never stored.
pub(crate) fn annotate(store: &farcooler_store::Store, workspace: Uuid, plan: &mut pb::Plan) -> Result<()> {
    let mut agents: Vec<_> = LIVE.lock().map(|live| live.values().filter(|a| a.workspace == workspace.to_string()).cloned().collect()).unwrap_or_default();
    agents.sort_by_key(|a| (a.row.started_at, a.row.id.clone()));
    let board = store.get_workspace(workspace)?;
    let tasks = store.list_tasks(farcooler_store::TaskScope::Workspace(board.id), None)?;
    for mut a in agents {
        for key in &a.tickets {
            let Some(task) = tasks.iter().find(|t| &t.key == key) else { continue };
            a.row.tickets.push(pb::AgentTicket { task_id:crate::wire::id_bytes(task.id),
                role: if a.row.role == pb::AgentRole::Reviewer as i32 { pb::WorkRole::Review } else { pb::WorkRole::Build } as i32,
                started_at:a.row.started_at });
            if !plan.cards.iter().any(|c| c.task_id.as_ref() == task.id.as_bytes()) {
                plan.cards.push(pb::PlanCard { task_id:crate::wire::id_bytes(task.id),key:task.key.clone(),title:task.title.clone(),
                    status:crate::rpc_plan::pb_status(task.status) });
            }
        }
        plan.agents.push(a.row);
    }
    Ok(())
}

/// Steering belongs to a person who controls the run, or their orchestrator, never a builder or rep.
pub(crate) async fn authorize(svc: &crate::service::Service, peer: &farcooler_transport::Peer,
    pid: Option<i32>, workspace: Uuid) -> Result<()> {
    let person = crate::people::person_of(&svc.store, peer)?;
    if !crate::people::acts_on_runs(&svc.store, &person, workspace)? {
        return Err(DomainError::NotYours { what:"not_lead" });
    }
    match crate::caller_seat::caller_seat(svc, pid).await {
        crate::caller_seat::Seat::Person => Ok(()),
        crate::caller_seat::Seat::Agent(Some(id)) if svc.store.get_terminal(id).is_ok_and(|t|
            t.role == farcooler_store::models::TerminalRole::Orchestrator && t.workspace_id == Some(workspace)) => Ok(()),
        _ => Err(DomainError::ScopeDenied { needed:"orchestrator" }),
    }
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_store::models::Actor;
    use farcooler_transport::Handler;
    use std::sync::Arc;

    #[tokio::test]
    async fn engine_rows_name_the_ticket_and_stop_uses_the_live_step_capability() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
        let task = svc.store.create_task(workspace, "Steer this build", Actor::User).unwrap();
        let id = Uuid::now_v7();
        let (send, mut inbox) = mpsc::channel(8);
        let listed = publish(&workspace.to_string(), &task.key, id, "codex", Some("codex:test".into()), false, vec![task.key.clone()], send);
        let watcher = crate::watch::Watcher::new(svc.clone());
        let plan = crate::rpc_plan::read_plan(&svc, &watcher, workspace, i64::MIN, true, true).await.unwrap();
        let agent = plan.agents.iter().find(|a| a.id.as_ref() == id.as_bytes()).unwrap();
        assert_eq!(agent.tickets[0].task_id.as_ref(), task.id.as_bytes());
        assert!(plan.cards.iter().any(|c| c.key == task.key));
        let peer = farcooler_transport::Peer { client_id:None, scope:pb::Scope::HostAdmin };
        let request = pb::Request { method:"agent.stop".into(), payload:Some(pb::request::Payload::AgentStop(pb::AgentStop {
            agent_id:crate::wire::id_bytes(id), reason:None, actor:"manager".into() })), ..Default::default() };
        let rpc = crate::rpc::RpcFactory::new(svc.clone(), watcher.clone(), Arc::new(tokio::sync::Notify::new()), peer.clone());
        let steering_request = |method: &str| pb::Request { method:method.into(), target_resource_id:Some(crate::wire::id_bytes(workspace)),
            payload:Some(pb::request::Payload::WorkflowCommand(pb::WorkflowCommand { schema:1,json:serde_json::json!({"item":task.key}).to_string() })),
            ..Default::default() };
        let changing = async {
            for method in ["workflow.pause", "workflow.resume"] {
                let reply = rpc.handle(steering_request(method)).await;
                assert!(matches!(reply.outcome, Some(pb::response::Outcome::Result(_))), "{reply:?}");
            }
        };
        let receiving = async {
            for expected in [Control::Pause, Control::Resume] {
                let (op, reply) = inbox.recv().await.unwrap();
                assert_eq!(std::mem::discriminant(&op), std::mem::discriminant(&expected));
                reply.send(Ok(())).unwrap();
            }
        };
        tokio::join!(changing, receiving);
        let read = crate::rpc::RpcFactory::new(svc.clone(), watcher.clone(), Arc::new(tokio::sync::Notify::new()),
            farcooler_transport::Peer { scope:pb::Scope::Read, ..peer.clone() });
        assert!(matches!(read.handle(steering_request("workflow.pause")).await.outcome, Some(pb::response::Outcome::Error(_))));
        let stopping = crate::rpc_agent_stop::dispatch(&svc, &watcher, &peer, None, request);
        let consume = async {
            let (op, reply) = inbox.recv().await.unwrap();
            assert!(matches!(op, Control::Stop));
            reply.send(Ok(())).unwrap();
        };
        let (result, ()) = tokio::join!(stopping, consume);
        assert!(matches!(result.unwrap(), pb::result::Value::Agent(a) if !a.working && a.stop_asked_at.is_some()));
        // A member who doesn't control this workspace, and a read-only connection, cannot steer it.
        svc.store.set_member("alice", "Alice", &["alice-phone".into()]).unwrap();
        let peer = farcooler_transport::Peer { client_id:Some("alice-phone".into()),scope:pb::Scope::Control };
        assert!(authorize(&svc, &peer, None, workspace).await.is_err());
        drop(listed);
        let mut plan = pb::Plan::default();
        annotate(&svc.store, workspace, &mut plan).unwrap();
        assert!(plan.agents.is_empty());
        assert!(control(id, Control::Resume).await.is_err(), "a stale step cannot reach a replacement");
        let second = svc.store.create_task(workspace, "Another group member", Actor::User).unwrap();
        let (send, _inbox) = mpsc::channel(8);
        let group = Uuid::now_v7();
        let _group = publish(&workspace.to_string(), "group-gate", group, "claude", None, true,
            vec![task.key, second.key.clone()], send);
        let mut plan = pb::Plan::default();
        annotate(&svc.store, workspace, &mut plan).unwrap();
        assert_eq!(plan.agents[0].tickets.len(), 2);
        assert_eq!(find(workspace, &second.key), Some(group));
        assert_eq!(plan.agents[0].role, pb::AgentRole::Reviewer as i32);
    }
}
