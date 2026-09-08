//! Hook payloads, as the events every Far Cooler client already renders.
//!
//! The target is `AgentEvent` and not a new type, which is the single decision
//! that makes this design cheap on the client side: the iOS `AgentStream`, the
//! Android `AgentScreen`, the Mac's `AgentSurface`, `Transcript` and the lock
//! screen card all read that stream today and none of them change.
//!
//! Stateful, because claude's prose arrives in pieces. Measured on 2.1.263: a
//! ~400 word answer arrived as six `MessageDisplay` flushes about two seconds
//! apart, each carrying only the lines completed since the last. Emitting an
//! event per flush would draw the same answer six times over.

use std::collections::HashMap;

use farcooler_agent_core::event::{AgentEvent, Role};

use crate::Agent;

/// One message's assembly state, held in `MessageAssembler::state`.
///
/// A plain `HashMap<_, String>` that removes its key on `final` cannot tell a
/// message that has already closed from one that was never opened at all —
/// and both a redelivered `final` and a stray delta arriving after close turn
/// on exactly that distinction: both look, from a map that only tracks open
/// accumulations, like a message starting for the first time. `Closed` fixes
/// that by staying in the map. It carries no payload, because once a message
/// closes every later arrival for it is dropped outright — nothing more than
/// the fact of its closing needs remembering.
enum MessageState {
    Open {
        text: String,
        /// The highest `index` folded into `text` so far. `None` before the
        /// first delta, so a genuine first delta at index `0` is never
        /// mistaken for a repeat of one already applied.
        last_index: Option<u64>,
    },
    Closed,
}

#[derive(Default)]
pub struct MessageAssembler {
    /// Assembly state, keyed by `(turn_id, message_id)` — the pair the
    /// design says `MessageDisplay` deltas are keyed by
    /// (docs/superpowers/specs/2026-09-07-live-agent-sessions-design.md:193-194).
    /// `index` is deliberately not part of the key: within one message it
    /// orders deltas and marks repeats (`MessageState::Open::last_index`),
    /// which is different work from identifying which message a delta
    /// belongs to.
    state: HashMap<(String, String), MessageState>,
}

impl MessageAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many messages are still being accumulated. For tests and for
    /// logs. Counts only `Open` entries: a closed message's text is gone
    /// from here the moment it closes, even though `state` keeps a small
    /// marker for it (`MessageState::Closed`) so a later arrival for it can
    /// be told apart from a message never seen at all.
    pub fn pending(&self) -> usize {
        self.state.values().filter(|s| matches!(s, MessageState::Open { .. })).count()
    }

    pub fn accept(
        &mut self,
        agent: Agent,
        event: &str,
        payload: &serde_json::Value,
    ) -> Vec<AgentEvent> {
        match (agent, event) {
            (Agent::Claude, "MessageDisplay") => self.claude_display(payload),
            _ => Vec::new(),
        }
    }

    fn claude_display(&mut self, payload: &serde_json::Value) -> Vec<AgentEvent> {
        let Some(turn_id) = payload.get("turn_id").and_then(|v| v.as_str()) else {
            return Vec::new();
        };
        let Some(message_id) = payload.get("message_id").and_then(|v| v.as_str()) else {
            return Vec::new();
        };
        let Some(index) = payload.get("index").and_then(|v| v.as_u64()) else {
            return Vec::new();
        };
        let delta = payload.get("delta").and_then(|v| v.as_str()).unwrap_or_default();
        let final_flush = payload.get("final").and_then(|v| v.as_bool()).unwrap_or(false);

        let key = (turn_id.to_string(), message_id.to_string());

        // Already closed: this message went out as an `AgentEvent` once
        // already. A repeat of the terminal flush and a straggler that
        // outran its own `final` both land here, and both must be dropped
        // rather than reopening the message — reopening would either draw
        // its tail a second time, or create an entry with no `final` left
        // ever to close it.
        if matches!(self.state.get(&key), Some(MessageState::Closed)) {
            return Vec::new();
        }

        let (mut text, last_index) = match self.state.remove(&key) {
            Some(MessageState::Open { text, last_index }) => (text, last_index),
            _ => (String::new(), None),
        };

        // An index at or below the last one folded in is a duplicate or an
        // out-of-order redelivery of a delta this accumulation already has.
        // `message_id` alone cannot tell it apart from the next real delta —
        // only `index` can.
        if last_index.is_some_and(|last| index <= last) {
            self.state.insert(key, MessageState::Open { text, last_index });
            return Vec::new();
        }

        text.push_str(delta);

        // `final_flush` alone marks the end of the message; an empty delta
        // on that flush is treated the same as any other.
        if !final_flush {
            self.state.insert(key, MessageState::Open { text, last_index: Some(index) });
            return Vec::new();
        }

        self.state.insert(key, MessageState::Closed);
        if text.is_empty() {
            return Vec::new();
        }
        vec![AgentEvent::Message { role: Role::Agent, text, parent: None }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_agent_core::event::{AgentEvent, Role};

    fn display(message_id: &str, index: u64, final_flush: bool, delta: &str) -> serde_json::Value {
        serde_json::json!({
            "hook_event_name": "MessageDisplay",
            "session_id": "s",
            "turn_id": "t",
            "message_id": message_id,
            "index": index,
            "final": final_flush,
            "delta": delta,
        })
    }

    /// Six flushes were measured for a 400-word answer. A `Message` per flush
    /// would draw the same answer six times.
    #[test]
    fn deltas_accumulate_and_only_the_final_flush_emits_a_message() {
        let mut a = MessageAssembler::new();
        assert!(a.accept(Agent::Claude, "MessageDisplay", &display("m1", 0, false, "one ")).is_empty());
        assert!(a.accept(Agent::Claude, "MessageDisplay", &display("m1", 1, false, "two ")).is_empty());

        let out = a.accept(Agent::Claude, "MessageDisplay", &display("m1", 2, true, "three"));
        assert_eq!(
            out,
            vec![AgentEvent::Message {
                role: Role::Agent,
                text: "one two three".to_string(),
                parent: None,
            }],
            "one message, assembled from every flush of it"
        );
    }

    /// Two messages in one turn must not run together.
    #[test]
    fn a_second_message_starts_its_own_text() {
        let mut a = MessageAssembler::new();
        a.accept(Agent::Claude, "MessageDisplay", &display("m1", 0, true, "first"));
        let out = a.accept(Agent::Claude, "MessageDisplay", &display("m2", 0, true, "second"));
        assert_eq!(
            out,
            vec![AgentEvent::Message {
                role: Role::Agent,
                text: "second".to_string(),
                parent: None,
            }]
        );
    }

    /// A finished message's text must not be held forever.
    #[test]
    fn a_finished_message_is_forgotten() {
        let mut a = MessageAssembler::new();
        a.accept(Agent::Claude, "MessageDisplay", &display("m1", 0, true, "done"));
        assert_eq!(a.pending(), 0, "nothing is still being accumulated once a message has ended");
    }

    /// A flush that arrives twice — a retry after a lost ack, say — must
    /// fold into the message once, not twice. Nothing but `index` can tell
    /// this apart from the next real delta: `message_id` is unchanged.
    #[test]
    fn a_repeated_index_does_not_double_its_own_delta() {
        let mut a = MessageAssembler::new();
        assert!(a.accept(Agent::Claude, "MessageDisplay", &display("m1", 0, false, "one ")).is_empty());
        // The same flush, resent.
        assert!(a.accept(Agent::Claude, "MessageDisplay", &display("m1", 0, false, "one ")).is_empty());

        let out = a.accept(Agent::Claude, "MessageDisplay", &display("m1", 1, true, "two"));
        assert_eq!(
            out,
            vec![AgentEvent::Message { role: Role::Agent, text: "one two".to_string(), parent: None }],
            "the repeated index-0 flush must not be folded in twice"
        );
    }

    /// A redelivered `final` — the same terminal flush arriving twice — must
    /// not draw the message a second time. This is this task's whole point
    /// (six flushes, one message) undone by exactly one retry.
    #[test]
    fn a_redelivered_final_does_not_emit_a_second_message() {
        let mut a = MessageAssembler::new();
        let first = a.accept(Agent::Claude, "MessageDisplay", &display("m1", 0, true, "done"));
        assert_eq!(
            first,
            vec![AgentEvent::Message { role: Role::Agent, text: "done".to_string(), parent: None }]
        );

        let second = a.accept(Agent::Claude, "MessageDisplay", &display("m1", 0, true, "done"));
        assert!(second.is_empty(), "a redelivered final must not emit a second message");
    }

    /// A delta that outruns its own message's `final` — arriving after the
    /// message already closed — must not reopen it. Reopening would create
    /// an entry with no `final` left to close it: exactly the leak `final`
    /// exists to prevent.
    #[test]
    fn a_delta_after_close_does_not_reopen_the_message() {
        let mut a = MessageAssembler::new();
        a.accept(Agent::Claude, "MessageDisplay", &display("m1", 0, true, "done"));
        let out = a.accept(Agent::Claude, "MessageDisplay", &display("m1", 1, false, "more"));
        assert!(out.is_empty(), "a stray delta after close emits nothing");
        assert_eq!(a.pending(), 0, "a stray delta after close must not open an entry that can never close");
    }
}
