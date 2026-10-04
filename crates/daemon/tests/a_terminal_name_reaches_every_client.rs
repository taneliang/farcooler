//! `terminal.rename` over the socket (ov-234): the reply carries the name, the
//! next list holds it, every other client is told the fleet moved, and a
//! client that may only look cannot name anything.

#[path = "support/in_process.rs"]
mod in_process;

use std::time::Duration;

use farcooler_protocol::v1::{self as pb, ErrorCode, Scope, event, request as payload, result};
use farcooler_transport::{ClientError, request};
use in_process::*;

fn rename(terminal: uuid::Uuid, name: &str) -> pb::Request {
    let mut req = request("terminal.rename");
    req.target_resource_id = Some(bytes::Bytes::copy_from_slice(terminal.as_bytes()));
    req.required_capabilities = vec![farcooler_protocol::capability::TERMINAL_NAMES.into()];
    req.payload = Some(payload::Payload::TerminalRename(pb::TerminalRename { name: name.into() }));
    req
}

/// How many `fleet_changed` arrive on `listener` within `window`.
async fn fleet_changes(listener: &mut Link, window: Duration) -> usize {
    let mut seen = 0;
    let deadline = tokio::time::Instant::now() + window;
    while let Ok(Ok(e)) = tokio::time::timeout_at(deadline, listener.next_event()).await {
        if matches!(e.payload, Some(event::Payload::FleetChanged(_))) {
            seen += 1;
        }
    }
    seen
}

/// Goes red when the handler's `announce_fleet_changed` is deleted: a rename
/// moves nothing the watcher observes, so no tick would say so.
#[tokio::test]
async fn a_rename_is_answered_listed_and_announced() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let pane = a_pane(&h, repo.worktree, None);
    let mut listener = connect(&h).await;
    // Anything the connection itself announced is over before the rename.
    let _ = fleet_changes(&mut listener, Duration::from_millis(500)).await;

    let mut client = connect(&h).await;
    let Some(result::Value::Terminal(t)) = client.call(rename(pane, "gcp proxy")).await.expect("rename").value
    else {
        panic!("wrong result")
    };
    assert_eq!(t.title, "gcp proxy", "the reply carries the name");
    assert_eq!(h.service.store.get_terminal(pane).unwrap().title, "gcp proxy", "kept in the store");
    assert!(fleet_changes(&mut listener, Duration::from_secs(1)).await >= 1, "every client is told");

    let Some(result::Value::TerminalList(list)) =
        client.call(request("terminal.list")).await.expect("terminal.list").value
    else {
        panic!("wrong result")
    };
    assert_eq!(list.items.iter().find(|x| x.id == t.id).map(|x| x.title.as_str()), Some("gcp proxy"));
}

/// A name is a write, so a client that may only read is refused and the old
/// name stands.
#[tokio::test]
async fn a_read_only_client_cannot_rename() {
    let h = start(Scope::Read).await;
    let repo = a_repository(&h);
    let pane = a_pane(&h, repo.worktree, None);
    match connect(&h).await.call(rename(pane, "x")).await {
        Err(ClientError::Daemon { code, .. }) => assert_eq!(code, ErrorCode::ScopeDenied as i32),
        other => panic!("expected a scope denial, got {other:?}"),
    }
    assert_eq!(h.service.store.get_terminal(pane).unwrap().title, "pane");
}

/// A name nobody can draw is refused as `name`, with the old one left alone.
#[tokio::test]
async fn a_name_with_a_newline_is_refused_over_the_wire() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let pane = a_pane(&h, repo.worktree, None);
    match connect(&h).await.call(rename(pane, "a\nb")).await {
        Err(ClientError::Daemon { code, .. }) => assert_eq!(code, ErrorCode::InvalidArgument as i32),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert_eq!(h.service.store.get_terminal(pane).unwrap().title, "pane");
}
