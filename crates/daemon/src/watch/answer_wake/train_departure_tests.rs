//! A train that leaves for its checks by itself is told to its orchestrator
//! once (ov-520), typed into a stand-in on the real tmux server of `tests`.

use super::*;
use farcooler_store::plan::{LaneCard, LaneState, LaneUpdate, NewLane};
use farcooler_store::trains::{NewTrain, TrainState, TrainUpdate};

impl Board {
    /// A lane on the task's card, taken to landing.
    fn ready_lane(&self, name: &str) -> farcooler_store::plan::Lane {
        let cards = [LaneCard { task_id: self.task.id, slice: String::new() }];
        let new = NewLane { name: name.into(), ..Default::default() };
        let mut lane = self.svc.store.create_lane(self.task.workspace_id, &new, &cards, None, Actor::Manager).unwrap();
        for s in [LaneState::Building, LaneState::Review, LaneState::Landing] {
            let up = LaneUpdate { state: Some(s), ..Default::default() };
            lane = self.svc.store.update_lane(lane.id, &up, Actor::Manager).unwrap();
        }
        lane
    }

    /// The runner's notices about trains leaving, filed for the orchestrator.
    fn departure_notices(&self) -> Vec<String> {
        let notes = self.svc.store.notes_for(self.task.id, None).unwrap();
        notes.into_iter().filter(|n| n.actor == Actor::Runner && n.body.contains(" is ready for its checks")).map(|n| n.body).collect()
    }
}

/// A ready lane with nothing ahead sends the collecting train off, and the
/// orchestrator is told once: not on the next tick, not by a restart's fresh
/// watcher. A train it sends off later is a new notice.
#[tokio::test]
async fn a_train_that_left_by_itself_is_told_to_the_orchestrator_exactly_once() {
    let b = board().await;
    let orchestrator = b.orchestrator().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.ready_lane("ios-overwrite");
    b.ready_lane("fix-x");
    let ws = b.task.workspace_id;
    // Both joined the one train that left when the first was ready, and the second rode the next.
    let first = b.svc.store.train_named(ws, "train-1").unwrap();
    assert_eq!(first.state, TrainState::Gating);

    b.watcher.tell_departures();
    b.watcher.tell_departures();
    Watcher::new(b.svc.clone()).tell_departures();
    let told = b.departure_notices();
    assert_eq!(told.len(), 1, "{told:?}");
    assert!(told[0].starts_with("Train 1 is ready for its checks: ios-overwrite."), "{}", told[0]);
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted().len(), 1, "{}", si.log());

    // The train ahead lands; the next one leaves, and is told as well.
    b.svc.store.set_train(first.id, &TrainUpdate { sha: Some("1a1b3275".into()), ..Default::default() }, Actor::Manager).unwrap();
    b.svc.store.set_train(first.id, &TrainUpdate { state: Some(TrainState::Landed), ..Default::default() }, Actor::Manager).unwrap();
    b.watcher.tell_departures();
    let told = b.departure_notices();
    assert_eq!(told.len(), 2, "{told:?}");
    assert!(told[1].starts_with("Train 2 is ready for its checks: fix-x."), "{}", told[1]);
}

/// With no orchestrator running, nothing is sent and nothing is recorded; once
/// there is one, the departure is told, once.
#[tokio::test]
async fn nothing_is_sent_or_recorded_without_a_running_orchestrator() {
    let b = board().await;
    b.ready_lane("ios-overwrite");
    let ws = b.task.workspace_id;
    let train = b.svc.store.train_named(ws, "train-1").unwrap();

    b.watcher.tell_departures();
    assert!(b.departure_notices().is_empty(), "no orchestrator, no notice");
    let told: Vec<_> = b.svc.store.train_events(train.id).unwrap().into_iter().filter(|e| e.kind == "told").collect();
    assert!(told.is_empty(), "and no record that it was told");

    let orchestrator = b.orchestrator().await;
    let _si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.watcher.tell_departures();
    b.watcher.tell_departures();
    assert_eq!(b.departure_notices().len(), 1, "told once there is someone to tell");
}

/// A train the coordinator started by hand never departs by itself, so there
/// is nothing to tell.
#[tokio::test]
async fn a_train_started_by_hand_is_not_announced() {
    let b = board().await;
    let orchestrator = b.orchestrator().await;
    let _si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let ws = b.task.workspace_id;
    b.svc.store.start_train(ws, &NewTrain { name: "integ-3".into(), ..Default::default() }, &[], Actor::Manager).unwrap();
    b.ready_lane("a");
    b.watcher.tell_departures();
    let hand = b.svc.store.train_named(ws, "integ-3").unwrap();
    assert_eq!(hand.state, TrainState::Integrating);
    assert!(b.departure_notices().is_empty());
}

/// A departure caused by an RPC is told in that call, not on the next backstop
/// tick: a lane passing review over `lane.update` sends the train off and the
/// orchestrator has the notice when the call returns.
#[tokio::test]
async fn a_departure_caused_by_an_rpc_is_told_in_that_call() {
    use farcooler_protocol::v1::{self as pb, Scope, request};
    let b = board().await;
    let orchestrator = b.orchestrator().await;
    let _si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let cards = [LaneCard { task_id: b.task.id, slice: String::new() }];
    let new = NewLane { name: "via-rpc".into(), ..Default::default() };
    let lane = b.svc.store.create_lane(b.task.workspace_id, &new, &cards, None, Actor::Manager).unwrap();
    for s in [LaneState::Building, LaneState::Review] {
        b.svc.store.update_lane(lane.id, &LaneUpdate { state: Some(s), ..Default::default() }, Actor::Manager).unwrap();
    }
    assert!(b.departure_notices().is_empty());
    let req = pb::Request {
        method: "lane.update".into(),
        payload: Some(request::Payload::LaneUpdate(pb::LaneUpdate {
            lane_id: bytes::Bytes::copy_from_slice(lane.id.as_bytes()),
            state: Some(pb::LaneState::Landing as i32),
            actor: "manager".into(),
            ..Default::default()
        })),
        ..Default::default()
    };
    crate::rpc_plan::dispatch(&b.svc, &b.watcher, Scope::Control, req).await.expect("lane.update");
    let told = b.departure_notices();
    assert_eq!(told.len(), 1, "{told:?}");
    assert!(told[0].starts_with("Train 1 is ready for its checks: via-rpc."), "{}", told[0]);
    assert!(!told[0].contains("--state integrating"), "{}", told[0]);
    assert!(told[0].contains("start your own train with `train start`"), "{}", told[0]);
}

#[path = "notice_wait_tests.rs"]
mod notice_wait_tests;
