//! The PR watch's turn answers how long to wait (ov-312): a minute while any
//! ticket is in review, fixing or landing, and the slow look-for-one pace
//! while none is. In process, so a test can call a turn and read its answer.
//!
//! The repository here has no checkout on disk, so `gh` is never started: a
//! turn that reads nothing is exactly what this asks about.

#[path = "support/in_process.rs"]
mod in_process;

use std::time::Duration;

use farcooler_daemon::pr_watch::{ACTIVE, IDLE, Memo, tick};
use farcooler_protocol::v1::Scope;
use farcooler_store::agents::{AgentPlace, AgentRole, NewAgent};
use farcooler_store::models::Actor;
use farcooler_store::ticket_lines::{LineUpdate, NewTicketLine, Stage};
use farcooler_store::ticket_work::{LinkedBy, WorkRole};
use in_process::*;

#[tokio::test]
async fn a_turn_waits_a_minute_while_a_ticket_is_in_review_and_the_idle_pace_otherwise() {
    let h = start(Scope::HostAdmin).await;
    let repo = a_repository(&h);
    let store = &h.service.store;
    let mut memo = Memo::default();
    let task = store.create_task(repo.workspace, "Card", Actor::User).unwrap();
    let to = |stage: Stage| LineUpdate { stage: Some(stage), ..Default::default() };

    // Nothing planned.
    assert_eq!(tick(&h.service, &h.watcher, &mut memo, &Default::default()).await, IDLE);

    // A ticket still building waits on no pull request.
    let new = NewTicketLine { branch: "work".into(), ..Default::default() };
    store.put_on_line(task.id, &new, Actor::Manager).unwrap();
    store.update_line(task.id, &to(Stage::Building), Actor::Manager).unwrap();
    assert_eq!(tick(&h.service, &h.watcher, &mut memo, &Default::default()).await, IDLE);

    // In review, fixing, landing: each is waiting on one.
    store.update_line(task.id, &to(Stage::Reviewing), Actor::Manager).unwrap();
    assert_eq!(tick(&h.service, &h.watcher, &mut memo, &Default::default()).await, ACTIVE, "reviewing");
    let builder = NewAgent {
        role: AgentRole::Builder,
        place: AgentPlace::Subagent { id: "a1".into(), parent: None },
        harness: "claude".into(),
        session_id: None,
        session_cwd: None,
        model: Some("sonnet".into()),
        title: None,
    };
    let builder = store.record_agent(repo.workspace, &builder).unwrap();
    store.start_work(builder.id, task.id, WorkRole::Fix, LinkedBy::Orchestrator, Actor::Manager).unwrap();
    store.update_line(task.id, &to(Stage::Building), Actor::Manager).unwrap();
    assert_eq!(tick(&h.service, &h.watcher, &mut memo, &Default::default()).await, ACTIVE, "fixing");
    for stage in [Stage::Reviewing, Stage::Landing] {
        store.update_line(task.id, &to(stage), Actor::Manager).unwrap();
        assert_eq!(tick(&h.service, &h.watcher, &mut memo, &Default::default()).await, ACTIVE, "{stage:?}");
    }

    // Landed: done waiting.
    let landed = LineUpdate { stage: Some(Stage::Landed), landed_sha: Some("abc".into()), ..Default::default() };
    store.update_line(task.id, &landed, Actor::Manager).unwrap();
    assert_eq!(tick(&h.service, &h.watcher, &mut memo, &Default::default()).await, IDLE);
    assert_eq!(ACTIVE, Duration::from_secs(60));
}
