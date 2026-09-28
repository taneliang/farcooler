//! `Worktree.open_tasks`: a worktree names the tasks whose lane it is, and a
//! connected client hears `worktree_changed` whenever that list moves (spec
//! §3.2 item 1).

#[path = "support/in_process.rs"]
mod in_process;

use std::time::Duration;

use farcooler_protocol::v1::{self as pb, Scope, event, request as payload, result};
use farcooler_store::models::{Actor, TaskStatus};
use farcooler_transport::request;
use in_process::*;
use uuid::Uuid;

fn id(uuid: Uuid) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(uuid.as_bytes())
}

/// File a task on `lane` over the wire, as `farcooler task dispatch` does.
async fn a_task_on(h: &Harness, repo: &Repo, lane: Uuid, title: &str) -> pb::Task {
    let mut create = request("task.create");
    create.payload = Some(payload::Payload::TaskCreate(pb::TaskCreate {
        repository_id: id(repo.id),
        title: title.into(),
        worktree_id: Some(id(lane)),
        actor: "manager".into(),
        ..Default::default()
    }));
    let r = connect(h).await.call(create).await.expect("task.create");
    let Some(result::Value::Task(task)) = r.value else { panic!("wrong result") };
    task
}

fn another_lane(h: &Harness, repo: &Repo, name: &str) -> Uuid {
    let path = format!("/repos/ny-{name}");
    h.service.store.create_worktree(repo.id, name, &path, false).unwrap().id
}

/// Every `worktree_changed` for `lane` within a second, as the keys of its
/// open tasks.
async fn changes_to(listener: &mut Link, lane: Uuid) -> Vec<Vec<String>> {
    let mut seen = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    while let Ok(Ok(e)) = tokio::time::timeout_at(deadline, listener.next_event()).await {
        if let Some(event::Payload::WorktreeChanged(w)) = e.payload
            && w.id == id(lane)
        {
            seen.push(w.open_tasks.iter().map(|t| t.title.clone()).collect());
        }
    }
    seen
}

async fn update(h: &Harness, task: &pb::Task, title: &str, lane: Uuid) {
    let mut revise = request("task.update");
    revise.payload = Some(payload::Payload::TaskUpdate(pb::TaskUpdate {
        task_id: task.id.clone(),
        expected_version: task.resource_version,
        title: title.into(),
        worktree_id: Some(id(lane)),
        actor: "manager".into(),
        ..Default::default()
    }));
    connect(h).await.call(revise).await.expect("task.update");
}

#[tokio::test]
async fn a_worktree_lists_its_open_tasks_and_not_its_done_ones() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let lane = another_lane(&h, &repo, "fc-3-webhooks");
    let open = a_task_on(&h, &repo, lane, "Invoice PDF export").await;
    let done = a_task_on(&h, &repo, lane, "Webhook retries").await;
    let done_id = Uuid::from_slice(&done.id).unwrap();
    h.service.store.set_task_status(done_id, TaskStatus::Done, Actor::Manager).unwrap();
    let cancelled = a_task_on(&h, &repo, lane, "Old idea").await;
    let cancelled_id = Uuid::from_slice(&cancelled.id).unwrap();
    h.service.store.set_task_status(cancelled_id, TaskStatus::Cancelled, Actor::Manager).unwrap();

    let r = connect(&h).await.call(request("worktree.list")).await.expect("worktree.list");
    let Some(result::Value::WorktreeList(list)) = r.value else { panic!("wrong result") };
    let row = list.items.iter().find(|w| w.id == id(lane)).expect("the lane is listed");
    let named: Vec<_> = row.open_tasks.iter().map(|t| (t.key.as_str(), t.title.as_str())).collect();
    assert_eq!(named, [(open.key.as_str(), "Invoice PDF export")]);
    assert_eq!(row.open_tasks[0].status, pb::TaskStatus::Backlog as i32);
    let main = list.items.iter().find(|w| w.id == id(repo.worktree)).expect("main is listed");
    assert!(main.open_tasks.is_empty(), "a task on another lane leaked onto main");
}

#[tokio::test]
async fn moving_a_tasks_lane_changes_both_worktrees() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let (from, to) = (another_lane(&h, &repo, "one"), another_lane(&h, &repo, "two"));
    let task = a_task_on(&h, &repo, from, "Invoice PDF export").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut listener = connect(&h).await;
    update(&h, &task, "Invoice PDF export", to).await;
    let (mut left, mut arrived) = (Vec::new(), Vec::new());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    while let Ok(Ok(e)) = tokio::time::timeout_at(deadline, listener.next_event()).await {
        if let Some(event::Payload::WorktreeChanged(w)) = e.payload {
            let titles: Vec<String> = w.open_tasks.iter().map(|t| t.title.clone()).collect();
            if w.id == id(from) {
                left.push(titles);
            } else if w.id == id(to) {
                arrived.push(titles);
            }
        }
    }
    assert_eq!(left, [Vec::<String>::new()], "the lane it left still names it");
    assert_eq!(arrived, [vec!["Invoice PDF export".to_string()]]);
}

#[tokio::test]
async fn finishing_a_task_changes_its_worktree() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let lane = another_lane(&h, &repo, "one");
    let task = a_task_on(&h, &repo, lane, "Invoice PDF export").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut listener = connect(&h).await;
    let mut finish = request("task.set_status");
    finish.payload = Some(payload::Payload::TaskSetStatus(pb::TaskSetStatus {
        task_id: task.id.clone(),
        status: pb::TaskStatus::Done as i32,
        actor: "manager".into(),
    }));
    connect(&h).await.call(finish).await.expect("task.set_status");
    assert_eq!(changes_to(&mut listener, lane).await, [Vec::<String>::new()]);
}

#[tokio::test]
async fn renaming_a_task_changes_its_worktree() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let lane = another_lane(&h, &repo, "one");
    let task = a_task_on(&h, &repo, lane, "Invoice PDF export").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut listener = connect(&h).await;
    update(&h, &task, "Invoice PDF and CSV export", lane).await;
    assert_eq!(changes_to(&mut listener, lane).await, [vec!["Invoice PDF and CSV export".to_string()]]);
}
