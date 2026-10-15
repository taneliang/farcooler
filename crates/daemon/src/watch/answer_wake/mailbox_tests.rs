//! A message for a claude pane goes through its mailbox when the session's
//! waiter is alive (ov-563), and is typed when it isn't. The test plays the
//! waiter: it takes from the mailbox as `farcooler hook wait` does. The real
//! claude never runs. On the board and tmux server of `tests`.

use farcooler_agent_hooks::mailbox::{Mailbox, Slot, pid_alive};

use super::*;

const SESSION: &str = "7ecba53c-31c6-4c8e-9704-b2d582039d04";

/// A claude pane whose session is `SESSION`, with its mailbox.
async fn pane_and_box(b: &Board) -> (Terminal, Mailbox) {
    let pane = b.agent("Agent 2", "claude").await;
    b.svc.store.set_pane_mode(pane.id, pane.resource_version, PaneMode::Terminal, Some(SESSION.into()), false).unwrap();
    let pane = b.svc.store.get_terminal(pane.id).unwrap();
    let mailbox = Mailbox::of_session(&crate::claude_mailbox::root_of(b.svc.root_dir()), SESSION).unwrap();
    (pane, mailbox)
}

/// The one message in the mailbox, by name.
fn only_name(b: &Board) -> String {
    let wake = b.messages().remove(0);
    format!("{:013}-{}", wake.enqueued_at, wake.note.simple())
}

#[tokio::test]
async fn a_message_waits_in_the_mailbox_until_the_waiter_takes_it() {
    let b = board().await;
    let (pane, mailbox) = pane_and_box(&b).await;
    let si = b.stand_in(&pane, "claude", "claude").await;
    b.doing(pane.id, AgentActivity::Working).await;
    // This process is the live waiter.
    assert!(mailbox.claim_waiter(std::process::id(), &pid_alive));
    b.send("manager", &b.task.key, "Rebase on main first").await.unwrap();
    b.pump().await;
    let name = only_name(&b);
    assert_eq!(mailbox.slot(&name), Slot::Waiting, "dropped");
    assert_eq!(b.messages().len(), 1, "not settled until it's taken");
    assert!(!si.log().contains("PASTE"), "typed too: {}", si.log());

    // The waiter wakes the model with it.
    assert_eq!(mailbox.take_oldest().unwrap().as_deref(), Some("[from the orchestrator] Rebase on main first"));
    b.pump().await;
    assert!(b.messages().is_empty(), "settled: {:?}", b.messages());
    assert_eq!(mailbox.slot(&name), Slot::Absent, "and cleared");
    assert!(!si.log().contains("PASTE"), "{}", si.log());
}

#[tokio::test]
async fn no_waiter_means_typing() {
    let b = board().await;
    let (pane, mailbox) = pane_and_box(&b).await;
    let si = b.stand_in(&pane, "claude", "claude").await;
    b.doing(pane.id, AgentActivity::Idle).await;
    // A pid file left by a process that's gone is no waiter.
    assert!(mailbox.claim_waiter(u32::MAX - 1, &|_| true));
    b.send("manager", &b.task.key, "Ship it").await.unwrap();
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), ["[from the orchestrator] Ship it"], "{}", si.log());
    assert_eq!(mailbox.take_oldest().unwrap(), None, "nothing was dropped");
}

#[tokio::test]
async fn a_message_nobody_takes_is_taken_back_and_typed() {
    let b = board().await;
    let (pane, mailbox) = pane_and_box(&b).await;
    let si = b.stand_in(&pane, "claude", "claude").await;
    b.doing(pane.id, AgentActivity::Idle).await;
    assert!(mailbox.claim_waiter(std::process::id(), &pid_alive));
    // Dropped by hand, as a pump pass would, a minute ago: the waiter is
    // alive and hasn't taken it.
    b.svc.store.send_message(b.task.id, Actor::Manager, "Ship it", Some(pane.id), serde_json::json!({})).unwrap();
    let name = only_name(&b);
    mailbox.drop_message(&name, "Ship it").unwrap();
    let file = std::fs::File::options().write(true).open(mailbox.dir().join("new").join(&name)).unwrap();
    file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(60)).unwrap();
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), ["[from the orchestrator] Ship it"], "{}", si.log());
    assert_eq!(mailbox.slot(&name), Slot::Absent, "taken back, so it can't arrive twice");
}

/// A wake that ends without a delivery (here, the board's switch turned off)
/// takes its message back first, or the waiter would deliver it after.
#[tokio::test]
async fn a_message_is_taken_back_when_its_wake_ends_undelivered() {
    let b = board().await;
    let (_pane, mailbox) = pane_and_box(&b).await;
    assert!(mailbox.claim_waiter(std::process::id(), &pid_alive));
    b.send("manager", &b.task.key, "Rebase on main first").await.unwrap();
    b.pump().await;
    let name = only_name(&b);
    assert_eq!(mailbox.slot(&name), Slot::Waiting);
    let ws = b.svc.store.get_workspace(b.task.workspace_id).unwrap();
    b.svc.store.set_workspace_wake_on_answer(ws.id, ws.resource_version, false).unwrap();
    b.pump().await;
    assert!(b.messages().is_empty(), "the wake ended");
    assert_eq!(mailbox.slot(&name), Slot::Absent, "and the message with it");
    assert_eq!(mailbox.take_oldest().unwrap(), None, "so no waiter can deliver it now");
}

/// The pane's session moves on (`/clear`) while the message waits: it is
/// looked for where it was dropped, and never dropped a second time in the
/// new session's mailbox.
#[tokio::test]
async fn a_session_that_moves_on_gets_no_second_copy() {
    let b = board().await;
    let (pane, mailbox) = pane_and_box(&b).await;
    const NEXT: &str = "0b0b0b0b-31c6-4c8e-9704-b2d582039d04";
    let next = Mailbox::of_session(&crate::claude_mailbox::root_of(b.svc.root_dir()), NEXT).unwrap();
    assert!(mailbox.claim_waiter(std::process::id(), &pid_alive));
    b.send("manager", &b.task.key, "Ship it").await.unwrap();
    b.pump().await;
    let name = only_name(&b);
    assert_eq!(mailbox.slot(&name), Slot::Waiting);

    let pane = b.svc.store.get_terminal(pane.id).unwrap();
    b.svc.store.set_pane_mode(pane.id, pane.resource_version, PaneMode::Terminal, Some(NEXT.into()), false).unwrap();
    assert!(next.claim_waiter(std::process::id(), &pid_alive), "the new session has a waiter too");
    b.pump().await;
    b.pump().await;
    assert_eq!(next.slot(&name), Slot::Absent, "no second copy");
    assert_eq!(mailbox.slot(&name), Slot::Waiting, "still where it went");
    assert_eq!(b.messages().len(), 1);

    assert_eq!(mailbox.take_oldest().unwrap().as_deref(), Some("[from the orchestrator] Ship it"));
    b.pump().await;
    assert!(b.messages().is_empty(), "settled where it was taken");
    assert_eq!(next.take_oldest().unwrap(), None);
}

/// Where a message went is as private as the message: 0700 and 0600.
#[tokio::test]
async fn a_route_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let b = board().await;
    let (_pane, mailbox) = pane_and_box(&b).await;
    assert!(mailbox.claim_waiter(std::process::id(), &pid_alive));
    b.send("manager", &b.task.key, "Ship it").await.unwrap();
    b.pump().await;
    let routes = crate::claude_mailbox::routes_of(b.svc.root_dir());
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&routes), 0o700);
    let files: Vec<_> = std::fs::read_dir(&routes).unwrap().flatten().collect();
    assert_eq!(files.len(), 1, "one dropped message, one route");
    assert_eq!(mode(&files[0].path()), 0o600);
}
