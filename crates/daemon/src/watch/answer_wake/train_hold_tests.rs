//! A held train is told to its orchestrator once (ov-503), typed into a
//! stand-in on the real tmux server of `tests`.

use super::*;
use farcooler_store::plan::{LaneCard, LaneState, LaneUpdate, NewLane};
use farcooler_store::trains::NewTrain;

impl Board {
    /// A lane on the task's card, taken as far as `state`.
    fn lane_at(&self, name: &str, state: LaneState) -> farcooler_store::plan::Lane {
        let cards = [LaneCard { task_id: self.task.id, slice: String::new() }];
        let new = NewLane { name: name.into(), ..Default::default() };
        let mut lane = self.svc.store.create_lane(self.task.workspace_id, &new, &cards, None, Actor::Manager).unwrap();
        let road = [LaneState::Building, LaneState::Review, LaneState::Landing];
        let stops = if state == LaneState::Building { 1 } else { 3 };
        for s in &road[..stops] {
            let up = LaneUpdate { state: Some(*s), ..Default::default() };
            lane = self.svc.store.update_lane(lane.id, &up, Actor::Manager).unwrap();
        }
        lane
    }

    /// Every lane and train clock moved `minutes` into the past: the wait the
    /// plan read measures is real time.
    fn aged(&self, minutes: i64) {
        let ms = crate::review::now_millis() - minutes * 60_000;
        farcooler_store::testing::set_plan_clocks(&self.svc.store, ms);
    }

    /// The runner's notices about trains, filed for the orchestrator.
    fn hold_notices(&self) -> Vec<String> {
        let notes = self.svc.store.notes_for(self.task.id, None).unwrap();
        notes.into_iter().filter(|n| n.actor == Actor::Runner && n.body.contains(" is held by ")).map(|n| n.body).collect()
    }
}

/// A train held 50 minutes by one lane tells its orchestrator once: not on
/// the next tick, not after a restart's fresh watcher, and again only for a new wait.
#[tokio::test]
async fn a_held_train_is_told_to_the_orchestrator_exactly_once() {
    let b = board().await;
    let orchestrator = b.orchestrator().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let ready = b.lane_at("ready-one", LaneState::Landing);
    let slow = b.lane_at("ios-overwrite", LaneState::Landing);
    let ws = b.task.workspace_id;
    let new = NewTrain { name: "train-86".into(), ..Default::default() };
    b.svc.store.start_train(ws, &new, &[ready.id, slow.id], Actor::Manager).unwrap();
    let back = LaneUpdate { state: Some(LaneState::Fixing), ..Default::default() };
    b.svc.store.update_lane(slow.id, &back, Actor::Manager).unwrap();

    // Waiting, but not for 45 minutes yet.
    b.watcher.tell_held_trains();
    assert!(b.hold_notices().is_empty(), "nothing is held yet");

    b.aged(50);
    b.watcher.tell_held_trains();
    b.watcher.tell_held_trains();
    // A watcher from a restart reads the same store, and the record keeps it quiet.
    Watcher::new(b.svc.clone()).tell_held_trains();
    let told = b.hold_notices();
    assert_eq!(told.len(), 1, "{told:?}");
    assert!(told[0].starts_with("Train 86 is held by ios-overwrite."), "{}", told[0]);
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted().len(), 1, "{}", si.log());
    assert!(si.submitted()[0].contains("Train 86 is held by ios-overwrite."), "{}", si.log());

    // The lane is fixed and fails again: a new wait, a new notice.
    let review = LaneUpdate { state: Some(LaneState::Review), ..Default::default() };
    b.svc.store.update_lane(slow.id, &review, Actor::Manager).unwrap();
    let landing = LaneUpdate { state: Some(LaneState::Landing), ..Default::default() };
    b.svc.store.update_lane(slow.id, &landing, Actor::Manager).unwrap();
    b.svc.store.update_lane(slow.id, &back, Actor::Manager).unwrap();
    b.aged(50);
    b.watcher.tell_held_trains();
    assert_eq!(b.hold_notices().len(), 2);
}
