//! `Plan.suggested_reviewers` names GitHub logins, so it goes only to a peer at
//! Control scope (ov-580, as `review_waits` do): over a real socket, through the
//! real dispatch and scope check. A pull request that already asks someone, or
//! that someone has reviewed, gets none.

#[path = "support/in_process.rs"]
mod in_process;

use farcooler_core::reviewers::{Source, Suggestion};
use farcooler_daemon::pr_timing::{Moment, PrTiming};
use farcooler_daemon::stack::{CheckState, PrInfo, PrReview, PrState, PrStatus, ReviewDecision};
use farcooler_protocol::v1::{self as pb, Scope};
use farcooler_store::models::Actor;
use farcooler_store::plan::{LaneCard, LaneState, LaneUpdate, NewLane};
use in_process::*;

/// A runner with one lane in review whose open PR (number 31, head `abc`) asks
/// `asks`, with alice and bob suggested for it.
async fn a_runner_with_suggestions(scope: Scope, asks: &[&str], first_review: Moment) -> (Harness, Repo, Vec<u8>) {
    let h = start(scope).await;
    let repo = a_repository(&h);
    let store = &h.service.store;
    let task = store.create_task(repo.workspace, "Needs a reviewer", Actor::User).unwrap();
    let key = store.get_task(task.id).unwrap().key;
    let new = NewLane { name: "review".into(), reason: "Because".into(), branch: "lane/review".into(), ..Default::default() };
    let lane = store.create_lane(repo.workspace, &new, &[LaneCard { task_id: task.id, slice: String::new() }], None, Actor::Manager).unwrap();
    for state in [LaneState::Building, LaneState::Review] {
        store.update_lane(lane.id, &LaneUpdate { state: Some(state), ..Default::default() }, Actor::Manager).unwrap();
    }
    let pr = PrInfo {
        head_ref: format!("{key}/work"),
        base_ref: "main".into(),
        title: format!("{key}: work"),
        is_fork: false,
        author: "owner".into(),
        review: PrReview { requested: asks.iter().map(|a| a.to_string()).collect(), ..Default::default() },
        timing: PrTiming { ready: Moment::At(1_000), first_review, ..Default::default() },
        status: PrStatus {
            number: 31,
            url: "https://github.example/o/r/pull/31".into(),
            state: PrState::Open,
            checks: CheckState::Unknown,
            review_decision: ReviewDecision::Unknown,
            head_oid: "abc".into(),
            merged_at: None,
            fetched_at: 0,
        },
    };
    h.service.pr_cache_put(repo.id, Some(vec![pr]));
    let some = |reviewer: &str, source| Suggestion { reviewer: reviewer.into(), source };
    farcooler_daemon::pr_suggest::put(repo.id, 31, "abc", vec![some("alice", Source::Codeowners), some("bob", Source::History)]);
    (h, repo, lane.id.as_bytes().to_vec())
}

async fn suggested(h: &Harness, repo: &Repo) -> Vec<pb::LaneReviewers> {
    let mut link = connect(h).await;
    let p = pb::request::Payload::PlanGet(pb::PlanGetRequest { workspace_id: repo.workspace.as_bytes().to_vec().into(), include_closed: false });
    let mut req = farcooler_transport::request("plan.get");
    req.payload = Some(p);
    req.required_capabilities = vec![farcooler_protocol::capability::BOARD_PLAN.into()];
    match link.call(req).await.expect("plan.get").value {
        Some(pb::result::Value::Plan(plan)) => plan.suggested_reviewers,
        other => panic!("wrong result: {other:?}"),
    }
}

#[tokio::test]
async fn a_control_peer_sees_whom_to_ask_and_where_from_and_a_read_peer_sees_none() {
    let (control, repo, lane) = a_runner_with_suggestions(Scope::Control, &[], Moment::NotYet).await;
    let seen = suggested(&control, &repo).await;
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!((seen[0].lane_id.to_vec(), seen[0].pr), (lane, 31));
    assert_eq!(seen[0].reviewers, ["alice", "bob"]);
    assert_eq!(seen[0].sources, ["CODEOWNERS", "recent commits"]);

    let (read, repo, _) = a_runner_with_suggestions(Scope::Read, &[], Moment::NotYet).await;
    assert!(suggested(&read, &repo).await.is_empty(), "a Read peer is told no one's login");
}

#[tokio::test]
async fn a_pull_request_that_already_asks_someone_or_was_reviewed_gets_no_suggestion() {
    let (asked, repo, _) = a_runner_with_suggestions(Scope::Control, &["carol"], Moment::NotYet).await;
    assert!(suggested(&asked, &repo).await.is_empty());
    let (reviewed, repo, _) = a_runner_with_suggestions(Scope::Control, &[], Moment::At(2_000)).await;
    assert!(suggested(&reviewed, &repo).await.is_empty());
}
