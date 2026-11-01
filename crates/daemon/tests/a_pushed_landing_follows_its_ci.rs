//! A pushed landing follows its CI (ov-309, ov-872), end to end: the real `farcoolerd`,
//! listening on its socket as a runner does, with a `gh` of this test's own
//! ahead on `PATH` that prints what a real `gh` printed for this repository
//! (`test/fixtures/ci/`).
//!
//! What this proves that the unit tests can't: that the daemon actually runs
//! the CI watch (`main.rs` starts it, and nothing else does), that giving a
//! landing a SHA kicks it to read at once, that a short SHA is resolved
//! through `gh`, and that the read reaches `plan.get` with the landing moved
//! to red.

use std::time::{Duration, Instant};

use farcooler_protocol::capability::{BOARD_LINE, BOARD_PLAN};
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::request as request_for;

mod common;
#[path = "support/gh_ci.rs"]
mod gh_ci;
use gh_ci::*;

#[tokio::test]
async fn a_pushed_landing_turns_red_with_its_ci() {
    let (_dir, _daemon, mut client, workspace) = a_runner().await;
    let mut start = request_for("landing.start");
    start.payload = Some(request::Payload::LandingStart(pb::LandingStart {
        workspace_id: workspace.clone(),
        route: pb::LandingRoute::Push as i32,
        base: "origin/main".into(),
        actor: "manager".into(),
        ..Default::default()
    }));
    let result::Value::Landing(landing) = call(&mut client, start, BOARD_LINE).await else { panic!("wrong result") };
    let mut push = request_for("landing.set");
    push.payload = Some(request::Payload::LandingSet(pb::LandingSet {
        landing_id: landing.id.clone(),
        sha: Some("c85bf83d".into()),
        actor: "manager".into(),
        ..Default::default()
    }));
    let result::Value::Landing(pushed) = call(&mut client, push, BOARD_LINE).await else { panic!("wrong result") };
    assert_eq!(pushed.state, pb::LandingState::Pushed as i32, "a SHA with no state is pushed");

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
    assert_eq!(plan.ci.len(), 1, "the watch read the landing's SHA: {plan:?}");
    let read = &plan.ci[0];
    assert_eq!(read.subject, "sha:c85bf83d");
    assert_eq!(read.sha, "c85bf83dce46a6b71d7312afc623899ae7914658", "resolved through gh");
    assert_eq!(read.status, pb::BoardCiStatus::Failed as i32);
    let swift = read.jobs.iter().find(|j| j.name == "CI / Swift (shared + macOS)").expect("the CI run's jobs");
    assert_eq!(swift.state, "failed");
    let landing = plan.landings.iter().find(|l| l.id == landing.id).expect("the landing");
    assert_eq!(landing.state, pb::LandingState::Red as i32, "the landing follows its CI");
    assert_eq!(landing.phase, pb::LandingPhase::CiFailed as i32);
}
