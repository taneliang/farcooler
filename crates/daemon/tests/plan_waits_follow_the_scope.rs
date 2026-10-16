//! `Plan.review_waits` names GitHub logins, so it goes only to a peer at
//! Control scope (ov-579, as ov-315's review requests do): over a real socket,
//! through the real dispatch and scope check.

#[path = "support/in_process.rs"]
mod in_process;

use farcooler_daemon::pr_timing::{Moment, PrTiming};
use farcooler_daemon::stack::{CheckState, PrInfo, PrReview, PrState, PrStatus, ReviewDecision};
use farcooler_protocol::v1::{self as pb, Scope};
use farcooler_store::models::Actor;
use farcooler_store::plan::{LaneCard, LaneState, LaneUpdate, NewLane};
use in_process::*;

/// A runner with one lane in review whose ready PR asks alice, and alice's one
/// finished review in the cache: the plan then has a wait for her.
async fn a_runner_waiting_on_alice(scope: Scope) -> (Harness, Repo) {
    let h = start(scope).await;
    let repo = a_repository(&h);
    let store = &h.service.store;
    let task = store.create_task(repo.workspace, "Waits on alice", Actor::User).unwrap();
    let key = store.get_task(task.id).unwrap().key;
    let new = NewLane { name: "review".into(), reason: "Because".into(), branch: "lane/review".into(), ..Default::default() };
    let lane = store.create_lane(repo.workspace, &new, &[LaneCard { task_id: task.id, slice: String::new() }], None, Actor::Manager).unwrap();
    for state in [LaneState::Building, LaneState::Review] {
        store.update_lane(lane.id, &LaneUpdate { state: Some(state), ..Default::default() }, Actor::Manager).unwrap();
    }
    let pr = |number: u32, asked: &[&str], first: Option<i64>| PrInfo {
        head_ref: format!("{key}/work"),
        base_ref: "main".into(),
        title: format!("{key}: work"),
        is_fork: false,
        review: PrReview { requested: asked.iter().map(|a| a.to_string()).collect(), ..Default::default() },
        timing: PrTiming {
            ready: Moment::At(1_000),
            first_by: first.map(|at| farcooler_daemon::pr_timing::FirstReview { reviewer: "alice".into(), at }).into_iter().collect(),
            ..Default::default()
        },
        status: PrStatus {
            number,
            url: format!("https://github.example/o/r/pull/{number}"),
            state: PrState::Open,
            checks: CheckState::Unknown,
            review_decision: ReviewDecision::Unknown,
            head_oid: String::new(),
            merged_at: None,
            fetched_at: now_millis(),
        },
    };
    h.service.pr_cache_put(repo.id, Some(vec![pr(1, &["alice"], Some(1_000 + 600_000))]));
    (h, repo)
}

async fn waits(h: &Harness, repo: &Repo) -> Vec<pb::ReviewerWait> {
    let mut link = connect(h).await;
    let p = pb::request::Payload::PlanGet(pb::PlanGetRequest { workspace_id: repo.workspace.as_bytes().to_vec().into(), include_closed: false });
    let mut req = farcooler_transport::request("plan.get");
    req.payload = Some(p);
    req.required_capabilities = vec![farcooler_protocol::capability::BOARD_PLAN.into()];
    match link.call(req).await.expect("plan.get").value {
        Some(pb::result::Value::Plan(plan)) => plan.review_waits,
        other => panic!("wrong result: {other:?}"),
    }
}

#[tokio::test]
async fn a_control_peer_sees_who_the_board_waits_on_and_a_read_peer_sees_none() {
    let (control, repo) = a_runner_waiting_on_alice(Scope::Control).await;
    let seen = waits(&control, &repo).await;
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!((seen[0].reviewer.as_str(), seen[0].prs, seen[0].median_wait_ms), ("alice", 1, Some(600_000)));

    let (read, repo) = a_runner_waiting_on_alice(Scope::Read).await;
    assert!(waits(&read, &repo).await.is_empty(), "a Read peer is told no one's login");
}
