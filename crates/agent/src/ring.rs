//! A session's recent events, bounded, replayable from a cursor.
//!
//! This is the terminal `ReplayBuffer`'s idea at a different granularity:
//! terminals count bytes because a byte offset is the only honest cursor into a
//! stream with no record boundaries, and agent events count events because they
//! have them. What is shared is the part that matters — dropping history is
//! always visible to the reader.
//!
//! It lives in the SHIM, not the daemon. The shim is co-located with the agent
//! and dies with the pane whose liveness is already authoritative, so a daemon
//! restart never costs a transcript.

use std::collections::VecDeque;

use crate::event::{AgentEvent, Seq, Sequenced};

/// Events retained per session. At roughly a few hundred bytes each this is a
/// small number of megabytes for an farcooler session, and trimming is visible.
pub const AGENT_RING_EVENTS: usize = 4096;

#[derive(Debug, PartialEq, Eq)]
pub enum AgentReplay {
    /// The cursor was still retained; these are the events after it.
    At { events: Vec<Sequenced> },
    /// The cursor had been trimmed. The reader must render a break before
    /// applying `events`.
    Gap { resumed_at: Seq, dropped: u64, events: Vec<Sequenced> },
}

#[derive(Debug)]
pub struct AgentRing {
    events: VecDeque<Sequenced>,
    capacity: usize,
    next_seq: Seq,
}

impl AgentRing {
    pub fn new() -> Self {
        Self::with_capacity(AGENT_RING_EVENTS)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self { events: VecDeque::new(), capacity: capacity.max(1), next_seq: 0 }
    }

    /// The seq the next pushed event will take.
    pub fn next_seq(&self) -> Seq {
        self.next_seq
    }

    /// The oldest seq still retained.
    pub fn oldest_seq(&self) -> Seq {
        self.events.front().map(|e| e.seq).unwrap_or(self.next_seq)
    }

    pub fn push(&mut self, event: AgentEvent) -> Seq {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.events.push_back(Sequenced { seq, event });
        while self.events.len() > self.capacity {
            self.events.pop_front();
        }
        seq
    }

    /// Everything from `from` onwards, or a gap if `from` is no longer held.
    pub fn since(&self, from: Seq) -> AgentReplay {
        let oldest = self.oldest_seq();
        if from < oldest {
            return AgentReplay::Gap {
                resumed_at: oldest,
                dropped: oldest - from,
                events: self.events.iter().cloned().collect(),
            };
        }
        AgentReplay::At {
            events: self.events.iter().filter(|e| e.seq >= from).cloned().collect(),
        }
    }
}

impl Default for AgentRing {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{AgentEvent, AgentGapReason, EndReason, Role};

    fn msg(text: &str) -> AgentEvent {
        AgentEvent::Message { role: Role::Agent, text: text.to_string() }
    }

    #[test]
    fn sequence_numbers_are_monotonic_from_zero() {
        let mut ring = AgentRing::new();
        assert_eq!(ring.push(msg("a")), 0);
        assert_eq!(ring.push(msg("b")), 1);
        assert_eq!(ring.next_seq(), 2);
    }

    #[test]
    fn replay_from_a_cursor_returns_exactly_what_came_after_it() {
        let mut ring = AgentRing::new();
        ring.push(msg("a"));
        ring.push(msg("b"));
        ring.push(msg("c"));
        let AgentReplay::At { events } = ring.since(1) else { panic!("expected retained replay") };
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq, 1);
        assert_eq!(events[1].seq, 2);
    }

    #[test]
    fn replay_from_the_head_is_empty_not_a_gap() {
        // A caller that is fully caught up must not be told it lost history.
        let mut ring = AgentRing::new();
        ring.push(msg("a"));
        let AgentReplay::At { events } = ring.since(1) else { panic!("expected retained replay") };
        assert!(events.is_empty());
    }

    #[test]
    fn a_trimmed_ring_reports_a_gap_rather_than_a_shorter_transcript() {
        // The rule this whole crate exists to keep. Dropping the oldest events
        // silently would give a client a transcript that is wrong and looks
        // complete.
        let mut ring = AgentRing::with_capacity(2);
        ring.push(msg("a"));
        ring.push(msg("b"));
        ring.push(msg("c"));
        let AgentReplay::Gap { resumed_at, dropped, events } = ring.since(0) else {
            panic!("expected a gap")
        };
        assert_eq!(resumed_at, 1);
        assert_eq!(dropped, 1);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn a_gap_event_can_itself_be_stored_and_replayed() {
        // `session/load` unsupported on reconnect pushes a Gap into the stream;
        // it must survive replay like any other event.
        let mut ring = AgentRing::new();
        ring.push(AgentEvent::Gap { reason: AgentGapReason::LoadUnsupported });
        ring.push(AgentEvent::TurnEnded { reason: EndReason::EndTurn });
        let AgentReplay::At { events } = ring.since(0) else { panic!("expected retained replay") };
        assert!(matches!(events[0].event, AgentEvent::Gap { .. }));
    }
}
