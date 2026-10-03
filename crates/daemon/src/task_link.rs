//! Which task an agent is working on: the one join between a terminal and the
//! board.
//!
//! **The link is `Terminal.task_id`**, set when `task dispatch` or Start Task
//! opens the pane and never moved. An orchestrator is never a task's agent,
//! whatever its row says: it works for its whole workspace.
//!
//! Three readers ask it, so "the agent working on X" means one thing:
//! `needs_you` files an agent's signal under its task (`bound_task`), the task
//! notice composer folds an agent's news into its task's thread (`task_of`,
//! ov-94), and ov-90's answer wake walks the same link from the other end
//! (`watch::answer_wake`'s `recipient`, through `Store::terminals_for_task`).
//!
//! **The lane fallback.** A terminal opened without a task, in a worktree that
//! is exactly one open task's lane, is that task's: the same rule ov-90's
//! recipient search uses from the task's side. With two open tasks on one
//! lane there is no answer, and a guess would file an agent's question under
//! the wrong task's thread, which is worse than leaving it on its own.

use farcooler_store::Store;
use farcooler_store::models::{Task, Terminal, TerminalRole};
use uuid::Uuid;

/// The task `terminal` was opened for, by its row alone: its `task_id`,
/// unless it is an orchestrator.
pub(crate) fn bound_task(terminal: &Terminal) -> Option<Uuid> {
    terminal.task_id.filter(|_| terminal.role != TerminalRole::Orchestrator)
}

/// The task `terminal` is working on, read from the store: its own, else the
/// one open task whose lane it is in, else none. See this module's docs.
pub(crate) fn task_of(store: &Store, terminal: &Terminal) -> Option<Task> {
    if terminal.role == TerminalRole::Orchestrator {
        return None;
    }
    if let Some(id) = terminal.task_id {
        return store.get_task(id).ok();
    }
    let mut open = store.open_tasks_in_worktree(terminal.worktree_id).ok()?;
    if open.len() != 1 {
        return None;
    }
    let only = open.remove(0);
    // Never another workspace's task, as ov-90's recipient never tells
    // another workspace's terminal.
    match terminal.workspace_id {
        Some(workspace) if workspace != only.workspace_id => None,
        _ => Some(only),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_store::models::Actor;

    /// A runner with a Main workspace and its checkout, and one task.
    async fn a_runner() -> (crate::test_support::ScratchDir, std::sync::Arc<crate::service::Service>, Uuid, Uuid) {
        let (dir, svc, repo) = crate::test_support::fixture().await;
        let main = svc.store.ensure_main_workspace(repo).unwrap();
        let rows = svc.store.list_worktrees_for_repository(repo).unwrap();
        let checkout = rows.iter().find(|w| w.is_main_checkout).unwrap().id;
        (dir, svc, main.id, checkout)
    }

    #[tokio::test]
    async fn an_agent_opened_for_a_task_is_that_tasks() {
        let (_dir, svc, workspace, checkout) = a_runner().await;
        let task = svc.store.create_task(workspace, "Wake the agent", Actor::User).unwrap();
        let pane = svc.store.create_terminal_for_test(checkout, workspace);
        let mut row = svc.store.get_terminal(pane).unwrap();
        row.task_id = Some(task.id);
        assert_eq!(task_of(&svc.store, &row).map(|t| t.id), Some(task.id));
        assert_eq!(bound_task(&row), Some(task.id));
    }

    #[tokio::test]
    async fn an_orchestrator_is_never_a_tasks_agent() {
        let (_dir, svc, workspace, checkout) = a_runner().await;
        let task = svc.store.create_task(workspace, "Wake the agent", Actor::User).unwrap();
        let pane = svc.store.create_terminal_for_test(checkout, workspace);
        let mut row = svc.store.get_terminal(pane).unwrap();
        row.task_id = Some(task.id);
        row.role = TerminalRole::Orchestrator;
        assert!(task_of(&svc.store, &row).is_none());
        assert!(bound_task(&row).is_none());
    }

    #[tokio::test]
    async fn a_lane_with_one_open_task_is_its_and_with_two_is_nobodys() {
        let (_dir, svc, workspace, checkout) = a_runner().await;
        let pane = svc.store.create_terminal_for_test(checkout, workspace);
        let row = svc.store.get_terminal(pane).unwrap();
        assert!(task_of(&svc.store, &row).is_none(), "no task on the lane");

        let lane = |title: &str| {
            let task = svc.store.create_task(workspace, title, Actor::User).unwrap();
            let update = farcooler_store::models::TaskUpdate {
                title: task.title.clone(),
                intent: String::new(),
                acceptance: Vec::new(),
                constraints: Vec::new(),
                labels: Vec::new(),
                worktree_id: Some(checkout),
            };
            svc.store.update_task(task.id, task.resource_version, &update).unwrap()
        };
        let first = lane("First");
        assert_eq!(task_of(&svc.store, &row).map(|t| t.id), Some(first.id));
        lane("Second");
        assert!(task_of(&svc.store, &row).is_none(), "two tasks on one lane is no answer");
    }
}
