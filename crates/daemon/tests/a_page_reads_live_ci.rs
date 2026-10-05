//! A page's CI reference reads live (ov-306), end to end: publishing a page
//! that names main's CI kicks the runner's CI watch, and what a `gh` of the
//! test's own printed (`test/fixtures/ci/`, a real `gh`'s output) reaches
//! `plan.get`, where the apps draw the reference from.

use std::time::{Duration, Instant};

use farcooler_protocol::capability::{BOARD_PAGES, BOARD_PLAN};
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::request as request_for;

mod common;
#[path = "support/gh_ci.rs"]
mod gh_ci;
use gh_ci::*;

#[tokio::test]
async fn a_page_naming_main_reads_main_s_ci() {
    let (_dir, _daemon, mut client, workspace) = a_runner().await;
    let mut set = request_for("page.set");
    set.payload = Some(request::Payload::PageSet(pb::PageSet {
        workspace_id: workspace.clone(),
        slot: "trains".into(),
        doc_json: r#"{"v":1,"title":"Trains","blocks":[{"type":"stats","items":[{"label":"Main","ref":{"ci":"main"}}]}]}"#.into(),
        actor: "manager".into(),
        ..Default::default()
    }));
    call(&mut client, set, BOARD_PAGES).await;
    let deadline = Instant::now() + BUDGET;
    let main = loop {
        let mut get = request_for("plan.get");
        get.payload = Some(request::Payload::PlanGet(pb::PlanGetRequest { workspace_id: workspace.clone(), include_closed: true }));
        let result::Value::Plan(plan) = call(&mut client, get, BOARD_PLAN).await else { panic!("wrong result") };
        if let Some(main) = plan.ci.iter().find(|r| r.subject == "main") {
            break main.clone();
        }
        assert!(Instant::now() < deadline, "the watch never read main for the page: {plan:?}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert_eq!(main.status, pb::BoardCiStatus::Passed as i32);
    assert!(main.sha.starts_with("898ae57d"), "main's newest commit with runs: {}", main.sha);
    let mut names: Vec<&str> = main.jobs.iter().map(|j| j.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["CI", "Canary", "Canary wire baseline", "Doc comments"]);
}
