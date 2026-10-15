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

/// Vouch, as this device (`voucher`, its Key A's OpenSSH private text), for
/// another of `account`'s devices: the one whose Key A (`key_a`, an
/// `ssh-ed25519` public key line) this device just scanned in person, in the
/// same-account ceremony (ov-550).
///
/// Key A only, because Key A is all a device registers (`registration`), and
/// the relay checks a certificate against the keys it holds for that device.
/// `key_a` must be the key in the scanned code, never one the relay named: a
/// directory that could choose what a device certifies could add "your new
/// laptop" itself.
///
/// Writes `{"certificate": "-----BEGIN SSH SIGNATURE…"}`, what
/// `directory/devices/certify` takes beside `keyA`, or `{"error": "<word>"}`:
/// `key` (an unreadable voucher, a scanned key that isn't Ed25519, or a device
/// certifying itself), `account` or `signing`. Never the private key. Same
/// buffer contract as `farcooler_client_device_registration`; 2048 is ample.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_client_device_certificate(
    voucher: *const c_char,
    account: *const c_char,
    key_a: *const c_char,
    out: *mut u8,
    capacity: usize,
) -> usize {
    guarded(0, || {
        let voucher = unsafe { read_str(voucher) }.unwrap_or_default();
        let account = unsafe { read_str(account) }.unwrap_or_default();
        let key_a = unsafe { read_str(key_a) }.unwrap_or_default();
        let answer = match crate::device_keys::certificate_for_scanned(&voucher, &account, &key_a) {
            Ok(certificate) => json!({ "certificate": certificate }),
            Err(error) => json!({ "error": error.word() }),
        };
        spill(&answer.to_string(), out, capacity)
    })
}
