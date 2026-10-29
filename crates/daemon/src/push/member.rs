//! A member's own phone hears that their Claude login stopped working
//! (ov-646), or that something on the runner needs them (ov-633). Its own
//! file to keep `push.rs` small.

use sha2::{Digest, Sha256};

use super::Pairing;

/// The id of one stoppage: the same login stopping at the same moment is the
/// same event, so a retry sends nothing new. Hex of `sha256(login_id ‖ at)`.
pub fn member_event_id(login_id: &str, stopped_at: i64) -> String {
    let mut hash = Sha256::new();
    hash.update(login_id.as_bytes());
    hash.update(stopped_at.to_string().as_bytes());
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The id of one push about a Needs You item (ov-633): who, what, and when,
/// so two pushes are never one event and the relay's throttle alone decides.
pub fn needs_you_event_id(handle: &str, about: &str, at_ms: i64) -> String {
    let mut hash = Sha256::new();
    for part in [handle, about, &at_ms.to_string()] {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// What a member push is about: the relay picks its words by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberKind {
    /// Their Claude login stopped working (ov-646).
    Login,
    /// Something on the runner that is theirs needs them (ov-633), of a
    /// task notice's class (`decision`, `blocked` or `review`): the relay
    /// sends it only to the member's devices that keep that class on.
    NeedsYou(&'static str),
    /// A Mac is waiting for this person's phone to approve an admin action (ov-858).
    Approval,
}

impl MemberKind {
    /// The wire's `kind`.
    pub fn word(self) -> &'static str {
        match self {
            MemberKind::Login => "login",
            MemberKind::NeedsYou(_) => "needs_you",
            MemberKind::Approval => "approval",
        }
    }

    /// The wire's `class`, for a Needs You push.
    pub fn class(self) -> Option<&'static str> {
        match self {
            MemberKind::Login | MemberKind::Approval => None,
            MemberKind::NeedsYou(class) => Some(class),
        }
    }
}

/// What a Needs You push opens on a tap (ov-705): its task, by key, or the
/// pane that is waiting. Ids the member's own app already lists, never words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opens {
    Task(String),
    Terminal(uuid::Uuid),
}

/// What the relay is told: whom, what about, and which event. The words are
/// the relay's own, from a fixed template per kind, so no text and no email
/// crosses here.
#[derive(Debug, serde::Serialize)]
struct MemberNotice<'a> {
    account: &'a str,
    kind: &'static str,
    event: &'a str,
    /// A Needs You push's class (ov-633 review L1).
    #[serde(skip_serializing_if = "Option::is_none")]
    class: Option<&'static str>,
    /// This runner's id, so a tap opens it.
    #[serde(skip_serializing_if = "Option::is_none")]
    runner: Option<&'a str>,
    /// A Needs You push's task key, for a tap to open (ov-705).
    #[serde(skip_serializing_if = "Option::is_none")]
    task: Option<&'a str>,
    /// Or its pane.
    #[serde(skip_serializing_if = "Option::is_none")]
    terminal: Option<String>,
}

/// Ask the relay to push `account` about `kind` on this runner, and for a
/// Needs You push, what a tap `opens`.
///
/// The relay checks the account is in this runner's team and not its owner,
/// limits it per member, runner and kind (six hours for a login, two minutes
/// for Needs You), and answers 200
/// whatever it did, so a `true` here means "accepted or quietly refused".
/// For a Needs You push, `true` only when the relay also says `"sent": true`:
/// it reached one of the member's installs, now or within the window. Any
/// other answer is `false`, and the owner is buzzed instead: a member with no
/// install, or none keeping the class on, a refusal, and a relay older than
/// the kind (a 400) or than `sent` (no field).
/// A 400 is a malformed request from this side and a 401 a token the relay
/// does not hold: both are logged as refusals.
/// Failure is logged and swallowed, as `notify` does; a relay too old to
/// know the route answers 404 and the member sees the item when their app
/// next refreshes.
pub async fn notify_member(
    client: &reqwest::Client,
    pairing: &Pairing,
    account: &str,
    kind: MemberKind,
    event: &str,
    runner: Option<&str>,
    opens: Option<&Opens>,
) -> bool {
    let url = format!("{}/v1/notify/member", pairing.relay.trim_end_matches('/'));
    let (task, terminal) = match opens {
        Some(Opens::Task(key)) => (Some(key.as_str()), None),
        Some(Opens::Terminal(id)) => (None, Some(id.to_string())),
        None => (None, None),
    };
    let body = MemberNotice { account, kind: kind.word(), event, class: kind.class(), runner, task, terminal };
    match client.post(&url).bearer_auth(&pairing.token).json(&body).send().await {
        Ok(response) if response.status().is_success() => match kind {
            MemberKind::Login | MemberKind::Approval => true,
            MemberKind::NeedsYou(_) => {
                let answer: serde_json::Value = response.json().await.unwrap_or_default();
                answer.get("sent").and_then(serde_json::Value::as_bool).unwrap_or(false)
            }
        },
        Ok(response) => {
            tracing::warn!(status = %response.status(), "relay refused a member's notification");
            false
        }
        Err(e) => {
            tracing::warn!(error = %e, "could not reach the relay");
            false
        }
    }
}
