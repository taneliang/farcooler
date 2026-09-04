//! The two QR payloads of the enrollment ceremony, and every rule that refuses
//! one.
//!
//! Here rather than in each app because these are the rules that decide whether
//! an enrollment is safe, and three apps agreeing about them by inspection is
//! three chances to disagree. iOS, Android and macOS get a camera and a screen;
//! they get no say in whether a scan is acceptable.
//!
//! ## Nothing in either code is a secret
//!
//! Three drafts of this design put one there and each was broken the same way.
//! Draft one proved possession of a short code with an HMAC, and the transcript
//! is fetchable, so all 2^40 codes fall on a GPU in about two minutes. Draft two
//! reached for SPAKE2 and specified none of it. Draft three put a 128-bit
//! symmetric secret in the QR — and a symmetric secret on a screen is a bearer
//! token, so whoever films it derives exactly what the scanner derives.
//!
//! So the first code carries public keys, a name, an opaque account id, a
//! channel and a random ceremony id; the reply carries addresses. A photograph
//! of the first is worth nothing on its own: enrolling those keys grants access
//! to a device the photographer does not hold. A photograph of the reply is
//! worth your fleet topology, which is why it is stated in the threat model
//! rather than waved away. `the_offer_carries_no_secret` exists so a future
//! field cannot add one quietly.
//!
//! ## The ceremony id is a correlator, not a secret
//!
//! An earlier draft removed it, reasoning that enrolling a key twice is the same
//! as enrolling it once. True of the first leg; false of the reply, which was
//! then left with nothing tying it to the request — two devices onboarded in one
//! room could scan each other's manifests, and yesterday's was as acceptable as
//! today's.
//!
//! ## What the reply leg is, precisely
//!
//! It is not authenticated. Coming off a screen identifies no signer, and an
//! earlier draft claimed otherwise and was wrong. What the echoed ceremony id,
//! account, channel and target fingerprint give is *correlation*: a reply can be
//! consumed only by the device that asked, for the ceremony it asked in, once.
//! The residual risk is someone who filmed the first code, is in the room, and
//! presents a forged reply before the real one arrives.

use std::time::Duration;

use rand::RngExt;
use russh::keys::ssh_key;
use serde::{Deserialize, Serialize};

/// The payload version both codes carry.
///
/// Checked before any other field is read, so a code from a build that knows
/// more than this one is refused rather than half-understood.
pub const VERSION: u8 = 2;

/// Which versions this build can act on.
///
/// Not `v == VERSION`. A v=1 offer comes from a phone in the field that cannot
/// be upgraded before it is enrolled, and everything in it is still true — it
/// simply names no node key, so it can be granted direct runners and nothing
/// else. A v=3 offer comes from a build that knows more than this one, and
/// half-understanding it is how a device gets enrolled with a field ignored.
pub fn accepts(v: u8) -> bool {
    v == 1 || v == VERSION
}

/// How long a scan may sit unanswered on the scanner's own clock.
///
/// Judged by the SCANNER, never by a timestamp inside the code — the displaying
/// device controls that timestamp, so a code carrying its own freshness proves
/// only that its author can write a number. What this window bounds is how long
/// a confirmation may sit open, not how old the photograph was: a screenshot of
/// the first code carries only public keys, so presenting one later gets someone
/// an enrollment of a device they still do not hold.
pub const FRESHNESS: Duration = Duration::from_secs(120);

/// The byte budget for one manifest, before QR encoding and error correction.
///
/// A stated budget is the mechanism and a runner count is the consequence.
/// Fifteen records at roughly 120 bytes is already about 1800 before versioning,
/// the account, the target key and the encoder's own overhead — so a design that
/// caps by counting runners has picked the number that is not the constraint.
///
/// Conservative on purpose: a version-40 code at medium error correction holds
/// 2331 binary bytes, and the app is still expected to ask its own encoder
/// whether what it built fits. This is the floor, not a promise.
pub const BUDGET: usize = 1_800;

/// The first code: what a new device shows.
///
/// Every field is public. `account` is an opaque relay identifier rather than a
/// credential — a WorkOS token here would be a bearer token for the account,
/// which is a worse version of draft three's mistake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    pub v: u8,
    /// The device's Far Cooler public key, in OpenSSH form.
    pub key_a: String,
    /// The device's shell public key. Macs only, and only when shell access was
    /// chosen — there is no Zed on a phone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_b: Option<String>,
    pub name: String,
    pub account: String,
    /// Which of the four deployments this is, so they cannot accept each
    /// other's ceremonies.
    pub channel: String,
    /// 128 random bits as hex. A correlator the reply must echo.
    pub ceremony: String,
    /// The device's tailcat node public key: 43 characters of unpadded base64.
    ///
    /// Empty from a v=1 device, which is not "unknown" — it is a device that
    /// cannot be granted a tunneled runner, and the join screen says so. Empty
    /// on a v=2 offer means the same thing: a device that has not joined a
    /// tunnel yet. The version says what a code can express; this field says
    /// what the device it came from can be granted.
    ///
    /// Public like every other field, and not a secret: a node public key names
    /// which peer a tunnel will admit, and holding it grants nothing. It is
    /// filled in by the app that has one, because `offer` is given the keys a
    /// device holds rather than reaching for them.
    #[serde(default)]
    pub node_key: String,
}

/// One runner in a reply: everything a device needs to reach it and nothing it
/// needs to trust it with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerEntry {
    pub id: String,
    pub label: String,
    /// The `~/.ssh/config` alias, which is per runner rather than per host —
    /// two runners on one box would both want `Host box`.
    pub alias: String,
    pub address: String,
    pub user: String,
    pub port: u16,
    /// The host key to pin, as a `SHA256:` fingerprint. It travels with the
    /// address it belongs to, which is what stops the unknown-host prompt from
    /// ever appearing and what makes an interception a refusal.
    pub host_key: String,
    /// This runner does NOT have the new device's key.
    ///
    /// A write that failed, a runner that could not be reached, and one never
    /// attempted are all the same value here, because the runner's
    /// `authorized_keys` is in the same state in all three. So the CAUSE is not
    /// recoverable from this flag and nothing downstream should guess at one.
    ///
    /// It is still listed, and it is the END STATE rather than a waiting room.
    /// Nothing retries it: the new device cannot enroll itself anywhere, and a
    /// phone may never enroll a device's key at all — granting is a
    /// Mac-and-CLI capability. Access follows only when somebody runs the
    /// ceremony again from a device that CAN reach this runner. Both join
    /// screens read this to decide what not to add and what to say about it;
    /// see `Joined` in each app's `JoinView.swift`.
    pub pending: bool,
}

/// The reply: the runners a trusted device granted, addressed to one ceremony.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub v: u8,
    pub ceremony: String,
    pub account: String,
    pub channel: String,
    /// The fingerprint of the Key A this reply answers, so it can be consumed
    /// only by the device that asked.
    pub target: String,
    pub runners: Vec<RunnerEntry>,
}

/// Why a code was refused.
///
/// The `Display` strings are for this crate's own logs. What crosses the FFI is
/// [`CeremonyError::code`] — a stable machine-readable word — because the apps
/// own the sentence a human reads and a Rust error string must never reach a
/// screen.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CeremonyError {
    #[error("this code was made by a newer Far Cooler (version {0})")]
    Version(u8),
    #[error("this code belongs to the {0} channel")]
    Channel(String),
    #[error("this code could not be read: {0}")]
    Malformed(String),
    #[error("this reply answers a different ceremony")]
    WrongCeremony,
    #[error("this reply belongs to a different account")]
    WrongAccount,
    #[error("this reply is addressed to a different device key")]
    WrongTarget,
    #[error("this scan is older than the freshness window")]
    Stale,
    #[error("this ceremony has already taken a reply")]
    AlreadyTaken,
    #[error("this manifest is larger than the byte budget")]
    TooLarge,
}

impl CeremonyError {
    /// The word the apps map to copy.
    ///
    /// Stable in the way a proto field name is stable: an app in the field
    /// matches on these, so a rename is a breaking change and not a tidy-up.
    pub fn code(&self) -> &'static str {
        match self {
            CeremonyError::Version(_) => "version",
            CeremonyError::Channel(_) => "channel",
            CeremonyError::Malformed(_) => "malformed",
            CeremonyError::WrongCeremony => "wrong_ceremony",
            CeremonyError::WrongAccount => "wrong_account",
            CeremonyError::WrongTarget => "wrong_target",
            CeremonyError::Stale => "stale",
            CeremonyError::AlreadyTaken => "already_taken",
            CeremonyError::TooLarge => "too_large",
        }
    }
}

/// Build the code a new device shows.
///
/// The channel comes from the build rather than from a caller: a device that
/// could be told which channel it is could be told wrong, and the whole point of
/// binding it is that four deployments refuse each other.
pub fn offer(name: &str, account: &str, key_a: &str, key_b: Option<&str>) -> Offer {
    Offer {
        v: VERSION,
        key_a: key_a.to_string(),
        key_b: key_b.map(str::to_string),
        name: name.to_string(),
        account: account.to_string(),
        channel: farcooler_protocol::CHANNEL.as_str().to_string(),
        ceremony: fresh_ceremony_id(),
        // Empty until a device has joined a tunnel and knows its own node key.
        // Set by the caller that holds one rather than taken as an argument
        // here, so that no app has to pass an empty string to say "none".
        node_key: String::new(),
    }
}

/// 128 random bits as hex.
///
/// Wide enough that two ceremonies in one room never collide by accident, which
/// is the only thing it has to do — it is a correlator, so guessing it buys an
/// attacker nothing they did not already have by filming the code it came from.
fn fresh_ceremony_id() -> String {
    let bits: u128 = rand::rng().random();
    format!("{bits:032x}")
}

/// The first code, as the string an app hands its QR encoder.
pub fn encode_offer(o: &Offer) -> String {
    // Compact rather than pretty: every byte here is a module in the code, and
    // `serde_json::to_string` cannot fail for a struct of strings.
    serde_json::to_string(o).unwrap_or_default()
}

/// Read a scanned first code, refusing one this build must not act on.
pub fn decode_offer(s: &str) -> Result<Offer, CeremonyError> {
    let value = parse(s)?;
    check_version(&value)?;

    let offer: Offer =
        serde_json::from_value(value).map_err(|e| CeremonyError::Malformed(e.to_string()))?;
    check_channel(&offer.channel)?;
    Ok(offer)
}

/// Read a scanned first code on behalf of the account that is asking, refusing
/// one that belongs to somebody else.
///
/// The account check is HERE rather than beside the call for the reason every
/// other rule in this module is here: it was implemented in Swift, because
/// `decode_offer` was never told who was scanning — so the rule that decides
/// whether a stranger's device may be granted your fleet existed in one app and
/// not in the other two. `accept_manifest` has always enforced the same rule on
/// the reply leg; this is that rule on the leg where it stops something, since
/// the reply leg is what a device does to itself.
///
/// Strict equality, with no exemption for an empty `expecting_account`: an
/// account that may be omitted is a check an app can switch off by passing
/// nothing, which is how it came to be missing in the first place. A device that
/// names no account is answered by one that names none either, and by nothing
/// else.
pub fn accept_offer(encoded: &str, expecting_account: &str) -> Result<Offer, CeremonyError> {
    let offer = decode_offer(encoded)?;
    if offer.account != expecting_account {
        return Err(CeremonyError::WrongAccount);
    }
    Ok(offer)
}

/// Build the reply for a scanned offer.
///
/// `target` is the fingerprint of the offer's Key A rather than the key itself:
/// it is an echo, so the shortest thing that names one key is the right size,
/// and the new device already holds the key to compare it against.
pub fn manifest(offer: &Offer, runners: Vec<RunnerEntry>) -> Manifest {
    Manifest {
        v: VERSION,
        ceremony: offer.ceremony.clone(),
        account: offer.account.clone(),
        channel: offer.channel.clone(),
        // A key that will not parse yields an empty target, which then matches
        // nothing: `accept_manifest` compares against a fingerprint it computed
        // itself, and that always starts with `SHA256:`. So an unreadable key
        // produces a reply the asking device refuses, rather than one that is
        // accidentally addressed to everybody.
        target: fingerprint(&offer.key_a).unwrap_or_default(),
        runners,
    }
}

/// The reply, as the string an app hands its QR encoder.
pub fn encode_manifest(m: &Manifest) -> String {
    serde_json::to_string(m).unwrap_or_default()
}

/// Whether a manifest fits a stated byte budget.
///
/// Measured, never assumed. The caller passes the budget its own encoder
/// reported for the error-correction level it chose; [`BUDGET`] is the
/// conservative default.
pub fn manifest_fits(m: &Manifest, budget_bytes: usize) -> bool {
    encode_manifest(m).len() <= budget_bytes
}

/// Whether a scan taken `held_for` ago may still be acted on.
///
/// Takes an elapsed duration rather than two instants because the only clock
/// that counts is the scanner's own, and a function that accepted a timestamp
/// would invite a caller to pass the one out of the code.
pub fn still_fresh(held_for: Duration) -> Result<(), CeremonyError> {
    if held_for > FRESHNESS { Err(CeremonyError::Stale) } else { Ok(()) }
}

/// Take a reply, or refuse it.
///
/// Every rule the design's security argument rests on is here, in this order:
/// a ceremony that already took a reply refuses before anything is parsed, then
/// version, then channel, then the three echoes — ceremony, account and the
/// target key. `already_taken` is the caller's, because "have I already taken
/// one" is state on the device rather than anything a code can say.
pub fn accept_manifest(
    encoded: &str,
    expecting: &Offer,
    already_taken: bool,
) -> Result<Manifest, CeremonyError> {
    // First, and before parsing: one reply per ceremony. A forged reply must not
    // be able to follow a real one, and whether this one is well-formed has no
    // bearing on that.
    if already_taken {
        return Err(CeremonyError::AlreadyTaken);
    }

    let value = parse(encoded)?;
    check_version(&value)?;
    let manifest: Manifest =
        serde_json::from_value(value).map_err(|e| CeremonyError::Malformed(e.to_string()))?;
    check_channel(&manifest.channel)?;

    if manifest.ceremony != expecting.ceremony {
        return Err(CeremonyError::WrongCeremony);
    }
    if manifest.account != expecting.account {
        return Err(CeremonyError::WrongAccount);
    }

    // The target is checked against a fingerprint computed here, from the key
    // this device is showing — not against anything either code carries.
    let mine = fingerprint(&expecting.key_a)
        .ok_or_else(|| CeremonyError::Malformed("this device's own key will not parse".into()))?;
    if manifest.target != mine {
        return Err(CeremonyError::WrongTarget);
    }

    Ok(manifest)
}

/// The SHA256 fingerprint of an OpenSSH public key, in the form a person reads
/// on screen.
///
/// `None` when the text is not a public key. Far Cooler implements no
/// cryptography of its own: this is `ssh-key`, which is what the daemon's fence
/// and the host-key check already use, so one string format is produced in one
/// place.
///
/// `pub` because the confirmation screen shows this string, and it must be the
/// same one `manifest` addresses a reply to. The app used to get it by building
/// a throwaway reply with no runners in it and reading `target` out; an entry
/// point of its own is not a new computation, it is this one, reached directly.
pub fn fingerprint(public_key: &str) -> Option<String> {
    ssh_key::PublicKey::from_openssh(public_key)
        .ok()
        .map(|k| k.fingerprint(ssh_key::HashAlg::Sha256).to_string())
}

/// The client id a device is enrolled under, derived from its own key.
///
/// Here, once, because nothing in the ceremony carries one — so without this
/// each app would invent its own format, and three apps would invent it three
/// ways. The daemon's fingerprint check keeps that from being a correctness bug,
/// but its "this device is already enrolled" arm compares client ids, and that
/// comparison only works if every app spells the same device the same way.
///
/// Derived from the key rather than random so it is stable: a device that
/// re-runs a ceremony against a runner it is already on must land on the id
/// already in that runner's fence, or it enrolls a second line naming one
/// device and the daemon can no longer say which session arrived on which key.
///
/// The fingerprint's own base64 is not safe here — it can contain `/` and `+`,
/// and this string goes inside a forced command in `authorized_keys`. Hex of the
/// leading bytes is, and `fence::render` refuses anything that would close the
/// quote regardless.
pub fn client_id(public_key: &str) -> Option<String> {
    let key = ssh_key::PublicKey::from_openssh(public_key).ok()?;
    let printed = key.fingerprint(ssh_key::HashAlg::Sha256);
    let hex: String =
        printed.as_bytes().iter().take(6).map(|b| format!("{b:02x}")).collect();
    Some(format!("farcooler-{hex}"))
}

fn parse(s: &str) -> Result<serde_json::Value, CeremonyError> {
    serde_json::from_str(s).map_err(|e| CeremonyError::Malformed(e.to_string()))
}

/// The version, before any other field is read.
///
/// Deliberately taken off the raw JSON rather than a decoded struct: a future
/// version is free to have moved every other field, so a build that insisted on
/// decoding first would report "malformed" for something it simply does not
/// speak yet — and the app would tell the user to try again instead of to
/// update.
///
/// Both legs come through here, so widening the window widens it for the reply
/// too — which is right: an older trusted device grants a v=1 manifest, and a
/// new device that refused one could never be granted anything by the fleet it
/// is joining.
fn check_version(value: &serde_json::Value) -> Result<(), CeremonyError> {
    let Some(v) = value.get("v").and_then(|v| v.as_u64()) else {
        return Err(CeremonyError::Malformed("no version".into()));
    };
    // Saturating rather than truncating, and now load-bearing rather than
    // tidy: truncation would land a `v` of 257 on 1, and 1 is a version this
    // build acts on. A number too large for a version is not a version.
    let v = v.min(u8::MAX as u64) as u8;
    if !accepts(v) {
        return Err(CeremonyError::Version(v));
    }
    Ok(())
}

fn check_channel(channel: &str) -> Result<(), CeremonyError> {
    if channel != farcooler_protocol::CHANNEL.as_str() {
        return Err(CeremonyError::Channel(channel.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real keys rather than plausible-looking strings: `target` is a
    /// fingerprint, and a fingerprint of something that will not parse is
    /// nothing at all.
    const KEY_A: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB1iLbeqDzK4CDeUC3t+ffVPDI9Gk+sBwIZqJZW1NfS5 device-a";
    const KEY_B: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDMdwe233CUbxjpEHkissIUGdCxhkTsDE/Zg7f+LB6S+ device-b";

    /// A node key as the fence spells one: 43 characters of unpadded base64.
    ///
    /// The same constant the fence and daemon suites use, on purpose — a node
    /// key that round-trips here and not through `fence::render` is a node key
    /// that never reaches a line.
    const NODE_KEY: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    /// An offer carrying a node key.
    ///
    /// `offer` itself takes none: which node key a device holds is the app's to
    /// supply once it has one, and a device that has not joined a tunnel yet
    /// shows an offer with this field empty.
    fn build_offer(
        name: &str,
        account: &str,
        key_a: &str,
        key_b: Option<&str>,
        node_key: &str,
    ) -> Offer {
        Offer { node_key: node_key.to_string(), ..offer(name, account, key_a, key_b) }
    }

    /// A payload as a build of version `v` would have written it — with no
    /// `node_key` key at all, which is the thing `#[serde(default)]` has to
    /// survive. Built as JSON rather than by serializing `Offer`, because
    /// serializing today's struct cannot reproduce yesterday's shape.
    fn v_offer_json(v: u64, name: &str, account: &str, key_a: &str) -> String {
        serde_json::json!({
            "v": v,
            "key_a": key_a,
            "name": name,
            "account": account,
            "channel": farcooler_protocol::CHANNEL.as_str(),
            "ceremony": "0123456789abcdef0123456789abcdef",
        })
        .to_string()
    }

    /// What a phone still on the shipped build shows.
    fn v1_offer_json(name: &str, account: &str, key_a: &str) -> String {
        v_offer_json(1, name, account, key_a)
    }

    fn a_runner() -> RunnerEntry {
        a_runner_named("box")
    }

    fn a_runner_named(label: &str) -> RunnerEntry {
        RunnerEntry {
            id: "0198f0c3-0000-7000-8000-00000000000a".into(),
            label: label.into(),
            alias: label.into(),
            address: format!("{label}.tail-1234.ts.net"),
            user: "you".into(),
            port: 22,
            host_key: "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4".into(),
            pending: false,
        }
    }

    // MARK: - The first code

    /// Nothing in the first code is a secret.
    ///
    /// Three drafts of this design put one there and each was broken by the
    /// same attack: a symmetric secret on a screen is a bearer token, and
    /// whoever films it holds what the scanner holds. This test exists so a
    /// future field cannot be added quietly.
    #[test]
    fn the_offer_carries_no_secret() {
        let o = offer("iPhone 17", "acct_1", KEY_A, None);
        let json = encode_offer(&o);
        assert!(json.contains("key_a"), "no key: {json}");
        for forbidden in ["secret", "token", "password", "seed", "jwt"] {
            assert!(!json.contains(forbidden), "{forbidden} in an offer: {json}");
        }
    }

    /// And neither does the reply. The reply is worth filming — it is a fleet
    /// topology — but filming it must not be the same as holding a key.
    #[test]
    fn the_manifest_carries_no_secret() {
        let o = offer("iPhone 17", "acct_1", KEY_A, None);
        let json = encode_manifest(&manifest(&o, vec![a_runner()]));
        for forbidden in ["secret", "token", "password", "seed", "jwt", "private"] {
            assert!(!json.contains(forbidden), "{forbidden} in a manifest: {json}");
        }
    }

    /// A ceremony id is a correlator, and two are never the same.
    #[test]
    fn every_offer_gets_its_own_ceremony_id() {
        let a = offer("iPhone", "acct_1", KEY_A, None);
        let b = offer("iPhone", "acct_1", KEY_A, None);
        assert_ne!(a.ceremony, b.ceremony);
        assert_eq!(a.ceremony.len(), 32, "128 bits as hex");
    }

    /// Four deployments cannot accept each other's ceremonies.
    #[test]
    fn an_offer_from_another_channel_is_refused() {
        let mut o = offer("iPhone", "acct_1", KEY_A, None);
        o.channel = "canary-but-we-are-not".into();
        let json = encode_offer(&o);
        assert!(matches!(decode_offer(&json), Err(CeremonyError::Channel(_))));
    }

    /// A future version is refused rather than half-understood.
    #[test]
    fn a_newer_version_is_refused_with_its_number() {
        let json = r#"{"v":99,"key_a":"x","name":"n","account":"a","channel":"stable","ceremony":"0"}"#;
        match decode_offer(json) {
            Err(CeremonyError::Version(99)) => {}
            other => panic!("a v99 offer was not refused by version: {other:?}"),
        }
    }

    /// A Mac offers both keys; a phone offers one.
    #[test]
    fn key_b_is_present_only_when_there_is_one() {
        assert!(offer("iPhone", "a", KEY_A, None).key_b.is_none());
        assert!(offer("MacBook", "a", KEY_A, Some(KEY_B)).key_b.is_some());
    }

    #[test]
    fn an_offer_round_trips() {
        let o = offer("MacBook Air", "acct_1", KEY_A, Some(KEY_B));
        let back = decode_offer(&encode_offer(&o)).expect("decode");
        assert_eq!(back.ceremony, o.ceremony);
        assert_eq!(back.key_a, o.key_a);
        assert_eq!(back.name, o.name);
    }

    /// Junk is a refusal with a code, not a panic and not a guess.
    #[test]
    fn something_that_is_not_a_far_cooler_code_is_refused() {
        assert!(matches!(decode_offer("https://example.com"), Err(CeremonyError::Malformed(_))));
        assert!(matches!(decode_offer("{}"), Err(CeremonyError::Malformed(_))));
    }

    /// The account is bound on the offer leg, by Rust.
    ///
    /// This is the leg where the check stops something. The reply leg's copy in
    /// `accept_manifest` protects a device from a manifest meant for another
    /// account; this one is what stops a trusted device from granting your fleet
    /// to a stranger's phone held up in front of its camera.
    #[test]
    fn an_offer_for_another_account_is_refused_when_it_is_scanned() {
        let theirs = encode_offer(&offer("someone else's iPhone", "acct_2", KEY_A, None));
        assert!(matches!(accept_offer(&theirs, "acct_1"), Err(CeremonyError::WrongAccount)));
        assert_eq!(accept_offer(&theirs, "acct_2").expect("the account it names").account, "acct_2");
    }

    /// No exemption for an account nobody named.
    ///
    /// An empty `expecting_account` that skipped the check would be a rule an
    /// app can switch off by passing nothing — which is precisely how this rule
    /// came to live in Swift instead of here.
    #[test]
    fn an_empty_account_matches_only_an_offer_that_names_none() {
        let anonymous = encode_offer(&offer("iPhone", "", KEY_A, None));
        assert!(accept_offer(&anonymous, "").is_ok());
        assert!(matches!(accept_offer(&anonymous, "acct_1"), Err(CeremonyError::WrongAccount)));

        let named = encode_offer(&offer("iPhone", "acct_1", KEY_A, None));
        assert!(matches!(accept_offer(&named, ""), Err(CeremonyError::WrongAccount)));
    }

    /// Version and channel still come first: a code this build must not act on
    /// is refused before anything about it is compared to anything.
    #[test]
    fn a_code_this_build_cannot_read_is_refused_before_the_account() {
        let mut wrong_channel = offer("iPhone", "acct_2", KEY_A, None);
        wrong_channel.channel = "a-channel-we-are-not".into();
        assert!(matches!(
            accept_offer(&encode_offer(&wrong_channel), "acct_1"),
            Err(CeremonyError::Channel(_))
        ));
        assert!(matches!(
            accept_offer(r#"{"v":99,"account":"acct_2"}"#, "acct_1"),
            Err(CeremonyError::Version(99))
        ));
    }

    // MARK: - The node key

    #[test]
    fn an_offer_carries_a_node_key() {
        let offer = build_offer("phone", "acct", KEY_A, None, NODE_KEY);
        let decoded = accept_offer(&encode_offer(&offer), "acct").unwrap();
        assert_eq!(decoded.node_key, NODE_KEY);
        assert_eq!(decoded.v, 2);
    }

    /// Rollout: a phone on the old build shows a v=1 offer with no node key. It
    /// is accepted, and it can be granted direct runners — which is exactly
    /// what it can use. Refusing it would strand every device in the field.
    #[test]
    fn a_version_one_offer_is_accepted_without_a_node_key() {
        let old = v1_offer_json("phone", "acct", KEY_A);
        let decoded = accept_offer(&old, "acct").unwrap();
        assert_eq!(decoded.v, 1);
        assert_eq!(decoded.node_key, "");
    }

    /// An empty node key is a device with no tunnel, not a malformed code.
    ///
    /// True on this version too: a phone on the new build that has not joined a
    /// tunnel yet shows v=2 with the field empty, and nothing downstream may
    /// read that as damage. The version says what the code can express; the
    /// node key says what this device can be granted.
    #[test]
    fn a_current_offer_without_a_node_key_is_not_malformed() {
        let decoded = accept_offer(&v_offer_json(VERSION as u64, "phone", "acct", KEY_A), "acct")
            .expect("a device with no tunnel yet still enrolls");
        assert_eq!(decoded.v, VERSION);
        assert_eq!(decoded.node_key, "");
    }

    #[test]
    fn an_offer_from_a_future_build_is_refused_rather_than_half_understood() {
        let future = v_offer_json(3, "phone", "acct", KEY_A);
        assert!(matches!(accept_offer(&future, "acct"), Err(CeremonyError::Version(3))));
    }

    /// The account check that already guards the reply leg guards this one too.
    /// A new field must not become a new way around it.
    #[test]
    fn another_accounts_offer_is_still_refused() {
        let offer = build_offer("phone", "theirs", KEY_A, None, NODE_KEY);
        assert!(accept_offer(&encode_offer(&offer), "ours").is_err());
    }

    /// A number too large to be a version is not version 1.
    ///
    /// The saturation in `check_version` is load-bearing rather than tidy now
    /// that more than one version is accepted: `257 as u8` truncates to 1, and
    /// 1 is a version this build acts on — so a truncating check would read a
    /// number that means nothing as a code from the field and act on it.
    #[test]
    fn a_version_that_overflows_a_byte_is_not_mistaken_for_an_old_one() {
        for v in [256u64, 257, 258, 513, u64::from(u32::MAX)] {
            let json = v_offer_json(v, "phone", "acct", KEY_A);
            assert!(
                matches!(accept_offer(&json, "acct"), Err(CeremonyError::Version(255))),
                "v{v} was not refused as a version this build cannot read"
            );
        }
    }

    /// Exactly two versions, and no others.
    ///
    /// Spelled out so that widening the window is a deliberate edit to a test
    /// rather than a side effect of bumping `VERSION`.
    #[test]
    fn this_build_acts_on_version_one_and_version_two_and_nothing_else() {
        assert!(accepts(1), "a device in the field would be stranded");
        assert!(accepts(2));
        assert_eq!(VERSION, 2);
        for v in [0u8, 3, 4, 99, u8::MAX] {
            assert!(!accepts(v), "v{v} was accepted by a build that cannot read it");
        }
    }

    // MARK: - The reply

    /// A reply for another ceremony is not this ceremony's reply.
    ///
    /// Two devices onboarded in one room would otherwise scan each other's
    /// manifests, and yesterday's would be as acceptable as today's.
    #[test]
    fn a_manifest_for_another_ceremony_is_refused() {
        let mine = offer("iPhone", "acct_1", KEY_A, None);
        let theirs = offer("iPad", "acct_1", KEY_B, None);
        let reply = encode_manifest(&manifest(&theirs, vec![a_runner()]));
        assert!(matches!(
            accept_manifest(&reply, &mine, false),
            Err(CeremonyError::WrongCeremony)
        ));
    }

    /// A reply addressed to another key is refused even with the right id.
    #[test]
    fn a_manifest_for_another_target_is_refused() {
        let mine = offer("iPhone", "acct_1", KEY_A, None);
        let mut m = manifest(&mine, vec![a_runner()]);
        m.target = "SHA256:someone-else".into();
        assert!(matches!(
            accept_manifest(&encode_manifest(&m), &mine, false),
            Err(CeremonyError::WrongTarget)
        ));
    }

    /// The target is the fingerprint of the key that asked, and a device holding
    /// a different key refuses a reply meant for its neighbour — same ceremony
    /// id or not.
    #[test]
    fn the_target_names_the_key_that_asked() {
        let mine = offer("iPhone", "acct_1", KEY_A, None);
        let m = manifest(&mine, vec![a_runner()]);
        assert_eq!(m.target, fingerprint(KEY_A).unwrap());
        assert!(m.target.starts_with("SHA256:"));

        let neighbour = Offer { key_a: KEY_B.into(), ..mine.clone() };
        assert!(matches!(
            accept_manifest(&encode_manifest(&m), &neighbour, false),
            Err(CeremonyError::WrongTarget)
        ));
    }

    /// One key, one client id, every time and on every platform.
    ///
    /// The daemon's "this device is already enrolled" arm compares client ids,
    /// so an id derived differently per app means the same device enrolls twice
    /// under two names and the daemon can no longer say which session arrived
    /// on which key. Derived from the key rather than random for the same
    /// reason: re-running a ceremony must land on the id already in the fence.
    #[test]
    fn a_key_always_derives_the_same_client_id() {
        let once = client_id(KEY_A).expect("an id for a real key");
        let again = client_id(KEY_A).expect("an id for a real key");
        assert_eq!(once, again);
        assert_ne!(once, client_id(KEY_B).expect("an id for the other key"));
    }

    /// The id cannot close the forced command's quote.
    ///
    /// It is interpolated inside `command="…"` in `authorized_keys`, so a `"`
    /// in it would end the command and let what follows become its own approved
    /// line — the same hole as a smuggled newline. `fence::render` refuses such
    /// an id anyway; this makes sure the one we generate never needs refusing.
    #[test]
    fn a_derived_client_id_is_safe_inside_a_forced_command() {
        let id = client_id(KEY_A).expect("an id");
        assert!(
            id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "an id that needs escaping: {id}"
        );
        assert!(id.starts_with("farcooler-"), "an id nobody will recognize: {id}");
    }

    /// Nonsense has no id, rather than an id nobody can trace to a key.
    #[test]
    fn text_that_is_not_a_key_has_no_client_id() {
        assert!(client_id("not a key").is_none());
        assert!(client_id("").is_none());
    }

    /// One reply per ceremony, so a forged one cannot follow a real one.
    #[test]
    fn a_second_manifest_is_refused_once_one_is_taken() {
        let mine = offer("iPhone", "acct_1", KEY_A, None);
        let reply = encode_manifest(&manifest(&mine, vec![a_runner()]));
        assert!(accept_manifest(&reply, &mine, false).is_ok());
        assert!(accept_manifest(&reply, &mine, true).is_err());
    }

    /// Account and channel are bound here too, not only in the offer.
    #[test]
    fn a_manifest_from_another_account_is_refused() {
        let mine = offer("iPhone", "acct_1", KEY_A, None);
        let mut m = manifest(&mine, vec![a_runner()]);
        m.account = "acct_2".into();
        assert!(accept_manifest(&encode_manifest(&m), &mine, false).is_err());
    }

    #[test]
    fn a_manifest_from_another_channel_is_refused() {
        let mine = offer("iPhone", "acct_1", KEY_A, None);
        let mut m = manifest(&mine, vec![a_runner()]);
        m.channel = "canary-but-we-are-not".into();
        assert!(matches!(
            accept_manifest(&encode_manifest(&m), &mine, false),
            Err(CeremonyError::Channel(_))
        ));
    }

    /// The cap is measured bytes, never an assumed runner count.
    ///
    /// Fifteen records at 120 bytes is already about 1800 before versioning,
    /// the account, the target key, encoding and error correction. A count is
    /// a consequence; the budget is the mechanism.
    #[test]
    fn the_manifest_is_capped_by_measured_bytes() {
        let mine = offer("iPhone", "acct_1", KEY_A, None);
        let many: Vec<_> =
            (0..200).map(|i| a_runner_named(&format!("runner-{i}"))).collect();
        let m = manifest(&mine, many);
        assert!(!manifest_fits(&m, 1_800), "a 200-runner manifest claimed to fit");
        let few = manifest(&mine, vec![a_runner()]);
        assert!(manifest_fits(&few, 1_800));
    }

    /// The budget is bytes of THIS manifest, so a long address costs what it
    /// costs — which a runner count would have hidden.
    #[test]
    fn the_budget_measures_this_manifest_rather_than_a_count() {
        let mine = offer("iPhone", "acct_1", KEY_A, None);
        let short = manifest(&mine, vec![a_runner(), a_runner()]);
        let mut wordy = a_runner();
        wordy.address = "a".repeat(1_600);
        let long = manifest(&mine, vec![wordy, a_runner()]);
        assert_eq!(short.runners.len(), long.runners.len());
        assert!(manifest_fits(&short, BUDGET));
        assert!(!manifest_fits(&long, BUDGET), "two runners fit only if they are small");
    }

    /// A pinned host key travels with the address it belongs to.
    #[test]
    fn every_runner_carries_the_host_key_to_pin() {
        let mine = offer("iPhone", "acct_1", KEY_A, None);
        let m = manifest(&mine, vec![a_runner()]);
        assert!(m.runners[0].host_key.starts_with("SHA256:"));
    }

    /// A runner the granting device could not write to is listed, marked
    /// pending, rather than dropped — the flag is what lets the receiving side
    /// tell it apart from a runner it can actually reach.
    #[test]
    fn a_manifest_round_trips_including_pending_runners() {
        let mine = offer("iPhone", "acct_1", KEY_A, None);
        let mut asleep = a_runner_named("build-vm");
        asleep.pending = true;
        let m = manifest(&mine, vec![a_runner(), asleep]);
        let back = accept_manifest(&encode_manifest(&m), &mine, false).expect("accept");
        assert_eq!(back.runners.len(), 2);
        assert!(back.runners[1].pending);
        assert_eq!(back.runners[1].port, 22);
    }

    // MARK: - Freshness

    /// Freshness is the scanner's clock and nothing else.
    #[test]
    fn a_scan_older_than_the_window_is_refused() {
        assert!(still_fresh(Duration::from_secs(1)).is_ok());
        assert!(still_fresh(FRESHNESS).is_ok());
        assert!(matches!(
            still_fresh(FRESHNESS + Duration::from_secs(1)),
            Err(CeremonyError::Stale)
        ));
    }

    /// A timestamp inside the code buys nothing, because nothing reads it.
    ///
    /// The displaying device controls that number, so a code that claimed to
    /// have been minted a second ago would be believed forever. Decoding
    /// ignores it, and the only clock that governs is the one passed in.
    #[test]
    fn a_timestamp_inside_the_code_cannot_buy_freshness() {
        let mut value = serde_json::to_value(offer("iPhone", "acct_1", KEY_A, None)).unwrap();
        value["issued_at"] = serde_json::json!(4_102_444_800u64);
        value["expires_at"] = serde_json::json!(4_102_444_800u64);
        let claiming = value.to_string();

        let back = decode_offer(&claiming).expect("an unknown field is not a parse failure");
        assert_eq!(back.account, "acct_1");
        // Whatever the code says about itself, the scanner's own elapsed time is
        // what refuses it.
        assert!(still_fresh(FRESHNESS + Duration::from_secs(1)).is_err());
    }

    // MARK: - Codes

    /// Every refusal has a stable word, and no two share one.
    #[test]
    fn every_refusal_has_its_own_stable_code() {
        let all = [
            CeremonyError::Version(2),
            CeremonyError::Channel("canary".into()),
            CeremonyError::Malformed("whatever".into()),
            CeremonyError::WrongCeremony,
            CeremonyError::WrongAccount,
            CeremonyError::WrongTarget,
            CeremonyError::Stale,
            CeremonyError::AlreadyTaken,
            CeremonyError::TooLarge,
        ];
        let mut codes: Vec<&str> = all.iter().map(CeremonyError::code).collect();
        codes.sort_unstable();
        let count = codes.len();
        codes.dedup();
        assert_eq!(codes.len(), count, "two refusals share a code: {codes:?}");
        for code in codes {
            assert!(
                code.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{code} is not a machine-readable word"
            );
        }
    }
}
