//! The two frames that cross the ingress socket.

use serde::{Deserialize, Serialize};

use crate::Agent;

/// One hook firing, as the hook binary sends it.
///
/// The payload is carried WHOLE and unparsed. The hook binary is on the
/// critical path of somebody's agent and must do as little as possible; and a
/// hook shape that changes under us should reach the daemon intact so the
/// daemon can decide what it can still read, rather than being dropped by a
/// strict parse in a process that cannot report anything.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HookLine {
    pub agent: Agent,
    /// The agent's own event name, verbatim, in its own spelling.
    pub event: String,
    pub payload: serde_json::Value,
}

/// What a gating hook is told to do.
///
/// `None` means the daemon declined to decide, which is the ordinary answer
/// and the safe one: the TUI then asks the person sitting in front of it,
/// exactly as it would with no hook installed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HookVerdict {
    #[serde(default)]
    pub decision: Option<Decision>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "behavior", rename_all = "snake_case")]
pub enum Decision {
    Allow,
    /// The message reaches the pane verbatim — measured, not assumed — so it
    /// names who decided rather than saying "hook".
    Deny { message: String },
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

    #[test]
    fn a_frame_survives_a_round_trip() {
        let line = HookLine {
            agent: Agent::Claude,
            event: "MessageDisplay".to_string(),
            payload: serde_json::json!({ "session_id": "abc" }),
        };
        let encoded = encode_line(&line).expect("encodes");
        assert!(encoded.ends_with('\n'), "a frame is one line");
        assert_eq!(decode_line::<HookLine>(encoded.trim()).expect("decodes"), line);
    }

    #[test]
    fn a_verdict_with_no_decision_is_the_default() {
        let verdict: HookVerdict = decode_line("{}").expect("an empty object is a verdict");
        assert_eq!(verdict.decision, None, "no decision means defer to the human");
    }
}
