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

/// The reply's real encoded size, with a kilobyte for the envelope around it.
fn encoded(terminal: Uuid, events: Vec<Sequenced>, epoch: u64) -> usize {
    prost::Message::encoded_len(&crate::wire::agent_batch(terminal, events, epoch)) + 1024
}

fn established(supervisor: &AgentSupervisor, terminal: Uuid) {
    let message = ShimMessage::Established { session_id: "s".into(), available_modes: Vec::new() };
    supervisor.apply(terminal, message, &|_, _| {});
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
    let size = encoded(terminal, events.clone(), epoch);
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

/// The path the forensics bug took: a view that holds no epoch asks from zero
/// of a daemon whose pane has established a shim, so the epochs differ and the
/// whole window is the answer.
#[test]
fn a_view_with_no_epoch_attaching_to_an_established_chat_gets_a_reply_that_fits() {
    let supervisor = AgentSupervisor::new();
    let terminal = Uuid::now_v7();
    established(&supervisor, terminal);
    supervisor.record(terminal, (0..200).map(fat).collect(), &|_, _| {});

    let (epoch, events) = supervisor.replay(terminal, 0, 0);
    assert!(epoch >= 1);
    assert!(encoded(terminal, events.clone(), epoch) < MAX_CONTROL_ENVELOPE_BYTES);
    assert!(is_gap(&events[0]));
    assert_eq!(events.last().unwrap().seq, 199);
}

/// The newest event alone is past the budget: it arrives cut, with a marker,
/// at its own number, rather than as nothing.
#[test]
fn a_newest_event_past_the_budget_arrives_cut_not_dropped() {
    let supervisor = AgentSupervisor::new();
    let terminal = Uuid::now_v7();
    let huge = AgentEvent::Message { role: Role::Agent, text: "y".repeat(3 * 1024 * 1024), parent: None };
    supervisor.record(terminal, vec![delta(0), delta(1), huge], &|_, _| {});

    let (epoch, events) = supervisor.replay(terminal, 0, 0);
    assert!(encoded(terminal, events.clone(), epoch) < MAX_CONTROL_ENVELOPE_BYTES);
    let last = events.last().unwrap();
    assert_eq!(last.seq, 2);
    let AgentEvent::Message { text, .. } = &last.event else { panic!("the message stays a message") };
    assert!(text.starts_with("yyyy") && text.contains("too long to load"));
}

/// An event with no text to cut cannot be shown. The Gap that stands in for it
/// takes its number, so a cursor moves past it instead of sitting before it.
#[test]
fn a_newest_event_that_cannot_be_cut_leaves_a_gap_at_its_own_number() {
    let supervisor = AgentSupervisor::new();
    let terminal = Uuid::now_v7();
    let wide = AgentEvent::ToolCall {
        id: "t".into(),
        title: "z".repeat(3 * 1024 * 1024),
        kind: "other".into(),
        status: farcooler_agent::event::ToolStatus::Pending,
        locations: Vec::new(),
        parent: None,
        subagent: false,
    };
    supervisor.record(terminal, vec![delta(0), delta(1), wide], &|_, _| {});

    let (epoch, events) = supervisor.replay(terminal, 0, 0);
    assert_eq!(events.len(), 1);
    assert!(is_gap(&events[0]));
    assert_eq!(events[0].seq, 2);
    supervisor.record(terminal, vec![delta(3), delta(4)], &|_, _| {});
    let (_, tail) = supervisor.replay(terminal, 3, epoch);
    assert_eq!(tail.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![3, 4]);
}

/// A live batch over the budget is cut to its newest events behind a Gap, not
/// silently dropped.
#[test]
fn a_live_batch_over_the_budget_is_handed_out_with_a_gap() {
    let supervisor = AgentSupervisor::new();
    let terminal = Uuid::now_v7();
    let fanned: Mutex<Vec<Sequenced>> = Mutex::new(Vec::new());
    supervisor.record(terminal, (0..200).map(fat).collect(), &|_, batch| fanned.lock().unwrap().extend(batch));
    let fanned = fanned.into_inner().unwrap();
    assert!(is_gap(&fanned[0]));
    assert_eq!(fanned.last().unwrap().seq, 199);
    assert!(fanned.len() < 200);
}

/// A daemon restart: the shim reconnects and the new daemon numbers from zero
/// again. A client holding a cursor from the old life must be reset by a new
/// epoch, never matched into the new window.
#[test]
fn a_client_from_an_earlier_daemon_life_is_reset_not_matched() {
    let supervisor = AgentSupervisor::new();
    let terminal = Uuid::now_v7();
    // What the old life handed out: some epoch below this life's start, after
    // a toggle or two.
    let old_epoch = super::epoch::boot() - 5_000 + 1;
    established(&supervisor, terminal);
    supervisor.record(terminal, (0..300).map(delta).collect(), &|_, _| {});

    let (epoch, events) = supervisor.replay(terminal, 6000, old_epoch);
    assert!(epoch > old_epoch, "a later life's epochs are above every earlier one's");
    assert_eq!(events.len(), 300, "the whole window, not nothing from a stale cursor");

    // A hook-fed chat never establishes anything and used to report 0 forever.
    let hooked = Uuid::now_v7();
    supervisor.record(hooked, vec![delta(0)], &|_, _| {});
    assert!(supervisor.replay(hooked, 0, 0).0 > old_epoch);
}

/// The first epoch of a later life is above every epoch an earlier one issued,
/// is never zero, and fits a JSON number exactly.
#[test]
fn epochs_are_never_reused_across_lives() {
    use super::epoch::{next_after, FIRST_ALLOWED};
    let earlier_boot = 1_790_000_000_000;
    let mut last = earlier_boot;
    for _ in 0..1000 {
        last = next_after(last, earlier_boot);
    }
    assert!(next_after(0, earlier_boot + 60_000) > last);
    assert!(next_after(0, 0) >= FIRST_ALLOWED);
    assert!(super::epoch::boot() < (1 << 53));
}
