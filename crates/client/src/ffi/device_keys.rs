//! This device's Key A, registered with the team directory (ov-549), over the
//! C boundary.
//!
//! One entry point. The rules are `crate::device_keys`'s: the statement is
//! built there, from what this device knows, and the relay's challenge is held
//! to the one shape it mints.

use std::ffi::c_char;

use farcooler_ffi_guard::guarded;
use serde_json::json;

use super::{read_str, spill};

/// Prove this device holds `private_key` (Key A, the OpenSSH text the app
/// keeps) to the directory, for `account` (the signed-in account's id, as
/// this device knows it), over `challenge` (what `directory/devices/challenge`
/// answered).
///
/// Writes `{"keyA": "ssh-ed25519 AAAA…", "proofA": "-----BEGIN SSH SIGNATURE…"}`,
/// the two fields `directory/devices/register` takes beside `label` and
/// `challenge`, or `{"error": "<word>"}`: `key`, `account`, `challenge` or
/// `signing`. Never the private key, and never a sentence.
///
/// Same buffer contract as `farcooler_client_generate_key`: the byte count is
/// returned, and nothing is written if it exceeds `capacity`. 2048 bytes is
/// ample.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_client_device_registration(
    private_key: *const c_char,
    account: *const c_char,
    challenge: *const c_char,
    out: *mut u8,
    capacity: usize,
) -> usize {
    guarded(0, || {
        let private_key = unsafe { read_str(private_key) }.unwrap_or_default();
        let account = unsafe { read_str(account) }.unwrap_or_default();
        let challenge = unsafe { read_str(challenge) }.unwrap_or_default();
        let answer = match crate::device_keys::registration(&private_key, &account, &challenge) {
            Ok(made) => json!({ "keyA": made.key_a, "proofA": made.proof_a }),
            Err(error) => json!({ "error": error.word() }),
        };
        spill(&answer.to_string(), out, capacity)
    })
}
