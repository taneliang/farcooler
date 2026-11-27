//! Joining with a browser and a terminal (ov-1490): the code a browser shows
//! in the one command a person runs on a runner, and what an offer must be
//! before a runner or another browser approves it.
//!
//! **The browser's offer travels in the command, never through the relay.**
//! The person carries it from the browser to the terminal themselves, so
//! nobody between them can put their own key in its place. The relay gives
//! only the ticket, which lets that one command post its sealed reply and,
//! once the person approves it in the browser, add the runner to the account.
//! Neither the ticket nor the code grants access to anything.

use base64ct::{Base64UrlUnpadded, Encoding};
use serde::{Deserialize, Serialize};

use super::{CeremonyError, Offer, Reach, RunnerEntry, VERSION, accept_offer, client_id};

/// A browser's offer: the only kind a runner's `join` command and a browser's
/// approval act on. A passkey as Key A and no Key B (a browser never gets a
/// shell), a node key to reach the runner through the tunnel, and a key the
/// reply is sealed to.
pub fn accept_browser_offer(encoded: &str, expecting_account: &str) -> Result<Offer, CeremonyError> {
    browser(accept_offer(encoded.trim(), expecting_account)?)
}

fn browser(offer: Offer) -> Result<Offer, CeremonyError> {
    let passkey = offer.key_a.starts_with("sk-ecdsa-sha2-nistp256@openssh.com ");
    if !passkey || offer.key_b.is_some() || offer.node_key.is_empty() || offer.reply_key.is_none() || client_id(&offer.key_a).is_none() {
        return Err(CeremonyError::NotABrowser);
    }
    Ok(offer)
}

/// A runner as a browser's reply grants it: reached through the tunnel at
/// `token`, and held to `host_key`.
pub fn tunneled_runner(id: &str, label: &str, user: &str, host_key: &str, token: &str) -> RunnerEntry {
    RunnerEntry {
        id: id.to_string(),
        label: label.to_string(),
        alias: label.to_string(),
        user: user.to_string(),
        host_key: host_key.to_string(),
        reach: Reach::Tailcat { token: token.to_string(), client_key: String::new() },
        alternative: None,
        pending: false,
        pending_why: None,
    }
}

/// What the command a person runs on a runner carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinCode {
    pub v: u8,
    /// The browser's offer, as [`super::encode_offer`] wrote it.
    pub offer: String,
    /// The relay's one-use ticket for this offer: 256 bits as hex.
    pub ticket: String,
    /// The account's team the runner is added to once approved.
    pub team: String,
}

impl JoinCode {
    pub fn new(offer: &str, ticket: &str, team: &str) -> Result<JoinCode, CeremonyError> {
        let code = JoinCode { v: VERSION, offer: offer.to_string(), ticket: ticket.to_string(), team: team.to_string() };
        code.check()?;
        Ok(code)
    }

    /// One word a shell takes unquoted: unpadded base64url.
    pub fn encode(&self) -> String {
        Base64UrlUnpadded::encode_string(&serde_json::to_vec(self).unwrap_or_default())
    }

    /// The code and the browser's offer inside it, refused unless this build
    /// may act on both.
    pub fn decode(text: &str) -> Result<(JoinCode, Offer), CeremonyError> {
        let bytes = Base64UrlUnpadded::decode_vec(text.trim()).map_err(|_| CeremonyError::Malformed("the join code is cut short or mistyped".into()))?;
        let code: JoinCode = serde_json::from_slice(&bytes).map_err(|e| CeremonyError::Malformed(e.to_string()))?;
        let offer = code.check()?;
        Ok((code, offer))
    }

    fn check(&self) -> Result<Offer, CeremonyError> {
        if self.v != VERSION {
            return Err(CeremonyError::Version(self.v));
        }
        let hex = self.ticket.len() == 64 && self.ticket.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        let team = !self.team.is_empty() && self.team.len() <= 64 && self.team.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !hex || !team {
            return Err(CeremonyError::Malformed("the join code's ticket or team is not one".into()));
        }
        let offer = super::decode_offer(&self.offer)?;
        let account = offer.account.clone();
        accept_browser_offer(&self.offer, &account)
    }
}

/// The one command a person runs on the runner, for this build's channel.
pub fn join_command(code: &JoinCode) -> String {
    format!("{} join {}", farcooler_protocol::CHANNEL.cli_binary_name(), code.encode())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ReplyKey, encode_offer};

    const PASSKEY: &str = "sk-ecdsa-sha2-nistp256@openssh.com AAAAInNrLWVjZHNhLXNoYTItbmlzdHAyNTZAb3BlbnNzaC5jb20AAAAIbmlzdHAyNTYAAABBBGsX0fLhLEJH+Lzm5WOkQPJ3A32BLeszoPShOUXYmMKWT+NC4v4af5uO5+tKfA+eFivOM1drMV7Oy7ZAaDe/UfUAAAAYYXBwLmNhbmFyeS5mYXJjb29sZXIuY29t farcooler-web";
    const ED25519: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFJhEUXBcOjzsos5aFyw6hfDrERWHei9akIudYJGG2gR phone";
    const NODE: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const TICKET: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    fn offer(key: &str) -> Offer {
        Offer {
            node_key: NODE.into(),
            reply_key: Some(ReplyKey::generate().public().to_string()),
            ..crate::offer("Chrome on Mac", "acct", key, None)
        }
    }

    #[test]
    fn a_join_code_reads_back_and_names_this_channels_command() {
        let encoded = encode_offer(&offer(PASSKEY));
        let code = JoinCode::new(&encoded, TICKET, "team-1").unwrap();
        let (read, read_offer) = JoinCode::decode(&code.encode()).unwrap();
        assert_eq!(read, code);
        assert_eq!(read_offer.name, "Chrome on Mac");
        let command = join_command(&code);
        let mut words = command.split(' ');
        assert_eq!(words.next(), Some(farcooler_protocol::CHANNEL.cli_binary_name()));
        assert_eq!(words.next(), Some("join"));
        assert!(words.next().unwrap().bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'), "a shell reads it as one word");
    }

    #[test]
    fn only_a_browsers_offer_for_this_account_is_approved() {
        let good = encode_offer(&offer(PASSKEY));
        assert!(accept_browser_offer(&good, "acct").is_ok());
        assert_eq!(accept_browser_offer(&good, "someone else"), Err(CeremonyError::WrongAccount));
        for (what, bad) in [
            ("a phone's key", offer(ED25519)),
            ("a Mac's second key", Offer { key_b: Some(ED25519.into()), ..offer(PASSKEY) }),
            ("no node key", Offer { node_key: String::new(), ..offer(PASSKEY) }),
            ("no reply key", Offer { reply_key: None, ..offer(PASSKEY) }),
        ] {
            assert_eq!(accept_browser_offer(&encode_offer(&bad), "acct"), Err(CeremonyError::NotABrowser), "{what}");
        }
    }

    #[test]
    fn a_code_with_another_ticket_shape_or_cut_short_is_refused() {
        let encoded = encode_offer(&offer(PASSKEY));
        assert!(JoinCode::new(&encoded, "abc", "team-1").is_err());
        assert!(JoinCode::new(&encoded, TICKET, "a; rm -rf /").is_err());
        let whole = JoinCode::new(&encoded, TICKET, "team-1").unwrap().encode();
        assert!(JoinCode::decode(&whole[..whole.len() - 9]).is_err());
        assert!(JoinCode::new(&encode_offer(&offer(ED25519)), TICKET, "team-1").is_err());
    }
}
