//! A train's phase, held_by and phase_since (ov-503) as a client meets them:
//! over a real socket in `plan.get` and in the answer to a train write, and
//! the move of a held lane through `train.set`.

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

/// A train with a ready lane and a lane sent back, 50 minutes ago, reads as
/// held over the socket; moving the lane off it with `train.set` releases it.
#[tokio::test]
async fn a_held_train_says_so_over_the_socket_and_a_move_releases_it() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let ready = lane_at(&h, repo.workspace, "ready-one", LaneState::Landing);
    let slow = lane_at(&h, repo.workspace, "slow-one", LaneState::Landing);
    let mut a = connect(&h).await;

    let started = start_train(&mut a, repo.workspace, "train-86", &[ready, slow]).await;
    assert_eq!(started.phase, pb::BoardTrainPhase::Integrating as i32, "a write answers with the phase too");
    assert!(started.held_by.is_empty());

    h.service.store.update_lane(slow, &LaneUpdate { state: Some(LaneState::Fixing), ..Default::default() }, Actor::Manager).unwrap();
    farcooler_store::testing::set_plan_clocks(&h.service.store, now_ms() - 50 * 60_000);
    let read = plan(&mut a, repo.workspace).await;
    let train = &read.trains[0];
    assert_eq!(train.phase, pb::BoardTrainPhase::WaitingForLanes as i32);
    assert_eq!(train.held_by, vec![id(slow)]);
    assert!(read.now_ms - train.phase_since >= 50 * 60_000 - 1_000, "waiting since {}", train.phase_since);

    // The move: one train.set that takes the lane off.
    let off = payload::Payload::TrainSet(pb::TrainSet {
        train_id: train.id.clone(),
        remove_lane_ids: vec![id(slow)],
        actor: "manager".into(),
        ..Default::default()
    });
    let moved = match call(&mut a, "train.set", BOARD_TRAINS, off).await.expect("train.set") {
        result::Value::BoardTrain(t) => t,
        other => panic!("wrong result: {other:?}"),
    };
    assert!(moved.held_by.is_empty());
    assert_eq!(moved.phase, pb::BoardTrainPhase::Integrating as i32);
    assert_eq!(moved.lane_ids, vec![id(ready)]);
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}
