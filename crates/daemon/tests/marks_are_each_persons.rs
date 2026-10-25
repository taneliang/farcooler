//! A member's connection, over the real socket (ov-740): it hears its own
//! read state and not the owner's, and the owner's look at a pane doesn't end
//! `Done` in what it's told.

#[path = "support/in_process.rs"]
mod in_process;

use std::time::Duration;

use farcooler_protocol::v1::{self as pb, AgentActivity, Scope, event};
use farcooler_transport::Peer;
use in_process::*;

/// How long to wait on something that happens at once, before calling it lost.
const WAIT: Duration = Duration::from_secs(10);

/// Carol's phone, a member's.
async fn carols_runner() -> Harness {
    let h = start_as(Peer { client_id: Some("phone-c".into()), scope: Scope::Control }).await;
    h.service.store.set_member("carol", "Carol", &["phone-c".to_string()]).unwrap();
    h
}

/// The next event on `link` that `wanted` picks out, within `WAIT`.
async fn next<T>(link: &mut Link, wanted: impl Fn(event::Payload) -> Option<T>) -> Option<T> {
    let deadline = tokio::time::Instant::now() + WAIT;
    while let Ok(Ok(e)) = tokio::time::timeout_at(deadline, link.next_event()).await {
        if let Some(found) = e.payload.and_then(&wanted) {
            return Some(found);
        }
    }
    None
}

/// The owner's read state goes to the owner's devices, and Carol's to hers:
/// her phone hears hers alone.
#[tokio::test]
async fn a_member_hears_their_own_read_state_alone() {
    let h = carols_runner().await;
    let repo = a_repository(&h);
    let mut phone = connect(&h).await;
    let reads = |member: &str, floor_ms| pb::BoardReads {
        workspace_id: bytes::Bytes::copy_from_slice(repo.workspace.as_bytes()),
        floor_ms,
        opened: vec![],
        member: member.into(),
    };
    h.watcher.announce_board_reads(reads("", 1));
    h.watcher.announce_board_reads(reads("bob", 2));
    h.watcher.announce_board_reads(reads("carol", 3));
    let heard = next(&mut phone, |p| match p {
        event::Payload::BoardReadsChanged(r) => Some(r),
        _ => None,
    })
    .await
    .expect("her own read state");
    assert_eq!((heard.member.as_str(), heard.floor_ms), ("carol", 3), "the owner's and Bob's never reach her");
}

/// The owner opening a finished pane announces it `Idle`; Carol, who
/// hasn't opened it, is told it's still `Done`.
#[tokio::test]
async fn the_owners_look_ends_no_members_done() {
    let h = carols_runner().await;
    let repo = a_repository(&h);
    let pane = a_pane(&h, repo.worktree, None);
    let finished = now_millis() - 5_000;
    let done = farcooler_daemon::needs_you::Observation {
        activity: AgentActivity::Done,
        state_since: finished,
        blocked_question: None,
        turn_failed: false,
        command: "claude".into(),
        trust_prompt: false,
        trust_folder: None,
        not_started: None,
    };
    h.watcher.observe_for_tests(pane, done).await;
    let mut phone = connect(&h).await;

    h.watcher.mark_seen(pane).await;
    assert_eq!(h.watcher.activity(pane).await.0, AgentActivity::Idle, "the owner has seen it");
    let id = bytes::Bytes::copy_from_slice(pane.as_bytes());
    let told = next(&mut phone, |p| match p {
        event::Payload::TerminalChanged(row) if row.id == id => Some(row),
        _ => None,
    })
    .await
    .expect("the pane's row");
    assert_eq!(told.activity(), AgentActivity::Done, "Carol hasn't");
}
