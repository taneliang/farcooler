//! An answer a dialog stopped short of its Enter (ov-385): it waits marked
//! pasted, and a later pass presses only the Enter, once the dialog has gone
//! and the box holds exactly its text; otherwise it settles as a paste left
//! in the box, and nothing is typed again. On `mid_turn_tests`' stand-in.

use super::*;

/// Answer while a tool call starts the moment the paste reaches the
/// stand-in: the Enter, under the fence, finds the call in flight, as it would
/// a permission dialog on its way.
async fn answered_into_a_dialog(b: &Board, agent: &Terminal, si: &StandIn) {
    working(b, agent, si, "working").await;
    let asks = b.svc.hooks().asks().clone();
    let log = si.log.clone();
    let gate = tokio::spawn(async move {
        for _ in 0..1_000 {
            if std::fs::read_to_string(&log).unwrap_or_default().contains("PASTE ") {
                asks.mark_tool_starting(SESSION, Some("toolu_dialog"));
                return true;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        false
    });
    b.answer("Drill in");
    b.pump().await;
    assert!(gate.await.unwrap(), "the paste never reached the stand-in");
    assert!(b.settled().is_empty(), "{:?}", b.settled());
    assert!(b.pending()[0].pasted_at.is_some(), "not marked pasted: {:?}", b.pending());
    assert!(!si.log().contains("ENTER"), "{}", si.log());
}

/// Once the call ends, the box still holding exactly the answer: the next
/// pass presses the Enter, and only it. While the call runs, nothing.
#[tokio::test]
async fn a_pasted_answer_is_finished_once_the_dialog_goes() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    answered_into_a_dialog(&b, &agent, &si).await;

    b.pump().await;
    assert!(!si.log().contains("ENTER"), "an Enter with the call in flight: {}", si.log());
    assert!(b.pending()[0].pasted_at.is_some());

    b.svc.hooks().asks().tool_ended(SESSION, Some("toolu_dialog"));
    b.pump().await;
    let told = b.told("Drill in");
    assert!(si.log().contains(&format!("QUEUED {told}")), "{}", si.log());
    assert_eq!(si.log().matches("PASTE ").count(), 1, "pasted again: {}", si.log());
    assert_eq!(b.settled(), ["Told Agent 2 about the decision. It was working, so it's queued for when it's ready"]);
    assert!(b.pending().is_empty());
}

/// The box holding anything but exactly the answer by then: no Enter, never
/// a second paste, and it settles as a paste left in the box.
#[tokio::test]
async fn a_pasted_answer_the_box_no_longer_holds_is_never_retyped() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    answered_into_a_dialog(&b, &agent, &si).await;

    b.svc.hooks().asks().tool_ended(SESSION, Some("toolu_dialog"));
    si.show("draft:Drill in, but").await;
    b.screen_with(agent.id, "Drill in, but").await;
    b.pump().await;
    assert!(!si.log().contains("ENTER"), "{}", si.log());
    assert_eq!(si.log().matches("PASTE ").count(), 1, "pasted again: {}", si.log());
    assert_eq!(b.settled(), ["Paste left in the composer; not sent"]);
    assert!(b.pending().is_empty());
}

/// A daemon that stops with the mark on reads it on its next start, and
/// finishes the same way: the mark is in the row, not in memory.
#[tokio::test]
async fn a_pasted_answer_is_finished_after_a_restart() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    answered_into_a_dialog(&b, &agent, &si).await;
    b.svc.hooks().asks().tool_ended(SESSION, Some("toolu_dialog"));

    let after = Watcher::new(b.svc.clone());
    observe(&after, agent.id, AgentActivity::Working).await;
    after.wakes_hint.store(true, std::sync::atomic::Ordering::SeqCst);
    after.pump_wakes().await;
    assert!(si.log().contains(&format!("QUEUED {}", b.told("Drill in"))), "{}", si.log());
    assert_eq!(si.log().matches("PASTE ").count(), 1, "pasted again: {}", si.log());
}

/// The same between turns in codex, where no fence reads the box again
/// before the Enter: this pass's own read is the guard. The row is marked as
/// a dialog in the read-back leaves it; the box holds another text.
#[tokio::test]
async fn a_pasted_answer_codex_no_longer_holds_is_never_entered() {
    let b = board().await;
    let agent = b.agent("Agent 2", "codex").await;
    let si = b.stand_in(&agent, "codex", "codex").await;
    si.show("draft:Drill in, but").await;
    b.screen_with(agent.id, "Drill in, but").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    let wake = b.pending().remove(0);
    assert!(b.svc.store.claim_wake(&wake).unwrap());
    assert!(b.svc.store.mark_wake_pasted(&wake, now_millis()).unwrap());
    b.pump().await;
    assert!(!si.log().contains("ENTER"), "{}", si.log());
    assert_eq!(b.settled(), ["Paste left in the composer; not sent"]);

    // And one that does hold it exactly: the Enter, and only it.
    b.answer("Drill in");
    let wake = b.pending().remove(0);
    si.show(&format!("draft:{}", b.told("Drill in"))).await;
    b.screen_with(agent.id, "Continue.").await;
    assert!(b.svc.store.claim_wake(&wake).unwrap());
    assert!(b.svc.store.mark_wake_pasted(&wake, now_millis() - 1).unwrap());
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), [b.told("Drill in")], "{}", si.log());
    assert!(!si.log().contains("PASTE "), "pasted: {}", si.log());
}

/// Say someone typed into `terminal` at `at` (Unix ms), as `mark_input`
/// writes it: long enough ago that the pane no longer reads as being typed in.
fn typed_at(b: &Board, terminal: Uuid, at: i64) {
    let mark = crate::runtime::input_mark(b.svc.root_dir(), terminal);
    std::fs::create_dir_all(mark.parent().unwrap()).unwrap();
    std::fs::write(mark, at.to_string()).unwrap();
}

/// A key typed into codex after the paste, the box holding exactly the
/// answer again by now: no Enter, since codex has no fence to read the
/// keyboard again, and it settles as a paste left in the box.
#[tokio::test]
async fn a_pasted_answer_someone_typed_beside_is_never_entered_in_codex() {
    let b = board().await;
    let agent = b.agent("Agent 2", "codex").await;
    let si = b.stand_in(&agent, "codex", "codex").await;
    // The box holds the text before the answer is queued, so the pass the
    // answer starts finds a draft there and claims nothing.
    si.show(&format!("draft:{}", b.told("Drill in"))).await;
    b.screen_with(agent.id, "Continue.").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    let wake = b.pending().remove(0);
    assert!(b.svc.store.claim_wake(&wake).unwrap());
    let now = now_millis();
    assert!(b.svc.store.mark_wake_pasted(&wake, now - 10_000).unwrap());
    typed_at(&b, agent.id, now - QUIET_MS - 1_000);
    b.pump().await;
    assert!(!si.log().contains("ENTER"), "{}", si.log());
    assert_eq!(b.settled(), ["Paste left in the composer; not sent"]);
}

/// The same in claude: the fence reads the keyboard from the original
/// paste, not from when the finishing pass began.
#[tokio::test]
async fn a_pasted_answer_someone_typed_beside_is_never_entered_in_claude() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    answered_into_a_dialog(&b, &agent, &si).await;
    let wake = b.pending().remove(0);
    let now = now_millis();
    assert!(b.svc.store.mark_wake_pasted(&wake, now - 10_000).unwrap());
    typed_at(&b, agent.id, now - QUIET_MS - 1_000);
    b.svc.hooks().asks().tool_ended(SESSION, Some("toolu_dialog"));
    b.pump().await;
    assert!(!si.log().contains("ENTER"), "{}", si.log());
    assert_eq!(b.settled(), ["Paste left in the composer; not sent"]);
}

/// Answer an idle claude, and put a permission menu up the moment the paste
/// reaches it; with `typing`, then type a key once the menu has been read.
async fn answered_as_a_menu_comes_up(b: &Board, agent: &Terminal, si: &StandIn, typing: bool) {
    hooked(b);
    b.doing(agent.id, AgentActivity::Idle).await;
    let (log, control, root, id) = (si.log.clone(), si.control.clone(), b.svc.root_dir().to_path_buf(), agent.id);
    let menu = tokio::spawn(async move {
        for _ in 0..1_000 {
            if std::fs::read_to_string(&log).unwrap_or_default().contains("PASTE ") {
                std::fs::write(&control, "menu").unwrap();
                if typing {
                    tokio::time::sleep(Duration::from_millis(700)).await;
                    crate::runtime::mark_input(&root, id);
                }
                return true;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        false
    });
    b.answer("Drill in");
    b.pump().await;
    assert!(menu.await.unwrap(), "the paste never reached the stand-in");
    assert!(!si.log().contains("ENTER"), "{}", si.log());
}

/// A menu drawn over the box during the read-back, nobody typing: the row
/// is marked pasted and the answer waits for the menu to go.
#[tokio::test]
async fn a_menu_during_the_read_back_marks_the_answer_pasted() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    answered_as_a_menu_comes_up(&b, &agent, &si, false).await;
    assert!(b.settled().is_empty(), "{:?}", b.settled());
    assert!(b.pending()[0].pasted_at.is_some(), "not marked pasted: {:?}", b.pending());
}

/// The same with a key typed: the key may have gone into the box, so it
/// settles as a paste left there, with no mark to finish from.
#[tokio::test]
async fn a_menu_and_a_key_during_the_read_back_leave_the_paste() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    answered_as_a_menu_comes_up(&b, &agent, &si, true).await;
    assert_eq!(b.settled(), ["Paste left in the composer; not sent"]);
    assert!(b.pending().is_empty(), "{:?}", b.pending());
}
