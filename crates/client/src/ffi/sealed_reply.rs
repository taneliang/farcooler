//! The reply, sealed for a device that can't scan it, over the C boundary
//! (ov-770).
//!
//! A browser has no camera, so when its offer names a `reply_key` the
//! granting phone seals the reply it built to that key and posts it to the
//! relay, and both screens show a six-character code for the person to
//! match. The seal and the code are `crate::ceremony`'s (`seal.rs`); an app
//! hands over the two strings it already holds and posts what comes back.
//! Neither Swift nor Kotlin computes either.
//!
//! The buffer contract and the refusal-as-a-word rule are `ffi.rs`'s.

use std::ffi::c_char;
use std::time::Duration;

use farcooler_ffi_guard::guarded;
use serde_json::json;

use super::{read_str, refusal, spill};
use crate::ceremony::{CeremonyError, Manifest, ReplyKey, comparison_code, decode_offer, encode_manifest, seal_reply, still_fresh, take_sealed_reply};

/// Seal a reply for the relay. Leg two, the trusted device's side, once the
/// reply is built.
///
/// `offer_json` is what `farcooler_client_ceremony_scan` returned and
/// `manifest_json` what `farcooler_client_ceremony_reply` returned for it.
/// `held_ms` is how long ago this device scanned, by its own clock: a phone
/// posts only for an offer it read inside the freshness window, judged here
/// and first, as the scan judges it.
///
/// Writes `{"sealed": "<base64url>", "code": "<six characters>"}`. The code
/// is over the manifest that was sealed, so it is the one the browser shows
/// for this reply. A refusal is a word, as everywhere: `stale`, or
/// `malformed` for an offer that asked for no sealed reply, a reply key that
/// is not a key, or a manifest that answers another offer. Nothing is posted
/// on a refusal; the reply's QR code is still on screen.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_client_ceremony_seal(
    offer_json: *const c_char,
    manifest_json: *const c_char,
    held_ms: u64,
    out: *mut u8,
    capacity: usize,
) -> usize {
    guarded(0, || {
        let malformed = |what: &str| refusal(&CeremonyError::Malformed(what.to_string()));
        let Some(offer_json) = (unsafe { read_str(offer_json) }) else {
            return spill(&malformed("no offer"), out, capacity);
        };
        let Some(manifest_json) = (unsafe { read_str(manifest_json) }) else {
            return spill(&malformed("no reply"), out, capacity);
        };
        let answer = still_fresh(Duration::from_millis(held_ms))
            .and_then(|()| decode_offer(&offer_json))
            .and_then(|offer| {
                let manifest: Manifest = serde_json::from_str(&manifest_json)
                    .map_err(|_| CeremonyError::Malformed("the reply is not a reply".into()))?;
                let sealed = seal_reply(&offer, &manifest)?;
                Ok(json!({ "sealed": sealed, "code": comparison_code(&manifest) }).to_string())
            });
        spill(&answer.unwrap_or_else(|e| refusal(&e)), out, capacity)
    })
}

/// The comparison code for a reply, as plain text: the six characters, or
/// nothing (0) for a string that is not a reply.
///
/// The same text contract as `farcooler_client_fingerprint`. For the device
/// that took a sealed reply, to show beside it; the phone that sealed one has
/// its code from `farcooler_client_ceremony_seal`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_client_ceremony_code(
    manifest_json: *const c_char,
    out: *mut u8,
    capacity: usize,
) -> usize {
    guarded(0, || {
        let Some(text) = (unsafe { read_str(manifest_json) }) else { return 0 };
        let Ok(manifest) = serde_json::from_str::<Manifest>(&text) else { return 0 };
        spill(&comparison_code(&manifest), out, capacity)
    })
}

/// Take a sealed reply, or refuse it. Leg two, the new device's side, for a
/// native device whose offer named a reply key.
///
/// `farcooler_client_ceremony_accept`'s twin, in `take_sealed_reply`'s order:
/// already taken, then freshness, then opening it with `secret` (the reply
/// key's secret half, as `ReplyKey::secret` writes it) against the offer this
/// device showed, then the reply's own rules. Writes the manifest, as
/// `ceremony_accept` does, or a refusal; one that would not open is
/// `{"error":"unopened"}`.
///
/// A browser takes its reply through `crates/web`. This is the same function
/// for an app, and what the phones' suites open their posted reply with.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_client_ceremony_take_sealed(
    sealed: *const c_char,
    secret: *const c_char,
    expecting_json: *const c_char,
    already_taken: bool,
    held_ms: u64,
    out: *mut u8,
    capacity: usize,
) -> usize {
    guarded(0, || {
        let malformed = |what: &str| refusal(&CeremonyError::Malformed(what.to_string()));
        let Some(sealed) = (unsafe { read_str(sealed) }) else {
            return spill(&malformed("nothing posted"), out, capacity);
        };
        let Some(secret) = (unsafe { read_str(secret) }) else {
            return spill(&malformed("no reply key"), out, capacity);
        };
        let Some(expecting_json) = (unsafe { read_str(expecting_json) }) else {
            return spill(&malformed("no offer to answer"), out, capacity);
        };
        let answer = ReplyKey::from_secret(&secret)
            .and_then(|key| {
                let expecting = decode_offer(&expecting_json)?;
                take_sealed_reply(&sealed, &key, &expecting, already_taken, Duration::from_millis(held_ms))
            })
            .map(|m| encode_manifest(&m));
        spill(&answer.unwrap_or_else(|e| refusal(&e)), out, capacity)
    })
}
