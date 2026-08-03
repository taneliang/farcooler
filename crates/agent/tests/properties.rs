//! The three invariants the whole design rests on.

use farcooler_agent::event::{AgentEvent, AgentGapReason, Role};
use farcooler_agent::ring::{AgentReplay, AgentRing};

fn msg(i: usize) -> AgentEvent {
    AgentEvent::Message { role: Role::Agent, text: format!("m{i}"), parent: None }
}

#[test]
fn sequence_numbers_are_monotonic_across_any_number_of_pushes() {
    let mut ring = AgentRing::with_capacity(16);
    let mut last = None;
    for i in 0..1000 {
        let seq = ring.push(msg(i));
        if let Some(prev) = last {
            assert!(seq > prev, "seq went backwards at {i}");
        }
        last = Some(seq);
    }
}

#[test]
fn replay_from_any_retained_cursor_reconstructs_an_identical_transcript() {
    let mut ring = AgentRing::with_capacity(64);
    for i in 0..50 {
        ring.push(msg(i));
    }
    let AgentReplay::At { events: all } = ring.since(0) else { panic!("nothing was trimmed") };

    for cursor in 0..50u64 {
        let AgentReplay::At { events } = ring.since(cursor) else { panic!("cursor {cursor}") };
        let expected: Vec<_> = all.iter().filter(|e| e.seq >= cursor).cloned().collect();
        assert_eq!(events, expected, "replay from {cursor} diverged");
    }
}

#[test]
fn dropped_history_is_always_a_gap_and_never_a_shorter_transcript() {
    // The contract that makes a derived transcript permissible in a product
    // that refuses to guess. A reader must never receive a short list that
    // looks complete.
    let mut ring = AgentRing::with_capacity(8);
    for i in 0..100 {
        ring.push(msg(i));
    }
    match ring.since(0) {
        AgentReplay::Gap { resumed_at, dropped, events } => {
            assert_eq!(dropped, resumed_at);
            assert_eq!(events.len(), 8);
        }
        AgentReplay::At { .. } => panic!("trimmed history reported as complete"),
    }
}

#[test]
fn an_unparsed_update_is_representable_end_to_end() {
    let mut ring = AgentRing::new();
    ring.push(AgentEvent::Gap { reason: AgentGapReason::Unparsed });
    let AgentReplay::At { events } = ring.since(0) else { panic!("retained") };
    assert!(matches!(events[0].event, AgentEvent::Gap { .. }));
}
