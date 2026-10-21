//! A train's phase and phase_since (ov-503), and the open train waiting behind
//! the one ahead (ov-520), as a client meets them: over a real socket in
//! `plan.get` and in the answer to a train write.

#[path = "support/in_process.rs"]
mod in_process;

use farcooler_protocol::capability::{BOARD_PLAN, BOARD_TRAINS};
use farcooler_protocol::v1::{self as pb, Scope, request as payload, result};
use farcooler_store::models::Actor;
use farcooler_store::plan::{LaneState, LaneUpdate, NewLane};
use farcooler_transport::{ClientError, request};
use in_process::*;
use uuid::Uuid;

fn id(uuid: Uuid) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(uuid.as_bytes())
}

async fn call(link: &mut Link, method: &str, capability: &str, p: payload::Payload) -> Result<result::Value, ClientError> {
    let mut r = request(method);
    r.required_capabilities = vec![capability.into()];
    r.payload = Some(p);
    Ok(link.call(r).await?.value.expect("a value"))
}

fn lane_at(h: &Harness, workspace: Uuid, name: &str, to: LaneState) -> Uuid {
    let lane = h
        .service
        .store
        .create_lane(workspace, &NewLane { name: name.into(), ..Default::default() }, &[], None, Actor::Manager)
        .unwrap();
    let road = [LaneState::Building, LaneState::Review, LaneState::Landing];
    let stops = if to == LaneState::Building { 1 } else { 3 };
    for s in &road[..stops] {
        h.service.store.update_lane(lane.id, &LaneUpdate { state: Some(*s), ..Default::default() }, Actor::Manager).unwrap();
    }
    lane.id
}

async fn start_train(link: &mut Link, workspace: Uuid, name: &str, lanes: &[Uuid]) -> pb::BoardTrain {
    let p = payload::Payload::TrainStart(pb::TrainStart {
        workspace_id: id(workspace),
        name: name.into(),
        lane_ids: lanes.iter().map(|l| id(*l)).collect(),
        actor: "manager".into(),
        ..Default::default()
    });
    match call(link, "train.start", BOARD_TRAINS, p).await.expect("train.start") {
        result::Value::BoardTrain(t) => t,
        other => panic!("wrong result: {other:?}"),
    }
}

async fn plan(link: &mut Link, workspace: Uuid) -> pb::Plan {
    let p = payload::Payload::PlanGet(pb::PlanGetRequest { workspace_id: id(workspace), include_closed: true });
    match call(link, "plan.get", BOARD_PLAN, p).await.expect("plan.get") {
        result::Value::Plan(p) => p,
        other => panic!("wrong result: {other:?}"),
    }
}

/// The open train says it is waiting for the earlier landing over the socket,
/// and a lane sent back for a fix comes off the train that was checking: the
/// train is never held for it.
#[tokio::test]
async fn the_open_train_waits_for_the_previous_one_and_a_lane_sent_back_comes_off() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let ready = lane_at(&h, repo.workspace, "ready-one", LaneState::Landing);
    let mut a = connect(&h).await;

    // The lane joined the open train when it passed review; starting one by hand adopts it.
    let started = start_train(&mut a, repo.workspace, "train-86", &[ready]).await;
    assert_eq!(started.phase, pb::BoardTrainPhase::Integrating as i32, "a write answers with the phase too");
    let gating = pb::TrainSet {
        train_id: started.id.clone(),
        state: pb::BoardTrainState::Gating as i32,
        actor: "manager".into(),
        ..Default::default()
    };
    let checking = match call(&mut a, "train.set", BOARD_TRAINS, payload::Payload::TrainSet(gating)).await.expect("train.set") {
        result::Value::BoardTrain(t) => t,
        other => panic!("wrong result: {other:?}"),
    };
    assert_eq!(checking.phase, pb::BoardTrainPhase::GatingLocally as i32);

    // The next lane that passes review rides the train opened behind it.
    let next = lane_at(&h, repo.workspace, "next-one", LaneState::Landing);
    let read = plan(&mut a, repo.workspace).await;
    let behind = read.trains.iter().find(|t| t.state == pb::BoardTrainState::Integrating as i32).expect("an open train");
    assert_ne!(behind.id, checking.id);
    assert_eq!(behind.phase, pb::BoardTrainPhase::WaitingForPreviousTrain as i32);
    assert_eq!(pb::BoardTrainPhase::WaitingForPreviousTrain as i32, 10, "the number the wire reserved");
    assert_eq!(behind.state, pb::BoardTrainState::Integrating as i32, "an old client keeps drawing the stored state");
    assert_eq!(behind.lane_ids, vec![id(next)]);
    assert!(behind.held_by.is_empty() && read.trains.iter().all(|t| t.held_by.is_empty()), "a train is never held");

    // A lane sent back for a fix comes off the train that was checking it.
    h.service.store.update_lane(ready, &LaneUpdate { state: Some(LaneState::Fixing), ..Default::default() }, Actor::Manager).unwrap();
    let read = plan(&mut a, repo.workspace).await;
    let train = read.trains.iter().find(|t| t.id == checking.id).unwrap();
    assert!(train.lane_ids.is_empty(), "the lane is off the train");
    assert!(read.now_ms - train.phase_since < 60 * 60_000);
}


