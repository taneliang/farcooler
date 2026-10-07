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
