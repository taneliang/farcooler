//! Read state on the runner (ov-113), as two devices meet it: over real
//! sockets, through the real dispatch table and scope check.
//!
//! Each test here is the one that goes red when the call it is about is
//! removed: the announce, the scope, the board's reads on `task.list`.

#[path = "support/in_process.rs"]
mod in_process;

use std::time::Duration;

use farcooler_protocol::v1::{self as pb, ErrorCode, Scope, event, request as payload, result};
use farcooler_store::models::Actor;
use farcooler_transport::{ClientError, request};
use in_process::*;
use uuid::Uuid;

fn id(uuid: Uuid) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(uuid.as_bytes())
}

fn mark(workspace: Uuid, opened: &[(Uuid, i64)], floor: Option<i64>) -> pb::Request {
    let mut r = request("workspace.mark_read");
    r.payload = Some(payload::Payload::WorkspaceMarkRead(pb::WorkspaceMarkRead {
        workspace_id: id(workspace),
        floor_ms: floor,
        opened: opened.iter().map(|(t, ms)| pb::TaskRead { task_id: id(*t), opened_ms: *ms }).collect(),
    }));
    r
}

async fn reads_of(link: &mut Link, req: pb::Request) -> Result<pb::BoardReads, ClientError> {
    match link.call(req).await?.value {
        Some(result::Value::BoardReads(r)) => Ok(r),
        other => panic!("wrong result: {other:?}"),
    }
}

async fn list(link: &mut Link, workspace: Option<Uuid>, repository: Uuid) -> pb::TaskList {
    let mut r = request("task.list");
    r.payload = Some(payload::Payload::TaskList(pb::TaskListRequest {
        repository_id: id(repository),
        workspace_id: workspace.map(id),
        ..Default::default()
    }));
    match link.call(r).await.expect("task.list").value {
        Some(result::Value::TaskList(l)) => l,
        other => panic!("wrong result: {other:?}"),
    }
}

/// The `board_reads_changed` events on `listener` within `window`.
async fn events(listener: &mut Link, window: Duration) -> Vec<pb::BoardReads> {
    let mut seen = vec![];
    let deadline = tokio::time::Instant::now() + window;
    while let Ok(Ok(e)) = tokio::time::timeout_at(deadline, listener.next_event()).await {
        if let Some(event::Payload::BoardReadsChanged(r)) = e.payload {
            seen.push(r);
        }
    }
    seen
}

/// Acceptance 1, cross-device: A opens a ticket, B hears A's mark, and a read
/// of the board on a third connection carries it too.
#[tokio::test]
async fn a_mark_on_one_device_reaches_the_others() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let task = h.service.store.create_task(repo.workspace, "Mac: a jump", Actor::User).unwrap();
    let mut b = connect(&h).await;
    let mut a = connect(&h).await;

    let at = now_millis() - 1_000;
    let merged = reads_of(&mut a, mark(repo.workspace, &[(task.id, at)], None)).await.expect("workspace.mark_read");
    assert_eq!(merged.opened.len(), 1);
    assert_eq!((merged.opened[0].task_id.as_ref(), merged.opened[0].opened_ms), (task.id.as_bytes().as_slice(), at));

    let heard = events(&mut b, Duration::from_millis(600)).await;
    assert_eq!(heard.len(), 1, "one event for one change");
    assert_eq!(heard[0], merged, "B hears the board's whole state, with A's mark");

    let board = list(&mut connect(&h).await, Some(repo.workspace), repo.id).await;
    assert_eq!(board.reads.expect("a board's list carries its reads").opened, merged.opened);
}

/// Two devices write out of order and the later time wins; the write that
/// changed nothing says nothing.
#[tokio::test]
async fn out_of_order_writes_end_at_the_max_and_a_no_op_is_silent() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let task = h.service.store.create_task(repo.workspace, "Mac: a jump", Actor::User).unwrap();
    let (mut a, mut b, mut listener) = (connect(&h).await, connect(&h).await, connect(&h).await);
    let at = now_millis() - 1_000;

    reads_of(&mut a, mark(repo.workspace, &[(task.id, at)], None)).await.unwrap();
    let after_older = reads_of(&mut b, mark(repo.workspace, &[(task.id, at - 500)], None)).await.unwrap();
    assert_eq!(after_older.opened[0].opened_ms, at, "the older write lost");
    assert_eq!(events(&mut listener, Duration::from_millis(600)).await.len(), 1, "only the first changed anything");
}

/// A floor reads every older ticket as read, on every device.
#[tokio::test]
async fn a_floor_clears_the_older_tickets() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let (old, new) = {
        let s = &h.service.store;
        (s.create_task(repo.workspace, "old", Actor::User).unwrap(), s.create_task(repo.workspace, "new", Actor::User).unwrap())
    };
    let mut a = connect(&h).await;
    let base = now_millis() - 1_000;
    reads_of(&mut a, mark(repo.workspace, &[(old.id, base + 10), (new.id, base + 90)], None)).await.unwrap();
    let after = reads_of(&mut a, mark(repo.workspace, &[], Some(base + 50))).await.unwrap();
    assert_eq!(after.floor_ms, base + 50);
    assert_eq!(after.opened.len(), 1, "the ticket at or under the floor is gone");
    assert_eq!(after.opened[0].task_id.as_ref(), new.id.as_bytes());
}

/// Saying you have read a board is `control`: a read-scoped client is refused
/// with the scope code, and writes nothing.
#[tokio::test]
async fn a_write_needs_control_access() {
    let h = start(Scope::Read).await;
    let repo = a_repository(&h);
    let mut link = connect(&h).await;
    match link.call(mark(repo.workspace, &[], Some(now_millis()))).await {
        Err(ClientError::Daemon { code, .. }) => assert_eq!(code, ErrorCode::ScopeDenied as i32),
        other => panic!("expected a scope denial, got {other:?}"),
    }
    // Reading the board's state is still a read.
    assert!(list(&mut link, Some(repo.workspace), repo.id).await.reads.is_some());
}

/// `task.list` carries reads only when it names a board.
#[tokio::test]
async fn reads_ride_a_board_list_and_not_a_repository_list() {
    let h = start(Scope::Read).await;
    let repo = a_repository(&h);
    let mut link = connect(&h).await;
    assert!(list(&mut link, Some(repo.workspace), repo.id).await.reads.is_some());
    assert!(list(&mut link, None, repo.id).await.reads.is_none());
}

/// A ticket on another board is skipped, not refused, so a queued upload with
/// one moved ticket still lands the rest.
#[tokio::test]
async fn a_mark_for_another_boards_ticket_is_skipped() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let billing = h.service.store.create_workspace(repo.id, "Billing", "bil").unwrap();
    let task = h.service.store.create_task(repo.workspace, "Mac: a jump", Actor::User).unwrap();
    let merged = reads_of(&mut connect(&h).await, mark(billing.id, &[(task.id, now_millis() - 5)], None))
        .await
        .expect("skipped, not refused");
    assert!(merged.opened.is_empty());
}

/// A device's clock a day ahead of the runner's lands at the runner's now, so
/// the news the runner writes after it is still unread.
#[tokio::test]
async fn a_future_time_from_a_device_cannot_hide_later_news() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let task = h.service.store.create_task(repo.workspace, "Mac: a jump", Actor::User).unwrap();
    let day = 86_400_000;
    let before = now_millis();
    let merged =
        reads_of(&mut connect(&h).await, mark(repo.workspace, &[(task.id, before + day)], Some(before + day)))
            .await
            .unwrap();
    assert!(merged.floor_ms <= now_millis(), "the floor is the runner's now, not {}", merged.floor_ms);
    assert!(merged.floor_ms >= before);
}
