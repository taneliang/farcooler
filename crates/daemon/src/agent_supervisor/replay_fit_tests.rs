//! ov-381: a returning chat loads when its window is bigger than one envelope.

use super::*;
use farcooler_agent::event::Role;
use farcooler_protocol::MAX_CONTROL_ENVELOPE_BYTES;

fn delta(n: usize) -> AgentEvent {
    AgentEvent::Message { role: Role::Agent, text: format!("tok{n} "), parent: None }
}

fn fat(n: usize) -> AgentEvent {
    AgentEvent::Message { role: Role::Agent, text: format!("{n}:{}", "x".repeat(8 * 1024)), parent: None }
}

fn is_gap(item: &Sequenced) -> bool {
    matches!(item.event, AgentEvent::Gap { reason: AgentGapReason::RingTrimmed })
}

/// A window past the envelope cap: the attach gets its newest events and a
/// Gap, all of it inside the budget, and it can follow from there.
#[test]
fn a_window_past_the_envelope_cap_replays_its_newest_events_behind_a_gap() {
    let supervisor = AgentSupervisor::new();
    let terminal = Uuid::now_v7();
    // 200 events of 8 KiB: about 1.6 MiB, well past the 1 MiB cap.
    supervisor.record(terminal, (0..200).map(fat).collect(), &|_, _| {});

    let (epoch, events) = supervisor.replay(terminal, 0, 0);
    let frames = crate::wire::agent_batch(terminal, events.clone(), epoch).events;
    let size: usize = frames.iter().map(|f| f.payload_json.len() + 24).sum();
    assert!(size < MAX_CONTROL_ENVELOPE_BYTES, "the replay is {size} bytes");

    assert!(is_gap(&events[0]), "the replay says older history was left out");
    assert!(events.len() > 50 && events.len() < 200, "newest events, not all and not none: {}", events.len());
    assert_eq!(events.last().unwrap().seq, 199, "it ends at the newest event");
    assert!(events.windows(2).all(|pair| pair[0].seq < pair[1].seq));
    let contiguous = &events[1..];
    assert!(contiguous.windows(2).all(|pair| pair[1].seq == pair[0].seq + 1), "no holes after the gap");

    // And it follows from there.
    let cursor = events.last().unwrap().seq + 1;
    supervisor.record(terminal, vec![fat(200)], &|_, _| {});
    let (_, more) = supervisor.replay(terminal, cursor, epoch);
    assert_eq!(more.len(), 1);
    assert_eq!(more[0].seq, 200);
}

/// A window that fits is handed over whole, with no gap invented.
#[test]
fn a_window_that_fits_is_replayed_whole() {
    let supervisor = AgentSupervisor::new();
    let terminal = Uuid::now_v7();
    supervisor.record(terminal, (0..300).map(delta).collect(), &|_, _| {});
    let (_, events) = supervisor.replay(terminal, 0, 0);
    assert_eq!(events.len(), 300);
    assert!(!events.iter().any(is_gap));
}
