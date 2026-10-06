//! The ring trims with a Gap, and what a trim hands the fan-out.

use super::*;

/// A transcript that has lost its head says so.
///
/// The window is renumbered by position, so trimming the front erases every
/// trace that anything was there: a client would receive a shorter
/// transcript with contiguous numbers and no reason to doubt it. A derived
/// transcript is only defensible because it can say where it is incomplete.
#[test]
fn trimming_the_window_leaves_a_gap_rather_than_a_shorter_story() {
    let supervisor = AgentSupervisor::new();
    let terminal = Uuid::now_v7();

    // Comfortably past the limit, so the front is dropped several times.
    let mut sent = 0;
    while sent < TRANSCRIPT_LIMIT + 500 {
        let batch: Vec<Sequenced> = (0..250)
            .map(|i| Sequenced {
                seq: i,
                event: AgentEvent::Message {
                    role: farcooler_agent::event::Role::Agent,
                    text: format!("line {}", sent + i as usize), parent: None },
            })
            .collect();
        supervisor.apply(terminal, ShimMessage::Events { events: batch }, &|_, _| {});
        sent += 250;
    }

    let (_, events) = supervisor.replay(terminal, 0, 0);
    assert_eq!(events.len(), TRANSCRIPT_LIMIT, "the window is bounded");
    assert!(
        matches!(
            events[0].event,
            AgentEvent::Gap { reason: AgentGapReason::RingTrimmed }
        ),
        "a trimmed transcript must open with the gap that says so, got {:?}",
        events[0].event
    );
    // Numbered by the life of the transcript, not by the window: the
    // numbers keep climbing across every trim (ov-380), so a cursor that
    // only moves forward is never left past the end.
    assert_eq!(events.last().unwrap().seq, sent as u64 - 1);
    assert!(events.windows(2).all(|pair| pair[0].seq < pair[1].seq));
}

/// A transcript filled by recorded events is bounded and says so too.
///
/// The trim, the gap and the renumber are the rest of the tail `record`
/// was split out of. A `record` that appended and numbered but stopped
/// short of them would pass every other test in this file and grow for as
/// long as the daemon stayed up — and the loss would arrive as a shorter
/// story with contiguous numbers, which is the one thing this design says
/// it will never do.
#[test]
fn a_transcript_filled_by_recorded_events_is_trimmed_and_says_so() {
    let supervisor = AgentSupervisor::new();
    let terminal = Uuid::now_v7();

    let mut sent = 0;
    while sent < TRANSCRIPT_LIMIT + 500 {
        let batch: Vec<AgentEvent> = (0..250)
            .map(|i| AgentEvent::Message {
                role: farcooler_agent::event::Role::Agent,
                text: format!("line {}", sent + i),
                parent: None,
            })
            .collect();
        supervisor.record(terminal, batch, &|_, _| {});
        sent += 250;
    }

    let (_, events) = supervisor.replay(terminal, 0, 0);
    assert_eq!(events.len(), TRANSCRIPT_LIMIT, "the window is bounded on this path as well");
    assert!(
        matches!(
            events[0].event,
            AgentEvent::Gap { reason: AgentGapReason::RingTrimmed }
        ),
        "a trimmed transcript must open with the gap that says so, got {:?}",
        events[0].event
    );
    assert_eq!(events.last().unwrap().seq, sent as u64 - 1, "numbers are never reset by a trim");
    assert!(events.windows(2).all(|pair| pair[0].seq < pair[1].seq));
}

/// What the fan-out is handed across a trim is numbered like the
/// transcript it came out of, and is all of what was submitted.
///
/// Numbers are never reassigned by a trim (ov-380), so the batch carries
/// the numbers it was given and a replay asked afterwards agrees with it
/// event for event. The length assertion is what stops
/// `on_events(terminal, renumbered)` being narrowed to the last event
/// alone, which the collecting test above cannot see because every batch
/// it submits is one event long.
#[test]
fn a_batch_handed_out_across_a_trim_carries_the_numbers_it_will_be_asked_for() {
    const OVERFLOW: usize = 10;
    let supervisor = AgentSupervisor::new();
    let terminal = Uuid::now_v7();
    let message = |n: usize| AgentEvent::Message {
        role: farcooler_agent::event::Role::Agent,
        text: format!("line {n}"),
        parent: None,
    };
    let fanned: Mutex<Vec<Vec<Sequenced>>> = Mutex::new(Vec::new());
    let sink = |_: Uuid, batch: Vec<Sequenced>| fanned.lock().unwrap().push(batch);

    // Exactly full, and deliberately not yet over: the limit is a maximum,
    // so this batch is handed out untouched and the NEXT one is the one
    // that drops a front.
    supervisor.record(terminal, (0..TRANSCRIPT_LIMIT).map(message).collect(), &sink);
    supervisor.record(terminal, (0..OVERFLOW).map(message).collect(), &sink);

    let fanned = fanned.into_inner().unwrap();
    let last = fanned.last().expect("both batches reached the fan-out");
    assert_eq!(last.len(), OVERFLOW, "every event submitted is handed on, not merely the last");
    assert_eq!(
        last.first().unwrap().seq,
        TRANSCRIPT_LIMIT as u64,
        "the batch starts where the events were numbered, trim or no trim"
    );
    assert_eq!(
        last.last().unwrap().seq,
        (TRANSCRIPT_LIMIT + OVERFLOW - 1) as u64,
        "and ends at its end: a subscriber's next cursor is built from this number \
         and must not point past a transcript it can ask for"
    );

    // The same events, by the same numbers, as a client that asked instead
    // of being told.
    let (_, replayed) = supervisor.replay(terminal, 0, 0);
    assert_eq!(
        last.as_slice(),
        &replayed[replayed.len() - OVERFLOW..],
        "the live batch and the replayed transcript disagree about the same events"
    );
}
