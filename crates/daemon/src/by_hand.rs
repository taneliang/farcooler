//! Work started or moved by hand, refused once a workspace runs its work through a workflow (ov-1011).
//!
//! A workspace with a published workflow starts its tickets with `workflow start` and moves them with
//! `workflow move`. So a pane opened on a ticket (agent dispatch, task dispatch) and a stage set on the plan
//! are refused there, in words that name the command to use instead. A pane with no ticket, the kind the owner
//! opens for their own scripts, is never asked, and a workspace with no published workflow works as before.

use farcooler_core::{DomainError, Result};
use farcooler_store::Store;
use uuid::Uuid;

/// A pane opened by hand on a ticket: its `DomainError::Conflict` word.
pub(crate) const PANE: &str = "engine_dispatch";
/// A stage set by hand on a ticket's plan line: its word.
pub(crate) const STAGE: &str = "engine_stage";

/// Refused as `what` when `task`'s workspace has a published workflow.
pub(crate) fn refuse(store: &Store, task: Uuid, what: &'static str) -> Result<()> {
    let workspace = store.get_task(task)?.workspace_id;
    match store.workflow_configuration(&workspace.to_string()) {
        Ok(None) => Ok(()),
        Ok(Some(_)) => Err(DomainError::Conflict { what }),
        Err(_) => Err(DomainError::OperationFailed),
    }
}

#[cfg(test)]
mod tests {
    use farcooler_core::DomainError;
    use farcooler_protocol::v1 as pb;
    use farcooler_store::models::Actor;

    fn refused<T: std::fmt::Debug>(r: farcooler_core::Result<T>, what: &str) {
        assert!(matches!(&r, Err(DomainError::Conflict { what: w }) if *w == what), "{r:?}");
    }

    /// Task dispatch and agent dispatch both open a pane on a ticket. Once the workflow is published both are
    /// refused and open nothing, while a plain pane, with no ticket, still opens.
    #[tokio::test]
    async fn a_published_workspace_refuses_a_pane_on_a_ticket_and_keeps_a_plain_one() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let board = svc.store.ensure_main_workspace(repo).unwrap().id;
        let checkout = svc.store.list_worktrees_for_repository(repo).unwrap().into_iter().find(|w| w.is_main_checkout).unwrap().id;
        let card = svc.store.create_task(board, "by hand", Actor::Manager).unwrap();
        let agent = pb::AgentDispatch { role: pb::AgentRole::Builder as i32, ..Default::default() };
        let panes = || svc.store.list_terminals_for_worktree(checkout).unwrap().len();
        // No workflow yet: a ticket's pane opens as today.
        let before = svc.create_terminal_with_prompt(checkout, "task", "claude", None, Some(card.id)).await.expect("opens as today");
        let _ = svc.stop_terminal(before.id).await;
        crate::test_support::publish_for_test(&svc, board);
        let opened = panes();
        refused(svc.create_terminal_with_prompt(checkout, "task", "claude", None, Some(card.id)).await, super::PANE);
        refused(svc.create_terminal_as_agent(checkout, "agent", "claude", None, Some(card.id), Some(&agent)).await, super::PANE);
        assert_eq!(panes(), opened, "a refusal opens nothing");
        let own = svc.create_terminal_with_prompt(checkout, "scripts", "claude", None, None).await.expect("a plain pane still opens");
        let _ = svc.stop_terminal(own.id).await;
    }

    /// A split beside a pane (`terminal.create` joining the active layout) opens a pane on a ticket without
    /// passing through `open_terminal`, so it is refused on its own, and opens nothing.
    #[tokio::test]
    async fn a_published_workspace_refuses_a_split_pane_on_a_ticket() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let board = svc.store.ensure_main_workspace(repo).unwrap().id;
        let checkout = svc.store.list_worktrees_for_repository(repo).unwrap().into_iter().find(|w| w.is_main_checkout).unwrap().id;
        let card = svc.store.create_task(board, "split by hand", Actor::Manager).unwrap();
        let anchor = svc.create_terminal_with_prompt(checkout, "own", "claude", None, None).await.expect("a plain pane");
        crate::test_support::publish_for_test(&svc, board);
        let opened = svc.store.list_terminals_for_worktree(checkout).unwrap().len();
        let side = pb::SplitSide::Right;
        refused(svc.split_terminal_with_prompt(checkout, anchor.id, side, "task", "claude", None, Some(card.id)).await, super::PANE);
        assert_eq!(svc.store.list_terminals_for_worktree(checkout).unwrap().len(), opened, "a refusal opens nothing");
        let _ = svc.stop_terminal(anchor.id).await;
    }
}
