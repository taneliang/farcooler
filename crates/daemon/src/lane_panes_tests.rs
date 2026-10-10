//! A pane opened for a plan lane (ov-457), on a scratch runner.

use super::*;

use farcooler_store::models::Actor;
use farcooler_store::plan::{LaneCard, LaneState, NewLane};

struct Runner {
    _dir: crate::test_support::ScratchDir,
    svc: std::sync::Arc<crate::service::Service>,
    workspace: Uuid,
    checkout: Uuid,
}

async fn a_runner() -> Runner {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let checkout = svc.store.list_worktrees_for_repository(repo).unwrap().into_iter().find(|w| w.is_main_checkout).unwrap().id;
    Runner { _dir: dir, svc, workspace, checkout }
}

impl Runner {
    fn task(&self, title: &str) -> farcooler_store::models::Task {
        self.svc.store.create_task(self.workspace, title, Actor::Manager).unwrap()
    }

    fn lane(&self, name: &str, cards: &[Uuid]) -> Lane {
        let cards: Vec<LaneCard> = cards.iter().map(|&task_id| LaneCard { task_id, slice: String::new() }).collect();
        let new = NewLane { name: name.into(), ..Default::default() };
        self.svc.store.create_lane(self.workspace, &new, &cards, None, Actor::Manager).unwrap()
    }
}

fn refused<T: std::fmt::Debug>(r: Result<T>) -> &'static str {
    match r {
        Err(DomainError::InvalidArgument { what }) => what,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// A lane is refused unless it is live on the task's board and holds the
/// task, and asking for none is no lane.
#[tokio::test]
async fn a_lane_must_hold_the_task() {
    let r = a_runner().await;
    let (one, two) = (r.task("One"), r.task("Two"));
    r.lane("phones", &[one.id]);
    assert_eq!(resolve(&r.svc.store, Some(one.id), None).unwrap(), None);
    assert_eq!(resolve(&r.svc.store, Some(one.id), Some("  ")).unwrap(), None);
    assert_eq!(resolve(&r.svc.store, Some(one.id), Some("PHONES")).unwrap().map(|l| l.name), Some("phones".into()));
    assert_eq!(refused(resolve(&r.svc.store, Some(two.id), Some("phones"))), "lane", "a card the lane doesn't hold");
    assert_eq!(refused(resolve(&r.svc.store, Some(one.id), Some("nope"))), "lane", "no such lane");
    assert_eq!(refused(resolve(&r.svc.store, None, Some("phones"))), "lane", "no task to find it by");
}

/// The brief names the lane and each card still to do, in the lane's order,
/// and how to move on; a finished card is left out.
#[tokio::test]
async fn the_brief_names_every_open_card_in_order() {
    let r = a_runner().await;
    let (one, two, three) = (r.task("Sidebar"), r.task("Toolbar"), r.task("Done already"));
    r.svc.store.set_task_status(three.id, farcooler_store::models::TaskStatus::Done, Actor::Manager).unwrap();
    let lane = r.lane("mac-ux", &[two.id, one.id, three.id]);
    let said = brief(&r.svc.store, &lane, "fc").unwrap();
    let (k1, k2) = (&one.key, &two.key);
    assert!(said.contains(&format!("the lane mac-ux, whose cards are, in order: {k2} (Toolbar), {k1} (Sidebar).")), "{said}");
    assert!(!said.contains(&three.key), "{said}");
    assert!(said.contains("fc task show <key>") && said.contains("fc plan lane show mac-ux"), "{said}");
    let prompt = with_brief(&r.svc.store, Some(&lane), Some("Opening.".into())).unwrap().unwrap();
    assert!(prompt.starts_with("Opening.\n\nThis pane works the lane mac-ux"), "{prompt}");
    assert_eq!(with_brief(&r.svc.store, None, Some("Opening.".into())).unwrap().as_deref(), Some("Opening."));
}

/// Opening a pane on a lane records it as the lane's build agent, moves the
/// lane to building on the pane's worktree, and the pane exports the lane;
/// with nobody recording anything by hand.
#[tokio::test]
async fn a_lane_pane_is_recorded_and_exports_its_lane() {
    let r = a_runner().await;
    let (one, two) = (r.task("One"), r.task("Two"));
    let lane = r.lane("phones", &[one.id, two.id]);
    assert_eq!(lane.state, LaneState::Queued);
    let pane = r.svc.create_terminal_in_lane(r.checkout, "Agent", "codex", None, Some(one.id), Some("phones")).await.unwrap();
    let after = r.svc.store.lane(lane.id).unwrap();
    assert_eq!(after.state, LaneState::Building);
    assert_eq!(after.worktree_id, Some(r.checkout));
    let plan = r.svc.store.plan(r.workspace, 0).unwrap();
    let agents = &plan.lanes.iter().find(|l| l.lane.id == lane.id).unwrap().agents;
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].agent_id, farcooler_store::plan_panes::pane_agent_id(pane.id));
    assert_eq!(agents[0].harness, "codex");
    assert_eq!(env(&r.svc.store, &pane), Some(("FARCOOLER_LANE".to_string(), "phones".to_string())));
    assert_eq!(pane.task_id, Some(one.id), "the pane starts on the lane's card it was opened for");
}

/// A pane opened for a lane that doesn't hold its task opens nothing.
#[tokio::test]
async fn a_refused_lane_opens_no_pane() {
    let r = a_runner().await;
    let (one, two) = (r.task("One"), r.task("Two"));
    r.lane("phones", &[one.id]);
    let before = r.svc.store.list_terminals_for_worktree(r.checkout).unwrap().len();
    let opened = r.svc.create_terminal_in_lane(r.checkout, "Agent", "claude", None, Some(two.id), Some("phones")).await;
    assert_eq!(refused(opened), "lane");
    assert_eq!(r.svc.store.list_terminals_for_worktree(r.checkout).unwrap().len(), before);
}

/// A pane that isn't a lane's agent exports no lane.
#[tokio::test]
async fn a_plain_pane_has_no_lane() {
    let r = a_runner().await;
    let one = r.task("One");
    let pane = r.svc.create_terminal_with_prompt(r.checkout, "Agent", "claude", None, Some(one.id)).await.unwrap();
    assert_eq!(env(&r.svc.store, &pane), None);
}

/// Removing a lane's pane records its agent as finished, so it exports the
/// lane no more and the lane shows no agent still working.
#[tokio::test]
async fn a_removed_pane_has_finished_its_lane() {
    let r = a_runner().await;
    let one = r.task("One");
    r.lane("phones", &[one.id]);
    let pane = r.svc.create_terminal_in_lane(r.checkout, "Agent", "claude", None, Some(one.id), Some("phones")).await.unwrap();
    assert!(r.svc.store.lane_of_pane(pane.id).unwrap().is_some());
    r.svc.stop_terminal(pane.id).await.unwrap();
    r.svc.remove_terminal(pane.id).await.unwrap();
    assert_eq!(r.svc.store.lane_of_pane(pane.id).unwrap(), None);
}
