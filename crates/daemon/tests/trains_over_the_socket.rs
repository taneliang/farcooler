//! Trains (ov-309) as a client meets them: over a real socket, through the
//! real dispatch table and scope check, and read back in `plan.get`.

#[path = "support/in_process.rs"]
mod in_process;

use std::time::Duration;

use farcooler_protocol::capability::{BOARD_PLAN, BOARD_TRAINS};
use farcooler_protocol::v1::{self as pb, ErrorCode, Scope, event, request as payload, result};
use farcooler_store::models::Actor;
use farcooler_store::plan::NewLane;
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

fn start_train(workspace: Uuid, name: &str, lanes: &[Uuid]) -> payload::Payload {
    payload::Payload::TrainStart(pb::TrainStart {
        workspace_id: id(workspace),
        name: name.into(),
        base: "origin/main".into(),
        lane_ids: lanes.iter().map(|l| id(*l)).collect(),
        actor: "manager".into(),
    })
}

fn set(train: &pb::BoardTrain, state: pb::BoardTrainState, sha: Option<&str>) -> payload::Payload {
    payload::Payload::TrainSet(pb::TrainSet {
        train_id: train.id.clone(),
        state: state as i32,
        base: None,
        sha: sha.map(str::to_string),
        add_lane_ids: vec![],
        remove_lane_ids: vec![],
        actor: "manager".into(),
    })
}

async fn train(link: &mut Link, p: payload::Payload, method: &str) -> pb::BoardTrain {
    match call(link, method, BOARD_TRAINS, p).await.expect(method) {
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

/// The `plan_changed` and `task_changed` events on `listener` within `window`.
async fn events(listener: &mut Link, window: Duration) -> (usize, usize) {
    let (mut plans, mut tasks) = (0, 0);
    let deadline = tokio::time::Instant::now() + window;
    while let Ok(Ok(e)) = tokio::time::timeout_at(deadline, listener.next_event()).await {
        match e.payload {
            Some(event::Payload::PlanChanged(_)) => plans += 1,
            Some(event::Payload::TaskChanged(_)) => tasks += 1,
            _ => {}
        }
    }
    (plans, tasks)
}

/// Started with a lane, pushed, and read back in the plan with the lane on it
/// and the lane saying so; each write announces `plan_changed` once and never
/// `task_changed`.
#[tokio::test]
async fn a_train_round_trips_through_the_plan_and_announces() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let lane = h
        .service
        .store
        .create_lane(repo.workspace, &NewLane { name: "mac-ux".into(), ..Default::default() }, &[], None, Actor::Manager)
        .unwrap();
    let mut a = connect(&h).await;
    let mut listener = connect(&h).await;

    let started = train(&mut a, start_train(repo.workspace, "integ-14", &[lane.id]), "train.start").await;
    assert_eq!((started.name.as_str(), started.state), ("integ-14", pb::BoardTrainState::Integrating as i32));
    assert_eq!(started.lane_ids, vec![id(lane.id)]);
    assert_eq!(started.ci_subject, "");
    let pushed = train(&mut a, set(&started, pb::BoardTrainState::Unspecified, Some("1A1B3275")), "train.set").await;
    assert_eq!((pushed.state, pushed.pushed_sha.as_deref()), (pb::BoardTrainState::Pushed as i32, Some("1a1b3275")));
    assert_eq!(pushed.ci_subject, "sha:1a1b3275");

    let read = plan(&mut a, repo.workspace).await;
    assert_eq!(read.trains.len(), 1);
    assert_eq!(read.trains[0].lane_ids, vec![id(lane.id)]);
    assert_eq!(read.lanes[0].train.as_deref(), Some("integ-14"), "the lane says it's on the train");
    assert!(read.ci.is_empty(), "nothing read yet");

    let (plans, tasks) = events(&mut listener, Duration::from_millis(400)).await;
    assert_eq!((plans, tasks), (2, 0));
}

/// Refusals arrive as the words the CLI maps to sentences.
#[tokio::test]
async fn refusals_name_what_was_wrong() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let mut a = connect(&h).await;
    let refused = |r: Result<result::Value, ClientError>| match r {
        Err(ClientError::Daemon { code, what, .. }) => {
            assert_eq!(code, ErrorCode::InvalidArgument as i32);
            what
        }
        other => panic!("expected a refusal, got {other:?}"),
    };
    assert_eq!(refused(call(&mut a, "train.start", BOARD_TRAINS, start_train(repo.workspace, "a b", &[])).await), "name");
    let t = train(&mut a, start_train(repo.workspace, "integ-1", &[]), "train.start").await;
    let again = call(&mut a, "train.start", BOARD_TRAINS, start_train(repo.workspace, "INTEG-1", &[])).await;
    assert_eq!(refused(again), "name_taken");
    assert_eq!(refused(call(&mut a, "train.set", BOARD_TRAINS, set(&t, pb::BoardTrainState::Green, None)).await), "sha");
    assert_eq!(refused(call(&mut a, "train.set", BOARD_TRAINS, set(&t, pb::BoardTrainState::Unspecified, Some("nope"))).await), "sha");
    train(&mut a, set(&t, pb::BoardTrainState::Dropped, None), "train.set").await;
    let moved = call(&mut a, "train.set", BOARD_TRAINS, set(&t, pb::BoardTrainState::Gating, None)).await;
    assert_eq!(refused(moved), "train_settled");
    let mut bad = pb::TrainSet { train_id: t.id.clone(), state: 99, actor: "manager".into(), ..Default::default() };
    bad.base = None;
    assert_eq!(refused(call(&mut a, "train.set", BOARD_TRAINS, payload::Payload::TrainSet(bad)).await), "state");
}

/// A read client sees trains in the plan and can't write one.
#[tokio::test]
async fn a_read_client_reads_trains_and_cannot_write_them() {
    let h = start(Scope::Read).await;
    let repo = a_repository(&h);
    let new = farcooler_store::trains::NewTrain { name: "integ-2".into(), base: String::new() };
    h.service.store.start_train(repo.workspace, &new, &[], Actor::Manager).unwrap();
    let mut link = connect(&h).await;
    assert_eq!(plan(&mut link, repo.workspace).await.trains.len(), 1);
    match call(&mut link, "train.start", BOARD_TRAINS, start_train(repo.workspace, "integ-3", &[])).await {
        Err(ClientError::Daemon { code, .. }) => assert_eq!(code, ErrorCode::ScopeDenied as i32),
        other => panic!("expected a scope denial, got {other:?}"),
    }
}
