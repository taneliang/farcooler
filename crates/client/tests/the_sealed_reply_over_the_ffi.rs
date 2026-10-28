//! The sealed reply as the phones reach it: through the C boundary (ov-770).
//!
//! A phone that scans a browser's offer seals the reply it built and posts
//! it. These prove that what crosses here opens, with the browser's key, to
//! the reply the phone built; that the phone's code is the one the ceremony
//! crate computes for that reply; and that every way to get it wrong is a
//! refusal word with nothing sealed.

use std::ffi::CString;

use farcooler_client::ceremony::{Manifest, Offer, ReplyKey, comparison_code, decode_offer, encode_offer, offer};
use farcooler_client::ffi::{
    farcooler_client_ceremony_code, farcooler_client_ceremony_reply, farcooler_client_ceremony_scan,
    farcooler_client_ceremony_seal, farcooler_client_ceremony_take_sealed,
};
use serde_json::Value;

const KEY_A: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB1iLbeqDzK4CDeUC3t+ffVPDI9Gk+sBwIZqJZW1NfS5 device-a";
const KEY_B: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDMdwe233CUbxjpEHkissIUGdCxhkTsDE/Zg7f+LB6S+ device-b";
const HOST_KEY: &str = "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4";

const RUNNERS: &str = r#"[{"id":"0198f0c3-0000-7000-8000-00000000000a","label":"box",
    "alias":"box","user":"you",
    "host_key":"SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4","pending":false,
    "reach":{"kind":"direct","host":"box.tail-1234.ts.net","port":22}}]"#;

/// Past the ceremony's two-minute window.
const STALE_MS: u64 = 121_000;

fn text(call: impl Fn(*mut u8, usize) -> usize) -> String {
    let mut buffer = vec![0u8; 8192];
    let n = call(buffer.as_mut_ptr(), buffer.len());
    assert!(n > 0 && n <= buffer.len(), "{n} bytes into an 8K buffer");
    String::from_utf8(buffer[..n].to_vec()).expect("utf-8")
}

fn json(call: impl Fn(*mut u8, usize) -> usize) -> Value {
    serde_json::from_str(&text(call)).expect("json")
}

/// A browser's offer, as its JSON, asking for a reply sealed to `key`.
fn browser_offer(key: &ReplyKey) -> String {
    encode_offer(&Offer { reply_key: Some(key.public().to_string()), ..offer("Chrome", "acct_1", KEY_A, None) })
}

fn scan(encoded: &str, held_ms: u64) -> Value {
    let encoded = CString::new(encoded).unwrap();
    let account = CString::new("acct_1").unwrap();
    json(|out, capacity| unsafe {
        farcooler_client_ceremony_scan(encoded.as_ptr(), account.as_ptr(), held_ms, out, capacity)
    })
}

/// Scan, then build the reply: what the phone holds when it seals.
fn scanned_and_replied(encoded: &str) -> (String, String) {
    let scanned = scan(encoded, 0).to_string();
    let offer = CString::new(scanned.as_str()).unwrap();
    let runners = CString::new(RUNNERS).unwrap();
    let reply = text(|out, capacity| unsafe {
        farcooler_client_ceremony_reply(offer.as_ptr(), runners.as_ptr(), 0, out, capacity)
    });
    (scanned, reply)
}

fn seal(offer_json: &str, manifest_json: &str, held_ms: u64) -> Value {
    let offer = CString::new(offer_json).unwrap();
    let manifest = CString::new(manifest_json).unwrap();
    json(|out, capacity| unsafe {
        farcooler_client_ceremony_seal(offer.as_ptr(), manifest.as_ptr(), held_ms, out, capacity)
    })
}

fn take(sealed: &str, secret: &str, expecting: &str) -> Value {
    let sealed = CString::new(sealed).unwrap();
    let secret = CString::new(secret).unwrap();
    let expecting = CString::new(expecting).unwrap();
    json(|out, capacity| unsafe {
        farcooler_client_ceremony_take_sealed(
            sealed.as_ptr(),
            secret.as_ptr(),
            expecting.as_ptr(),
            false,
            0,
            out,
            capacity,
        )
    })
}

fn code_of(manifest_json: &str) -> String {
    let manifest = CString::new(manifest_json).unwrap();
    text(|out, capacity| unsafe { farcooler_client_ceremony_code(manifest.as_ptr(), out, capacity) })
}

#[test]
fn a_scanned_offer_keeps_its_reply_key() {
    let key = ReplyKey::generate();
    assert_eq!(scan(&browser_offer(&key), 0)["reply_key"], key.public());
}

/// Acceptance 1 and 2, at the boundary: the phone's sealed reply opens with
/// the browser's key to the reply it built, and its code is the crate's.
#[test]
fn the_phones_sealed_reply_opens_to_its_reply_and_carries_the_crates_code() {
    let key = ReplyKey::generate();
    let shown = browser_offer(&key);
    let (scanned, reply) = scanned_and_replied(&shown);

    let sealed = seal(&scanned, &reply, 1_000);
    let body = sealed["sealed"].as_str().expect("sealed");
    assert!(!body.contains(HOST_KEY) && !body.contains("runners"));

    let opened = take(body, &key.secret(), &shown);
    assert_eq!(opened, serde_json::from_str::<Value>(&reply).unwrap());
    assert!(opened.to_string().contains(HOST_KEY));

    let manifest: Manifest = serde_json::from_str(&reply).unwrap();
    let code = sealed["code"].as_str().expect("code");
    assert_eq!(code, comparison_code(&manifest));
    assert_eq!(code_of(&opened.to_string()), code, "the browser's side shows the phone's code");
}

#[test]
fn a_reply_key_that_is_not_a_key_is_refused_at_the_scan() {
    let shown = encode_offer(&Offer { reply_key: Some("not-a-key".into()), ..offer("Chrome", "acct_1", KEY_A, None) });
    assert_eq!(scan(&shown, 0), serde_json::json!({ "error": "malformed" }));
}

#[test]
fn nothing_is_sealed_for_an_offer_held_past_the_window() {
    let key = ReplyKey::generate();
    let (scanned, reply) = scanned_and_replied(&browser_offer(&key));
    assert_eq!(seal(&scanned, &reply, STALE_MS), serde_json::json!({ "error": "stale" }));
}

#[test]
fn nothing_is_sealed_for_an_offer_that_asked_for_no_sealed_reply() {
    let (scanned, reply) = scanned_and_replied(&encode_offer(&offer("iPhone", "acct_1", KEY_A, None)));
    assert_eq!(seal(&scanned, &reply, 0), serde_json::json!({ "error": "malformed" }));
}

/// A reply built for one offer can't be sealed to another's key.
#[test]
fn nothing_is_sealed_for_a_reply_to_another_offer() {
    let key = ReplyKey::generate();
    let (_, reply) = scanned_and_replied(&browser_offer(&key));
    let other = encode_offer(&Offer { reply_key: Some(key.public().into()), ..offer("Chrome", "acct_1", KEY_B, None) });
    let other = scan(&other, 0).to_string();
    assert_eq!(seal(&other, &reply, 0), serde_json::json!({ "error": "malformed" }));
    assert_eq!(seal(&other, "not json", 0), serde_json::json!({ "error": "malformed" }));
}

#[test]
fn another_key_does_not_open_it() {
    let key = ReplyKey::generate();
    let shown = browser_offer(&key);
    let (scanned, reply) = scanned_and_replied(&shown);
    let sealed = seal(&scanned, &reply, 0);
    let other = ReplyKey::generate();
    let refused = take(sealed["sealed"].as_str().unwrap(), &other.secret(), &shown);
    assert!(refused.get("error").is_some(), "{refused}");
    assert!(decode_offer(&shown).is_ok());
}

#[test]
fn the_code_of_something_that_is_not_a_reply_is_nothing() {
    let manifest = CString::new("{}").unwrap();
    let mut buffer = [0u8; 16];
    let n = unsafe { farcooler_client_ceremony_code(manifest.as_ptr(), buffer.as_mut_ptr(), buffer.len()) };
    assert_eq!(n, 0);
}
