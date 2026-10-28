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

/// A runner notice waiting in the box of a claude no hook is heard from:
/// one frame read between turns mid-turn (0.7 s of it, longer than a pump
/// cycle so a pass surely reads it, the registry idle with it) is no turn's
/// end. Its Enter needs two reads `idle_twice` apart, 2 s here, as a waiting
/// compose's does. No Enter.
#[tokio::test]
async fn one_stray_idle_frame_gets_a_waiting_notice_no_enter() {
    let b = board().await;
    b.watcher.with_timing(|t| (t.told_spacing, t.idle_twice) = (crate::watch::Timing::SHORT, Duration::from_secs(2)));
    let orchestrator = b.orchestrator().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    si.show("working-on-paste").await;
    b.ready_lane("ios-overwrite");
    b.watcher.tell_departures();
    b.pump().await;
    crate::watch::answer_wake::tests::compose_held_tests::logs(&si, "PASTE ").await;
    pumping(&b, Duration::from_millis(600)).await;
    // The frame, ended on its own clock: a pass reading it holds the pump
    // through its second read.
    si.show("idle").await;
    let ender = si.clone();
    let frame = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(700)).await;
        ender.show("working").await;
    });
    pumping(&b, Duration::from_secs(3)).await;
    frame.await.unwrap();
    assert!(!si.log().contains("ENTER"), "an Enter on one idle frame: {}", si.log());
    assert_eq!(b.svc.store.pending_message_wakes().unwrap().len(), 1, "still waiting: {:?}", b.progress());
}

/// Review S1: the notice's turn stopped by the person, a dialog answered
/// `key` (No, `2`, or Esc) from the phone: never entered after it, but taken
/// back out of the box once nobody is typing, and the task says so.
async fn a_notice_whose_turn_was_stopped_is_taken_back(key: &str) {
    let b = board().await;
    b.watcher.with_timing(|t| t.told_spacing = crate::watch::Timing::SHORT);
    let orchestrator = b.orchestrator().await;
    let si = b.stand_in_with(&orchestrator, "claude", "claude", "STAND_IN_DRAFT=1").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    si.show("working-on-paste").await;
    b.ready_lane("ios-overwrite");
    b.watcher.tell_departures();
    b.pump().await;
    crate::watch::answer_wake::tests::compose_held_tests::logs(&si, "PASTE ").await;
    pumping(&b, Duration::from_millis(600)).await;
    si.show("menu").await;
    pumping(&b, Duration::from_millis(600)).await;
    typed_marked(&b, orchestrator.id, key).await;
    crate::watch::answer_wake::tests::compose_held_tests::logs(&si, "MODE idle\n").await;
    for _ in 0..60 {
        b.pump().await;
        if b.svc.store.pending_message_wakes().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    pumping(&b, Duration::from_secs(1)).await;
    let log = si.log();
    assert!(!log.contains("ENTER"), "entered after the stop: {log}");
    assert!(log.contains("CLEARROW"), "never taken back: {log}");
    let said = b.progress();
    assert!(said.iter().any(|n| n.contains("turn was stopped") && n.contains("took it back out")), "{said:?}");
}

#[tokio::test]
async fn a_notice_whose_dialog_was_answered_no_is_taken_back() {
    a_notice_whose_turn_was_stopped_is_taken_back("32").await;
}

#[tokio::test]
async fn a_notice_whose_dialog_was_answered_esc_is_taken_back() {
    a_notice_whose_turn_was_stopped_is_taken_back("1b").await;
}

/// Re-review R1: the turn stopped with no input mark (Far Cooler's Stop, or
/// Esc straight in tmux) after the pass's first look at the transcript, and
/// before its Enter: the stop is read again right before the Enter, so none
/// goes, and the next pass takes the notice back out.
#[tokio::test]
async fn a_notice_whose_turn_is_stopped_mid_pass_gets_no_enter() {
    let b = board().await;
    b.watcher.with_timing(|t| t.told_spacing = crate::watch::Timing::SHORT);
    let orchestrator = b.orchestrator().await;
    let si = b.stand_in_with(&orchestrator, "claude", "claude", "STAND_IN_DRAFT=1").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    si.show("working-on-paste").await;
    b.ready_lane("ios-overwrite");
    b.watcher.tell_departures();
    b.pump().await;
    crate::watch::answer_wake::tests::compose_held_tests::logs(&si, "PASTE ").await;
    pumping(&b, Duration::from_millis(600)).await;
    // The stop's record, as claude writes it on Esc, landing mid-pass.
    let projects = si.control.parent().unwrap().join("config").join("projects");
    let project = std::fs::read_dir(&projects).unwrap().next().unwrap().unwrap().path();
    let transcript = project.join("stand-in.jsonl");
    *b.watcher.before_notice_enter.lock().unwrap() = Some(Box::new(move || {
        let stamp = farcooler_store::testing::now_millis();
        let at = chrono_free_iso(stamp);
        let line = format!("{{\"type\":\"user\",\"timestamp\":\"{at}\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"[Request interrupted by user]\"}}]}}}}\n");
        use std::io::Write;
        std::fs::OpenOptions::new().create(true).append(true).open(&transcript).unwrap().write_all(line.as_bytes()).unwrap();
    }));
    si.show("idle").await;
    for _ in 0..60 {
        b.pump().await;
        if b.svc.store.pending_message_wakes().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(b.watcher.before_notice_enter.lock().unwrap().is_none(), "the pass never reached its Enter");
    let log = si.log();
    assert!(!log.contains("ENTER"), "entered after the stop: {log}");
    assert!(log.contains("CLEARROW"), "never taken back: {log}");
    assert!(b.progress().iter().any(|n| n.contains("turn was stopped")), "{:?}", b.progress());
}

/// `at` (Unix ms) as claude stamps a record: `2026-10-27T05:03:04.123Z`.
fn chrono_free_iso(at: i64) -> String {
    let (secs, ms) = (at.div_euclid(1000), at.rem_euclid(1000));
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil from days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{ms:03}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}
