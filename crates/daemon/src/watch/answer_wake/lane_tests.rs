//! Whom an answer wakes when the task has a lane (ov-240): the lane's only
//! agent pane stands in for the task's own, except in the main checkout,
//! where a hand-opened agent is nobody's (`task_link::task_of`, ov-112).
//! On the board and tmux server of `tests`, whose helpers these use.

use super::*;

/// `b.task`, set to work in `worktree`, and read back.
fn put_on(b: &Board, worktree: Uuid) -> Task {
    let task = b.svc.store.get_task(b.task.id).unwrap();
    let update = farcooler_store::models::TaskUpdate {
        title: task.title.clone(),
        intent: task.intent.clone(),
        acceptance: task.acceptance.clone(),
        constraints: task.constraints.clone(),
        labels: task.labels.clone(),
        worktree_id: Some(worktree),
    };
    b.svc.store.update_task(task.id, task.resource_version, &update).unwrap();
    b.svc.store.get_task(task.id).unwrap()
}

/// A worktree of the task's workspace that isn't the main checkout, with a
/// real directory to start panes in.
fn a_side_lane(b: &Board) -> Worktree {
    let path = b.dir.path().join("side-lane");
    std::fs::create_dir_all(&path).unwrap();
    let side = b.svc.store.create_worktree(b.lane.repository_id, "side", path.to_str().unwrap(), false).unwrap();
    b.svc.store.assign_worktree(side.id, b.task.workspace_id).unwrap()
}

/// An agent opened by hand in the main checkout is nobody's, so an answer
/// on the task dispatched there doesn't type into it, and the task says
/// nobody was told.
#[tokio::test]
async fn an_answer_never_wakes_a_main_checkout_agent_as_the_tasks() {
    let b = board().await;
    assert!(b.lane.is_main_checkout, "the board's lane is the checkout");
    let task = put_on(&b, b.lane.id);
    let by_hand = b.svc.create_terminal_with_prompt(b.lane.id, "Agent 1", "claude", None, None).await.unwrap();
    assert_eq!(b.svc.store.get_terminal(by_hand.id).unwrap().task_id, None);
    assert!(b.watcher.recipient(&task).await.is_none(), "the main checkout's only agent");
    let si = b.stand_in(&by_hand, "claude", "claude").await;
    b.doing(by_hand.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    assert!(si.submitted().is_empty(), "{}", si.log());
    assert_eq!(b.progress(), [NOBODY]);
}

/// A pane bound to the task is its agent wherever it runs, the main
/// checkout included: the link is `Terminal.task_id`.
#[tokio::test]
async fn a_pane_bound_to_the_task_is_woken_in_the_main_checkout() {
    let b = board().await;
    let task = put_on(&b, b.lane.id);
    let bound = b.agent("Agent 2", "claude").await;
    assert_eq!(b.watcher.recipient(&task).await.map(|t| t.id), Some(bound.id));
}

/// Outside the main checkout the lane's only agent pane is still the
/// task's, bound or not, and a bound one is woken too.
#[tokio::test]
async fn a_lane_terminal_is_still_woken_outside_the_main_checkout() {
    let b = board().await;
    let side = a_side_lane(&b);
    let task = put_on(&b, side.id);
    let loose = b.svc.create_terminal_with_prompt(side.id, "Agent 1", "claude", None, None).await.unwrap();
    assert_eq!(b.watcher.recipient(&task).await.map(|t| t.id), Some(loose.id), "the lane's only agent");
    let bound = b.svc.create_terminal_with_prompt(side.id, "Agent 2", "claude", None, Some(task.id)).await.unwrap();
    assert_eq!(b.watcher.recipient(&task).await.map(|t| t.id), Some(bound.id), "the bound pane wins");
}
