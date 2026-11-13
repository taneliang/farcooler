//! Teammate browser authority (ov-1020). The runner attests that the device
//! administering it answered THIS offer; that device signs the full topology.
//! Runner keys and the poster's device fingerprints arrive independently from
//! the directory, never from the signed payload itself.

use serde::{Deserialize, Serialize};
use ssh_key::{HashAlg, LineEnding, PrivateKey, PublicKey, SshSig};

use crate::{CeremonyError, Manifest, Offer, fingerprint};

pub const DEVICE_NAMESPACE: &str = "farcooler-teammate-reply-v1";
pub const RUNNER_NAMESPACE: &str = "farcooler-ceremony-authority-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Authority {
    pub runner_key: String,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proof {
    pub device_key: String,
    pub authorities: Vec<Authority>,
    pub signature: String,
}

/// The relay's current directory view for this poster. Empty or missing
/// authority fails closed, including while a device waits out a trust reset.
#[derive(Debug, Default, Deserialize)]
pub struct Trust {
    pub devices: Vec<String>,
    pub runners: Vec<String>,
}

/// JSON's array framing is unambiguous even when a field contains newlines.
/// The HPKE recipient is bound too: a second offer cannot reuse an attestation.
pub fn authority_statement(offer: &Offer, device: &str) -> Vec<u8> {
    serde_json::to_vec(&(&offer.ceremony, &offer.account, &offer.channel,
        fingerprint(&offer.key_a), &offer.reply_key, device)).unwrap_or_default()
}

fn payload(manifest: &Manifest, authorities: &[Authority]) -> Vec<u8> {
    let mut unsigned = manifest.clone();
    unsigned.proof = None;
    serde_json::to_vec(&(unsigned, authorities)).unwrap_or_default()
}

pub fn signature(key: &PrivateKey, namespace: &str, bytes: &[u8]) -> Result<String, CeremonyError> {
    key.sign(namespace, HashAlg::Sha512, bytes).and_then(|s| s.to_pem(LineEnding::LF))
        .map_err(|_| CeremonyError::UnregisteredKey)
}

fn verifies(key: &str, namespace: &str, bytes: &[u8], signature: &str) -> bool {
    let (Ok(key), Ok(signature)) = (PublicKey::from_openssh(key), SshSig::from_pem(signature)) else { return false };
    key.verify(namespace, bytes, &signature).is_ok()
}

/// The phone calls this after collecting one attestation per ENROLLED runner,
/// in order. A pending entry (offline, or its enrollment failed) carries none
/// and is signed as pending: the browser shows it as not enrolled and never
/// dials it (ov-1149). A missing attestation for an enrolled entry is a
/// refusal before anything is sealed or posted.
pub fn sign(manifest: &mut Manifest, offer: &Offer, private: &str, authorities: Vec<Authority>) -> Result<(), CeremonyError> {
    let key = PrivateKey::from_openssh(private).map_err(|_| CeremonyError::UnregisteredKey)?;
    let device_key = key.public_key().to_openssh().map_err(|_| CeremonyError::UnregisteredKey)?;
    check_authorities(manifest, offer, &device_key, &authorities)?;
    let signature = signature(&key, DEVICE_NAMESPACE, &payload(manifest, &authorities))?;
    manifest.proof = Some(Proof { device_key, authorities, signature });
    Ok(())
}

/// The entries a reply claims this browser's key is on. Only these need a
/// runner attestation and a directory pin; a pending entry grants nothing, so
/// turning one into an enrolled one meets both checks like any other.
fn enrolled(manifest: &Manifest) -> impl Iterator<Item = &crate::RunnerEntry> {
    manifest.runners.iter().filter(|r| !r.pending)
}

fn check_authorities(manifest: &Manifest, offer: &Offer, device: &str, authorities: &[Authority]) -> Result<(), CeremonyError> {
    let device = fingerprint(device).ok_or(CeremonyError::UnregisteredKey)?;
    let statement = authority_statement(offer, &device);
    // At least one runner must vouch: a reply that enrolls nowhere is refused,
    // so with every runner offline the phone posts nothing, as before.
    if authorities.is_empty() || enrolled(manifest).count() != authorities.len() {
        return Err(CeremonyError::UnregisteredKey);
    }
    for (runner, authority) in enrolled(manifest).zip(authorities) {
        if fingerprint(&authority.runner_key).as_deref() != Some(runner.id.as_str())
            || !verifies(&authority.runner_key, RUNNER_NAMESPACE, &statement, &authority.signature) {
            return Err(CeremonyError::UnregisteredKey);
        }
    }
    Ok(())
}

/// Every enrolled runner is pinned independently, and the entire reply,
/// pending entries included, is covered by the device's signature. One wrong
/// enrolled runner refuses the whole reply.
pub fn verify(manifest: &Manifest, offer: &Offer, trust: &Trust) -> Result<(), CeremonyError> {
    let proof = manifest.proof.as_ref().ok_or(CeremonyError::UnregisteredKey)?;
    let device = fingerprint(&proof.device_key).ok_or(CeremonyError::UnregisteredKey)?;
    if !trust.devices.contains(&device) || enrolled(manifest).any(|r| !trust.runners.contains(&r.id)) {
        return Err(CeremonyError::UnregisteredKey);
    }
    check_authorities(manifest, offer, &proof.device_key, &proof.authorities)?;
    if !verifies(&proof.device_key, DEVICE_NAMESPACE, &payload(manifest, &proof.authorities), &proof.signature) {
        return Err(CeremonyError::UnregisteredKey);
    }
    Ok(())
}
