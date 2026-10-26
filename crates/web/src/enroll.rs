//! The browser's side of the enrollment ceremony.
//!
//! Every rule is `farcooler-ceremony`'s: this module only builds the offer a
//! browser shows and says which granted runners a browser can actually dial.

use std::time::Duration;

use farcooler_ceremony::{CeremonyError, Manifest, Offer, Reach};
use serde::Serialize;

/// The code this browser shows: today's v2 offer, with its node key.
///
/// The channel is the build's (`farcooler_protocol::CHANNEL`), so a canary page
/// makes a code only a canary phone accepts.
pub fn offer(name: &str, account: &str, key_a: &str, node_key: &str) -> Offer {
    Offer { node_key: node_key.to_string(), ..farcooler_ceremony::offer(name, account, key_a, None) }
}

/// How long the browser's code has been up, by its own clock, in milliseconds.
///
/// A start in the future (a clock moved back, or a stored time that was
/// tampered with) is not a short wait: it is a time this device can't vouch
/// for, and it counts as forever.
fn held_for(shown_at_ms: f64, now_ms: f64) -> Duration {
    let held = now_ms - shown_at_ms;
    if !held.is_finite() || held < 0.0 { Duration::MAX } else { Duration::from_millis(held as u64) }
}

/// Take the phone's reply to `offer`, or refuse it with the ceremony's own
/// reason: already taken, then too late, then the reply itself, exactly as
/// every native device does (`farcooler_ceremony::take_reply`).
///
/// `shown_at_ms` is when this browser's code went up and `now_ms` is now, both
/// on this browser's clock. The window bounds how long a code on a screen can
/// be answered: the reply leg is unauthenticated, and everything a forged reply
/// must echo is printed in the code.
pub fn accept(
    reply: &str,
    offer: &Offer,
    already_taken: bool,
    shown_at_ms: f64,
    now_ms: f64,
) -> Result<Manifest, CeremonyError> {
    farcooler_ceremony::take_reply(reply, offer, already_taken, held_for(shown_at_ms, now_ms))
}

/// Whether a code shown at `shown_at_ms` is past the window, so a stored offer
/// is discarded and a new code shown rather than kept answerable for days.
pub fn offer_is_stale(shown_at_ms: f64, now_ms: f64) -> bool {
    farcooler_ceremony::still_fresh(held_for(shown_at_ms, now_ms)).is_err()
}

/// What a taken reply grants: whose runners, when they aren't this account's
/// own, and the runners.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Taken {
    /// The granting person's email on a teammate's reply. The page asks before
    /// joining, as a phone does; never joined silently.
    pub granted_by: Option<String>,
    pub runners: Vec<Granted>,
}

pub fn taken(manifest: &Manifest) -> Taken {
    Taken {
        granted_by: manifest.granted_by.as_ref().map(|g| g.trim().to_string()).filter(|g| !g.is_empty()),
        runners: granted(manifest),
    }
}

/// One granted runner, as the page keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Granted {
    pub id: String,
    pub label: String,
    pub user: String,
    /// The `SHA256:` fingerprint every connection is refused without.
    pub host_key: String,
    /// The tunnel token to dial. Empty when the runner can't be dialed from a
    /// browser, and `why` says why.
    pub token: String,
    /// Empty for a runner this browser can dial; otherwise `pending` (the
    /// phone could not write this browser's key there) or `address_only` (a
    /// browser has no raw TCP, so it reaches runners through the tunnel only).
    pub why: String,
}

/// Every runner in a reply, with whether and how this browser reaches it.
pub fn granted(manifest: &Manifest) -> Vec<Granted> {
    manifest
        .runners
        .iter()
        .map(|r| {
            let (token, why) = match (&r.reach, r.pending) {
                (_, true) => (String::new(), "pending"),
                (Reach::Tailcat { token, .. }, false) => (token.clone(), ""),
                _ => (String::new(), "address_only"),
            };
            Granted {
                id: r.id.clone(),
                label: r.label.clone(),
                user: r.user.clone(),
                host_key: r.host_key.clone(),
                token,
                why: why.to_string(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFJhEUXBcOjzsos5aFyw6hfDrERWHei9akIudYJGG2gR browser";
    const NODE: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const SHOWN: f64 = 1_000_000.0;

    fn reply_to(o: &Offer, granted_by: Option<&str>) -> String {
        let mut m = farcooler_ceremony::manifest(o, Vec::new());
        m.granted_by = granted_by.map(str::to_string);
        farcooler_ceremony::encode_manifest(&m)
    }

    /// The blocker ov-764's review found: a reply held up long after the code
    /// went up is refused, however right it is.
    #[test]
    fn a_reply_after_the_window_is_refused() {
        let o = offer("Chrome on Mac", "acct", KEY, NODE);
        let reply = reply_to(&o, None);
        let window = farcooler_ceremony::FRESHNESS.as_millis() as f64;
        assert!(accept(&reply, &o, false, SHOWN, SHOWN + window).is_ok());
        assert_eq!(accept(&reply, &o, false, SHOWN, SHOWN + window + 1.0), Err(CeremonyError::Stale));
        // A start in the future is a time this device can't vouch for.
        assert_eq!(accept(&reply, &o, false, SHOWN, SHOWN - 1.0), Err(CeremonyError::Stale));
        assert_eq!(accept(&reply, &o, false, f64::NAN, SHOWN), Err(CeremonyError::Stale));
        assert_eq!(accept(&reply, &o, true, SHOWN, SHOWN), Err(CeremonyError::AlreadyTaken));
    }

    /// A stored code past the window is discarded, not kept answerable.
    #[test]
    fn a_code_past_the_window_is_stale() {
        let window = farcooler_ceremony::FRESHNESS.as_millis() as f64;
        assert!(!offer_is_stale(SHOWN, SHOWN + window));
        assert!(offer_is_stale(SHOWN, SHOWN + window + 1.0));
        assert!(offer_is_stale(SHOWN, SHOWN - 1.0));
    }

    /// A teammate's reply says whose runners these are, so the page can ask.
    #[test]
    fn a_teammates_reply_names_who_granted_it() {
        let o = offer("Chrome on Mac", "acct", KEY, NODE);
        let own = accept(&reply_to(&o, None), &o, false, SHOWN, SHOWN).unwrap();
        assert_eq!(taken(&own).granted_by, None);
        let blank = accept(&reply_to(&o, Some("  ")), &o, false, SHOWN, SHOWN).unwrap();
        assert_eq!(taken(&blank).granted_by, None);
        let theirs = accept(&reply_to(&o, Some("ana@example.com")), &o, false, SHOWN, SHOWN).unwrap();
        assert_eq!(taken(&theirs).granted_by.as_deref(), Some("ana@example.com"));
    }
}
