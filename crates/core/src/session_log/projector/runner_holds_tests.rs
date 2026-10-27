//! A message the runner holds (ov-468): Queued while held, taken back once
//! typed, Withdrawn with a notice when given up.

use super::*;

fn held(p: &Projection, id: &str) -> Option<Row> {
    p.rows().iter().find(|r| r.id == format!("held:{id}")).cloned()
}

#[test]
fn a_held_message_is_queued_then_taken_back_or_withdrawn_with_why() {
    let mut p = Projection::default();
    p.runner_held("a", "after the tests", 5);
    p.runner_held("a", "after the tests", 6);
    let row = held(&p, "a").unwrap();
    assert_eq!(p.rows().iter().filter(|r| r.id == "held:a").count(), 1, "made once");
    assert_eq!(row.kind, RowKind::Queued(Queued { text: "after the tests".into(), state: QueuedState::Waiting, at_ms: Some(5) }));
    p.runner_told("a");
    assert!(held(&p, "a").unwrap().retracted);

    p.runner_held("b", "and the docs", 7);
    p.runner_dropped("b", "Not sent: “and the docs”. Claude isn’t running in this pane anymore.", 9);
    let RowKind::Queued(q) = held(&p, "b").unwrap().kind else { panic!() };
    assert_eq!(q.state, QueuedState::Withdrawn);
    let notice = p.rows().last().unwrap();
    assert_eq!(
        notice.kind,
        RowKind::Notice(Notice {
            kind: NoticeKind::NotSent,
            text: "Not sent: “and the docs”. Claude isn’t running in this pane anymore.".into(),
            at_ms: Some(9)
        })
    );
}
<<<<<<< HEAD

/// ov-827: a message in the box whose Enter waits for the turn's end is
/// `InBox` ("Waiting to send"), made once; a held row moves to it; and it's
/// taken back or Withdrawn as a held one is.
#[test]
fn a_message_waiting_in_the_box_is_in_box_then_taken_back_or_withdrawn() {
    let mut p = Projection::default();
    p.runner_in_box("a", "what did they find", 5);
    p.runner_in_box("a", "what did they find", 6);
    assert_eq!(p.rows().iter().filter(|r| r.id == "held:a").count(), 1, "made once");
    let want = Queued { text: "what did they find".into(), state: QueuedState::InBox, at_ms: Some(5) };
    assert_eq!(held(&p, "a").unwrap().kind, RowKind::Queued(want));
    p.runner_told("a");
    assert!(held(&p, "a").unwrap().retracted);

    p.runner_held("b", "and the docs", 7);
    p.runner_in_box("b", "and the docs", 8);
    let RowKind::Queued(q) = held(&p, "b").unwrap().kind else { panic!() };
    assert_eq!(q.state, QueuedState::InBox, "a held row moves to it");
    p.runner_dropped("b", "Claude’s box changed while “and the docs” waited, so the runner didn’t send it. Check the terminal.", 9);
    let RowKind::Queued(q) = held(&p, "b").unwrap().kind else { panic!() };
    assert_eq!(q.state, QueuedState::Withdrawn);
}
||||||| parent of f0e87faef (daemon, Mac: a give-up is said in its own turn and once, a Pair that can't reach the runner says so, a replaced connect leaves the newer spinner, and the first page's join and an answered ask's replay are tested (ov-830))
=======

/// A give-up told twice, a replay racing the live call (ov-830 review),
/// says its notice once. Goes red with the `runner_dropped` set gone.
#[test]
fn a_give_up_told_twice_says_so_once() {
    let mut p = Projection::new();
    p.runner_held("c", "Run it", 1);
    p.runner_dropped("c", "Not sent: “Run it”.", 2);
    p.runner_dropped("c", "Not sent: “Run it”.", 2);
    let notices = p.rows().iter().filter(|r| matches!(r.kind, RowKind::Notice(_))).count();
    assert_eq!(notices, 1);
}
>>>>>>> f0e87faef (daemon, Mac: a give-up is said in its own turn and once, a Pair that can't reach the runner says so, a replaced connect leaves the newer spinner, and the first page's join and an answered ask's replay are tested (ov-830))
