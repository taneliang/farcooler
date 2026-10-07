//! A draft held behind a dialog (ov-385): held when the client asks and a
//! dialog is up, pasted with no Enter once it closes, and never pasted once
//! it expires or is withdrawn. On the board and stand-in of `tests`.

use farcooler_protocol::v1::DraftHoldState;
use farcooler_protocol::v1::event::Payload;

use super::draft_hold::Drafted;
use super::*;

/// An orchestrator's claude with a permission dialog up, and a draft held
/// behind it: nothing typed yet.
async fn held(b: &Board) -> (Terminal, StandIn, farcooler_protocol::v1::DraftHold) {
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    si.show("menu").await;
    b.screen_with(orchestrator.id, "Tab to amend").await;
    let Drafted::Held(hold) = b.watcher.draft_into(orchestrator.id, "About ov-1 (“Fix”): ", true).await.expect("held")
    else {
        panic!("pasted over a dialog: {}", si.log())
    };
    assert_eq!(hold.state, DraftHoldState::Waiting as i32);
    assert_eq!(hold.expires_ms - hold.held_ms, GIVE_UP_AFTER_MS);
    assert_eq!(b.watcher.draft_hold(orchestrator.id), Some(hold.clone()));
    nothing_typed(&si);
    (orchestrator, si, hold)
}

/// The dialog closes: the next tick pastes the draft, presses no Enter, and
/// says so on the terminal's events.
#[tokio::test]
async fn a_draft_held_behind_a_dialog_is_pasted_once_it_closes() {
    let b = board().await;
    let (orchestrator, si, hold) = held(&b).await;
    let mut events = b.watcher.subscribe();

    b.watcher.pump_draft_holds().await;
    nothing_typed(&si);
    assert_eq!(b.watcher.draft_hold(orchestrator.id).unwrap().state, DraftHoldState::Waiting as i32);

    si.show("idle").await;
    b.screen_with(orchestrator.id, "? for shortcuts").await;
    b.watcher.pump_draft_holds().await;
    si.pasted().await;
    assert!(si.log().contains("PASTE About ov-1 (“Fix”): "), "{}", si.log());
    assert!(!si.log().contains("ENTER"), "Enter was pressed: {}", si.log());
    let sent = b.watcher.draft_hold(orchestrator.id).unwrap();
    assert_eq!((sent.id.clone(), sent.state), (hold.id.clone(), DraftHoldState::Sent as i32));
    assert!(sent.ended_ms > 0);

    // Told: a terminal event carries the hold as sent.
    let mut said = None;
    for _ in 0..100 {
        while let Ok(event) = events.try_recv() {
            if let Some(Payload::TerminalChanged(t)) = event.payload
                && t.id.as_ref() == orchestrator.id.as_bytes()
            {
                said = t.draft_hold.map(|h| h.state);
            }
        }
        // The waiting hold's own event may come first: it's announced
        // off the caller's path too.
        if said == Some(DraftHoldState::Sent as i32) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(said, Some(DraftHoldState::Sent as i32));

    // Once only.
    b.watcher.pump_draft_holds().await;
    assert_eq!(si.log().matches("PASTE ").count(), 1, "{}", si.log());
}

/// Half an hour behind the dialog: it expires, and isn't pasted when the
/// dialog closes after.
#[tokio::test]
async fn a_draft_held_too_long_expires() {
    let b = board().await;
    let (orchestrator, si, _) = held(&b).await;
    b.watcher.age_draft_holds_for_tests(GIVE_UP_AFTER_MS + 1);
    b.watcher.pump_draft_holds().await;
    assert_eq!(b.watcher.draft_hold(orchestrator.id).unwrap().state, DraftHoldState::Expired as i32);

    si.show("idle").await;
    b.screen_with(orchestrator.id, "? for shortcuts").await;
    b.watcher.pump_draft_holds().await;
    nothing_typed(&si);
}

/// Withdrawn: never pasted. Withdrawing again says how it ended; another
/// hold's id is one this runner doesn't have.
#[tokio::test]
async fn a_withdrawn_draft_is_never_pasted() {
    let b = board().await;
    let (orchestrator, si, hold) = held(&b).await;
    let id = Uuid::from_slice(&hold.id).unwrap();
    let withdrawn = b.watcher.withdraw_draft(orchestrator.id, id).await.expect("withdrawn");
    assert_eq!(withdrawn.state, DraftHoldState::Withdrawn as i32);

    si.show("idle").await;
    b.screen_with(orchestrator.id, "? for shortcuts").await;
    b.watcher.pump_draft_holds().await;
    nothing_typed(&si);
    assert_eq!(b.watcher.withdraw_draft(orchestrator.id, id).await.unwrap().state, DraftHoldState::Withdrawn as i32);
    assert!(matches!(b.watcher.withdraw_draft(orchestrator.id, Uuid::now_v7()).await, Err(DomainError::NotFound)));
}

/// A client that didn't ask is refused, as before; one that asked is held
/// only for a dialog, and refused for anything else in the way.
#[tokio::test]
async fn only_a_dialog_holds_a_draft_and_only_when_asked() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    si.show("menu").await;
    b.screen_with(orchestrator.id, "Tab to amend").await;
    assert!(b.watcher.draft_into(orchestrator.id, "About ov-1: ", false).await.is_err());
    si.show("draft:fix the flaky").await;
    b.screen_with(orchestrator.id, "fix the flaky").await;
    assert!(b.watcher.draft_into(orchestrator.id, "About ov-1: ", true).await.is_err());
    assert_eq!(b.watcher.draft_hold(orchestrator.id), None);
    nothing_typed(&si);
}

/// A newer draft takes a waiting one's place: held, under a new id; pasted
/// at once, the older reads as withdrawn. Only the newer is ever pasted.
#[tokio::test]
async fn a_newer_draft_replaces_a_waiting_one() {
    let b = board().await;
    let (orchestrator, si, first) = held(&b).await;
    let Drafted::Held(second) = b.watcher.draft_into(orchestrator.id, "About ov-2: ", true).await.unwrap() else {
        panic!("pasted over a dialog")
    };
    assert_ne!(first.id, second.id);
    assert_eq!(b.watcher.draft_hold(orchestrator.id).unwrap().id, second.id);

    si.show("idle").await;
    b.screen_with(orchestrator.id, "? for shortcuts").await;
    assert!(matches!(b.watcher.draft_into(orchestrator.id, "About ov-3: ", true).await, Ok(Drafted::Pasted)));
    assert_eq!(b.watcher.draft_hold(orchestrator.id).unwrap().state, DraftHoldState::Withdrawn as i32);
    b.watcher.pump_draft_holds().await;
    si.pasted().await;
    assert_eq!(si.log().matches("PASTE ").count(), 1, "{}", si.log());
    assert!(si.log().contains("PASTE About ov-3: "), "{}", si.log());
}

/// The dialog closed and someone else's words are in the box: the draft
/// waits. Once they type there, they've moved on, and it expires rather than
/// land ahead of what they type next.
#[tokio::test]
async fn a_draft_expires_once_someone_types_after_the_dialog_closes() {
    let b = board().await;
    let (orchestrator, si, _) = held(&b).await;
    si.show("draft:fix the flaky").await;
    b.screen_with(orchestrator.id, "fix the flaky").await;
    b.watcher.pump_draft_holds().await;
    assert_eq!(b.watcher.draft_hold(orchestrator.id).unwrap().state, DraftHoldState::Waiting as i32);
    tokio::time::sleep(Duration::from_millis(5)).await;
    crate::runtime::mark_input(b.svc.root_dir(), orchestrator.id);
    b.watcher.pump_draft_holds().await;
    assert_eq!(b.watcher.draft_hold(orchestrator.id).unwrap().state, DraftHoldState::Expired as i32);
    si.show("idle").await;
    b.screen_with(orchestrator.id, "? for shortcuts").await;
    b.watcher.pump_draft_holds().await;
    nothing_typed(&si);
}


/// A second dialog answered with a key (re-review F1): the key went to the
/// dialog, not the box, so the draft isn't expired for it. Held behind menu
/// 1, answered with a key; menu 2 up for a pass, answered with a key; then
/// the box is free again and the draft still waits to go in.
#[tokio::test]
async fn answering_a_second_dialog_keeps_the_draft_waiting() {
    let b = board().await;
    let (orchestrator, si, _) = held(&b).await;
    // Menu 1 answered with a key: the next pass sees no dialog, and a key.
    crate::runtime::mark_input(b.svc.root_dir(), orchestrator.id);
    si.show("idle").await;
    b.screen_with(orchestrator.id, "? for shortcuts").await;
    b.watcher.pump_draft_holds().await;
    // Menu 2, read for one pass, then answered with a key.
    tokio::time::sleep(Duration::from_millis(5)).await;
    si.show("menu").await;
    b.screen_with(orchestrator.id, "Tab to amend").await;
    b.doing(orchestrator.id, AgentActivity::Blocked).await;
    b.watcher.pump_draft_holds().await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    crate::runtime::mark_input(b.svc.root_dir(), orchestrator.id);
    si.show("idle").await;
    b.screen_with(orchestrator.id, "? for shortcuts").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.watcher.pump_draft_holds().await;
    assert_eq!(b.watcher.draft_hold(orchestrator.id).unwrap().state, DraftHoldState::Waiting as i32);
    nothing_typed(&si);
}

/// An answer pass in progress (`wake_pump` held) delays the drafts' pass
/// rather than skipping it (re-review F2): the draft lands once the answers
/// are done.
#[tokio::test]
async fn a_held_draft_lands_after_an_answer_pass_in_progress() {
    let b = board().await;
    let (orchestrator, si, _) = held(&b).await;
    si.show("idle").await;
    b.screen_with(orchestrator.id, "? for shortcuts").await;
    let answering = b.watcher.wake_pump.lock().await;
    let drafts = tokio::spawn({
        let watcher = b.watcher.clone();
        async move { watcher.pump_draft_holds().await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    nothing_typed(&si);
    drop(answering);
    drafts.await.unwrap();
    si.pasted().await;
    assert_eq!(b.watcher.draft_hold(orchestrator.id).unwrap().state, DraftHoldState::Sent as i32);
}

/// An answer waiting on another pane for several ticks never keeps a held
/// draft out: the tick runs both pumps in one task (`spawn_pumps`).
#[tokio::test]
async fn a_held_draft_lands_while_an_answer_waits_on_another_pane() {
    let b = board().await;
    let (orchestrator, si, _) = held(&b).await;
    let agent = b.agent("Agent 2", "claude").await;
    let other = b.stand_in(&agent, "claude", "claude").await;
    other.show("menu").await;
    b.screen_with(agent.id, "Tab to amend").await;
    b.doing(agent.id, AgentActivity::Blocked).await;
    b.answer("Drill in");
    si.show("idle").await;
    b.screen_with(orchestrator.id, "? for shortcuts").await;
    // Starting the second stand-in sampled every pane, the menu included.
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    for _ in 0..3 {
        b.watcher.wakes_hint.store(true, Ordering::SeqCst);
        b.watcher.spawn_pumps();
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    si.pasted().await;
    assert_eq!(b.watcher.draft_hold(orchestrator.id).unwrap().state, DraftHoldState::Sent as i32, "{}", si.log());
    assert!(!b.pending().is_empty() && !other.log().contains("PASTE"), "the answer still waits on its own pane: {:?}", b.pending());
}

/// A drafts' pass waiting behind an answer pass holds nothing a person's own
/// draft waits on: `draft_into` answers at once (third review).
#[tokio::test]
async fn a_draft_prompt_answers_promptly_while_an_answer_pass_runs() {
    let b = board().await;
    let (orchestrator, _si, _) = held(&b).await;
    let answering = b.watcher.wake_pump.lock().await;
    let waiting_pass = tokio::spawn({
        let watcher = b.watcher.clone();
        async move { watcher.pump_draft_holds().await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let asked = tokio::time::timeout(
        Duration::from_secs(5),
        b.watcher.draft_into(orchestrator.id, "About ov-2: ", true),
    )
    .await;
    assert!(matches!(asked, Ok(Ok(Drafted::Held(_)))), "the draft waited on the answer pass: {asked:?}");
    drop(answering);
    waiting_pass.await.unwrap();
}
