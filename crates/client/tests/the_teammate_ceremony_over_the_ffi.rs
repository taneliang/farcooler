//! The teammate door (ov-525) as the apps reach it: through the C boundary.
//!
//! The rules are proved in `ceremony/teammate_tests.rs`. These prove that a
//! Mac reaches them with the directory's own answers, that a refusal arrives
//! as a word, and that a teammate's reply is one their phone takes.

use std::ffi::CString;

use farcooler_client::ffi::{
    farcooler_client_ceremony_accept, farcooler_client_ceremony_offer,
    farcooler_client_ceremony_scan, farcooler_client_ceremony_scan_teammate,
    farcooler_client_ceremony_teammate_reply,
};
use serde_json::{Value, json};

const KEY_A: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB1iLbeqDzK4CDeUC3t+ffVPDI9Gk+sBwIZqJZW1NfS5 device-a";

const RUNNERS: &str = r#"[{"id":"0198f0c3-0000-7000-8000-00000000000a","label":"build-box",
    "alias":"build-box","user":"alice",
    "host_key":"SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4","pending":false,
    "reach":{"kind":"direct","host":"build-box.example.com","port":22}}]"#;

/// The relay's answers, as an app passes them on: `directory/me`, then
/// `directory/teams/members` for the one team Alice shares with Bob.
fn directory() -> String {
    json!({
        "me": {
            "user": { "id": "user_alice", "email": "alice@example.com" },
            "teams": [
                { "id": "t-personal", "name": "Personal", "personal": true, "role": "admin" },
                { "id": "t-platform", "name": "Platform", "personal": false, "role": "admin" },
            ],
            "devices": [],
        },
        "rosters": [{
            "team": { "id": "t-platform", "name": "Platform", "personal": false },
            "members": [
                { "user": "user_alice", "email": "alice@example.com", "role": "admin", "joinedAt": 1 },
                { "user": "user_bob", "email": "bob@example.com", "role": "member", "joinedAt": 2 },
            ],
        }],
    })
    .to_string()
}

fn text(call: impl Fn(*mut u8, usize) -> usize) -> String {
    let mut buffer = vec![0u8; 8192];
    let n = call(buffer.as_mut_ptr(), buffer.len());
    assert!(n > 0 && n <= buffer.len(), "{n} bytes into an 8K buffer");
    String::from_utf8(buffer[..n].to_vec()).expect("utf-8")
}

fn json_of(call: impl Fn(*mut u8, usize) -> usize) -> Value {
    serde_json::from_str(&text(call)).expect("json")
}

fn bobs_offer() -> String {
    let (name, account, key) =
        (CString::new("Bob’s iPhone").unwrap(), CString::new("user_bob").unwrap(), CString::new(KEY_A).unwrap());
    text(|out, capacity| unsafe {
        farcooler_client_ceremony_offer(
            name.as_ptr(),
            account.as_ptr(),
            key.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            out,
            capacity,
        )
    })
}

fn scan_teammate(encoded: &str, directory: Option<&str>, held_ms: u64) -> Value {
    scan_teammate_confirming(encoded, directory, held_ms, true)
}

fn scan_teammate_confirming(encoded: &str, directory: Option<&str>, held_ms: u64, will_confirm: bool) -> Value {
    let encoded = CString::new(encoded).unwrap();
    let account = CString::new("user_alice").unwrap();
    let directory = directory.map(|d| CString::new(d).unwrap());
    json_of(|out, capacity| unsafe {
        farcooler_client_ceremony_scan_teammate(
            encoded.as_ptr(),
            account.as_ptr(),
            directory.as_ref().map_or(std::ptr::null(), |d| d.as_ptr()),
            held_ms,
            will_confirm,
            out,
            capacity,
        )
    })
}

/// The whole teammate exchange: the same-account door says `wrong_account`,
/// the teammate door says who Bob is by the directory, and the reply it
/// builds is one Bob's phone takes, naming who granted it.
#[test]
fn a_teammate_is_scanned_granted_and_joins() {
    let code = bobs_offer();
    let account = CString::new("user_alice").unwrap();
    let encoded = CString::new(code.clone()).unwrap();
    let same = json_of(|out, capacity| unsafe {
        farcooler_client_ceremony_scan(encoded.as_ptr(), account.as_ptr(), 0, out, capacity)
    });
    assert_eq!(same["error"], "wrong_account", "the same-account door is unchanged");

    let accepted = scan_teammate(&code, Some(&directory()), 0);
    assert_eq!(accepted["teammate"], json!({ "account": "user_bob", "email": "bob@example.com", "team": "Platform" }));
    let offer = accepted["offer"].as_str().expect("the offer, as a string to answer");

    let (offer_c, runners, by) =
        (CString::new(offer).unwrap(), CString::new(RUNNERS).unwrap(), CString::new("alice@example.com").unwrap());
    let reply = text(|out, capacity| unsafe {
        farcooler_client_ceremony_teammate_reply(offer_c.as_ptr(), runners.as_ptr(), by.as_ptr(), 0, out, capacity)
    });
    assert!(!reply.contains("error"), "{reply}");

    let (reply_c, expecting) = (CString::new(reply).unwrap(), CString::new(code).unwrap());
    let taken = json_of(|out, capacity| unsafe {
        farcooler_client_ceremony_accept(reply_c.as_ptr(), expecting.as_ptr(), false, 0, out, capacity)
    });
    assert_eq!(taken["granted_by"], "alice@example.com");
    assert_eq!(taken["runners"][0]["label"], "build-box");
}

/// Every refusal crosses as a word: no directory, an unreadable one, a
/// stranger, and a code held too long.
#[test]
fn a_teammate_refusal_is_a_word() {
    let code = bobs_offer();
    assert_eq!(scan_teammate(&code, None, 0), json!({ "error": "directory_unreachable" }));
    assert_eq!(scan_teammate(&code, Some("<html>502</html>"), 0), json!({ "error": "directory_unreachable" }));
    let strangers = directory().replace("user_bob", "user_carol");
    assert_eq!(scan_teammate(&code, Some(&strangers), 0), json!({ "error": "not_a_teammate" }));
    assert_eq!(scan_teammate(&code, Some(&directory()), 10 * 60 * 1000), json!({ "error": "stale" }));
}

/// A key the roster holds after a trust reset (R-76) crosses with the team to
/// confirm it with and when its hold ends; one it lists crosses with none.
#[test]
fn a_held_key_crosses_with_its_team_and_end() {
    let code = bobs_offer();
    assert_eq!(scan_teammate(&code, Some(&directory()), 0)["held"], json!([]));

    let fingerprint = farcooler_client::ceremony::fingerprint(KEY_A).unwrap();
    let mut held: Value = serde_json::from_str(&directory()).unwrap();
    held["rosters"][0]["members"][1]["keys"] = json!([]);
    held["rosters"][0]["members"][1]["held"] = json!([{ "key": fingerprint, "until": 1_700_000_000_000_i64 }]);
    let accepted = scan_teammate(&code, Some(&held.to_string()), 0);
    assert_eq!(accepted["held"], json!([{ "team": "t-platform", "until": 1_700_000_000_000_i64 }]));
    assert_eq!(accepted["teammate"]["email"], "bob@example.com");
    // A caller that won't confirm it is refused, as a word (Low 1).
    assert_eq!(scan_teammate_confirming(&code, Some(&held.to_string()), 0, false), json!({ "error": "held_key" }));
}
