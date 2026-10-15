//! `farcooler hook wait` as claude runs it (ov-563): the process, with the
//! hook payload on stdin. A message in the session's mailbox comes out on
//! stderr with exit 2, which is what wakes claude (`asyncRewake`); a second
//! waiter for the same session leaves at once; and a payload with no session
//! does nothing. The real claude never runs.

use std::process::{Output, Stdio};
use std::time::Duration;

use farcooler_agent_hooks::mailbox::Mailbox;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const SESSION: &str = "7ecba53c-31c6-4c8e-9704-b2d582039d04";

/// A minute: it returns as soon as the process does.
const WEDGED: Duration = Duration::from_secs(60);

async fn wait(root: &std::path::Path, payload: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_farcooler"))
        .args(["hook", "wait", "--mailbox"])
        .arg(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("the binary starts");
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(payload.as_bytes()).await.unwrap();
    drop(stdin);
    tokio::time::timeout(WEDGED, child.wait_with_output()).await.expect("it ended").expect("it ran")
}

fn payload() -> String {
    format!(r#"{{"session_id":"{SESSION}","hook_event_name":"Stop"}}"#)
}

#[tokio::test]
async fn a_message_is_printed_on_stderr_and_exits_forty_two() {
    let root = tempfile::tempdir().unwrap();
    let mailbox = Mailbox::of_session(root.path(), SESSION).unwrap();
    mailbox.drop_message("0001-a", "[from the orchestrator] Rebase on main first").unwrap();
    mailbox.drop_message("0002-b", "second").unwrap();
    let out = wait(root.path(), &payload()).await;
    assert_eq!(out.status.code(), Some(42), "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "[from the orchestrator] Rebase on main first");
    assert!(out.stdout.is_empty());
    // One message per wake: the next waits for the next Stop.
    let out = wait(root.path(), &payload()).await;
    assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "second");
}

#[tokio::test]
async fn a_second_waiter_and_a_payload_without_a_session_leave_quietly() {
    let root = tempfile::tempdir().unwrap();
    let mailbox = Mailbox::of_session(root.path(), SESSION).unwrap();
    // This test process stands in for the live waiter.
    assert!(mailbox.claim_waiter(std::process::id(), &farcooler_agent_hooks::mailbox::pid_alive));
    mailbox.drop_message("0001-a", "stays").unwrap();
    let out = wait(root.path(), &payload()).await;
    assert_eq!((out.status.code(), out.stderr.is_empty()), (Some(0), true), "{out:?}");
    assert_eq!(mailbox.slot("0001-a"), farcooler_agent_hooks::mailbox::Slot::Waiting, "not taken");
    for bad in ["{}", "not json", r#"{"session_id":"../x"}"#, ""] {
        let out = wait(root.path(), bad).await;
        assert_eq!((out.status.code(), out.stderr.is_empty()), (Some(0), true), "{bad:?}: {out:?}");
    }
}

/// A waiter whose mailbox is removed leaves, so one for a session the pane
/// has moved on from doesn't outlive it.
#[tokio::test]
async fn a_waiter_leaves_when_its_mailbox_is_removed() {
    let root = tempfile::tempdir().unwrap();
    let mailbox = Mailbox::of_session(root.path(), SESSION).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_farcooler"))
        .args(["hook", "wait", "--mailbox"])
        .arg(root.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(payload().as_bytes()).await.unwrap();
    drop(stdin);
    // Waiting once it has made itself the mailbox's waiter.
    for _ in 0..3000 {
        if mailbox.waiter(&farcooler_agent_hooks::mailbox::pid_alive).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(mailbox.waiter(&farcooler_agent_hooks::mailbox::pid_alive).is_some(), "it never started waiting");
    std::fs::remove_dir_all(mailbox.dir()).unwrap();
    let status = tokio::time::timeout(WEDGED, child.wait()).await.expect("it left").unwrap();
    assert_eq!(status.code(), Some(0));
}
