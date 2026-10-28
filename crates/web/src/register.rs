//! This browser's registration with the account's device directory (ov-771).
//!
//! The directory lists a person's devices, and a device on the list is
//! `pending` until one of their other devices certifies it. A browser's phone
//! can only certify a browser the directory already knows, so the browser
//! registers its passkey (Key A) when it makes it, before it shows a code.
//!
//! Registering proves the browser holds the key: an SSH signature
//! (`ssh-keygen -Y sign`'s SSHSIG format) by Key A over a statement naming the
//! account, a challenge the relay chose and the key. For a passkey the "signing"
//! is one `navigator.credentials.get`, whose challenge is the SSHSIG's signed
//! data, so the page asks for the data ([`signed_data`]), has the passkey sign
//! it, and hands the assertion back for the armored proof ([`proof`]).
//!
//! The statement is the relay's `registerStatement` and the phones'
//! `device_keys::register_statement`, byte for byte. That module is native-only
//! (it signs with the fence's Ed25519), so this is the one place the browser
//! spells the statement, and `tests/a_passkey_registers_with_the_directory.rs`
//! holds it to the relay's fixture. Like the phones' it names no node key and
//! no Key B: the certificate a phone signs covers the Key A only
//! (`device_keys::certificate_for_scanned`), and a relay holding a node key
//! would expect the statement to cover it.

use base64ct::{Base64, Encoding as _};
use sha2::{Digest as _, Sha512};

use crate::passkey::{self, Assertion};

/// The namespace a registration's signature is made in.
pub const NAMESPACE: &str = "farcooler-device-v1";

/// Why nothing was built, as the stable word the page maps to copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RegisterError {
    /// The account id is empty or has characters an account id never has.
    #[error("account")]
    Account,
    /// The challenge isn't 64 lowercase hex characters, the only shape the
    /// relay mints: anything else could carry a line of the relay's own.
    #[error("challenge")]
    Challenge,
    /// The key isn't an OpenSSH public key.
    #[error("key")]
    Key,
    /// The passkey's answer isn't a touch on this build's host, over this data.
    #[error("assertion")]
    Assertion,
}

impl RegisterError {
    pub fn word(self) -> &'static str {
        match self {
            RegisterError::Account => "account",
            RegisterError::Challenge => "challenge",
            RegisterError::Key => "key",
            RegisterError::Assertion => "assertion",
        }
    }
}

fn account_ok(account: &str) -> bool {
    !account.is_empty()
        && account.len() <= 128
        && account.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

fn challenge_ok(challenge: &str) -> bool {
    challenge.len() == 64 && challenge.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The statement the registration signs, from this browser's own account and
/// key, and the relay's challenge only as 64 lowercase hex characters.
pub fn statement(account: &str, challenge: &str, key_line: &str) -> Result<String, RegisterError> {
    if !account_ok(account) {
        return Err(RegisterError::Account);
    }
    if !challenge_ok(challenge) {
        return Err(RegisterError::Challenge);
    }
    let fingerprint = farcooler_ceremony::fingerprint(key_line.trim()).ok_or(RegisterError::Key)?;
    Ok(format!(
        "farcooler register-device\naccount {account}\nchallenge {challenge}\nkey-a {fingerprint}\nkey-b none\nnode none\n"
    ))
}

fn string(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// SSHSIG's signed data for `statement`: what the passkey signs a challenge of.
fn signed(statement: &str) -> Vec<u8> {
    let mut data = b"SSHSIG".to_vec();
    string(&mut data, NAMESPACE.as_bytes());
    string(&mut data, b"");
    string(&mut data, b"sha512");
    string(&mut data, &Sha512::digest(statement.as_bytes()));
    data
}

/// The bytes the passkey is asked to sign (the `challenge` of its assertion)
/// to register `key_line` for `account` against the relay's `challenge`.
pub fn signed_data(account: &str, challenge: &str, key_line: &str) -> Result<Vec<u8>, RegisterError> {
    Ok(signed(&statement(account, challenge, key_line)?))
}

/// The armored SSH signature `devices/register` takes as `proofA`, from the
/// passkey's answer to [`signed_data`].
///
/// The assertion is checked as an SSH login's is (`passkey::signature_blob`):
/// for this build's host, with a touch, a `webauthn.get` from this page over
/// exactly this data. So a proof can't be built from a prompt that signed
/// anything else.
pub fn proof(account: &str, challenge: &str, key_line: &str, assertion: &Assertion) -> Result<String, RegisterError> {
    let statement = statement(account, challenge, key_line)?;
    let signature = passkey::signature_blob(&signed(&statement), assertion).map_err(|_| RegisterError::Assertion)?;
    let key_blob = key_line.split_whitespace().nth(1).ok_or(RegisterError::Key)?;
    let key_blob = Base64::decode_vec(key_blob).map_err(|_| RegisterError::Key)?;

    let mut blob = b"SSHSIG".to_vec();
    blob.extend_from_slice(&1u32.to_be_bytes());
    string(&mut blob, &key_blob);
    string(&mut blob, NAMESPACE.as_bytes());
    string(&mut blob, b"");
    string(&mut blob, b"sha512");
    string(&mut blob, &signature);

    let body = Base64::encode_string(&blob);
    let mut armored = String::from("-----BEGIN SSH SIGNATURE-----\n");
    for line in body.as_bytes().chunks(70) {
        armored.push_str(std::str::from_utf8(line).expect("base64 is ASCII"));
        armored.push('\n');
    }
    armored.push_str("-----END SSH SIGNATURE-----\n");
    Ok(armored)
}
