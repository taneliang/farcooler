//! The runner's own key: the proof, to the relay's directory, that a runner is
//! the runner it says it is (ov-487 B1, ov-497).
//!
//! An install id is in `ps` and in every worktree, so it proves nothing. This
//! key is made once, in the runtime directory, and its private half never
//! leaves: nothing here returns it, prints it or logs it, and [`RunnerKey`]'s
//! `Debug` shows only the fingerprint. What leaves is a signature. The app asks
//! the box (over the ssh channel it already trusts) to sign "this account puts
//! this install in this team", and posts that to the relay, which checks it
//! against the public key it is also given (`services/relay/src/directory`).
//!
//! The signature is OpenSSH's SSHSIG in namespace `farcooler-runner-v1`, over
//! exactly the statement the relay rebuilds (`runnerStatement`). Both sides
//! pin the same fixture, `services/relay/test/fixtures/runner-proof.json`, so
//! neither can change the format alone.

use std::io::Read as _;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use farcooler_protocol::v1::Scope;
use ssh_key::private::{Ed25519Keypair, KeypairData};
use ssh_key::{HashAlg, LineEnding, PrivateKey};

/// The namespace of a runner's own signatures, the relay's `RUNNER_NAMESPACE`.
pub const NAMESPACE: &str = "farcooler-runner-v1";

/// What a runner signs for: a new registration, or taking a squatted runner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Register,
    Claim,
}

impl Action {
    fn word(self) -> &'static str {
        match self {
            Action::Register => "register",
            Action::Claim => "claim",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("the runner key could not be read or written")]
    Unreadable,
    #[error("the runner key's file is readable by other users")]
    Exposed,
    #[error("only someone with a shell on this runner can sign for it")]
    NotShell,
    #[error("that is not a valid {0}")]
    Field(&'static str),
    #[error("the runner key could not sign")]
    Signing,
}

/// This runner's signing key. Holds the private half and shows none of it.
pub struct RunnerKey(PrivateKey);

impl std::fmt::Debug for RunnerKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RunnerKey({})", self.0.fingerprint(HashAlg::Sha256))
    }
}

/// The file the key lives in.
pub fn key_path(dir: &Path) -> PathBuf {
    dir.join("runner-key")
}

impl RunnerKey {
    /// The key in `dir`, made the first time it is asked for.
    ///
    /// Made once: it is written to a private temporary file, flushed, and
    /// linked into place, and `link` fails if the name is taken. Two callers
    /// racing each end up with the same winner's key, and a crash leaves
    /// either no key or a whole one, never half of one.
    pub fn load_or_create_in(dir: &Path) -> Result<Self, KeyError> {
        let path = key_path(dir);
        match Self::load(&path) {
            Err(KeyError::Unreadable) if !path.exists() => {}
            other => return other,
        }
        std::fs::create_dir_all(dir).map_err(|_| KeyError::Unreadable)?;
        let key = Self::from_seed(&fresh_seed()?);
        let pem = key.0.to_openssh(LineEnding::LF).map_err(|_| KeyError::Unreadable)?;
        let temp = dir.join(format!(".runner-key.{}", uuid::Uuid::now_v7().simple()));
        let written = (|| {
            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp)?;
            file.write_all(pem.as_bytes())?;
            file.sync_all()
        })();
        let linked = written.and_then(|()| std::fs::hard_link(&temp, &path));
        let _ = std::fs::remove_file(&temp);
        match linked {
            Ok(()) => Ok(key),
            // Somebody else's key got there first; theirs is the key.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Self::load(&path),
            Err(_) => Err(KeyError::Unreadable),
        }
    }

    fn load(path: &Path) -> Result<Self, KeyError> {
        let meta = std::fs::metadata(path).map_err(|_| KeyError::Unreadable)?;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err(KeyError::Exposed);
        }
        let text = std::fs::read(path).map_err(|_| KeyError::Unreadable)?;
        let key = PrivateKey::from_openssh(&text).map_err(|_| KeyError::Unreadable)?;
        match key.key_data() {
            KeypairData::Ed25519(_) => Ok(Self(key)),
            _ => Err(KeyError::Unreadable),
        }
    }

    pub(crate) fn from_seed(seed: &[u8; 32]) -> Self {
        Self(PrivateKey::from(Ed25519Keypair::from_seed(seed)))
    }

    /// `ssh-ed25519 AAAA…`, no comment: what the relay stores and fingerprints.
    pub fn public_line(&self) -> String {
        self.0.public_key().to_openssh().unwrap_or_default()
    }

    /// The signature the relay verifies, armored, for `account` putting
    /// `install` in `team`, at `issued_at` (milliseconds since the epoch).
    ///
    /// `caller` is the scope of whoever asked, and only `host_admin`, the scope
    /// of someone who already has a shell on the box, may have the box sign. A
    /// `claim` moves a runner to a new owner, so a `control` or `read` device
    /// that could ask for one would turn its grant into ownership. The CLI is a
    /// process on the box under the runner's own user, and says `HostAdmin`;
    /// anything that serves a remote device must pass that device's scope.
    ///
    /// The three names go into a line-based statement, so anything that could
    /// start a new line, or is not plainly a name, is refused: a newline in
    /// `account` would otherwise let one statement read as another.
    pub fn sign(
        &self,
        caller: Scope,
        action: Action,
        account: &str,
        team: &str,
        install: &str,
        issued_at: u64,
    ) -> Result<String, KeyError> {
        if caller != Scope::HostAdmin {
            return Err(KeyError::NotShell);
        }
        for (what, value) in [("account", account), ("team", team), ("install", install)] {
            if !well_formed(value) {
                return Err(KeyError::Field(what));
            }
        }
        let statement = statement(action, account, team, install, issued_at);
        let signature = self
            .0
            .sign(NAMESPACE, HashAlg::Sha512, statement.as_bytes())
            .map_err(|_| KeyError::Signing)?;
        signature.to_pem(LineEnding::LF).map_err(|_| KeyError::Signing)
    }
}

/// An id as the relay mints them: ASCII letters, digits and a few separators.
fn well_formed(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

/// The relay's `runnerStatement`, byte for byte.
pub fn statement(action: Action, account: &str, team: &str, install: &str, issued_at: u64) -> String {
    format!(
        "farcooler {}-runner\naccount {account}\nteam {team}\ninstall {install}\nissued {issued_at}\n",
        action.word()
    )
}

fn fresh_seed() -> Result<[u8; 32], KeyError> {
    let mut seed = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut seed))
        .map_err(|_| KeyError::Unreadable)?;
    Ok(seed)
}

#[cfg(test)]
#[path = "runner_key_tests.rs"]
mod tests;
