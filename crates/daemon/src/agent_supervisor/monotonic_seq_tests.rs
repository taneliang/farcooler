//! ov-380: a long chat keeps updating past the window.

use super::*;
use farcooler_agent::event::Role;

fn delta(n: usize) -> AgentEvent {
    AgentEvent::Message { role: Role::Agent, text: format!("tok{n} "), parent: None }
}

fn is_gap(item: &Sequenced) -> bool {
    matches!(item.event, AgentEvent::Gap { reason: AgentGapReason::RingTrimmed })
}

/// A reader that follows the way `AgentStream` and `agent_follow.rs` do: ask
/// from one past the highest seq seen, same epoch. It must receive every event
/// the agent says, however far past the window the transcript runs.
#[test]
fn a_following_reader_receives_every_event_past_the_window() {
    let supervisor = AgentSupervisor::new();
    let terminal = Uuid::now_v7();

    supervisor.record(terminal, (0..TRANSCRIPT_LIMIT).map(delta).collect(), &|_, _| {});
    let (epoch, first) = supervisor.replay(terminal, 0, 0);
    assert_eq!(first.len(), TRANSCRIPT_LIMIT);
    let mut cursor = first.iter().map(|e| e.seq + 1).max().unwrap();

    let mut seen = Vec::new();
    for n in 0..500 {
        supervisor.record(terminal, vec![delta(TRANSCRIPT_LIMIT + n)], &|_, _| {});
        let (e, batch) = supervisor.replay(terminal, cursor, epoch);
        assert_eq!(e, epoch);
        for item in &batch {
            cursor = cursor.max(item.seq + 1);
        }
        seen.extend(batch);
    }
    assert_eq!(seen.len(), 500, "every event after the window filled reaches a following reader");
    assert!(
        seen.iter().all(|item| !is_gap(item)),
        "a reader that kept up was not told it missed anything"
    );
    assert_eq!(cursor, (TRANSCRIPT_LIMIT + 500) as u64);
}

/// A reader that attaches late and from nothing still sees the trim.
#[test]
fn a_reader_attaching_after_the_trim_gets_the_gap_and_the_newest() {
    let supervisor = AgentSupervisor::new();
    let terminal = Uuid::now_v7();
    supervisor.record(terminal, (0..TRANSCRIPT_LIMIT + 100).map(delta).collect(), &|_, _| {});

    let (_, events) = supervisor.replay(terminal, 0, 0);
    assert!(is_gap(&events[0]));
    assert_eq!(events.last().unwrap().seq, (TRANSCRIPT_LIMIT + 99) as u64);
    assert!(events.windows(2).all(|pair| pair[0].seq < pair[1].seq));
}
