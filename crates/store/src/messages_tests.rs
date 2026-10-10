//! The message queue (ov-455), on an in-memory board.

use super::*;

fn board() -> (Store, Uuid, Uuid) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let task = store.create_task(main, "Drill-in layout", Actor::Manager).unwrap().id;
    (store, main, task)
}

/// A message is a note and a queued row, told in the order sent, carrying
/// whom it is for, and written as a comment, or progress from the runner.
#[test]
fn a_message_is_filed_and_queued_in_order() {
    let (store, _, task) = board();
    let pane = Uuid::now_v7();
    let agent = Actor::Agent { terminal: pane };
    let first = store.send_message(task, agent, "PR is up", None, serde_json::json!({})).unwrap();
    let second = store.send_message(task, Actor::Manager, "Rebase first", Some(pane), serde_json::json!({})).unwrap();
    let third = store.send_message(task, Actor::Runner, "It stopped", None, serde_json::json!({})).unwrap();
    assert_eq!((first.kind, second.kind, third.kind), (NoteKind::Comment, NoteKind::Comment, NoteKind::Progress));
    let pending = store.pending_message_wakes().unwrap();
    let got: Vec<_> = pending.iter().map(|w| (w.note, w.to, w.actor, w.body.as_str())).collect();
    assert_eq!(
        got,
        [
            (first.id, None, agent, "PR is up"),
            (second.id, Some(pane), Actor::Manager, "Rebase first"),
            (third.id, None, Actor::Runner, "It stopped"),
        ]
    );
    assert!(pending.iter().all(|w| w.kind == WakeKind::Message));
}

/// Told is told once: claimed, then finished, a message leaves the queue, and
/// a second claim gets nothing to type.
#[test]
fn a_message_is_told_at_most_once() {
    let (store, _, task) = board();
    store.send_message(task, Actor::Manager, "Go", None, serde_json::json!({})).unwrap();
    let wake = store.pending_message_wakes().unwrap().remove(0);
    assert!(store.claim_wake(&wake).unwrap());
    assert!(!store.claim_wake(&wake).unwrap(), "claimed once");
    store.finish_wake(&wake, Some("Told the orchestrator")).unwrap().unwrap();
    assert!(store.pending_message_wakes().unwrap().is_empty());
    assert_eq!(store.finish_wake(&wake, None).unwrap(), None, "a second finisher writes nothing");
}

/// A board that doesn't wake on answers takes no message, and writes nothing.
#[test]
fn a_board_that_types_nothing_takes_no_message() {
    let (store, main, task) = board();
    let ws = store.get_workspace(main).unwrap();
    store.set_workspace_wake_on_answer(main, ws.resource_version, false).unwrap();
    let refused = store.send_message(task, Actor::Manager, "Go", None, serde_json::json!({}));
    assert!(matches!(refused, Err(DomainError::Conflict { what: "typing_off" })), "{refused:?}");
    assert!(store.pending_message_wakes().unwrap().is_empty());
    assert!(store.notes_for(task, Some(NoteKind::Comment)).unwrap().is_empty());
}

/// The counts the flood limits read: what waits for a recipient, and what one
/// sender sent one recipient lately.
#[test]
fn waiting_and_sent_are_counted_per_recipient() {
    let (store, main, task) = board();
    let pane = Uuid::now_v7();
    let agent = Actor::Agent { terminal: pane };
    for _ in 0..3 {
        store.send_message(task, agent, "hi", None, serde_json::json!({})).unwrap();
    }
    store.send_message(task, Actor::Manager, "hi", Some(pane), serde_json::json!({})).unwrap();
    assert_eq!(store.messages_waiting(main, None).unwrap(), 3);
    assert_eq!(store.messages_waiting(main, Some(pane)).unwrap(), 1);
    assert_eq!(store.messages_sent_since(main, agent, None, 0).unwrap(), 3);
    assert_eq!(store.messages_sent_since(main, Actor::Manager, None, 0).unwrap(), 0);
    assert_eq!(store.messages_sent_since(main, agent, None, i64::MAX).unwrap(), 0);
}

/// An agent has reported when it wrote on its card since a time, or when the
/// runner's own notice about the card still waits.
#[test]
fn reported_reads_the_agents_writes_and_a_waiting_notice() {
    let (store, _, task) = board();
    let agent = Actor::Agent { terminal: Uuid::now_v7() };
    assert!(!store.reported_since(task, agent, 0).unwrap());
    store.add_note(task, NoteKind::Decision, agent, "Chose B", serde_json::json!({})).unwrap();
    assert!(store.reported_since(task, agent, 0).unwrap());
    assert!(!store.reported_since(task, agent, i64::MAX).unwrap());
    store.send_message(task, Actor::Runner, "It stopped", None, serde_json::json!({})).unwrap();
    assert!(store.reported_since(task, agent, i64::MAX).unwrap(), "a notice already waits");
}
