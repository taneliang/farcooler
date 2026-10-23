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

/// Answer an idle claude whose stand-in puts a permission menu up in the
/// very pass that takes the paste (`menu-on-paste`), so the menu is up before
/// the read-back can run. With `typing`, a key is marked as typed right after
/// the read-back first reads that menu (`after_dialog_read`): after the menu
/// was seen, before the pass settles, at no clock's mercy.
async fn answered_as_a_menu_comes_up(b: &Board, agent: &Terminal, si: &StandIn, typing: bool) {
    hooked(b);
    b.doing(agent.id, AgentActivity::Idle).await;
    si.show("menu-on-paste").await;
    if typing {
        let (root, id) = (b.svc.root_dir().to_path_buf(), agent.id);
        *b.watcher.after_dialog_read.lock().unwrap() = Some(Box::new(move || crate::runtime::mark_input(&root, id)));
    }
    b.answer("Drill in");
    b.pump().await;
    assert!(si.log().contains("PASTE "), "the paste never reached the stand-in: {}", si.log());
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

/// The runner's notice of a train that left on its own (`open_train_scan`).
const NOTICE: &str = "Train 222 is ready for its checks: mac-691. It left on its own. Gate it, or start your own train with `train start`.";

/// The owner's orchestrator as it ran when ov-711 was found: claude started
/// by hand in a shell pane adopted as the orchestrator, so none of Far
/// Cooler's hooks were ever heard from its session. The runner's notice is
/// queued while it works; the turn ends, and another begins as the paste
/// lands (a background task's result arriving), so the Enter waits, the row
/// marked pasted. Returns the pane and its stand-in, the notice in its box.
/// The stand-in has `env` (`STAND_IN_DRAFT=1` draws claude's cursor, which
/// a clear needs).
async fn notice_pasted_into_a_hookless_orchestrator(b: &Board, env: &str) -> (Terminal, StandIn) {
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in_with(&orchestrator, "claude", "claude", env).await;
    si.show("working").await;
    b.screen_with(orchestrator.id, "esc to interrupt").await;
    b.doing(orchestrator.id, AgentActivity::Working).await;
    let extra = serde_json::json!({ "message": { "to": "the orchestrator", "about": "train" } });
    b.svc.store.send_runner_notice(b.task.id, NOTICE, extra).unwrap();
    b.pump().await;
    nothing_typed(&si);
    assert!(b.progress().contains(&"Waiting to tell the orchestrator the message: it's busy.".to_string()), "{:?}", b.progress());

    si.show("working-on-paste").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.pump().await;
    si.pasted().await;
    assert!(si.submitted().is_empty() && !si.log().contains("QUEUED"), "{}", si.log());
    let pending = b.messages();
    assert!(pending.len() == 1 && pending[0].pasted_at.is_some(), "not marked pasted: {pending:?}");
    (orchestrator, si)
}

/// ov-711: once that turn ends, the notice sitting in the box is submitted,
/// exactly once, though no hook can vouch for the Enter: claude's own
/// registry says it's between turns. Before, only a hook-heard session's
/// Enter was ever pressed, so the notice sat typed and unsent until the
/// four-hour give-up, and then was left there.
#[tokio::test]
async fn a_notice_held_for_a_hookless_orchestrator_is_sent_once_it_goes_idle() {
    let b = board().await;
    let (orchestrator, si) = notice_pasted_into_a_hookless_orchestrator(&b, "").await;

    si.show("idle").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    for _ in 0..3 {
        b.pump().await;
        if !si.submitted().is_empty() {
            break;
        }
    }
    si.submits(1).await;
    assert_eq!(si.submitted(), [format!("[Far Cooler] {NOTICE}")], "{}", si.log());
    assert!(b.messages().is_empty(), "{:?}", b.messages());
    b.pump().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(si.submitted().len(), 1, "sent twice: {}", si.log());
    assert_eq!(si.log().matches("PASTE ").count(), 1, "pasted again: {}", si.log());
}

/// A notice that can't be sent before the give-up, the orchestrator working
/// all along, is taken back out of the box, never left there as a draft for
/// someone to send hours later (ov-711), and the record says so.
#[tokio::test]
async fn a_notice_given_up_on_is_taken_back_out_of_the_box() {
    let b = board().await;
    let (orchestrator, si) = notice_pasted_into_a_hookless_orchestrator(&b, "STAND_IN_DRAFT=1").await;
    b.pump().await;
    assert!(si.submitted().is_empty() && !si.log().contains("QUEUED"), "{}", si.log());

    farcooler_store::testing::backdate_message_wakes(&b.svc.store, super::super::messages::GIVE_UP_AFTER_MS + 1);
    b.pump().await;
    assert!(b.messages().is_empty(), "{:?}", b.messages());
    assert!(si.log().contains("CLEARROW") && !si.log().contains("PUTBACK"), "{}", si.log());
    let (screen, _, _) = b.svc.screen(orchestrator.id).await.unwrap();
    assert_eq!(farcooler_core::composer::read("claude", &screen), farcooler_core::composer::Composer::Empty, "{screen}");
    assert!(si.submitted().is_empty() && !si.log().contains("QUEUED"), "{}", si.log());
    assert_eq!(
        b.settled().last().map(String::as_str),
        Some("Not delivered: the agent stayed busy. Far Cooler took it back out of the agent's box, so nothing is left there to send by mistake.")
    );
}

/// Two passes with the watcher reading the orchestrator idle, and time for
/// any key they sent to reach the stand-in.
async fn passes_while_idle(b: &Board, orchestrator: &Terminal) {
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.pump().await;
    b.pump().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
}

/// No Enter reached the stand-in, and the notice still waits, marked pasted.
fn no_enter_yet(b: &Board, si: &StandIn) {
    assert!(!si.log().contains("ENTER") && si.submitted().is_empty(), "{}", si.log());
    let pending = b.messages();
    assert!(pending.len() == 1 && pending[0].pasted_at.is_some(), "{pending:?}");
}

/// A permission menu up over the box: no Enter, which would answer it.
#[tokio::test]
async fn a_hookless_orchestrator_with_a_menu_up_gets_no_enter() {
    let b = board().await;
    let (orchestrator, si) = notice_pasted_into_a_hookless_orchestrator(&b, "").await;
    si.show("menu").await;
    b.screen_with(orchestrator.id, "Do you want to create").await;
    passes_while_idle(&b, &orchestrator).await;
    no_enter_yet(&b, &si);
}

/// The screen working while the registry still says idle, or says a dialog
/// is up: no Enter. Then the stand-in with its turn hidden from the screen
/// (`working-hidden`: no spinner, the box holding text), its registry busy:
/// the registry is all that says a turn runs, and no Enter either.
#[tokio::test]
async fn a_hookless_orchestrator_whose_registry_lags_or_works_gets_no_enter() {
    let b = board().await;
    let (orchestrator, si) = notice_pasted_into_a_hookless_orchestrator(&b, "").await;
    for mode in ["working-lagging", "working-waiting", "working-hidden"] {
        si.show(mode).await;
        passes_while_idle(&b, &orchestrator).await;
        no_enter_yet(&b, &si);
    }
}

/// A key typed in the pane since the paste: no Enter, which would send
/// whatever the key made of the box; it settles as a paste left there.
#[tokio::test]
async fn a_hookless_orchestrator_typed_into_during_the_wait_gets_no_enter() {
    let b = board().await;
    let (orchestrator, si) = notice_pasted_into_a_hookless_orchestrator(&b, "").await;
    let wake = b.messages().remove(0);
    let now = now_millis();
    assert!(b.svc.store.mark_wake_pasted(&wake, now - 10_000).unwrap());
    typed_at(&b, orchestrator.id, now - QUIET_MS - 1_000);
    si.show("idle").await;
    passes_while_idle(&b, &orchestrator).await;
    assert!(!si.log().contains("ENTER"), "{}", si.log());
    assert_eq!(b.settled().last().map(String::as_str), Some("Paste left in the composer; not sent"));
}

/// A claude its hooks were heard from keeps waiting with no witness, as it
/// always has, even idle with its registry saying so: only the hooks vouch
/// for its Enter. Once they can (a turn boundary seen), the fenced Enter goes.
#[tokio::test]
async fn a_hooked_orchestrator_with_no_witness_still_waits() {
    let b = board().await;
    let (orchestrator, si) = notice_pasted_into_a_hookless_orchestrator(&b, "").await;
    b.svc.hooks().asks().heard(SESSION, false);
    si.show("idle").await;
    passes_while_idle(&b, &orchestrator).await;
    no_enter_yet(&b, &si);

    b.svc.hooks().asks().turn_bounded(SESSION);
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), [format!("[Far Cooler] {NOTICE}")], "{}", si.log());
}

/// At the give-up, a box whose text was edited since the paste, or that
/// holds only part of it, is left alone: no key, and it says so.
#[tokio::test]
async fn a_notice_given_up_on_leaves_an_edited_or_partial_box_alone() {
    // The notice's first words, edited after; and its first words alone.
    for shown in ["[Far Cooler] Train 222 is ready for its checks: mac-691, edited by hand", "[Far Cooler] Train 222 is ready for its"] {
        let b = board().await;
        let (orchestrator, si) = notice_pasted_into_a_hookless_orchestrator(&b, "STAND_IN_DRAFT=1").await;
        si.show(&format!("draft:{shown}")).await;
        b.screen_with(orchestrator.id, &shown[shown.len() - 8..]).await;
        farcooler_store::testing::backdate_message_wakes(&b.svc.store, super::super::messages::GIVE_UP_AFTER_MS + 1);
        b.pump().await;
        assert!(b.messages().is_empty(), "{:?}", b.messages());
        assert!(!si.log().contains("CLEARROW") && !si.log().contains("ENTER"), "{shown:?}: {}", si.log());
        assert_eq!(b.settled().last().map(String::as_str), Some("Paste left in the composer; not sent"), "{shown:?}");
    }
}
