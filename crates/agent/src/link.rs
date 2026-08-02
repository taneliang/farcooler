//! The private channel between a shim and its daemon.
//!
//! JSON lines rather than protobuf, and that is a deliberate exception to the
//! project's protocol rule rather than an oversight: the shim IS the daemon
//! binary under another subcommand, so the two can never disagree about a
//! schema. There is no compatibility surface here to protect.

use serde::{Deserialize, Serialize};

use crate::event::{Seq, Sequenced};

/// Shim to daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ShimMessage {
    /// Events at and after the subscribed cursor.
    Events { events: Vec<Sequenced> },
    /// The subscriber's cursor had been trimmed. A `Gap` is already the first
    /// entry in `events`; this carries the accounting for logs.
    Trimmed { resumed_at: Seq, dropped: u64, events: Vec<Sequenced> },
    /// The session is established and this is its id, for durable intent.
    Established { session_id: String, available_modes: Vec<String> },
    /// The adapter could not be started. Terminal-mode fallback remains.
    Failed { reason: String },
}

/// Daemon to shim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DaemonMessage {
    Subscribe { from_seq: Seq },
    Prompt { text: String },
    Answer { request_id: String, option_id: String },
    SetMode { agent_mode: String },
    SetModel { model: String },
    SetConfig { id: String, value: String },
    Cancel,
    /// Rewrite a prompt that has not been sent yet.
    EditQueued { id: String, text: String },
    /// Take back a prompt that has not been sent yet.
    CancelQueued { id: String },
}

pub fn encode_line<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    Ok(format!("{}\n", serde_json::to_string(value)?))
}

pub fn decode_line<T: for<'de> Deserialize<'de>>(line: &str) -> Result<T, serde_json::Error> {
    serde_json::from_str(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{AgentEvent, Role, Sequenced};

    #[test]
    fn a_message_round_trips_through_one_line() {
        let msg = ShimMessage::Events {
            events: vec![Sequenced {
                seq: 3,
                event: AgentEvent::Message { role: Role::Agent, text: "hi".into() },
            }],
        };
        let line = encode_line(&msg).expect("encodes");
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        let back: ShimMessage = decode_line(line.trim()).expect("decodes");
        assert_eq!(back, msg);
    }

    #[test]
    fn the_daemon_subscribes_from_a_cursor() {
        // Reconnect after a daemon restart is the whole reason this field
        // exists: the shim outlived the daemon and still holds the history.
        let line = encode_line(&DaemonMessage::Subscribe { from_seq: 12 }).expect("encodes");
        let back: DaemonMessage = decode_line(line.trim()).expect("decodes");
        assert_eq!(back, DaemonMessage::Subscribe { from_seq: 12 });
    }

    #[test]
    fn an_unknown_message_is_an_error_not_a_silent_drop() {
        assert!(decode_line::<DaemonMessage>(r#"{"kind":"from_the_future"}"#).is_err());
    }
}
