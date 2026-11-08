//! The first-key code (ov-988): how an owner whose runner has no Mac app
//! trusts their phone's step-up key, with nobody but themselves.
//!
//! Since ov-980 a key is trusted only with someone present: the Mac app's
//! Touch ID on the runner's own Mac, or a device already trusted. A Linux
//! runner has neither, and on it every local route (the socket, a terminal,
//! the CLI) is one an agent running as the same Unix user can take too.
//! So presence rests on a secret that is never on the runner:
//!
//! - **`runner install` makes the code on the computer it runs on** and shows
//!   it there. Only its hash goes to the runner ([`keep`]), into
//!   `<runtime dir>/first-key-code`, and only when no code was ever kept
//!   there. The installer makes none when the runner is the machine it runs
//!   on (`runner_install::first_key`).
//! - **The phone sends it as its own key's approval**: `client.approve_step_up`
//!   naming itself, with the code, signed by the key it proposed
//!   (`step_up::admit` lets that one request verify against a proposal). So
//!   the code trusts the key of the phone that holds it, never a key someone
//!   else proposed.
//! - **[`trust`] serves it once**, for the owner's own device with a line
//!   here, while no key of the owner's is trusted. A used code is renamed to
//!   `first-key-code.used`, and a kept hash is never replaced, so the code
//!   can't be made again on the runner. The key is vouched for (the owner
//!   was there, `cutover_grant`), as the Mac app's Touch ID vouches.
//!
//! 125 random bits: the runner holds the hash, which anyone on it can read,
//! and guessing the code from it is hopeless.

use std::path::Path;

use farcooler_core::{DomainError, Result};
use sha2::{Digest, Sha256};

use crate::service::Service;
use crate::step_up::Stepped;

/// Where the hash is kept, under the runtime dir.
pub const FILE: &str = "first-key-code";
/// The same file once its code was used.
pub const USED: &str = "first-key-code.used";
/// Who vouched for a key the code trusted.
pub const VOUCHER: &str = "first-key-code";

/// Crockford's base32: no I, L, O or U, so nothing reads as another.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// 25 characters, 5 bits each.
const LENGTH: usize = 25;

/// A fresh code, as the installer shows it: five groups of five.
pub fn code() -> String {
    let bytes = crate::step_up::fresh_nonce().expect("/dev/urandom");
    let chars: Vec<char> = (0..LENGTH).map(|i| ALPHABET[(bytes[i] & 31) as usize] as char).collect();
    chars.chunks(5).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("-")
}

/// The code as typed, the way it was made: case, spaces and dashes dropped,
/// and the letters Crockford reads as digits read as them. `None` for
/// anything that isn't 25 characters of the alphabet.
pub fn normalized(typed: &str) -> Option<String> {
    let out: String = typed
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        })
        .collect();
    (out.len() == LENGTH && out.bytes().all(|b| ALPHABET.contains(&b))).then_some(out)
}

/// What the runner keeps of `code`. Empty for one that isn't a code.
pub fn hash_of(code: &str) -> String {
    let Some(code) = normalized(code) else { return String::new() };
    let digest = Sha256::new().chain_update(b"farcooler-first-key-v1\0").chain_update(code.as_bytes()).finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// What [`keep`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
    /// The hash is kept now.
    Kept,
    /// A code was made before and is still waiting to be used; this one isn't kept.
    Waiting,
    /// A code was made and used; none is kept again.
    Used,
}

impl Kept {
    /// The word `runner keep-first-key` prints, and the installer reads.
    pub fn word(self) -> &'static str {
        match self {
            Kept::Kept => "kept",
            Kept::Waiting => "waiting",
            Kept::Used => "used",
        }
    }

    pub fn from_word(word: &str) -> Option<Kept> {
        [Kept::Kept, Kept::Waiting, Kept::Used].into_iter().find(|k| k.word() == word.trim())
    }
}

/// Keep `hash` in `dir`, unless a code was ever kept there. Created with
/// `create_new`, owner-only, so two installers can't both keep one.
pub fn keep(dir: &Path, hash: &str) -> Result<Kept> {
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(DomainError::InvalidArgument { what: "first_key_hash" });
    }
    if dir.join(USED).exists() {
        return Ok(Kept::Used);
    }
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(dir.join(FILE)) {
        Ok(mut file) => {
            file.write_all(hash.as_bytes()).and_then(|()| file.sync_all()).map_err(|_| DomainError::OperationFailed)?;
            Ok(Kept::Kept)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(Kept::Waiting),
        Err(_) => Err(DomainError::OperationFailed),
    }
}

fn wrong() -> DomainError {
    DomainError::Conflict { what: "first_key_wrong" }
}

/// `client.approve_step_up` by `device` for its own key, with `code`: trust
/// it, if `stepped` is that key's signature over this approval, the device
/// is the owner's with a line here, no key of the owner's is trusted yet,
/// and `code` is the one kept. Spends the code.
pub async fn trust(svc: &Service, device: &str, stepped: Option<&Stepped>, code: &str) -> Result<()> {
    let signed = stepped.is_some_and(|s| s.method == "client.approve_step_up" && s.device == device && !s.browser);
    if !signed {
        return Err(DomainError::Conflict { what: "step_up_needed" });
    }
    let key = svc.store.step_up_key(device)?.ok_or(DomainError::NotFound)?;
    if key.alg != crate::step_up::ALG_DEVICE || crate::step_up_rpc::is_browser(svc, device)? {
        return Err(DomainError::Conflict { what: "defensive_only" });
    }
    let has_line = farcooler_fence::read(svc.authorized_keys(), farcooler_fence::AUTHORIZED_KEYS)
        .map_err(|_| DomainError::OperationFailed)?
        .iter()
        .any(|e| e.client_id == device);
    if !has_line || crate::people::person_of_device(&svc.store, Some(device))? != crate::people::Person::Owner {
        tracing::warn!(device, "refused: a first-key code from a device that isn't the owner's");
        return Err(DomainError::NotYours { what: "not_yours" });
    }
    if owner_has_trusted_key(svc)? {
        return Err(DomainError::Conflict { what: "first_key_taken" });
    }
    if crate::step_up_rpc::now_ms() - key.registered_at > farcooler_store::step_up::PROPOSAL_MS {
        return Err(DomainError::Conflict { what: "proposal_expired" });
    }
    let dir = svc.root_dir();
    let kept = std::fs::read_to_string(dir.join(FILE)).map_err(|_| wrong())?;
    let given = hash_of(code);
    if given.is_empty() || !same(kept.trim().as_bytes(), given.as_bytes()) {
        tracing::warn!(device, "refused: a first-key code that doesn't match");
        return Err(wrong());
    }
    // Spent before the key is trusted, by a rename only one caller wins.
    std::fs::rename(dir.join(FILE), dir.join(USED)).map_err(|_| wrong())?;
    svc.store.trust_step_up_key(device, crate::step_up_rpc::now_ms())?;
    svc.store.vouch_step_up_key(device, VOUCHER)?;
    tracing::info!(device, "the owner's first step-up key was trusted with the first-key code");
    Ok(())
}

/// Whether any device of the owner's, the Mac app's own key included, holds a
/// trusted key: then the code has nothing to do, and a trusted device approves.
fn owner_has_trusted_key(svc: &Service) -> Result<bool> {
    for device in svc.store.trusted_step_up_devices()? {
        if device == crate::step_up::LOCAL_APP || crate::people::person_of_device(&svc.store, Some(&device))? == crate::people::Person::Owner {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Equal, in time that doesn't depend on where they differ.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_reads_back_however_it_is_typed() {
        let code = code();
        assert_eq!(code.len(), 29, "{code}");
        let hash = hash_of(&code);
        assert_eq!(hash.len(), 64);
        assert_eq!(hash_of(&code.to_lowercase().replace('-', "  ")), hash);
        assert_eq!(hash_of(&code.replace('0', "o").replace('1', "l")), hash);
        assert_eq!(hash_of("123456"), "", "a six-digit key code is not a first-key code");
        assert_ne!(hash_of(&super::code()), hash);
    }

    #[test]
    fn a_code_is_kept_once_and_never_after_its_use() {
        let dir = tempfile::tempdir().unwrap();
        let a = hash_of(&code());
        assert_eq!(keep(dir.path(), &a).unwrap(), Kept::Kept);
        assert_eq!(keep(dir.path(), &hash_of(&code())).unwrap(), Kept::Waiting);
        assert_eq!(std::fs::read_to_string(dir.path().join(FILE)).unwrap(), a, "the first one stays");
        std::fs::rename(dir.path().join(FILE), dir.path().join(USED)).unwrap();
        assert_eq!(keep(dir.path(), &hash_of(&code())).unwrap(), Kept::Used);
        assert!(!dir.path().join(FILE).exists());
        assert!(keep(dir.path(), "not a hash").is_err());
    }
}
