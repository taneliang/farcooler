//! The ceremony as the apps actually reach it: through the C boundary.
//!
//! The module's own tests prove the rules. These prove the boundary — that the
//! rules are reachable from Swift and Kotlin at all, that the JSON shapes are
//! the ones the apps parse, and above all that a refusal arrives as a stable
//! machine-readable code rather than a Rust error string. This repo already
//! renders error strings from these layers in Settings, so a `serde_json`
//! message crossing here is one `Text(error)` away from a person's screen.

use std::ffi::CString;

use farcooler_client::ffi::{
    farcooler_client_ceremony_accept, farcooler_client_ceremony_offer,
    farcooler_client_ceremony_reply, farcooler_client_ceremony_scan,
};
use serde_json::Value;

const KEY_A: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB1iLbeqDzK4CDeUC3t+ffVPDI9Gk+sBwIZqJZW1NfS5 device-a";
const KEY_B: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDMdwe233CUbxjpEHkissIUGdCxhkTsDE/Zg7f+LB6S+ device-b";

const RUNNERS: &str = r#"[{"id":"0198f0c3-0000-7000-8000-00000000000a","label":"box",
    "alias":"box","address":"box.tail-1234.ts.net","user":"you","port":22,
    "host_key":"SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4","pending":false}]"#;

/// Call one of the entry points the way an app does: size a buffer, call, and
/// read back what landed in it.
fn text(call: impl Fn(*mut u8, usize) -> usize) -> String {
    let mut buffer = vec![0u8; 8192];
    let n = call(buffer.as_mut_ptr(), buffer.len());
    assert!(n > 0 && n <= buffer.len(), "{n} bytes into an 8K buffer");
    String::from_utf8(buffer[..n].to_vec()).expect("utf-8")
}

fn json(call: impl Fn(*mut u8, usize) -> usize) -> Value {
    serde_json::from_str(&text(call)).expect("json")
}

fn an_offer(name: &str, account: &str, key_a: &str) -> String {
    let name = CString::new(name).unwrap();
    let account = CString::new(account).unwrap();
    let key = CString::new(key_a).unwrap();
    text(|out, capacity| unsafe {
        farcooler_client_ceremony_offer(
            name.as_ptr(),
            account.as_ptr(),
            key.as_ptr(),
            std::ptr::null(),
            out,
            capacity,
        )
    })
}

fn a_reply(offer_json: &str) -> String {
    let offer = CString::new(offer_json).unwrap();
    let runners = CString::new(RUNNERS).unwrap();
    text(|out, capacity| unsafe {
        farcooler_client_ceremony_reply(offer.as_ptr(), runners.as_ptr(), 0, out, capacity)
    })
}

fn accept(reply: &str, expecting: &str, already_taken: bool, held_ms: u64) -> Value {
    let reply = CString::new(reply).unwrap();
    let expecting = CString::new(expecting).unwrap();
    json(|out, capacity| unsafe {
        farcooler_client_ceremony_accept(
            reply.as_ptr(),
            expecting.as_ptr(),
            already_taken,
            held_ms,
            out,
            capacity,
        )
    })
}

/// The whole ceremony, end to end, through the boundary the apps compile
/// against.
#[test]
fn both_legs_round_trip_through_the_c_boundary() {
    let offer = an_offer("iPhone 17", "acct_1", KEY_A);
    let shown: Value = serde_json::from_str(&offer).expect("json");
    assert_eq!(shown["key_a"], KEY_A);
    assert_eq!(shown["name"], "iPhone 17");
    assert_eq!(shown["ceremony"].as_str().unwrap().len(), 32, "128 bits as hex");
    assert!(shown["key_b"].is_null(), "a phone has one key");

    // The trusted device reads it, then answers it.
    let scanned = CString::new(offer.clone()).unwrap();
    let read = json(|out, capacity| unsafe {
        farcooler_client_ceremony_scan(scanned.as_ptr(), 1_000, out, capacity)
    });
    assert_eq!(read["ceremony"], shown["ceremony"]);

    let reply = a_reply(&offer);
    let taken = accept(&reply, &offer, false, 1_000);
    assert_eq!(taken["ceremony"], shown["ceremony"]);
    assert_eq!(taken["runners"][0]["address"], "box.tail-1234.ts.net");
    assert_eq!(taken["runners"][0]["host_key"].as_str().unwrap()[..7], *"SHA256:");
    // The reply is addressed to the key that asked, by fingerprint.
    assert!(taken["target"].as_str().unwrap().starts_with("SHA256:"));
}

/// The point of the whole boundary: a refusal is a word an app maps to copy,
/// and never a Rust error string.
#[test]
fn a_refusal_crosses_as_a_code_and_not_a_rust_string() {
    let mine = an_offer("iPhone", "acct_1", KEY_A);
    let theirs = an_offer("iPad", "acct_1", KEY_B);
    let crossed = a_reply(&theirs);

    let refused = accept(&crossed, &mine, false, 1_000);
    assert_eq!(refused["error"], "wrong_ceremony");
    assert!(refused.get("runners").is_none(), "a refusal carries no manifest");

    // Nothing that reads like Rust: no Debug formatting, no serde's own words,
    // no error type names.
    let raw = refused.to_string();
    for leak in ["Err(", "CeremonyError", "expected", "line 1", "column"] {
        assert!(!raw.contains(leak), "{leak} leaked into a refusal: {raw}");
    }
}

/// Every refusal the reply leg can produce, as the apps will see it.
#[test]
fn each_refusal_has_the_code_the_apps_switch_on() {
    let mine = an_offer("iPhone", "acct_1", KEY_A);
    let reply = a_reply(&mine);

    // One reply per ceremony.
    assert_eq!(accept(&reply, &mine, true, 1_000)["error"], "already_taken");

    // Freshness is this device's clock. The reply is untouched and perfectly
    // valid; what refuses it is how long the scan has been sitting here.
    assert_eq!(accept(&reply, &mine, false, 10 * 60 * 1_000)["error"], "stale");

    // A reply addressed to another device's key.
    let mut manifest: Value = serde_json::from_str(&reply).unwrap();
    manifest["target"] = serde_json::json!("SHA256:someone-else");
    assert_eq!(accept(&manifest.to_string(), &mine, false, 0)["error"], "wrong_target");

    // Another account's reply, with everything else right.
    let mut manifest: Value = serde_json::from_str(&reply).unwrap();
    manifest["account"] = serde_json::json!("acct_2");
    assert_eq!(accept(&manifest.to_string(), &mine, false, 0)["error"], "wrong_account");

    // Another channel's, and another version's — the latter carrying its number,
    // because "update Far Cooler" is a different screen from "try again".
    let mut manifest: Value = serde_json::from_str(&reply).unwrap();
    manifest["channel"] = serde_json::json!("a-channel-we-are-not");
    let refused = accept(&manifest.to_string(), &mine, false, 0);
    assert_eq!(refused["error"], "channel");
    assert_eq!(refused["channel"], "a-channel-we-are-not");

    let mut manifest: Value = serde_json::from_str(&reply).unwrap();
    manifest["v"] = serde_json::json!(99);
    let refused = accept(&manifest.to_string(), &mine, false, 0);
    assert_eq!(refused["error"], "version");
    assert_eq!(refused["version"], 99);

    // Something that is not a Far Cooler code at all.
    assert_eq!(accept("https://example.com", &mine, false, 0)["error"], "malformed");
}

/// A stale scan is refused on the first leg too, before any runner is on
/// screen — a confirmation sheet left open all afternoon is not a scan.
#[test]
fn a_scan_held_past_the_window_is_refused_before_the_confirmation() {
    let offer = CString::new(an_offer("iPhone", "acct_1", KEY_A)).unwrap();
    let refused = json(|out, capacity| unsafe {
        farcooler_client_ceremony_scan(offer.as_ptr(), 10 * 60 * 1_000, out, capacity)
    });
    assert_eq!(refused["error"], "stale");
}

/// The cap is measured bytes against the budget the app's own encoder reported.
#[test]
fn a_manifest_over_the_budget_is_refused_rather_than_split() {
    let offer = CString::new(an_offer("iPhone", "acct_1", KEY_A)).unwrap();
    let runners = CString::new(RUNNERS).unwrap();

    // A budget of 100 bytes cannot hold a header, let alone a runner.
    let refused = json(|out, capacity| unsafe {
        farcooler_client_ceremony_reply(offer.as_ptr(), runners.as_ptr(), 100, out, capacity)
    });
    assert_eq!(refused["error"], "too_large");

    // And the same runner fits the default budget, so the refusal above is the
    // budget talking rather than the runner being unrepresentable.
    let fits = json(|out, capacity| unsafe {
        farcooler_client_ceremony_reply(offer.as_ptr(), runners.as_ptr(), 0, out, capacity)
    });
    assert_eq!(fits["runners"].as_array().unwrap().len(), 1);
}

/// The buffer contract, unchanged from `farcooler_client_generate_key`: a short
/// buffer reports the size and writes nothing, because a truncated payload
/// parses as nothing and looks like a corrupt scan.
#[test]
fn a_short_buffer_reports_the_size_and_writes_nothing() {
    let name = CString::new("iPhone").unwrap();
    let account = CString::new("acct_1").unwrap();
    let key = CString::new(KEY_A).unwrap();

    let mut tiny = [0u8; 8];
    let needed = unsafe {
        farcooler_client_ceremony_offer(
            name.as_ptr(),
            account.as_ptr(),
            key.as_ptr(),
            std::ptr::null(),
            tiny.as_mut_ptr(),
            tiny.len(),
        )
    };
    assert!(needed > tiny.len());
    assert_eq!(tiny, [0; 8], "a truncated offer is worse than none");

    // And NULL asks the size without writing at all.
    let asked = unsafe {
        farcooler_client_ceremony_offer(
            name.as_ptr(),
            account.as_ptr(),
            key.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            0,
        )
    };
    assert_eq!(asked, needed);
}

/// A Mac shows both keys, and they survive the boundary.
#[test]
fn a_mac_offers_its_shell_key_too() {
    let name = CString::new("MacBook Air").unwrap();
    let account = CString::new("acct_1").unwrap();
    let key_a = CString::new(KEY_A).unwrap();
    let key_b = CString::new(KEY_B).unwrap();
    let offer = json(|out, capacity| unsafe {
        farcooler_client_ceremony_offer(
            name.as_ptr(),
            account.as_ptr(),
            key_a.as_ptr(),
            key_b.as_ptr(),
            out,
            capacity,
        )
    });
    assert_eq!(offer["key_b"], KEY_B);
}

/// A UI bug must not take down the app, here as everywhere else at this
/// boundary.
#[test]
fn null_arguments_are_survivable() {
    let mut buffer = vec![0u8; 1024];
    unsafe {
        // No key: a refusal, not a crash and not an offer for nothing.
        let n = farcooler_client_ceremony_offer(
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            buffer.as_mut_ptr(),
            buffer.len(),
        );
        let refused: Value = serde_json::from_slice(&buffer[..n]).unwrap();
        assert_eq!(refused["error"], "malformed");

        let n = farcooler_client_ceremony_scan(
            std::ptr::null(),
            0,
            buffer.as_mut_ptr(),
            buffer.len(),
        );
        assert!(n > 0);

        let n = farcooler_client_ceremony_reply(
            std::ptr::null(),
            std::ptr::null(),
            0,
            buffer.as_mut_ptr(),
            buffer.len(),
        );
        assert!(n > 0);

        let n = farcooler_client_ceremony_accept(
            std::ptr::null(),
            std::ptr::null(),
            false,
            0,
            buffer.as_mut_ptr(),
            buffer.len(),
        );
        assert!(n > 0);
    }
}
