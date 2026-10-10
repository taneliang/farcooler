//! The PR watch's turn answers how long to wait (ov-312): a minute while any
//! lane is in review, fixing or landing, and the slow look-for-one pace while
//! none is. In process, so a test can call a turn and read its answer.
//!
//! The repository here has no checkout on disk, so `gh` is never started: a
//! turn that reads nothing is exactly what this asks about.

#[path = "support/in_process.rs"]
mod in_process;

use std::time::Duration;

use farcooler_daemon::pr_watch::{ACTIVE, IDLE, Memo, tick};
use farcooler_protocol::v1::Scope;
use farcooler_store::models::Actor;
use farcooler_store::plan::{AgentRecord, AgentRole, LaneCard, LaneState, LaneUpdate, NewLane};
use in_process::*;

fn new_lane(name: &str) -> NewLane {
    NewLane {
        name: name.into(),
        title: String::new(),
        reason: String::new(),
        worktree_id: None,
        worktree_path: String::new(),
        branch: name.into(),
        harness: "claude".into(),
        model: "sonnet".into(),
    }
}

#[tokio::test]
async fn a_turn_waits_a_minute_while_a_lane_is_in_review_and_the_idle_pace_otherwise() {
    let h = start(Scope::HostAdmin).await;
    let repo = a_repository(&h);
    let store = &h.service.store;
    let mut memo = Memo::default();
    let task = store.create_task(repo.workspace, "Card", Actor::User).unwrap();
    let cards = [LaneCard { task_id: task.id, slice: String::new() }];

    // Nothing planned.
    assert_eq!(tick(&h.service, &h.watcher, &mut memo, &Default::default()).await, IDLE);

    // A lane still building waits on no pull request.
    let agent = AgentRecord { harness: "claude".into(), agent_id: "a1".into(), role: AgentRole::Build, model: None, ended: false };
    let lane = store.create_lane(repo.workspace, &new_lane("work"), &cards, Some(&agent), Actor::Manager).unwrap();
    assert_eq!(lane.state, LaneState::Building);
    assert_eq!(tick(&h.service, &h.watcher, &mut memo, &Default::default()).await, IDLE);

    // In review, fixing, landing: each is waiting on one.
    // Along the arrows: review, fixing, back to review, landing.
    for state in [LaneState::Review, LaneState::Fixing, LaneState::Review, LaneState::Landing] {
        store.update_lane(lane.id, &LaneUpdate { state: Some(state), ..Default::default() }, Actor::Manager).unwrap();
        assert_eq!(tick(&h.service, &h.watcher, &mut memo, &Default::default()).await, ACTIVE, "{state:?}");
    }

    // Landed: done waiting.
    let landed = LaneUpdate { state: Some(LaneState::Landed), landed_sha: Some("abc".into()), ..Default::default() };
    store.update_lane(lane.id, &landed, Actor::Manager).unwrap();
    assert_eq!(tick(&h.service, &h.watcher, &mut memo, &Default::default()).await, IDLE);
    assert_eq!(ACTIVE, Duration::from_secs(60));
}
