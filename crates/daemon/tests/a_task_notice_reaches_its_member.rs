//! The live `notice` event on a runner with members, over the real socket
//! (ov-705): a member's connection is told only of their own tasks, and the
//! owner's of every one, as the relay's pushes are routed (ov-633).

#[path = "support/in_process.rs"]
mod in_process;

use std::time::Duration;

/// How long to wait on something that happens at once, before calling it lost.
const WAIT: Duration = Duration::from_secs(10);

use farcooler_protocol::v1::{self as pb, Scope, event};
use farcooler_store::claude_logins::TerminalLogin;
use farcooler_store::models::{Actor, NoteKind};
use farcooler_store::people::Directory;
use farcooler_transport::Peer;
use in_process::*;

/// A task with a decision waiting, on a lane launched on `handle`'s login,
/// or with no lane at all for `None`. Its key.
fn a_decision(h: &Harness, repo: &Repo, title: &str, handle: Option<&str>) -> String {
    let store = &h.service.store;
    let task = store.create_task(repo.workspace, title, Actor::User).unwrap();
    if let Some(handle) = handle {
        let pane = a_pane(h, repo.worktree, Some(task.id));
        let login_id = store.ensure_member_login(handle).unwrap();
        let login = TerminalLogin { handle: handle.into(), login_id, config_dir: format!("/r/claude/{handle}"), shared: false };
        store.set_terminal_login(pane, &login).unwrap();
    }
    store.add_note(task.id, NoteKind::Question, Actor::Manager, "Which one?", serde_json::json!({})).unwrap();
    let moved = pb::TaskSetStatus {
        task_id: bytes::Bytes::copy_from_slice(task.id.as_bytes()),
        status: pb::TaskStatus::NeedsDecision as i32,
        actor: "manager".into(),
    };
    farcooler_daemon::task_ops::set_status(&h.service, &h.watcher, &moved).expect("task.set_status");
    task.key
}

/// Whose task each `notice` the connection `peer` is told of was, when
/// Alice's, Bob's and nobody's task each wait on a decision. Carol was a
/// member and has been removed, so her phone is nobody's now. Sorted. And
/// whether the connection heard the fleet event sent after them: a released
/// device hears no event at all (ov-758), so it is waited for only `quiet`.
async fn told(peer: Peer, quiet: Duration) -> (Vec<String>, bool) {
    let h = start_as(peer).await;
    let store = &h.service.store;
    let directory = |account| Directory { account, email: "" };
    store.set_member_in("alice", "Alice Chen", &["alice-phone".into()], directory("acct-alice")).unwrap();
    store.set_member_in("bob", "Bob Okafor", &["bob-phone".into()], directory("acct-bob")).unwrap();
    store.set_member("carol", "Carol Diaz", &["carol-phone".into()]).unwrap();
    store.remove_member("carol").unwrap();
    let repo = a_repository(&h);
    let mut listener = connect(&h).await;
    // An unfiltered subscriber, to know when every notice has gone out.
    let mut sent = h.watcher.subscribe();
    let mut whose = std::collections::HashMap::new();
    for (title, handle) in [("Alice's", Some("alice")), ("Bob's", Some("bob")), ("Nobody's", None)] {
        whose.insert(a_decision(&h, &repo, title, handle), title.to_string());
    }
    // A decision's notice goes at once. Wait until all three have, then flush
    // the listener with a fleet event, which every connection hears and which
    // comes after them: whatever it was told is in `seen` by then.
    let mut gone = 0;
    while gone < whose.len() {
        let e = tokio::time::timeout(WAIT, sent.recv()).await.expect("a notice went out").expect("the watcher");
        gone += usize::from(matches!(e.payload, Some(event::Payload::Notice(_))));
    }
    h.watcher.announce_fleet_changed();
    let mut seen = Vec::new();
    let flushed = loop {
        let Ok(e) = tokio::time::timeout(quiet, listener.next_event()).await else { break false };
        match e.expect("an event").payload {
            Some(event::Payload::Notice(n)) => seen.push(whose.get(&n.task_key).cloned().unwrap_or(n.task_key)),
            Some(event::Payload::FleetChanged(_)) => break true,
            _ => {}
        }
    };
    seen.sort();
    (seen, flushed)
}

#[tokio::test]
async fn a_member_is_told_of_their_own_task_alone() {
    let seen = told(Peer { client_id: Some("alice-phone".into()), scope: Scope::Control }, WAIT).await;
    assert_eq!(seen, (vec!["Alice's".to_string()], true));
}

#[tokio::test]
async fn the_owner_is_told_of_every_task() {
    let seen = told(Peer { client_id: None, scope: Scope::Control }, WAIT).await;
    assert_eq!(seen, (vec!["Alice's".to_string(), "Bob's".into(), "Nobody's".into()], true));
}

/// A device that was a removed member's is never the owner's, and hears
/// nothing: no notice, and since ov-758 no event of any kind, the fleet's
/// included.
#[tokio::test]
async fn a_removed_members_device_is_told_of_nothing() {
    let seen = told(Peer { client_id: Some("carol-phone".into()), scope: Scope::Control }, Duration::from_secs(2)).await;
    assert_eq!(seen, (Vec::<String>::new(), false));
}
