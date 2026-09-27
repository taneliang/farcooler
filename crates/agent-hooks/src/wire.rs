//! The two frames that cross the ingress socket.

use std::time::Duration;

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
///
/// A daemon's first line to a gating hook is one of three things: `{}` (no
/// decision), a decision, or a hold, `{"hold_ms":60000}`. A hold is the daemon
/// saying "a phone has been offered this ask; wait for me", and it is the only
/// thing that ever widens the hook's wait. The verdict then follows as a second
/// line, a plain `HookVerdict` with no hold of its own.
///
/// A hook that predates holds reads one as no decision and defers to the
/// keyboard at once, because nothing here denies unknown fields. So a newer
/// daemon and an older hook never approve anything they should not.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HookVerdict {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<Decision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_ms: Option<u64>,
}

/// The longest a held hook waits for its verdict, after the first contact.
///
/// The owner's number: a minute for a phone to answer. Both sides read it, the
/// daemon to give up on a hold and the hook to cap one, so neither can wait on
/// the other longer than this.
pub const LONGEST_HOLD: Duration = Duration::from_secs(60);

/// The hooks that gate: an agent waits on their answer before it goes on.
///
/// Claude's `PermissionRequest` only. The installer registers exactly these as
/// gating, and the daemon's ingress treats exactly these as asks. `HookLine`
/// carries no gating flag, because this table is the whole of that fact.
pub const GATES: &[(Agent, &str)] = &[(Agent::Claude, "PermissionRequest")];

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
    fn a_hold_survives_a_round_trip() {
        let hold = HookVerdict { decision: None, hold_ms: Some(60_000) };
        let encoded = encode_line(&hold).expect("encodes");
        assert_eq!(decode_line::<HookVerdict>(encoded.trim()).expect("decodes"), hold);
    }

    #[test]
    fn a_hold_is_not_a_decision_to_a_hook_that_predates_it() {
        // `HookVerdict` as it was before holds, which an older `farcooler`
        // binary still decodes with.
        #[derive(Deserialize)]
        struct Before {
            #[serde(default)]
            decision: Option<Decision>,
        }
        let before: Before = decode_line(r#"{"hold_ms":60000}"#).expect("an old hook reads it");
        assert_eq!(before.decision, None, "a hold must never read as a decision");
    }

    #[test]
    fn a_verdict_with_no_hold_writes_no_hold_key() {
        // An older hook tolerates an unknown key, but `{}` is the frame the
        // daemon has always meant by "no decision", and it stays that.
        assert_eq!(encode_line(&HookVerdict::default()).expect("encodes"), "{}\n");
        let deny = HookVerdict {
            decision: Some(Decision::Deny { message: "no".to_string() }),
            hold_ms: None,
        };
        assert!(!encode_line(&deny).expect("encodes").contains("hold_ms"));
    }

    #[test]
    fn a_verdict_with_no_decision_is_the_default() {
        let verdict: HookVerdict = decode_line("{}").expect("an empty object is a verdict");
        assert_eq!(verdict.decision, None, "no decision means defer to the human");
    }
}
