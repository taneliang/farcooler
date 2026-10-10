//! A pushed train follows its CI (ov-309), end to end: the real `farcoolerd`,
//! listening on its socket as a runner does, with a `gh` of this test's own
//! ahead on `PATH` that prints what a real `gh` printed for this repository
//! (`test/fixtures/ci/`).
//!
//! What this proves that the unit tests can't: that the daemon actually runs
//! the CI watch (`main.rs` starts it, and nothing else does), that giving a
//! train a SHA kicks it to read at once, that a short SHA is resolved through
//! `gh`, and that the read reaches `plan.get` with the train moved to red.

use std::time::{Duration, Instant};

use farcooler_protocol::capability::{BOARD_PLAN, BOARD_TRAINS};
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::request as request_for;

mod common;
#[path = "support/gh_ci.rs"]
mod gh_ci;
use gh_ci::*;

#[tokio::test]
async fn a_pushed_train_turns_red_with_its_ci() {
    let (_dir, _daemon, mut client, workspace) = a_runner().await;
    let mut start = request_for("train.start");
    start.payload = Some(request::Payload::TrainStart(pb::TrainStart {
        workspace_id: workspace.clone(),
        name: "integ-9".into(),
        base: "origin/main".into(),
        lane_ids: vec![],
        actor: "manager".into(),
        ..Default::default()
    }));
    let result::Value::BoardTrain(train) = call(&mut client, start, BOARD_TRAINS).await else { panic!("wrong result") };
    let mut push = request_for("train.set");
    push.payload = Some(request::Payload::TrainSet(pb::TrainSet {
        train_id: train.id.clone(),
        sha: Some("c85bf83d".into()),
        actor: "manager".into(),
        ..Default::default()
    }));
    call(&mut client, push, BOARD_TRAINS).await;

    let deadline = Instant::now() + BUDGET;
    let plan = loop {
        let mut get = request_for("plan.get");
        get.payload = Some(request::Payload::PlanGet(pb::PlanGetRequest { workspace_id: workspace.clone(), include_closed: true }));
        let result::Value::Plan(plan) = call(&mut client, get, BOARD_PLAN).await else { panic!("wrong result") };
        if !plan.ci.is_empty() || Instant::now() > deadline {
            break plan;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert_eq!(plan.ci.len(), 1, "the watch read the train's SHA: {plan:?}");
    let read = &plan.ci[0];
    assert_eq!(read.subject, "sha:c85bf83d");
    assert_eq!(read.sha, "c85bf83dce46a6b71d7312afc623899ae7914658", "resolved through gh");
    assert_eq!(read.status, pb::BoardCiStatus::Failed as i32);
    let swift = read.jobs.iter().find(|j| j.name == "CI / Swift (shared + macOS)").expect("the CI run's jobs");
    assert_eq!(swift.state, "failed");
    assert_eq!(plan.trains[0].state, pb::BoardTrainState::Red as i32, "the train follows its CI");
}
