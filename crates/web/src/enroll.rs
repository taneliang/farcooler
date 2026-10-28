//! The browser's side of the enrollment ceremony.
//!
//! Every rule is `farcooler-ceremony`'s: this module only builds the offer a
//! browser shows and says which granted runners a browser can actually dial.

use std::time::Duration;

use farcooler_ceremony::{CeremonyError, Manifest, Offer, Reach, ReplyKey};
use serde::{Deserialize, Serialize};

/// The code this browser shows: today's v2 offer, with its node key.
///
/// The channel is the build's (`farcooler_protocol::CHANNEL`), so a canary page
/// makes a code only a canary phone accepts.
///
/// `reply_secret` is the secret half of this ceremony's [`ReplyKey`] (made by
/// [`new_reply_secret`]); its public half goes in the offer, so a phone with no
/// camera to read a reply back with can post one sealed to it.
pub fn offer(name: &str, account: &str, key_a: &str, node_key: &str, reply_secret: &str) -> Result<Offer, CeremonyError> {
    let reply_key = ReplyKey::from_secret(reply_secret)?;
    Ok(Offer {
        node_key: node_key.to_string(),
        reply_key: Some(reply_key.public().to_string()),
        ..farcooler_ceremony::offer(name, account, key_a, None)
    })
}

/// A new ceremony's reply key, as the secret the page keeps while its code is up.
pub fn new_reply_secret() -> String {
    ReplyKey::generate().secret()
}

/// One reply as the relay's read returns it (`{replies: [{sealed, posted_by}]}`).
#[derive(Debug, Deserialize)]
struct Posted {
    sealed: String,
    #[serde(default)]
    posted_by: String,
}

#[derive(Debug, Deserialize)]
struct Replies {
    replies: Vec<Posted>,
}

/// The sealed reply this browser opens out of the relay's answer: the one the
/// offer's own account posted (its owner's phone, which the relay always lets
/// in), and only when there is none, the earliest teammate's.
///
/// Anyone who saw the offer and may post can post a reply that opens, so which
/// one is tried must not be the poster's choice. A teammate who posts first
/// therefore cannot displace the owner's reply; the code still has to match.
pub fn earliest(relay_answer: &str, account: &str) -> Result<String, CeremonyError> {
    let answer: Replies = serde_json::from_str(relay_answer).map_err(|_| CeremonyError::Unopened)?;
    let mut replies = answer.replies;
    let at = replies.iter().position(|r| r.posted_by == account).unwrap_or(0);
    if at >= replies.len() {
        return Err(CeremonyError::Unopened);
    }
    Ok(replies.swap_remove(at).sealed)
}

/// A reply opened, waiting for the person to match its code against the
/// phone's. Held by the page in memory only: nothing here is kept until then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Opened {
    /// The six characters from this browser's own open of the reply.
    pub code: String,
    #[serde(flatten)]
    pub taken: Taken,
}

/// The relay's answer to a read, down to what the earliest reply grants:
/// [`earliest`], then [`open`], then [`taken`].
pub fn open_earliest(
    relay_answer: &str,
    offer: &Offer,
    reply_secret: &str,
    already_taken: bool,
    shown_at_ms: f64,
    now_ms: f64,
) -> Result<Opened, CeremonyError> {
    let sealed = earliest(relay_answer, &offer.account)?;
    let (manifest, code) = open(&sealed, offer, reply_secret, already_taken, shown_at_ms, now_ms)?;
    Ok(Opened { code, taken: taken(&manifest) })
}

/// A sealed reply, opened and judged by the ceremony's own order: already
/// taken, too late, opened with this ceremony's key, then the reply's own rules.
///
/// Returns what the reply grants and the comparison code from THIS browser's
/// own open. Nothing is kept by this: the page holds both until the person
/// says the code matches the phone's.
pub fn open(
    sealed: &str,
    offer: &Offer,
    reply_secret: &str,
    already_taken: bool,
    shown_at_ms: f64,
    now_ms: f64,
) -> Result<(Manifest, String), CeremonyError> {
    let key = ReplyKey::from_secret(reply_secret)?;
    farcooler_ceremony::take_sealed_reply(sealed, &key, offer, already_taken, held_for(shown_at_ms, now_ms))
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
    /// Why `pending`, when the phone said (ov-828): one of [`CAUSES`], or
    /// empty. A word the page has no sentence for is empty here, so the page
    /// says the general thing rather than a reply's own text.
    pub cause: String,
}

/// The reasons a phone gives for a runner it couldn't add this browser to
/// (`RunnerEntry::pending_why`).
pub const CAUSES: [&str; 7] = ["too_old", "no_passkeys", "not_allowed", "not_owner", "read_only", "not_connected", "failed"];

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
                cause: match (r.pending, r.pending_why.as_deref()) {
                    (true, Some(word)) if CAUSES.contains(&word) => word.to_string(),
                    _ => String::new(),
                },
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
        let o = offer("Chrome on Mac", "acct", KEY, NODE, &new_reply_secret()).unwrap();
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

    /// A pending runner carries the phone's reason when it is one the page
    /// has a sentence for, and nothing otherwise (ov-828).
    #[test]
    fn a_pending_runner_carries_the_phones_reason() {
        use farcooler_ceremony::RunnerEntry;
        let o = offer("Chrome on Mac", "acct", KEY, NODE, &new_reply_secret()).unwrap();
        let entry = |id: &str, pending: bool, why: Option<&str>| RunnerEntry {
            id: id.into(),
            label: id.into(),
            alias: id.into(),
            user: "you".into(),
            host_key: String::new(),
            reach: Reach::Tailcat { token: "tc".into(), client_key: String::new() },
            alternative: None,
            pending,
            pending_why: why.map(str::to_string),
        };
        let runners = vec![
            entry("cosmo", true, Some("not_allowed")),
            entry("odd", true, Some("<b>update now</b>")),
            entry("quiet", true, None),
            entry("fine", false, Some("failed")),
        ];
        let reply = farcooler_ceremony::encode_manifest(&farcooler_ceremony::manifest(&o, runners));
        let got = granted(&accept(&reply, &o, false, SHOWN, SHOWN).unwrap());
        let causes: Vec<(&str, &str, &str)> = got.iter().map(|g| (g.id.as_str(), g.why.as_str(), g.cause.as_str())).collect();
        assert_eq!(
            causes,
            vec![("cosmo", "pending", "not_allowed"), ("odd", "pending", ""), ("quiet", "pending", ""), ("fine", "", "")]
        );
    }

    /// A teammate's reply says whose runners these are, so the page can ask.
    #[test]
    fn a_teammates_reply_names_who_granted_it() {
        let o = offer("Chrome on Mac", "acct", KEY, NODE, &new_reply_secret()).unwrap();
        let own = accept(&reply_to(&o, None), &o, false, SHOWN, SHOWN).unwrap();
        assert_eq!(taken(&own).granted_by, None);
        let blank = accept(&reply_to(&o, Some("  ")), &o, false, SHOWN, SHOWN).unwrap();
        assert_eq!(taken(&blank).granted_by, None);
        let theirs = accept(&reply_to(&o, Some("ana@example.com")), &o, false, SHOWN, SHOWN).unwrap();
        assert_eq!(taken(&theirs).granted_by.as_deref(), Some("ana@example.com"));
    }

    /// One runner whose host key prints as `host_key`, reached by tunnel.
    fn runner(host_key: &str) -> farcooler_ceremony::RunnerEntry {
        farcooler_ceremony::RunnerEntry {
            id: "01990000-0000-7000-8000-00000000a001".into(),
            label: "studio".into(),
            alias: "studio".into(),
            user: "owner".into(),
            host_key: host_key.into(),
            reach: Reach::Tailcat { token: "tc-token".into(), client_key: String::new() },
            alternative: None,
            pending: false,
            pending_why: None,
        }
    }

    /// What the relay's read answers with `sealed` replies, earliest first.
    fn relay_says(sealed: &[&str]) -> String {
        let replies: Vec<_> = sealed.iter().map(|s| serde_json::json!({"sealed": s, "posted_by": "acct"})).collect();
        serde_json::json!({ "replies": replies }).to_string()
    }

    /// The same, with who posted each: `(sealed, posted_by)`.
    fn relay_says_by(posts: &[(&str, &str)]) -> String {
        let replies: Vec<_> = posts.iter().map(|(s, by)| serde_json::json!({"sealed": s, "posted_by": by})).collect();
        serde_json::json!({ "replies": replies }).to_string()
    }

    const HOST: &str = "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const FORGED: &str = "SHA256:BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

    /// Ov-771: a sealed reply opens with the offer's own key, and the code the
    /// browser shows is the code the sealing phone showed.
    #[test]
    fn a_sealed_reply_opens_to_the_code_the_phone_shows() {
        let secret = new_reply_secret();
        let o = offer("Chrome on Mac", "acct", KEY, NODE, &secret).unwrap();
        let (sealed, phone_code) = farcooler_ceremony::seal_reply(&o, &farcooler_ceremony::manifest(&o, vec![runner(HOST)])).unwrap();
        let opened = open_earliest(&relay_says(&[&sealed]), &o, &secret, false, SHOWN, SHOWN).unwrap();
        assert_eq!(opened.code, phone_code);
        assert_eq!(opened.taken.runners[0].host_key, HOST);
    }

    /// The second acceptance line: a reply whose host key was altered (a
    /// forger's own seal of it) opens, passes the ceremony's rules, and shows a
    /// DIFFERENT code from the one the person's phone shows.
    #[test]
    fn a_reply_with_an_altered_host_key_shows_a_different_code() {
        let secret = new_reply_secret();
        let o = offer("Chrome on Mac", "acct", KEY, NODE, &secret).unwrap();
        let (_, phone_code) = farcooler_ceremony::seal_reply(&o, &farcooler_ceremony::manifest(&o, vec![runner(HOST)])).unwrap();
        let (forged, _) = farcooler_ceremony::seal_reply(&o, &farcooler_ceremony::manifest(&o, vec![runner(FORGED)])).unwrap();
        let opened = open_earliest(&relay_says(&[&forged]), &o, &secret, false, SHOWN, SHOWN).unwrap();
        assert_eq!(opened.taken.runners[0].host_key, FORGED, "the forgery opens, so the code is the only defense");
        assert_ne!(opened.code, phone_code);
    }

    /// Only the earliest reply is opened. A later one, even the honest one, is
    /// not tried, so the forger's earlier reply shows ITS code and the honest
    /// reply behind it is not reached.
    #[test]
    fn only_the_earliest_reply_is_opened() {
        let secret = new_reply_secret();
        let o = offer("Chrome on Mac", "acct", KEY, NODE, &secret).unwrap();
        let (honest, honest_code) = farcooler_ceremony::seal_reply(&o, &farcooler_ceremony::manifest(&o, vec![runner(HOST)])).unwrap();
        let (forged, forged_code) = farcooler_ceremony::seal_reply(&o, &farcooler_ceremony::manifest(&o, vec![runner(FORGED)])).unwrap();
        let first = open_earliest(&relay_says(&[&forged, &honest]), &o, &secret, false, SHOWN, SHOWN).unwrap();
        assert_eq!((first.code.as_str(), first.taken.runners[0].host_key.as_str()), (forged_code.as_str(), FORGED));
        let first = open_earliest(&relay_says(&[&honest, &forged]), &o, &secret, false, SHOWN, SHOWN).unwrap();
        assert_eq!(first.code, honest_code);
        // None at all is not a reply.
        assert_eq!(open_earliest(&relay_says(&[]), &o, &secret, false, SHOWN, SHOWN), Err(CeremonyError::Unopened));
    }

    /// The owner's own reply wins over a teammate's that was posted first; with
    /// no owner reply the earliest teammate's is opened.
    #[test]
    fn the_owners_reply_wins_over_a_teammate_who_posted_first() {
        let secret = new_reply_secret();
        let o = offer("Chrome on Mac", "acct", KEY, NODE, &secret).unwrap();
        let (honest, honest_code) = farcooler_ceremony::seal_reply(&o, &farcooler_ceremony::manifest(&o, vec![runner(HOST)])).unwrap();
        let (forged, forged_code) = farcooler_ceremony::seal_reply(&o, &farcooler_ceremony::manifest(&o, vec![runner(FORGED)])).unwrap();
        let won = open_earliest(&relay_says_by(&[(&forged, "mate"), (&honest, "acct")]), &o, &secret, false, SHOWN, SHOWN).unwrap();
        assert_eq!((won.code.as_str(), won.taken.runners[0].host_key.as_str()), (honest_code.as_str(), HOST));
        let only = open_earliest(&relay_says_by(&[(&forged, "mate"), (&honest, "mate2")]), &o, &secret, false, SHOWN, SHOWN).unwrap();
        assert_eq!(only.code, forged_code);
    }

    /// A reply sealed to another key, or held past the window or after one was
    /// taken, is refused with the ceremony's own word.
    #[test]
    fn a_sealed_reply_is_refused_for_the_wrong_key_a_late_hour_or_a_second_take() {
        let secret = new_reply_secret();
        let o = offer("Chrome on Mac", "acct", KEY, NODE, &secret).unwrap();
        let (sealed, _) = farcooler_ceremony::seal_reply(&o, &farcooler_ceremony::manifest(&o, vec![runner(HOST)])).unwrap();
        let answer = relay_says(&[&sealed]);
        let other = new_reply_secret();
        assert!(matches!(open_earliest(&answer, &o, &other, false, SHOWN, SHOWN), Err(CeremonyError::Malformed(_))));
        let window = farcooler_ceremony::FRESHNESS.as_millis() as f64;
        assert_eq!(open_earliest(&answer, &o, &secret, false, SHOWN, SHOWN + window + 1.0), Err(CeremonyError::Stale));
        assert_eq!(open_earliest(&answer, &o, &secret, true, SHOWN, SHOWN), Err(CeremonyError::AlreadyTaken));
        // A reply sealed to a different offer's key doesn't open for this one.
        let elsewhere = offer("Chrome on Mac", "acct", KEY, NODE, &other).unwrap();
        let (stray, _) = farcooler_ceremony::seal_reply(&elsewhere, &farcooler_ceremony::manifest(&elsewhere, vec![runner(HOST)])).unwrap();
        assert!(open_earliest(&relay_says(&[&stray]), &o, &secret, false, SHOWN, SHOWN).is_err());
    }

    /// The offer this browser shows asks for a sealed reply, to the key it will
    /// open with: without it a phone could only fall back to the QR code.
    #[test]
    fn the_offer_names_the_key_its_reply_is_sealed_to() {
        let secret = new_reply_secret();
        let o = offer("Chrome on Mac", "acct", KEY, NODE, &secret).unwrap();
        assert_eq!(o.reply_key.as_deref(), Some(farcooler_ceremony::ReplyKey::from_secret(&secret).unwrap().public()));
        assert!(offer("Chrome on Mac", "acct", KEY, NODE, "not a key").is_err());
    }
}
