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

#[derive(Default)]
pub struct MessageAssembler {
    /// Text so far, keyed by the agent's own message id.
    partial: HashMap<String, String>,
}

impl MessageAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many messages are still open. For tests and for logs.
    pub fn pending(&self) -> usize {
        self.partial.len()
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
        let Some(id) = payload.get("message_id").and_then(|v| v.as_str()) else {
            return Vec::new();
        };
        let delta = payload.get("delta").and_then(|v| v.as_str()).unwrap_or_default();
        let text = self.partial.entry(id.to_string()).or_default();
        text.push_str(delta);

        // `final` is the end-of-message signal regardless of whether the last
        // delta is empty, which it is whenever the message ended on a newline.
        if !payload.get("final").and_then(|v| v.as_bool()).unwrap_or(false) {
            return Vec::new();
        }
        let text = self.partial.remove(id).unwrap_or_default();
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

    /// A finished message must not be held forever.
    #[test]
    fn a_finished_message_is_forgotten() {
        let mut a = MessageAssembler::new();
        a.accept(Agent::Claude, "MessageDisplay", &display("m1", 0, true, "done"));
        assert_eq!(a.pending(), 0, "nothing is kept once a message has ended");
    }
}
