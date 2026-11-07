//! ov-951: the orchestrator moves a card by saying the column it wants; the move is the engine's.
//! `task set --status` from an agent on a tracker-only ticket becomes a `Move` command, so the card
//! changes only when the engine accepts it. A person's hand move is refused by the store.
use crate::service::Service;
use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1 as pb;
use farcooler_store::models::TaskStatus;

/// Whether new and existing cards are brought under the engine on their own (`FARCOOLER_TRACK_TICKETS=1`).
/// Off until a worker can be put on a tracked ticket by `Start`: an engine-owned card refuses a hand-linked
/// pane (`workflow_excludes_terminals_*`), and today's dispatch links panes by hand.
pub fn tracking_on() -> bool { std::env::var("FARCOOLER_TRACK_TICKETS").is_ok_and(|v| v == "1") }

/// The engine column a status names.
pub(crate) fn column(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Backlog => "backlog", TaskStatus::Todo => "todo", TaskStatus::InProgress => "doing",
        TaskStatus::InReview => "review", TaskStatus::NeedsDecision => "decision",
        TaskStatus::Done => "done", TaskStatus::Cancelled => "cancelled",
    }
}

/// Moves `task` through the engine. `None` when the engine does not hold it as a ticket at rest (a run owns
/// it, or the engine is off), and the caller carries on as before.
pub(crate) async fn move_through_engine(svc: &Service, task: uuid::Uuid, req: &pb::TaskSetStatus) -> Result<Option<pb::Task>> {
    let Some(status) = crate::task_ops::status_from_wire(req.status) else { return Ok(None) };
    let Some(handle) = svc.workflow_handle() else { return Ok(None) };
    let card = svc.store.get_task(task)?;
    let moved = handle.move_ticket(&card.workspace_id.to_string(), &card.key, column(status)).await
        .map_err(|_| DomainError::OperationFailed)?;
    if !moved { return Ok(None); }
    Ok(Some(crate::task_starts::pb_one(svc, &svc.store.get_task(task)?)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_store::models::Actor;

    fn set(task: uuid::Uuid, status: TaskStatus) -> pb::TaskSetStatus {
        pb::TaskSetStatus { task_id: task.as_bytes().to_vec().into(), status: crate::task_ops::pb_status(status), actor: "manager".into(), ..Default::default() }
    }

    #[tokio::test]
    async fn the_orchestrator_moves_a_tracked_card_through_the_engine_and_a_hand_cannot() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let board = svc.store.ensure_main_workspace(repo).unwrap();
        let card = svc.store.create_task(board.id, "moved", Actor::User).unwrap();
        // Before the engine holds it, nothing is moved here.
        assert!(move_through_engine(&svc, card.id, &set(card.id, TaskStatus::Todo)).await.unwrap().is_none());
        svc.start_workflows().unwrap();
        svc.track_tickets(None).await;
        for status in [TaskStatus::Todo, TaskStatus::InProgress, TaskStatus::InReview, TaskStatus::NeedsDecision, TaskStatus::Backlog] {
            let moved = move_through_engine(&svc, card.id, &set(card.id, status)).await.unwrap().expect("the engine holds it");
            assert_eq!(moved.status, crate::task_ops::pb_status(status));
            assert_eq!(svc.store.get_task(card.id).unwrap().status, status);
        }
        // Where it already is changes nothing and is not a failure.
        assert!(move_through_engine(&svc, card.id, &set(card.id, TaskStatus::Backlog)).await.unwrap().is_some());
        // A hand on the store is refused in words.
        assert!(svc.store.set_task_status(card.id, TaskStatus::Todo, Actor::User).is_err());
        let closed = move_through_engine(&svc, card.id, &set(card.id, TaskStatus::Done)).await.unwrap().unwrap();
        assert_eq!(closed.status, crate::task_ops::pb_status(TaskStatus::Done));
        svc.stop_workflows().await.unwrap();
    }
}
