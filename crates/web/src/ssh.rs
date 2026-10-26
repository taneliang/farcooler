//! SSH to one runner over a byte stream, with its host key pinned.
//!
//! The stream is whatever carries the bytes: in a browser, a Tailcat
//! connection the Go module dialed (see `js.rs`); in the tests, one end of an
//! in-memory pipe to an in-process server. Nothing here knows which.
//!
//! **The host key is pinned, never learned.** The enrollment reply carries each
//! runner's `SHA256:` fingerprint beside its route (`RunnerEntry.host_key`),
//! and the handshake is refused before authentication unless the key the
//! runner presents prints to exactly that string. That is the first key
//! exchange's key: russh 0.62 does not re-check a key a rekey presents, as the
//! native client's verifier doesn't either, so only the peer already
//! authenticated by the pin could change it. There is no first-use
//! acceptance and no prompt: a browser has no earlier connection to compare
//! with, and the reply is the only statement anybody made about this runner.

use std::future::Future;
use std::sync::{Arc, Mutex};

use russh::client::{self, Handle, Handler};
use russh::keys::{HashAlg, PublicKey};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::error::WebError;

/// Why the signer failed. russh wants its own send error convertible into it.
#[derive(Debug)]
pub enum SignError {
    Send,
    /// The person dismissed the passkey prompt, or the browser refused to
    /// show one (Safari, without a click just before it).
    Declined,
    Sign(String),
}

impl From<russh::SendError> for SignError {
    fn from(_: russh::SendError) -> Self {
        SignError::Send
    }
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
/// authenticate as `user` with `public_key`, signed by `signer`: in a browser
/// the passkey's (`passkey::PasskeySigner`), and in the tests whatever key the
/// in-process server can verify.
///
/// The pin is checked before authentication is attempted, so a runner that is
/// not the pinned one never sees a signature from this key.
pub async fn connect<S, G>(
    stream: S,
    pinned_host_key: &str,
    user: &str,
    public_key: &str,
    signer: &mut G,
) -> Result<Connection, WebError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    G: russh::Signer<Error = SignError>,
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
            SignError::Declined => WebError::PasskeyDeclined,
            SignError::Sign(why) => WebError::Signer(why),
        })?;
    if !auth.success() {
        return Err(WebError::KeyRejected);
    }
    Ok(handle)
}
