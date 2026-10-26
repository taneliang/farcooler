//! The browser's side of the enrollment ceremony.
//!
//! Every rule is `farcooler-ceremony`'s: this module only builds the offer a
//! browser shows and says which granted runners a browser can actually dial.

use farcooler_ceremony::{CeremonyError, Manifest, Offer, Reach};
use serde::Serialize;

/// The code this browser shows: today's v2 offer, with its node key.
///
/// The channel is the build's (`farcooler_protocol::CHANNEL`), so a canary page
/// makes a code only a canary phone accepts.
pub fn offer(name: &str, account: &str, key_a: &str, node_key: &str) -> Offer {
    Offer { node_key: node_key.to_string(), ..farcooler_ceremony::offer(name, account, key_a, None) }
}

/// Take the phone's reply to `offer`, or refuse it with the ceremony's own
/// reason. `already_taken` is this browser's memory of an earlier reply.
pub fn accept(reply: &str, offer: &Offer, already_taken: bool) -> Result<Manifest, CeremonyError> {
    farcooler_ceremony::accept_manifest(reply, offer, already_taken)
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
