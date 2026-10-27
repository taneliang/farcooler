//! ov-827, acceptance 2: a runner notice (a train ready, a wake) takes the
//! same path as a person's message left in the box by a turn begun after the
//! paste. It waits in the box, marked pasted (`finish`), through a long turn
//! and a dialog answered from the phone, and is sent once the turn ends:
//! after two reads between turns, never into the dialog. Before ov-827 the
//! dialog's answer, a key typed since the paste, settled it as a paste left
//! in the box, where it stayed (the owner's train notice, Oct 26).

use super::*;

/// A key typed in Far Cooler's terminal, input-marked as the phone's is.
async fn typed_marked(b: &Board, terminal: Uuid, hex: &str) {
    b.svc.runtime().send_bytes_hex(terminal, hex).await.unwrap();
}

/// Pump for `long`, as the watcher's ticks do, a pass every 200 ms.
async fn pumping(b: &Board, long: Duration) {
    let began = std::time::Instant::now();
    while began.elapsed() < long {
        b.pump().await;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// The owner's orchestrator: claude no hook is heard from (ov-711), between
/// turns, the runner's notice of a train that left about to reach it, and a
/// turn begun the moment the notice is pasted.
#[tokio::test]
async fn a_runner_notice_left_by_a_long_turn_is_sent_when_it_ends() {
    let b = board().await;
    b.watcher.with_timing(|t| t.told_spacing = crate::watch::Timing::SHORT);
    let orchestrator = b.orchestrator().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    si.show("working-on-paste").await;
    b.ready_lane("ios-overwrite");
    b.watcher.tell_departures();
    b.pump().await;
    assert_eq!(b.departure_notices().len(), 1);
    crate::watch::answer_wake::tests::compose_held_tests::logs(&si, "PASTE ").await;
    let waiting = b.svc.store.pending_message_wakes().unwrap();
    assert!(waiting.len() == 1 && waiting[0].pasted_at.is_some(), "it waits in the box: {waiting:?}\n{}", si.log());

    // The turn runs on, a permission dialog partway, answered from the
    // phone; the box keeps the notice.
    pumping(&b, Duration::from_secs(2)).await;
    si.show("menu").await;
    pumping(&b, Duration::from_secs(1)).await;
    typed_marked(&b, orchestrator.id, "31").await;
    crate::watch::answer_wake::tests::compose_held_tests::logs(&si, "ANSWERED 1").await;
    si.show("working").await;
    // Past the dialog's answer by more than a pane's quiet (`QUIET_MS`).
    pumping(&b, Duration::from_secs(6)).await;
    assert!(!si.log().contains("ENTER"), "no Enter during the turn: {}", si.log());
    assert_eq!(b.svc.store.pending_message_wakes().unwrap().len(), 1, "still waiting: {:?}", b.progress());

    si.show("idle").await;
    for _ in 0..40 {
        b.pump().await;
        if !si.submitted().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let log = si.log();
    let sent = si.submitted();
    assert!(sent.len() == 1 && sent[0].contains("Train 1 is ready for its checks: ios-overwrite."), "{log}\n{:?}", b.progress());
    assert_eq!(log.matches("ENTER").count(), 1, "{log}");
    assert!(log.find("ENTER").unwrap() > log.rfind("MODE idle").unwrap(), "{log}");
    assert!(b.svc.store.pending_message_wakes().unwrap().is_empty());
}
