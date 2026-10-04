//! Which task an agent is working on: the one join between a terminal and the
//! board.
//!
//! **The link is `Terminal.task_id`**, set when `task dispatch` or Start Task
//! opens the pane and never moved. An orchestrator is never a task's agent,
//! whatever its row says: it works for its whole workspace.
//!
//! Two functions, so "the agent working on X" means one thing: `task_of` is
//! the task an agent works for (`needs_you` files its signal there, usage is
//! attributed there), and `notice_task` is the task its notices fold into,
//! which is `task_of` minus a closed task (ov-112). The task notice composer
//! folds by `notice_task` (ov-94) and every terminal message carries its
//! answer as `notice_task_id`, so no client keeps a copy of the rule. ov-90's
//! answer wake walks the same link from the other end (`watch::answer_wake`'s
//! `recipient`, through `Store::terminals_for_task`).
//!
//! **The lane fallback.** A terminal opened without a task, in a worktree that
//! is exactly one open task's lane, is that task's: the same rule ov-90's
//! recipient search uses from the task's side. With two open tasks on one
//! lane there is no answer, and a guess would file an agent's question under
//! the wrong task's thread, which is worse than leaving it on its own. The
//! main checkout has no fallback: an agent opened by hand there is nobody's.

use farcooler_store::Store;
use farcooler_store::models::{Task, TaskStatus, Terminal, TerminalRole};
use uuid::Uuid;

/// The task `terminal` is working on, read from the store: its own, else the
/// one open task whose lane it is in, else none. See this module's docs.
pub(crate) fn task_of(store: &Store, terminal: &Terminal) -> Option<Task> {
    if terminal.role == TerminalRole::Orchestrator {
        return None;
    }
    if let Some(id) = terminal.task_id {
        return store.get_task(id).ok();
    }
    // A hand-opened agent in the main checkout is nobody's: one `task dispatch
    // --worktree` there would otherwise absorb every ad hoc agent in the
    // repository (ov-112).
    if store.get_worktree(terminal.worktree_id).ok()?.is_main_checkout {
        return None;
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

/// The task `terminal`'s own notifications fold into: `task_of`, unless that
/// task is Done or Cancelled. An agent still running on a closed task
/// notifies as itself, since a Done-class task notice is off by default and
/// its own banner would go out silent.
pub(crate) fn notice_task(store: &Store, terminal: &Terminal) -> Option<Task> {
    task_of(store, terminal).filter(|t| !matches!(t.status, TaskStatus::Done | TaskStatus::Cancelled))
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

    /// A worktree beside `checkout`'s, which is not the main checkout.
    fn a_lane(svc: &crate::service::Service, checkout: Uuid) -> Uuid {
        let repo = svc.store.get_worktree(checkout).unwrap().repository_id;
        svc.store.create_worktree(repo, "lane", "/tmp/fc-t/ov-112-lane", false).unwrap().id
    }

    fn put_on(svc: &crate::service::Service, workspace: Uuid, lane: Uuid, title: &str) -> Task {
        let task = svc.store.create_task(workspace, title, Actor::User).unwrap();
        let update = farcooler_store::models::TaskUpdate {
            title: task.title.clone(),
            intent: String::new(),
            acceptance: Vec::new(),
            constraints: Vec::new(),
            labels: Vec::new(),
            worktree_id: Some(lane),
        };
        svc.store.update_task(task.id, task.resource_version, &update).unwrap()
    }

    #[tokio::test]
    async fn an_agent_opened_for_a_task_is_that_tasks() {
        let (_dir, svc, workspace, checkout) = a_runner().await;
        let task = svc.store.create_task(workspace, "Wake the agent", Actor::User).unwrap();
        let pane = svc.store.create_terminal_for_test(checkout, workspace);
        let mut row = svc.store.get_terminal(pane).unwrap();
        row.task_id = Some(task.id);
        assert_eq!(task_of(&svc.store, &row).map(|t| t.id), Some(task.id));
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
    }

    #[tokio::test]
    async fn a_lane_with_one_open_task_is_its_and_with_two_is_nobodys() {
        let (_dir, svc, workspace, checkout) = a_runner().await;
        let checkout = a_lane(&svc, checkout);
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

    #[tokio::test]
    async fn a_hand_opened_agent_in_the_main_checkout_is_nobodys() {
        let (_dir, svc, workspace, checkout) = a_runner().await;
        let task = put_on(&svc, workspace, checkout, "Dispatched here");
        let pane = svc.store.create_terminal_for_test(checkout, workspace);
        let row = svc.store.get_terminal(pane).unwrap();
        assert!(task_of(&svc.store, &row).is_none(), "the main checkout's one task is not a hand-opened agent's");
        assert!(notice_task(&svc.store, &row).is_none());
        // Opened for the task, it is still the task's, wherever it sits.
        let mut bound = row;
        bound.task_id = Some(task.id);
        assert_eq!(task_of(&svc.store, &bound).map(|t| t.id), Some(task.id));
    }

    #[tokio::test]
    async fn a_closed_tasks_agent_notifies_as_itself() {
        let (_dir, svc, workspace, checkout) = a_runner().await;
        let lane = a_lane(&svc, checkout);
        let task = put_on(&svc, workspace, lane, "Finished");
        let pane = svc.store.create_terminal_for_test(lane, workspace);
        let mut row = svc.store.get_terminal(pane).unwrap();
        row.task_id = Some(task.id);
        assert_eq!(notice_task(&svc.store, &row).map(|t| t.id), Some(task.id), "open: folds");
        for status in [TaskStatus::Done, TaskStatus::Cancelled] {
            svc.store.set_task_status(task.id, status, Actor::User).unwrap();
            assert_eq!(task_of(&svc.store, &row).map(|t| t.id), Some(task.id), "still its task");
            assert!(notice_task(&svc.store, &row).is_none(), "{status:?}: notifies as itself");
        }
    }
}
