//! A member's own phone hears that their Claude login stopped working
//! (ov-646). Its own file to keep `push.rs` small.

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

/// What the relay is told: whom, and which stoppage. The words are the
/// relay's own, from a fixed template, so no text and no email crosses here.
#[derive(Debug, serde::Serialize)]
struct MemberNotice<'a> {
    account: &'a str,
    kind: &'static str,
    event: &'a str,
    /// This runner's id, so a tap opens it.
    #[serde(skip_serializing_if = "Option::is_none")]
    runner: Option<&'a str>,
}

/// Ask the relay to push `account` that their login on this runner stopped.
///
/// The relay checks the account is in this runner's team and not its owner,
/// limits it to one per member per runner every six hours, and answers 202
/// whatever it did, so a `true` here means "accepted or quietly refused".
/// A 400 is a malformed request from this side and a 401 a token the relay
/// does not hold: both are logged as refusals.
/// Failure is logged and swallowed, as `notify` does; a relay too old to
/// know the route answers 404 and the member sees the item when their app
/// next refreshes.
pub async fn notify_member(client: &reqwest::Client, pairing: &Pairing, account: &str, event: &str, runner: Option<&str>) -> bool {
    let url = format!("{}/v1/notify/member", pairing.relay.trim_end_matches('/'));
    let body = MemberNotice { account, kind: "login", event, runner };
    match client.post(&url).bearer_auth(&pairing.token).json(&body).send().await {
        Ok(response) if response.status().is_success() => true,
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
