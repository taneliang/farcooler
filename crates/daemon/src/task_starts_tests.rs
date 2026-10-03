use std::sync::Arc;

use farcooler_protocol::v1::event::Payload;
use farcooler_store::models::{NoteKind, TaskStatus};
use tokio::sync::broadcast::Receiver;

use super::*;

/// A runner with one board: `n` tasks on Main, in the backlog, and a
/// watcher whose events the test hears.
async fn runner(n: usize) -> (tempfile::TempDir, Arc<Service>, Arc<Watcher>, Receiver<pb::Event>, Vec<Task>) {
    let dir = tempfile::tempdir().unwrap();
    let (repository, first) = (Uuid::now_v7(), Uuid::now_v7());
    farcooler_store::testing::write_prefixless_board_at_schema_11(&dir.path().join("farcooler.db"), repository, first);
    let svc = Arc::new(Service::open_in(dir.path().to_path_buf()).await.unwrap());
    let main = svc.store.ensure_main_workspace(repository).unwrap().id;
    let tasks = (0..n).map(|i| svc.store.create_task(main, &format!("task {i}"), Actor::Manager).unwrap()).collect();
    let watcher = Watcher::new(svc.clone());
    let events = watcher.subscribe();
    (dir, svc, watcher, events, tasks)
}

/// Every task a `TaskChanged` named since the last call.
fn changed(events: &mut Receiver<pb::Event>) -> Vec<Uuid> {
    let mut ids = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let Some(Payload::TaskChanged(c)) = event.payload {
            ids.push(Uuid::from_slice(&c.task_id).unwrap());
        }
    }
    ids
}

/// (5) At a time the test picks: before a hold is due nothing happens;
/// once it is, the hold is let go, the runner says so on the card, and every
/// board hears about it. Without the announce, a board would go on reading
/// "Starts at 9:00 AM" until something else moved the card.
#[tokio::test]
async fn a_hold_is_let_go_when_its_time_comes_and_boards_hear() {
    let (_dir, svc, watcher, mut events, t) = runner(1).await;
    let due = farcooler_store::testing::now_millis() + 3_600_000;
    svc.store.set_wait(t[0].id, Some(Wait::Until(due)), Actor::Manager).unwrap();

    watcher.release_due_holds(due - 1);
    assert!(changed(&mut events).is_empty(), "not yet");
    assert!(svc.store.get_task(t[0].id).unwrap().wait.is_some());

    watcher.release_due_holds(due);
    assert_eq!(changed(&mut events), [t[0].id]);
    assert_eq!(svc.store.get_task(t[0].id).unwrap().wait, None);
    let note = svc.store.notes_for(t[0].id, Some(NoteKind::Wait)).unwrap().pop().unwrap();
    assert_eq!(note.actor, Actor::Runner);
    assert!(note.body.ends_with("That time has come."), "{}", note.body);
}

/// Both `task block` bugs, on the wire: a done blocker drops out of
/// `waiting_on`, and the task it blocked is announced when it finishes, so
/// its board re-reads rather than going on saying "Waiting on ov-36".
#[tokio::test]
async fn a_blocker_finishing_announces_what_it_blocked() {
    let (_dir, svc, watcher, mut events, t) = runner(2).await;
    let (phase_b, follower) = (&t[0], &t[1]);
    svc.store.set_block(follower.id, phase_b.id, Some("follows Phase B landing")).unwrap();
    let read = |svc: &Service| pb_one(svc, &svc.store.get_task(follower.id).unwrap()).unwrap().waiting_on;
    assert_eq!(read(&svc), [phase_b.key.clone()]);

    let done = pb::TaskSetStatus {
        task_id: id_bytes(phase_b.id),
        status: pb::TaskStatus::Done as i32,
        actor: "manager".into(),
    };
    crate::task_ops::set_status(&svc, &watcher, &done).unwrap();
    assert!(read(&svc).is_empty());
    let heard = changed(&mut events);
    assert!(heard.contains(&follower.id), "the follower's board must re-read: {heard:?}");
}

/// The wire's line: a place, who's ahead, and what the route answers.
#[tokio::test]
async fn a_line_reaches_the_wire_with_its_positions() {
    let (_dir, svc, watcher, mut events, t) = runner(3).await;
    let workspace = t[0].workspace_id;
    let set = |ids: &[&Task]| pb::TaskSetLine {
        workspace_id: id_bytes(workspace),
        line: pb::TaskLine::Agent as i32,
        task_ids: ids.iter().map(|t| id_bytes(t.id)).collect(),
        actor: "manager".into(),
    };
    set_line(&svc, &watcher, &set(&[&t[0], &t[1], &t[2]])).unwrap();
    changed(&mut events);
    let line = set_line(&svc, &watcher, &set(&[&t[2], &t[0]])).unwrap().items;

    let wait = line[1].wait.clone().unwrap();
    assert_eq!((wait.kind, wait.line, wait.position), (pb::TaskWaitKind::InLine as i32, pb::TaskLine::Agent as i32, 2));
    assert_eq!(wait.ahead, [t[2].key.clone()]);
    let heard = changed(&mut events);
    assert!(heard.contains(&t[1].id), "the one that left the line is announced too: {heard:?}");
    let left = pb_one(&svc, &svc.store.get_task(t[1].id).unwrap()).unwrap();
    assert_eq!(left.wait, None);
}

/// `task.set_wait` on the wire: a hold, an event without one refused, and
/// IN_LINE refused, since a line is set whole.
#[tokio::test]
async fn set_wait_reads_the_wire() {
    let (_dir, svc, watcher, _events, t) = runner(1).await;
    let ask = |kind: pb::TaskWaitKind, event: pb::TaskWaitEvent| pb::TaskSetWait {
        task_id: id_bytes(t[0].id),
        kind: kind as i32,
        until: 0,
        event: event as i32,
        actor: "manager".into(),
    };
    let held = set_wait(&svc, &watcher, &ask(pb::TaskWaitKind::After, pb::TaskWaitEvent::Release)).unwrap();
    assert_eq!(held.wait.unwrap().event, pb::TaskWaitEvent::Release as i32);
    let refused = |kind, event| set_wait(&svc, &watcher, &ask(kind, event)).unwrap_err();
    assert!(matches!(refused(pb::TaskWaitKind::After, pb::TaskWaitEvent::Unspecified), DomainError::InvalidArgument { what: "event" }));
    assert!(matches!(refused(pb::TaskWaitKind::InLine, pb::TaskWaitEvent::Unspecified), DomainError::InvalidArgument { what: "kind" }));
    let cleared = set_wait(&svc, &watcher, &ask(pb::TaskWaitKind::Unspecified, pb::TaskWaitEvent::Unspecified)).unwrap();
    assert_eq!(cleared.wait, None);
}

/// `task.worker` on the wire: recorded, the task started, and the worker
/// shown as recorded and not observed; then ended.
#[tokio::test]
async fn a_subagent_recorded_on_the_wire() {
    let (_dir, svc, watcher, _events, t) = runner(1).await;
    let mut ask = pb::TaskWorkerSet {
        task_id: id_bytes(t[0].id),
        harness: "claude".into(),
        agent_id: "a3fd8fceef581c787".into(),
        session_id: Some("76e86926".into()),
        label: Some("ov-1 Mac polish".into()),
        actor: "manager".into(),
        ..Default::default()
    };
    let task = worker(&svc, &watcher, &ask, None).unwrap();
    assert_eq!(task.status, pb::TaskStatus::InProgress as i32);
    let w = &task.workers[0];
    assert_eq!((w.state, w.label.as_str(), w.ended_at), (pb::TaskWorkerState::Unobserved as i32, "ov-1 Mac polish", 0));

    ask.end = true;
    ask.end_reason = "stopped".into();
    let task = worker(&svc, &watcher, &ask, None).unwrap();
    assert_eq!(task.workers[0].state, pb::TaskWorkerState::Stopped as i32);
    assert_eq!(svc.store.get_task(t[0].id).unwrap().status, TaskStatus::InProgress, "an end moves nothing");
    ask.end_reason = "vanished".into();
    assert!(matches!(worker(&svc, &watcher, &ask, None), Err(DomainError::InvalidArgument { what: "end_reason" })));
}
