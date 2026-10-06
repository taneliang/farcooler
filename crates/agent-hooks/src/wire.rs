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
///
/// Read-only on purpose: it does not derive `Serialize`. What is written is a
/// `Reply`, which cannot carry a decision and a hold together.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct HookVerdict {
    #[serde(default)]
    pub decision: Option<Decision>,
    #[serde(default)]
    pub hold_ms: Option<u64>,
}

/// What the daemon writes to a gating hook: one of these, and never two.
///
/// `HookVerdict` reads every frame, so it has room for a decision and a hold at
/// once, and the two binaries would read such a frame differently: a hook that
/// predates holds acts on the decision, a newer one waits out the hold. So the
/// daemon never writes a `HookVerdict`. It writes a `Reply`, which is a
/// decision or a hold by construction.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Reply {
    /// `{}`: no decision. The TUI asks at the keyboard.
    NoDecision {},
    Decide { decision: Decision },
    /// Wait up to `hold_ms` more for a second line, the verdict.
    Hold { hold_ms: u64 },
}

impl Reply {
    pub fn hold(wait: Duration) -> Self {
        Reply::Hold { hold_ms: u64::try_from(wait.as_millis()).unwrap_or(u64::MAX) }
    }

    /// The verdict line for how an ask ended: `None` is no decision.
    pub fn verdict(decision: Option<Decision>) -> Self {
        match decision {
            Some(decision) => Reply::Decide { decision },
            None => Reply::NoDecision {},
        }
    }
}

/// The longest a held hook waits for its verdict, after the first contact.
///
/// The owner's number: a minute for a phone to answer. Both sides read it, the
/// daemon to give up on a hold and the hook to cap one, so neither can wait on
/// the other longer than this.
pub const LONGEST_HOLD: Duration = Duration::from_secs(60);

/// How much longer than its hold a held hook keeps reading.
///
/// The two sides start the hold's clock at different moments: the hook when it
/// reads the hold frame, the daemon only after writing it. So the daemon's
/// verdict, and its own "no decision" when the hold runs out, can land a few
/// milliseconds after the hook's hold has ended. Without this margin a phone's
/// answer written in that gap would be acked as sent into a socket nobody was
/// reading any more.
pub const HOLD_GRACE: Duration = Duration::from_secs(2);

/// The hooks that gate: an agent waits on their answer before it goes on.
///
/// Claude's `PermissionRequest` only. The installer registers exactly these as
/// gating, and the daemon's ingress treats exactly these as asks. `HookLine`
/// carries no gating flag, because this table is the whole of that fact.
pub const GATES: &[(Agent, &str)] = &[(Agent::Claude, "PermissionRequest")];

/// The hooks that fence (ov-360): an agent waits on the daemon's word before
/// it goes on, as on a gate, but there's nothing to decide. The word is
/// always "no decision", sent once the daemon has marked a tool call in
/// flight for the session, under the session's lock.
///
/// Claude's `PreToolUse` only. claude runs it, and waits for it, before it
/// draws any permission dialog, `AskUserQuestion` and `ExitPlanMode`
/// included (measured on 2.1.290: over 25, 10 and 10 dialogs, none drawn
/// before the hook returned; a hook that sleeps 300 ms delays each by
/// 300 ms). So a daemon pressing Enter in claude's box while it works holds
/// the same lock from its last check until the key has landed, and a dialog
/// can't be drawn in between. The installer registers these as
/// waiting hooks (`--gating`) like the gates.
pub const FENCES: &[(Agent, &str)] = &[(Agent::Claude, "PreToolUse")];

/// Whether `event` from `agent` is a fence (`FENCES`).
pub fn is_fence(agent: Agent, event: &str) -> bool {
    FENCES.iter().any(|&(a, e)| a == agent && e == event)
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
    fn a_hold_survives_a_round_trip() {
        let encoded = encode_line(&Reply::hold(LONGEST_HOLD)).expect("encodes");
        assert_eq!(
            decode_line::<HookVerdict>(encoded.trim()).expect("decodes"),
            HookVerdict { decision: None, hold_ms: Some(60_000) }
        );
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
        assert_eq!(encode_line(&Reply::verdict(None)).expect("encodes"), "{}\n");
        let deny = Reply::verdict(Some(Decision::Deny { message: "no".to_string() }));
        assert!(!encode_line(&deny).expect("encodes").contains("hold_ms"));
    }

    /// Every frame the daemon can write reads as exactly what it says, to a
    /// hook of either age.
    #[test]
    fn a_reply_is_a_decision_or_a_hold_never_both() {
        let deny = Decision::Deny { message: "Denied from iPhone".to_string() };
        let cases = [
            (Reply::verdict(None), "{}", None, None),
            (
                Reply::verdict(Some(deny.clone())),
                r#"{"decision":{"behavior":"deny","message":"Denied from iPhone"}}"#,
                Some(deny),
                None,
            ),
            (Reply::hold(LONGEST_HOLD), r#"{"hold_ms":60000}"#, None, Some(60_000)),
        ];
        for (reply, wire, decision, hold_ms) in cases {
            let encoded = encode_line(&reply).expect("encodes");
            assert_eq!(encoded, format!("{wire}\n"), "{reply:?} on the wire");
            let read = decode_line::<HookVerdict>(encoded.trim()).expect("a hook reads it");
            assert_eq!((read.decision, read.hold_ms), (decision, hold_ms), "{reply:?} as read");
        }
    }

    #[test]
    fn a_verdict_with_no_decision_is_the_default() {
        let verdict: HookVerdict = decode_line("{}").expect("an empty object is a verdict");
        assert_eq!(verdict.decision, None, "no decision means defer to the human");
    }
}
