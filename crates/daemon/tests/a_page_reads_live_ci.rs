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
    // Main is at c85bf83d (the shim's commits/main), whose CI failed; a run
    // created later on an older commit (runs-main.json's 898ae57d) mustn't
    // stand for it (review train-1005c M3).
    assert!(main.sha.starts_with("c85bf83d"), "the commit main is at: {}", main.sha);
    assert_eq!(main.status, pb::BoardCiStatus::Failed as i32);
}
