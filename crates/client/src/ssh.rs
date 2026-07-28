//! An in-process SSH client.
//!
//! The Mac client shells out to `ssh`, which is the right answer there: OpenSSH
//! is the most scrutinised SSH implementation in existence and it is already on
//! the machine. iOS and Android cannot do that — there is no `ssh` binary and
//! no way to run one — so the transport has to be a library.
//!
//! russh is that library. It is maintained by the Warp terminal team, and its
//! default crypto backend is `aws-lc-rs`, the same FIPS-validated implementation
//! AWS ships. Overnight does not implement any cryptography itself, here or
//! anywhere.
//!
//! What this deliberately does NOT do:
//!
//! - **No password authentication.** A phone holding a reusable password is a
//!   worse asset than a phone holding a key that a host can revoke by deleting
//!   one line from `authorized_keys`.
//! - **No trust-on-first-use without saying so.** An unknown host key is
//!   reported to the caller with its fingerprint, and connecting anyway is the
//!   user's explicit decision, not a default.

use std::sync::Arc;

use russh::client::{self, Config, Handle, Handler, Msg};
use russh::keys::{PrivateKeyWithHashAlg, ssh_key};
use russh::{Channel, ChannelMsg};
use tokio::io::{AsyncRead, AsyncWrite};

#[derive(Debug, thiserror::Error)]
pub enum SshError {
    #[error("cannot reach {host}:{port}: {source}")]
    Connect {
        host: String,
        port: u16,
        #[source]
        source: std::io::Error,
    },
    #[error("the SSH handshake failed: {0}")]
    Handshake(String),
    #[error("{user}@{host} rejected this key. Add its public key to ~/.ssh/authorized_keys there.")]
    AuthRejected { user: String, host: String },
    #[error("the private key could not be read: {0}")]
    BadKey(String),
    #[error(
        "the host key for {host} is not the one Overnight has recorded.\n\
         Expected {expected}\n\
         Got      {actual}\n\
         This is either a changed server or an interception. Overnight will not connect."
    )]
    HostKeyChanged { host: String, expected: String, actual: String },
    #[error("{host} is unknown. Its key fingerprint is {fingerprint}")]
    HostKeyUnknown { host: String, fingerprint: String },
    #[error("the remote command could not be started: {0}")]
    Exec(String),
}

/// What to do about the host's key.
#[derive(Debug, Clone)]
pub enum HostKeyPolicy {
    /// Require this exact key. The normal case once a host is known.
    Pinned(String),
    /// First contact: report the fingerprint and refuse, so the caller can show
    /// it to a human. Silently accepting is what makes an interception
    /// invisible.
    RequireApproval,
    /// The user has seen the fingerprint and said yes.
    Accept,
}

/// The verdict on a host key, recorded by the handler so it survives the
/// connection attempt that produced it.
#[derive(Default)]
struct KeyVerdict {
    fingerprint: std::sync::Mutex<Option<String>>,
    mismatch: std::sync::Mutex<Option<(String, String)>>,
}

struct Verifier {
    policy: HostKeyPolicy,
    verdict: Arc<KeyVerdict>,
}

impl Handler for Verifier {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        // The same fingerprint format OpenSSH prints, so a user can compare it
        // against `ssh-keygen -lf` output without conversion.
        let fingerprint = key.fingerprint(ssh_key::HashAlg::Sha256).to_string();
        *self.verdict.fingerprint.lock().unwrap() = Some(fingerprint.clone());

        match &self.policy {
            HostKeyPolicy::Pinned(expected) => {
                if expected == &fingerprint {
                    Ok(true)
                } else {
                    *self.verdict.mismatch.lock().unwrap() =
                        Some((expected.clone(), fingerprint));
                    Ok(false)
                }
            }
            // Refused on purpose. The caller turns this into a prompt showing
            // the fingerprint, and only then retries with Accept.
            HostKeyPolicy::RequireApproval => Ok(false),
            HostKeyPolicy::Accept => Ok(true),
        }
    }
}

/// Where and as whom to connect.
#[derive(Debug, Clone)]
pub struct Destination {
    pub host: String,
    pub port: u16,
    pub user: String,
    /// An OpenSSH private key, as text. ed25519 in practice.
    pub private_key: String,
    /// Passphrase, if the key has one.
    pub passphrase: Option<String>,
    pub host_key: HostKeyPolicy,
}

/// A live SSH session.
pub struct Session {
    handle: Handle<Verifier>,
    /// The fingerprint this session actually verified, so a caller can record
    /// it after a first-contact approval.
    pub host_fingerprint: Option<String>,
}

impl Session {
    /// Connect and authenticate.
    pub async fn open(destination: &Destination) -> Result<Self, SshError> {
        let key = decode_key(&destination.private_key, destination.passphrase.as_deref())?;

        let verdict = Arc::new(KeyVerdict::default());
        let verifier =
            Verifier { policy: destination.host_key.clone(), verdict: Arc::clone(&verdict) };

        let config = Arc::new(Config {
            // A phone sleeps and wakes; without keepalives a session that the
            // network dropped hours ago looks alive until the first write.
            keepalive_interval: Some(std::time::Duration::from_secs(30)),
            keepalive_max: 3,
            ..Config::default()
        });

        let address = (destination.host.as_str(), destination.port);
        let mut handle = match client::connect(config, address, verifier).await {
            Ok(handle) => handle,
            Err(russh::Error::IO(source)) => {
                return Err(SshError::Connect {
                    host: destination.host.clone(),
                    port: destination.port,
                    source,
                });
            }
            Err(other) => {
                // A refused host key surfaces as a generic handshake failure,
                // so translate it into the thing the user has to decide about.
                if let Some((expected, actual)) = verdict.mismatch.lock().unwrap().take() {
                    return Err(SshError::HostKeyChanged {
                        host: destination.host.clone(),
                        expected,
                        actual,
                    });
                }
                if matches!(destination.host_key, HostKeyPolicy::RequireApproval)
                    && let Some(fingerprint) = verdict.fingerprint.lock().unwrap().clone()
                {
                    return Err(SshError::HostKeyUnknown {
                        host: destination.host.clone(),
                        fingerprint,
                    });
                }
                return Err(SshError::Handshake(other.to_string()));
            }
        };

        let authenticated = handle
            .authenticate_publickey(
                &destination.user,
                PrivateKeyWithHashAlg::new(Arc::new(key), None),
            )
            .await
            .map_err(|e| SshError::Handshake(e.to_string()))?;

        if !authenticated.success() {
            return Err(SshError::AuthRejected {
                user: destination.user.clone(),
                host: destination.host.clone(),
            });
        }

        let host_fingerprint = verdict.fingerprint.lock().unwrap().clone();
        Ok(Self { handle, host_fingerprint })
    }

    /// Run a command and get its stdin and stdout as byte streams.
    ///
    /// This is the whole remote transport: `overnightd --stdio` on one end, the
    /// protocol client on the other, and SSH in between doing what it is for.
    pub async fn exec(&mut self, command: &str) -> Result<Streams, SshError> {
        let mut channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|e| SshError::Exec(e.to_string()))?;
        channel.exec(true, command).await.map_err(|e| SshError::Exec(e.to_string()))?;
        Ok(Streams::new(channel))
    }
}

/// Decode an OpenSSH private key, with a message that says what to do about it.
fn decode_key(text: &str, passphrase: Option<&str>) -> Result<ssh_key::PrivateKey, SshError> {
    let key = match passphrase {
        Some(pass) => russh::keys::decode_secret_key(text, Some(pass)),
        None => russh::keys::decode_secret_key(text, None),
    };
    key.map_err(|e| {
        let hint = if text.contains("ENCRYPTED") && passphrase.is_none() {
            " The key is encrypted and no passphrase was supplied."
        } else {
            ""
        };
        SshError::BadKey(format!("{e}.{hint}"))
    })
}

/// The exec channel, as an `AsyncRead` and an `AsyncWrite`.
///
/// russh speaks in channel messages; the protocol client speaks in byte
/// streams. This is the only place that conversion happens — everything above
/// it is transport-agnostic and identical to the code running over a Unix
/// socket.
pub struct Streams {
    pub reader: ChannelReader,
    /// russh already provides an `AsyncWrite` for the channel; pinning it in a
    /// box is all that is needed to name the type.
    pub writer: std::pin::Pin<Box<dyn AsyncWrite + Send>>,
}

impl Streams {
    fn new(mut channel: Channel<Msg>) -> Self {
        let writer = Box::pin(channel.make_writer());
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();

        tokio::spawn(async move {
            while let Some(msg) = channel.wait().await {
                match msg {
                    ChannelMsg::Data { data } => {
                        if tx.send(data.to_vec()).is_err() {
                            break;
                        }
                    }
                    // stderr is the daemon's log stream, not the wire. It goes
                    // to tracing rather than into the protocol, where a single
                    // byte of it would corrupt a frame.
                    ChannelMsg::ExtendedData { data, .. } => {
                        let text = String::from_utf8_lossy(&data);
                        for line in text.lines().filter(|l| !l.trim().is_empty()) {
                            tracing::debug!(target: "overnight::remote", "{line}");
                        }
                    }
                    ChannelMsg::Eof | ChannelMsg::Close => break,
                    _ => {}
                }
            }
        });

        Self { reader: ChannelReader::new(rx), writer }
    }
}

/// Reads channel data as a byte stream.
pub struct ChannelReader {
    rx: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    pending: Vec<u8>,
    offset: usize,
}

impl ChannelReader {
    fn new(rx: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) -> Self {
        Self { rx, pending: Vec::new(), offset: 0 }
    }
}

impl AsyncRead for ChannelReader {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        use std::task::Poll;

        // Drain the previous chunk first: a frame reader asks for exactly the
        // bytes it needs, which is rarely a chunk boundary.
        if self.offset < self.pending.len() {
            let n = (self.pending.len() - self.offset).min(buf.remaining());
            let start = self.offset;
            let slice = self.pending[start..start + n].to_vec();
            buf.put_slice(&slice);
            self.offset += n;
            return Poll::Ready(Ok(()));
        }

        match self.rx.poll_recv(cx) {
            Poll::Ready(Some(chunk)) => {
                self.pending = chunk;
                self.offset = 0;
                let n = self.pending.len().min(buf.remaining());
                let slice = self.pending[..n].to_vec();
                buf.put_slice(&slice);
                self.offset = n;
                Poll::Ready(Ok(()))
            }
            // The channel closed: end of stream, not an error.
            Poll::Ready(None) => Poll::Ready(Ok(())),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_malformed_key_is_refused_with_a_reason() {
        let err = decode_key("not a key at all", None).unwrap_err();
        assert!(matches!(err, SshError::BadKey(_)));
    }

    #[test]
    fn an_encrypted_key_with_no_passphrase_says_so() {
        // The most common setup mistake, and the error russh gives for it is
        // otherwise indistinguishable from a corrupt file.
        let encrypted = "-----BEGIN OPENSSH PRIVATE KEY-----\nENCRYPTED\n-----END OPENSSH PRIVATE KEY-----";
        let message = decode_key(encrypted, None).unwrap_err().to_string();
        assert!(message.contains("passphrase"), "got: {message}");
    }

    #[test]
    fn a_changed_host_key_names_both_fingerprints() {
        // The user has to be able to tell which is which to decide anything.
        let err = SshError::HostKeyChanged {
            host: "box".into(),
            expected: "SHA256:aaa".into(),
            actual: "SHA256:bbb".into(),
        };
        let message = err.to_string();
        assert!(message.contains("SHA256:aaa") && message.contains("SHA256:bbb"));
        assert!(message.contains("will not connect"));
    }

    #[test]
    fn a_rejected_key_says_what_to_do_about_it() {
        let message =
            SshError::AuthRejected { user: "me".into(), host: "box".into() }.to_string();
        assert!(message.contains("authorized_keys"));
    }
}
