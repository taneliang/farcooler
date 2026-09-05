//! An in-process SSH client.
//!
//! The Mac client shells out to `ssh`, which is the right answer there: OpenSSH
//! is the most scrutinised SSH implementation in existence and it is already on
//! the machine. iOS and Android cannot do that — there is no `ssh` binary and
//! no way to run one — so the transport has to be a library.
//!
//! russh is that library. It is maintained by the Warp terminal team, and its
//! default crypto backend is `aws-lc-rs`, the same FIPS-validated implementation
//! AWS ships. Far Cooler does not implement any cryptography itself, here or
//! anywhere.
//!
//! What this deliberately does NOT do:
//!
//! - **No password authentication.** A phone holding a reusable password is a
//!   worse asset than a phone holding a key that a runner can revoke by deleting
//!   one line from `authorized_keys`.
//! - **No trust-on-first-use without saying so.** An unknown host key is
//!   reported to the caller with its fingerprint, and connecting anyway is the
//!   user's explicit decision, not a default.

use std::sync::Arc;

use farcooler_tailcat::TunnelError;
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
        "the host key for {host} is not the one Far Cooler has recorded.\n\
         Expected {expected}\n\
         Got      {actual}\n\
         This is either a changed server or an interception. Far Cooler will not connect."
    )]
    HostKeyChanged { host: String, expected: String, actual: String },
    #[error("{host} is unknown. Its key fingerprint is {fingerprint}")]
    HostKeyUnknown { host: String, fingerprint: String },
    #[error("the remote command could not be started: {0}")]
    Exec(String),
    /// The tunnel never opened. `code` is the stable word from
    /// `farcooler_tailcat::TunnelError::code`, which the apps map to a
    /// sentence — no Rust error string reaches a screen.
    #[error("cannot open the tunnel: {code}")]
    Tunnel { code: &'static str },
    /// The tunnel itself worked — it reached the runner — but nothing is
    /// listening on the port `OnTCP` maps onto. Deliberately its own
    /// sentence rather than `Connect`'s: "cannot reach {host}:{port}" would
    /// be false here (the tunnel *did* reach it), and interpolating a label
    /// like "the tunnel" into an address slot reads as an address to a
    /// person who cannot tell the difference. The underlying OS error is
    /// kept as `#[source]` for logs, never interpolated into the message
    /// itself — no raw error string reaches a screen here either.
    #[error("the tunnel reached the runner, but nothing is listening for SSH there")]
    TunnelPortClosed {
        #[source]
        source: std::io::Error,
    },
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

/// How to reach a runner: an address, or a connection token that dials the
/// tunnel that runner holds open. Which one this device has is decided when the
/// runner is set up, not per connection.
///
/// **A token comes from `client.set_node_key` and from nowhere else, so
/// `Tailcat` is unreachable from the ceremony today.** Enrollment answers with
/// no token — `ClientEnrollResult` has no such field — and nothing yet mints a
/// node key for a phone, so an offer carries none, a granting runner writes
/// none, and every entry a ceremony produces is `Direct`. The one path that
/// reaches this variant is a device that already holds a session, calling
/// `client.set_node_key` and keeping the `conn_blob` it answers with. Where a
/// phone's node key would come from is an open decision, not an oversight in
/// this enum — and saying otherwise here would be the comment lying.
///
/// One reach per runner, and no fallback. A `Direct` runner never quietly
/// tries the tunnel and a `Tailcat` runner never quietly tries an address —
/// because a transport that races two paths reports the wrong failure, and the
/// failure message is most of what this product is.
/// Serialized because a ceremony reply carries one per runner: this is the
/// enum `crates/client/src/ceremony.rs` re-exports rather than redefining, so
/// the shape a manifest writes is the shape a `Destination` reads.
///
/// Internally tagged on `kind` so a third variant is additive on the wire —
/// two optional fields would admit "both set" and "neither set", and then
/// something downstream picks a winner.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reach {
    /// You exchanged a key yourself and you know where the box is.
    Direct { host: String, port: u16 },
    /// The runner told you its token and you told it your node key.
    Tailcat {
        token: String,
        /// This device's OWN tailcat node private key, and the one field here
        /// that is a secret.
        ///
        /// `skip` rather than a convention that callers blank it: a manifest
        /// is photographable — the design says so in as many words — and a
        /// private key in one would be draft three's mistake with a new
        /// name. Nothing else needs it on a wire either, because the key is
        /// per DEVICE and not per runner: whoever dials fills in the key it
        /// already holds. A reply therefore decodes with this empty, which is
        /// not "unknown" — it is "yours, and you have it".
        #[serde(skip)]
        client_key: String,
    },
}

impl Reach {
    /// What to name in an error. Never a token: it is long, it is meaningless
    /// to a person, and it is the one field here worth stealing.
    pub fn label(&self) -> String {
        match self {
            Self::Direct { host, port } => format!("{host}:{port}"),
            Self::Tailcat { .. } => "the tunnel".to_string(),
        }
    }
}

/// Where and as whom to connect.
#[derive(Debug, Clone)]
pub struct Destination {
    pub reach: Reach,
    pub user: String,
    /// An OpenSSH private key, as text. ed25519 in practice.
    pub private_key: String,
    /// Passphrase, if the key has one.
    pub passphrase: Option<String>,
    /// Pinned the same way whichever reach is in play: a tunnel carries
    /// plain SSH end to end, so the host key it presents is the runner's own,
    /// not the tunnel's.
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

        let mut handle = match &destination.reach {
            Reach::Direct { host, port } => {
                match client::connect(config, (host.as_str(), *port), verifier).await {
                    Ok(handle) => handle,
                    Err(russh::Error::IO(source)) => {
                        return Err(SshError::Connect {
                            host: host.clone(),
                            port: *port,
                            source,
                        });
                    }
                    Err(other) => return Err(translate(other, destination, &verdict)),
                }
            }
            Reach::Tailcat { token, client_key } => {
                // Port 22 is a name, not an address. It is the number the
                // tunnel is keyed on; where sshd actually listens is a fact
                // only the runner has, and its OnTCP handler maps it.
                let stream = match farcooler_tailcat::dial(token, client_key, 22).await {
                    Ok(stream) => stream,
                    Err(e) => return Err(tunnel_error(e)),
                };
                match client::connect_stream(config, stream, verifier).await {
                    Ok(handle) => handle,
                    Err(other) => return Err(translate(other, destination, &verdict)),
                }
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
                host: destination.reach.label(),
            });
        }

        let host_fingerprint = verdict.fingerprint.lock().unwrap().clone();
        Ok(Self { handle, host_fingerprint })
    }

    /// Run a command and get its stdin and stdout as byte streams.
    ///
    /// This is the whole remote transport: `farcoolerd --stdio` on one end, the
    /// protocol client on the other, and SSH in between doing what it is for.
    pub async fn exec(&mut self, command: &str) -> Result<Streams, SshError> {
        let channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|e| SshError::Exec(e.to_string()))?;
        channel.exec(true, command).await.map_err(|e| SshError::Exec(e.to_string()))?;
        Ok(Streams::new(channel))
    }
}

/// Turn a russh handshake failure into the specific thing a person has to
/// decide about, when the verifier caught one — a changed or unknown host
/// key — and a generic message otherwise.
///
/// Shared by both transports, direct and tunneled: a host key is pinned the
/// same way in either case (see `Destination::host_key`'s doc), because it is
/// still plain SSH inside the tunnel, so a mismatch or an unknown key reads
/// the same regardless of which reach carried the bytes.
fn translate(other: russh::Error, destination: &Destination, verdict: &KeyVerdict) -> SshError {
    if let Some((expected, actual)) = verdict.mismatch.lock().unwrap().take() {
        return SshError::HostKeyChanged { host: destination.reach.label(), expected, actual };
    }
    if matches!(destination.host_key, HostKeyPolicy::RequireApproval)
        && let Some(fingerprint) = verdict.fingerprint.lock().unwrap().clone()
    {
        return SshError::HostKeyUnknown { host: destination.reach.label(), fingerprint };
    }
    SshError::Handshake(other.to_string())
}

/// Map a failed `dial` to the specific thing it deserves.
///
/// `TunnelError::Io` with `kind() == ConnectionRefused` is special: the
/// tunnel reached the runner and nothing was listening on the port `OnTCP`
/// maps onto — a dead or misconfigured sshd, not a rejected handshake. It
/// gets its own `SshError::TunnelPortClosed`, not `Connect`: `Connect`'s
/// message is "cannot reach {host}:{port}", which would be false here (the
/// tunnel DID reach it) and has no real host/port to put in that slot for a
/// `Tailcat` reach anyway — `destination.reach.label()` there would read as
/// an address ("the tunnel:22") to someone who cannot tell it is not one.
///
/// Everything else — `NoTailcatLinked`, `Derp`, `NoAnswer`, and every other
/// `Io` — crosses as `SshError::Tunnel`'s stable word. `Io`'s word in
/// particular, `"io"`, has to stay true of a malformed token, a dead sshd
/// reached before this special case (should the errno ever differ across
/// platforms) and `EMFILE` alike, so the word is deliberately generic and the
/// apps own the sentence.
///
/// Public because the CLI dials the same tunnel for `farcooler runner pipe`
/// and has to render the same two outcomes. A second copy of "connection
/// refused means the tunnel worked and sshd did not" is a second dialect, and
/// the one that drifts is whichever copy is not the one being read.
pub fn tunnel_error(error: TunnelError) -> SshError {
    match error {
        TunnelError::Io(source) if source.kind() == std::io::ErrorKind::ConnectionRefused => {
            SshError::TunnelPortClosed { source }
        }
        other => SshError::Tunnel { code: other.code() },
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

/// Closes the ssh channel once both halves of a stream have been dropped.
///
/// Every stream is a `channel_open_session`, and sshd counts those against
/// `MaxSessions` — ten by default, on the ONE TCP connection this client
/// makes. Nothing here used to give one back. The pump task below owns the
/// `Channel`, so dropping the reader and the writer left it parked in
/// `wait()` on a quiet pane forever: russh sends no CHANNEL_CLOSE from a
/// plain `Channel`'s destructor (only `ChannelCloseOnDrop` does, and that is
/// reached through `into_stream()`, which this does not use), and dropping
/// the writer sends nothing either — `ChannelTx::drop` only notifies.
///
/// So `~/.local/bin/farcoolerd --stream` never saw its stdin close, never hit
/// the hangup path that exists to catch exactly this, and held its session
/// slot alive. Nine tab switches later every `channel_open_session` came back
/// `ConnectFailed`, which the phone rendered as "Could not load" on a pane
/// that was perfectly healthy.
///
/// An `Arc` around the sender rather than a field on one half, because the
/// two halves go to different owners: `open_stream` keeps the writer purely
/// so the far end does not see a closed stdin (see its comment), while the
/// reader is what the FFI task holds. The channel should outlive both and
/// neither, so it closes when the last clone goes.
type ChannelGuard = Arc<tokio::sync::oneshot::Sender<()>>;

impl Streams {
    fn new(mut channel: Channel<Msg>) -> Self {
        let inner = Box::pin(channel.make_writer());
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        // Dropped, not sent on: a `oneshot::Sender` going out of scope wakes
        // its receiver with an error, which is the signal. Nothing ever has a
        // value to send here — the event IS the drop.
        let (closed, mut closing) = tokio::sync::oneshot::channel::<()>();
        let guard: ChannelGuard = Arc::new(closed);

        tokio::spawn(async move {
            loop {
                let msg = tokio::select! {
                    // Both halves gone. Stop reading and say so, below.
                    _ = &mut closing => break,
                    msg = channel.wait() => msg,
                };
                let Some(msg) = msg else { break };
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
                            tracing::debug!(target: "farcooler::remote", "{line}");
                        }
                    }
                    ChannelMsg::Eof | ChannelMsg::Close => break,
                    _ => {}
                }
            }
            // EOF first, then close. The EOF is what the far end reads as
            // "nobody is watching any more" and exits on; the close is what
            // hands the session slot back to sshd. Both are best-effort — a
            // channel that died with the connection has nothing to say and
            // nowhere to say it.
            let _ = channel.eof().await;
            let _ = channel.close().await;
        });

        Self {
            reader: ChannelReader::new(rx, guard.clone()),
            writer: Box::pin(ChannelWriter { inner, _guard: guard }),
        }
    }
}

/// The channel's write half, holding its end of the channel open.
///
/// A wrapper purely to carry the guard: `make_writer` hands back an opaque
/// `impl AsyncWrite` with nowhere to put one, and the writer is the half
/// `open_stream` keeps alive on purpose.
struct ChannelWriter {
    inner: std::pin::Pin<Box<dyn AsyncWrite + Send>>,
    _guard: ChannelGuard,
}

impl AsyncWrite for ChannelWriter {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.get_mut().inner.as_mut().poll_write(cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.get_mut().inner.as_mut().poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.get_mut().inner.as_mut().poll_shutdown(cx)
    }
}

/// Reads channel data as a byte stream.
pub struct ChannelReader {
    rx: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    pending: Vec<u8>,
    offset: usize,
    /// Held, never read. See `ChannelGuard`: this is one of the two halves
    /// whose disappearance gives the ssh session slot back.
    _guard: ChannelGuard,
}

impl ChannelReader {
    fn new(rx: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>, guard: ChannelGuard) -> Self {
        Self { rx, pending: Vec::new(), offset: 0, _guard: guard }
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

/// Load a key, for tests in sibling modules that need to prove one is usable.
#[cfg(test)]
pub(crate) fn decode_key_for_test(text: &str) -> Result<ssh_key::PrivateKey, SshError> {
    decode_key(text, None)
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

    /// A syntactically valid, unencrypted key — good enough to get past
    /// `decode_key`. Neither test below reaches a real sshd, so nothing here
    /// needs to be usable beyond that.
    fn test_key() -> String {
        use russh::keys::ssh_key::{Algorithm, LineEnding, PrivateKey};
        let key =
            PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("generate a key");
        key.to_openssh(LineEnding::LF).expect("encode the key").to_string()
    }

    /// `Session` holds a russh `Handle`, which has no `Debug` impl, so
    /// `unwrap_err` — which formats the `Ok` side for its panic message —
    /// cannot be used on `Session::open`'s result. This does the same thing
    /// without that bound.
    async fn open_err(destination: &Destination) -> SshError {
        match Session::open(destination).await {
            Ok(_) => panic!("expected the connection to fail"),
            Err(error) => error,
        }
    }

    #[tokio::test]
    async fn a_direct_destination_still_reports_where_it_could_not_reach() {
        let destination = Destination {
            // Port 1 is reserved and nothing listens there.
            reach: Reach::Direct { host: "127.0.0.1".into(), port: 1 },
            user: "u".into(),
            private_key: test_key(),
            passphrase: None,
            host_key: HostKeyPolicy::RequireApproval,
        };
        let error = open_err(&destination).await;
        assert!(matches!(error, SshError::Connect { .. }), "{error:?}");
        let message = error.to_string();
        assert!(message.contains("127.0.0.1:1"), "lost the address: {message}");
    }

    /// A build with no archive must fail at the tunnel with the one word that
    /// names what is missing, and must not fall back to anything.
    #[cfg(not(feature = "tailcat"))]
    #[tokio::test]
    async fn a_tunnel_destination_without_an_archive_says_so() {
        let destination = Destination {
            reach: Reach::Tailcat { token: "tc-x".into(), client_key: "k".into() },
            user: "u".into(),
            private_key: test_key(),
            passphrase: None,
            host_key: HostKeyPolicy::RequireApproval,
        };
        let error = open_err(&destination).await;
        assert!(matches!(error, SshError::Tunnel { code: "no_tailcat" }), "{error:?}");
    }

    /// The reason this branch exists at all: `ECONNREFUSED` on the tunnel
    /// means the tunnel worked and sshd is not listening, not that SSH
    /// refused a handshake — and not that the tunnel could not be reached
    /// either, which `Connect`'s "cannot reach ..." would have said. This
    /// pins the actual meaning, not just that the wrong words are absent: a
    /// version that dropped the special case (falling through to the
    /// generic `Tunnel { code: "io" }`) or one that kept the misleading
    /// "cannot reach the tunnel:22" wording would both have to fail this.
    #[test]
    fn a_refused_tunnel_port_reads_like_a_dead_sshd_not_a_rejected_handshake() {
        let source = std::io::Error::from(std::io::ErrorKind::ConnectionRefused);
        let error = tunnel_error(TunnelError::Io(source));

        assert!(matches!(error, SshError::TunnelPortClosed { .. }), "{error:?}");
        let message = error.to_string().to_lowercase();
        assert!(!message.contains("ssh refused"), "wrong story: {message}");
        assert!(!message.contains("cannot reach"), "says the opposite of what happened: {message}");
        assert!(
            message.contains("reached the runner") && message.contains("nothing is listening"),
            "lost the actual meaning: {message}"
        );
        assert!(!message.contains("os error"), "leaked the raw OS error text: {message}");
    }

    /// Every other tunnel failure — including `NoAnswer`, whose entire
    /// reason for existing is that a revoked device looks like a timeout
    /// rather than a refusal, and every other `io::Error`, whose kind might
    /// be `EMFILE` or anything else — crosses as the stable word and nothing
    /// more specific, because `SshError::Tunnel`'s whole point is that the
    /// apps own the sentence.
    #[test]
    fn every_other_tunnel_error_crosses_as_its_stable_word() {
        let derp = tunnel_error(TunnelError::Derp);
        assert!(matches!(derp, SshError::Tunnel { code: "derp" }), "{derp:?}");

        let no_answer = tunnel_error(TunnelError::NoAnswer);
        assert!(matches!(no_answer, SshError::Tunnel { code: "no_answer" }), "{no_answer:?}");

        let unrelated_io = tunnel_error(TunnelError::Io(std::io::Error::from(
            std::io::ErrorKind::PermissionDenied,
        )));
        assert!(matches!(unrelated_io, SshError::Tunnel { code: "io" }), "{unrelated_io:?}");
    }
}
