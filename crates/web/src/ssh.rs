//! SSH to one runner over a byte stream, with its host key pinned.
//!
//! The stream is whatever carries the bytes: in a browser, a Tailcat
//! connection the Go module dialed (see `js.rs`); in the tests, one end of an
//! in-memory pipe to an in-process server. Nothing here knows which.
//!
//! **The host key is pinned, never learned.** The enrollment reply carries each
//! runner's `SHA256:` fingerprint beside its route (`RunnerEntry.host_key`),
//! and the handshake is refused before authentication unless the key the
//! runner presents prints to exactly that string. There is no first-use
//! acceptance and no prompt: a browser has no earlier connection to compare
//! with, and the reply is the only statement anybody made about this runner.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use russh::client::{self, Handle, Handler};
use russh::keys::agent::AgentIdentity;
use russh::keys::{HashAlg, PublicKey, ssh_key};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::error::WebError;

/// Signs SSH's user-auth data with an Ed25519 key this crate never holds, and
/// answers the raw 64-byte signature.
///
/// In a browser that is WebCrypto with a non-extractable key; in the tests a
/// key in memory. The future must be `Send` because russh's signer is; a
/// browser's is not, and `js.rs` says why it may be marked so there.
pub trait RawSign {
    fn sign(&mut self, data: Vec<u8>) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, String>> + Send>>;
}

/// A [`RawSign`] presented to russh as an `ssh-ed25519` signer.
///
/// The only key type slice 1 has: the fence accepts Ed25519 and nothing else
/// (`crates/fence/src/lib.rs`, `render`), so a browser that enrolled anything
/// else could never be written into a line.
pub struct Ed25519Signer<S>(pub S);

/// Why the signer failed. russh wants its own send error convertible into it.
#[derive(Debug)]
pub enum SignError {
    Send,
    Sign(String),
}

impl From<russh::SendError> for SignError {
    fn from(_: russh::SendError) -> Self {
        SignError::Send
    }
}

impl<S: RawSign> russh::Signer for Ed25519Signer<S> {
    type Error = SignError;

    fn auth_sign(
        &mut self,
        key: &AgentIdentity,
        _hash_alg: Option<HashAlg>,
        to_sign: Vec<u8>,
    ) -> impl Future<Output = Result<Vec<u8>, SignError>> + Send {
        let ed25519 = matches!(key.public_key().algorithm(), ssh_key::Algorithm::Ed25519);
        let signing = self.0.sign(to_sign.clone());
        async move {
            if !ed25519 {
                return Err(SignError::Sign("only an Ed25519 key signs here".into()));
            }
            let raw = signing.await.map_err(SignError::Sign)?;
            ed25519_signature_blob(to_sign, &raw).map_err(SignError::Sign)
        }
    }
}

/// What russh sends: the data it asked to have signed, followed by the
/// signature as an SSH string holding `string("ssh-ed25519") string(sig)`
/// (RFC 8709 §6).
pub fn ed25519_signature_blob(mut to_sign: Vec<u8>, raw: &[u8]) -> Result<Vec<u8>, String> {
    if raw.len() != 64 {
        return Err(format!("an Ed25519 signature is 64 bytes, not {}", raw.len()));
    }
    const ALG: &[u8] = b"ssh-ed25519";
    let inner = 4 + ALG.len() + 4 + raw.len();
    to_sign.extend_from_slice(&(inner as u32).to_be_bytes());
    to_sign.extend_from_slice(&(ALG.len() as u32).to_be_bytes());
    to_sign.extend_from_slice(ALG);
    to_sign.extend_from_slice(&(raw.len() as u32).to_be_bytes());
    to_sign.extend_from_slice(raw);
    Ok(to_sign)
}

/// An Ed25519 public key's 32 raw bytes as the OpenSSH line an enrollment
/// offer carries (`Offer.key_a`).
pub fn ed25519_public_line(raw: &[u8], comment: &str) -> Result<String, WebError> {
    let bytes: [u8; 32] = raw
        .try_into()
        .map_err(|_| WebError::Signer(format!("an Ed25519 public key is 32 bytes, not {}", raw.len())))?;
    let key = PublicKey::new(ssh_key::public::KeyData::Ed25519(ssh_key::public::Ed25519PublicKey(bytes)), comment);
    key.to_openssh().map_err(|e| WebError::Signer(e.to_string()))
}

/// Checks the runner's host key against the pin, and remembers what it saw.
pub struct Pinned {
    expected: String,
    seen: Arc<Mutex<Option<String>>>,
}

impl Handler for Pinned {
    type Error = russh::Error;

    fn check_server_key(&mut self, key: &PublicKey) -> impl Future<Output = Result<bool, Self::Error>> + Send {
        // OpenSSH's own format, so it compares with `ssh-keygen -lf` and with
        // the string `ceremony::RunnerEntry.host_key` carries.
        let actual = key.fingerprint(HashAlg::Sha256).to_string();
        let matches = actual == self.expected;
        *self.seen.lock().unwrap() = Some(actual);
        async move { Ok(matches) }
    }
}

/// An authenticated SSH connection to one runner.
pub type Connection = Handle<Pinned>;

/// Run SSH over `stream`, refuse any host key but `pinned_host_key`, and
/// authenticate as `user` with `public_key`, signed by `signer`.
///
/// The pin is checked before authentication is attempted, so a runner that is
/// not the pinned one never sees a signature from this key.
pub async fn connect<S, R>(
    stream: S,
    pinned_host_key: &str,
    user: &str,
    public_key: &str,
    signer: &mut Ed25519Signer<R>,
) -> Result<Connection, WebError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    R: RawSign,
{
    if pinned_host_key.is_empty() {
        return Err(WebError::HostKeyUnknown);
    }
    let public = PublicKey::from_openssh(public_key).map_err(|e| WebError::Signer(e.to_string()))?;

    let seen = Arc::new(Mutex::new(None));
    let handler = Pinned { expected: pinned_host_key.to_string(), seen: seen.clone() };
    // No keepalive and no inactivity timer: russh runs both on tokio's clock,
    // which a browser does not have, and a timer that cannot fire is worse
    // than none. A dead tunnel is noticed when a read or a write fails.
    let config = Arc::new(client::Config::default());
    let mut handle = match client::connect_stream(config, stream, handler).await {
        Ok(handle) => handle,
        Err(e) => {
            let actual = seen.lock().unwrap().clone();
            return Err(match actual {
                Some(actual) if actual != pinned_host_key => {
                    WebError::HostKeyChanged { expected: pinned_host_key.to_string(), actual }
                }
                _ => WebError::Handshake(e.to_string()),
            });
        }
    };

    let auth = handle
        .authenticate_publickey_with(user, public, None, signer)
        .await
        .map_err(|e| match e {
            SignError::Send => WebError::Closed,
            SignError::Sign(why) => WebError::Signer(why),
        })?;
    if !auth.success() {
        return Err(WebError::KeyRejected);
    }
    Ok(handle)
}
