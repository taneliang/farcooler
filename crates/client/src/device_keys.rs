//! This device's keys, registered with the team directory (ov-549, ov-480 §3).
//!
//! On sign-in each device tells the directory its Key A, with proof that it
//! holds the private half: an SSH signature (`ssh-keygen -Y sign`'s format) by
//! Key A over a statement naming the account, a challenge the relay chose, and
//! the key. The relay rebuilds the same statement
//! (`services/relay/src/directory/devices.ts`, `registerStatement`) and spends
//! the challenge, so a proof registers once. Both sides pin one fixture,
//! `services/relay/test/fixtures/device-proof.json`, so neither can change the
//! format alone.
//!
//! ## Who may be lying
//!
//! - **The relay.** It chooses the challenge, and a relay that could put any
//!   text into what this device signs could try to make a register statement
//!   say more than this device means (a second `key-a` line after a newline).
//!   So the statement is built here, from this device's own account id and
//!   its own public key, and the challenge is taken only as exactly 64
//!   lowercase hex characters. Each namespace is a constant here, never an
//!   argument, so nothing the relay sends can make this sign for another
//!   purpose.
//! - **Whoever reads the answer.** Only the public key and the signature leave
//!   this module. The private key is read, used and dropped.
//!
//! A certificate (one of a person's devices vouching for another, `certificate`
//! below) is the opposite case: it is checked later by runners, offline, so it
//! names no challenge, and the keys it covers must come from the code this
//! device scanned in person, never from the relay. A directory that could pick
//! the keys a device certifies could add "Bob's new laptop" itself, which is
//! the whole reason certificates exist (ov-480 §3).

use russh::keys::ssh_key::{self, HashAlg, LineEnding, PrivateKey, PublicKey};

/// The namespace of a device's proof that it holds a key: the relay's
/// `REGISTER_NAMESPACE`.
pub const REGISTER_NAMESPACE: &str = "farcooler-device-v1";

/// The namespace of a certificate one device signs for another: the relay's
/// `CERTIFY_NAMESPACE`.
pub const CERTIFY_NAMESPACE: &str = "farcooler-certify-v1";

/// Why nothing was signed, as one stable word for the apps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DeviceKeyError {
    /// The private key isn't an unencrypted Ed25519 OpenSSH key, or a public
    /// key to certify isn't a key the fence enrolls as Key A (Ed25519, or a
    /// browser's passkey for the app's host) or, for Key B, Ed25519.
    #[error("key")]
    Key,
    /// The account id is empty or has characters an account id never has.
    #[error("account")]
    Account,
    /// The challenge isn't 64 lowercase hex characters.
    #[error("challenge")]
    Challenge,
    /// A node key that isn't `nodekey:` and 64 lowercase hex characters.
    #[error("node")]
    Node,
    /// The signer itself failed.
    #[error("signing")]
    Signing,
}

impl DeviceKeyError {
    pub fn word(self) -> &'static str {
        match self {
            DeviceKeyError::Key => "key",
            DeviceKeyError::Account => "account",
            DeviceKeyError::Challenge => "challenge",
            DeviceKeyError::Node => "node",
            DeviceKeyError::Signing => "signing",
        }
    }
}

/// What `devices/register` takes beside the label and the challenge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration {
    /// `ssh-ed25519 AAAA…`, with no comment: a Mac's names its host.
    pub key_a: String,
    /// The armored signature over [`register_statement`].
    pub proof_a: String,
}

/// The relay's `registerStatement`, byte for byte.
pub fn register_statement(
    account: &str,
    challenge: &str,
    key_a: &str,
    key_b: Option<&str>,
    node: Option<&str>,
) -> String {
    format!(
        "farcooler register-device\naccount {account}\nchallenge {challenge}\nkey-a {key_a}\nkey-b {}\nnode {}\n",
        key_b.unwrap_or("none"),
        node.unwrap_or("none"),
    )
}

/// The relay's `certifyStatement`, byte for byte.
pub fn certify_statement(account: &str, key_a: &str, key_b: Option<&str>, node: Option<&str>) -> String {
    format!(
        "farcooler certify-device\naccount {account}\nkey-a {key_a}\nkey-b {}\nnode {}\n",
        key_b.unwrap_or("none"),
        node.unwrap_or("none"),
    )
}

/// Prove this device holds `private_key`, for `account`, over `challenge`.
///
/// Key A only: no Key B and no node key. The relay holds a device to the keys
/// it first registered with, so a Mac that later sends its Key B too is a new
/// device to it (`keys changed`); adding one is a later change to the relay.
pub fn registration(private_key: &str, account: &str, challenge: &str) -> Result<Registration, DeviceKeyError> {
    if !account_ok(account) {
        return Err(DeviceKeyError::Account);
    }
    if !challenge_ok(challenge) {
        return Err(DeviceKeyError::Challenge);
    }
    let key = ed25519_private(private_key)?;
    let public = bare(key.public_key())?;
    let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
    let statement = register_statement(account, challenge, &fingerprint, None, None);
    Ok(Registration { key_a: public, proof_a: sign(&key, REGISTER_NAMESPACE, &statement)? })
}

/// Vouch, as `voucher` (one of `account`'s trusted or certified devices'
/// Key A), for another of that person's devices: its Key A, its Key B when it
/// has one, and its node key when it has one.
///
/// The keys must be the ones this device scanned in person from the other
/// device's code: never ones the relay supplied (see the module notes).
pub fn certificate(
    voucher: &str,
    account: &str,
    key_a: &str,
    key_b: Option<&str>,
    node: Option<&str>,
) -> Result<String, DeviceKeyError> {
    if !account_ok(account) {
        return Err(DeviceKeyError::Account);
    }
    if let Some(node) = node
        && !node_ok(node)
    {
        return Err(DeviceKeyError::Node);
    }
    let key = ed25519_private(voucher)?;
    let key_a = key_a_fingerprint(key_a)?;
    let key_b = key_b.map(ed25519_fingerprint).transpose()?;
    if key_b.as_deref() == Some(key_a.as_str()) {
        return Err(DeviceKeyError::Key);
    }
    let statement = certify_statement(account, &key_a, key_b.as_deref(), node);
    sign(&key, CERTIFY_NAMESPACE, &statement)
}

/// What the same-account ceremony signs (ov-550): a certificate for the Key A
/// this device scanned in person, and nothing else, since Key A is all a
/// device registers ([`registration`]) and the relay checks the certificate
/// against what it holds.
///
/// A device certifying its own key is refused (`Key`): it is either pending,
/// and vouches for nothing, or already vouched for, and needs nothing. Either
/// way a code that carries this device's own key is not another device.
pub fn certificate_for_scanned(voucher: &str, account: &str, key_a: &str) -> Result<String, DeviceKeyError> {
    let own = ed25519_private(voucher)?;
    let scanned = PublicKey::from_openssh(key_a.trim()).map_err(|_| DeviceKeyError::Key)?;
    if scanned.key_data() == own.public_key().key_data() {
        return Err(DeviceKeyError::Key);
    }
    certificate(voucher, account, key_a, None, None)
}

/// What `client.rekey` takes (ov-569): the new key, the client id its line
/// will carry, and the new key's proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RekeyProof {
    /// `ssh-ed25519 AAAA…`, with no comment.
    pub public_key: String,
    /// Derived from the new key (`ceremony::client_id`). The runner ignores
    /// it and keeps the line's own id, which is who the device is there.
    pub client_id: String,
    /// The armored signature over `farcooler_fence::rekey_statement`.
    pub proof: String,
}

/// Prove, with the NEW key (`new_private`), that this device moves its line
/// from `old_public` (its old Key A, `ssh-ed25519 …`) to the new key, for a
/// phone's "Make a New Key" (ov-569).
///
/// The statement names both keys by fingerprint and nothing a runner sent, so
/// a runner can't make this sign anything else. The runner rebuilds it from
/// the line the connection authenticated as, so it moves only this device's
/// line. The same key twice is refused (`Key`): that moves nothing.
pub fn rekey_proof(new_private: &str, old_public: &str) -> Result<RekeyProof, DeviceKeyError> {
    let key = ed25519_private(new_private)?;
    let old = ed25519_fingerprint(old_public)?;
    let new = key.public_key().fingerprint(HashAlg::Sha256).to_string();
    if old == new {
        return Err(DeviceKeyError::Key);
    }
    let statement = farcooler_fence::rekey_statement(&old, &new).ok_or(DeviceKeyError::Key)?;
    let public_key = bare(key.public_key())?;
    let client_id = crate::ceremony::client_id(&public_key).ok_or(DeviceKeyError::Key)?;
    Ok(RekeyProof { public_key, client_id, proof: sign(&key, farcooler_fence::REKEY_NAMESPACE, &statement)? })
}

fn sign(key: &PrivateKey, namespace: &str, statement: &str) -> Result<String, DeviceKeyError> {
    key.sign(namespace, HashAlg::Sha512, statement.as_bytes())
        .and_then(|signature| signature.to_pem(LineEnding::LF))
        .map_err(|_| DeviceKeyError::Signing)
}

/// An unencrypted Ed25519 private key, or nothing: the fence admits no other
/// kind, so no other kind could be anyone's Key A.
fn ed25519_private(text: &str) -> Result<PrivateKey, DeviceKeyError> {
    let key = PrivateKey::from_openssh(text.trim()).map_err(|_| DeviceKeyError::Key)?;
    if key.is_encrypted() || key.algorithm() != ssh_key::Algorithm::Ed25519 {
        return Err(DeviceKeyError::Key);
    }
    Ok(key)
}

/// Key A to certify: Ed25519, or a browser's passkey for this channel's app host
/// (ov-765). Exactly what the fence would write a Key A line for, asked of
/// the fence, so a phone never vouches for a key no runner would enroll.
fn key_a_fingerprint(line: &str) -> Result<String, DeviceKeyError> {
    let key = PublicKey::from_openssh(line.trim()).map_err(|_| DeviceKeyError::Key)?;
    if !farcooler_fence::admits_as_key_a(&key) {
        return Err(DeviceKeyError::Key);
    }
    Ok(key.fingerprint(HashAlg::Sha256).to_string())
}

fn ed25519_fingerprint(line: &str) -> Result<String, DeviceKeyError> {
    let key = PublicKey::from_openssh(line.trim()).map_err(|_| DeviceKeyError::Key)?;
    if key.algorithm() != ssh_key::Algorithm::Ed25519 {
        return Err(DeviceKeyError::Key);
    }
    Ok(key.fingerprint(HashAlg::Sha256).to_string())
}

/// The public key as `ssh-ed25519 AAAA…`, its comment dropped.
fn bare(key: &PublicKey) -> Result<String, DeviceKeyError> {
    PublicKey::new(key.key_data().clone(), "").to_openssh().map_err(|_| DeviceKeyError::Key)
}

/// An account id as the relay's sessions carry them (`user_01H…`): nothing
/// that could end a line of the statement or start another.
fn account_ok(account: &str) -> bool {
    !account.is_empty()
        && account.len() <= 128
        && account.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

/// Exactly what the relay mints: 32 bytes as lowercase hex.
fn challenge_ok(challenge: &str) -> bool {
    challenge.len() == 64 && challenge.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The relay's `NODE_KEY`.
fn node_ok(node: &str) -> bool {
    node.strip_prefix("nodekey:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

#[cfg(test)]
#[path = "device_keys_tests.rs"]
mod tests;
