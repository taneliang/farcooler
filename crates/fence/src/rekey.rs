//! What a device signs to move its own line to a new key (ov-569, a phone's
//! "Make a New Key"), spelled once for the two sides that need it: the phone,
//! which signs it in `crates/client/src/device_keys.rs`, and the runner, which
//! rebuilds it in `crates/daemon/src/enrollment/rekey.rs` and checks the
//! signature. Here because both crates already depend on this one, and two
//! spellings of a signed statement are two answers about what was signed.
//!
//! The signer is the NEW key, which proves the device holds it. The statement
//! names the old key too, and the runner fills that in from the line the
//! connection authenticated as, never from the request, so a proof made for
//! one device's line moves nothing else. It names no runner: replaying it on
//! another runner needs a session as the same old key, which is the device
//! itself, and it would only do there what the device asked for here.

/// The SSH signature namespace, its own so a rekey proof can never be read as
/// a registration, a certificate or a grant.
pub const REKEY_NAMESPACE: &str = "farcooler-rekey-v1";

/// The statement, from the two keys' `SHA256:…` fingerprints.
///
/// `None` for anything that isn't a fingerprint as `ssh-key` prints one, so a
/// caller can't add a line to what gets signed.
pub fn rekey_statement(old_fingerprint: &str, new_fingerprint: &str) -> Option<String> {
    if !fingerprint_ok(old_fingerprint) || !fingerprint_ok(new_fingerprint) {
        return None;
    }
    Some(format!("farcooler rekey-device\nold-key {old_fingerprint}\nnew-key {new_fingerprint}\n"))
}

/// `SHA256:` and 43 characters of unpadded base64.
fn fingerprint_ok(text: &str) -> bool {
    text.strip_prefix("SHA256:").is_some_and(|b64| {
        b64.len() == 43 && b64.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const NEW: &str = "SHA256:BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

    #[test]
    fn the_statement_names_both_keys_in_order() {
        assert_eq!(
            rekey_statement(OLD, NEW).unwrap(),
            format!("farcooler rekey-device\nold-key {OLD}\nnew-key {NEW}\n")
        );
    }

    #[test]
    fn nothing_but_a_fingerprint_reaches_the_statement() {
        for bad in [
            "",
            "SHA256:",
            "MD5:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n",
            "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA ",
            "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ] {
            assert!(rekey_statement(bad, NEW).is_none(), "old {bad:?}");
            assert!(rekey_statement(OLD, bad).is_none(), "new {bad:?}");
        }
    }
}
