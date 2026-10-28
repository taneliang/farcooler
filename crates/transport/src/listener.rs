//! Rule 1: the daemon binds no port on any interface. This is the one entry
//! point it accepts on directly, and it is a filesystem-permissioned Unix
//! socket, never a TCP port. The other two arrive through sshd — a stdio child
//! sshd launches, and the outbound tunnel that carries a dialing device to that
//! same sshd on loopback. See `lib.rs` for all three.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::UnixListener;

use crate::Handler;
use crate::connection::{Connection, HandshakeConfig, refuse, serve_connection};

/// What a caller said about itself in one line before the first frame.
///
/// Sent by exactly one thing: a `farcoolerd --stdio` process relaying an ssh
/// session into the daemon already listening here. That process is a dumb pipe
/// by design, so what sshd's forced command told it cannot ride inside the
/// protocol — it would have to parse a conversation the two ends already agree
/// about. One line before the conversation starts is the whole mechanism.
///
/// Nothing is interpreted here. Which scope words exist is the daemon's policy,
/// living in one table beside the methods they gate; a copy in this crate would
/// be a second table to keep in step.
#[derive(Debug, Clone)]
pub struct SessionPreamble {
    /// The scope word exactly as it arrived.
    pub scope: String,
    /// Which enrolled device this is. `-` on the wire means none was named.
    pub client: Option<String>,
}

/// The line a session may open with, before any frame.
const PREAMBLE_PREFIX: &[u8] = b"farcooler-session ";

/// Longer than any scope and client id worth having, and short enough that a
/// peer which opens with the prefix and then never sends a newline cannot make
/// this buffer without bound.
const PREAMBLE_MAX_BYTES: usize = 512;

/// How a connection opened: with a preamble, or straight into the protocol.
/// Either way, the bytes read while deciding come back to be replayed.
enum Opening {
    Preamble(SessionPreamble, Vec<u8>),
    Frames(Vec<u8>),
}

/// Read the optional preamble, consuming nothing that is not one.
///
/// Optional on purpose. Absence is what every already-installed client does,
/// and requiring it would break them all on upgrade — in front of the version
/// negotiation built to explain exactly this class of mismatch, so the user
/// would get a hang-up instead of a reason.
///
/// Telling the two apart is unambiguous rather than heuristic: a frame opens
/// with a four-byte big-endian length, so its first byte is zero for anything
/// under sixteen megabytes and can never be the `f` this looks for. Whatever was
/// read while deciding is handed back and replayed in front of the reader, so a
/// client that sent no preamble is left with its byte stream untouched.
async fn read_opening<R: AsyncRead + Unpin>(read: &mut R) -> std::io::Result<Opening> {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let compared = buf.len().min(PREAMBLE_PREFIX.len());
        if buf[..compared] != PREAMBLE_PREFIX[..compared] {
            return Ok(Opening::Frames(buf));
        }
        if buf.len() >= PREAMBLE_PREFIX.len() {
            if let Some(end) = buf.iter().position(|b| *b == b'\n') {
                // Lossy rather than refused: what a malformed word means is the
                // daemon's decision, one seam up, and it refuses there.
                let line = String::from_utf8_lossy(&buf[PREAMBLE_PREFIX.len()..end]).into_owned();
                let rest = buf.split_off(end + 1);
                let mut fields = line.split_whitespace();
                let scope = fields.next().unwrap_or_default().to_string();
                let client = fields.next().filter(|c| *c != "-").map(str::to_string);
                return Ok(Opening::Preamble(SessionPreamble { scope, client }, rest));
            }
            if buf.len() > PREAMBLE_MAX_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "a session preamble that never ends",
                ));
            }
        }

        let mut chunk = [0u8; 256];
        let read_bytes = read.read(&mut chunk).await?;
        if read_bytes == 0 {
            // Closed before saying anything. Let the protocol path report that
            // in its own terms rather than inventing a second vocabulary here.
            return Ok(Opening::Frames(buf));
        }
        buf.extend_from_slice(&chunk[..read_bytes]);
    }
}

pub struct UnixListenerServer {
    listener: UnixListener,
    path: PathBuf,
}

impl UnixListenerServer {
    /// Rule 5: binds `path` with mode 0600 under a user-only (0700) parent
    /// directory. Bind-then-chmod leaves a brief window at default
    /// permissions; the 0700 parent covers it, since nothing but the owner
    /// can even traverse into the directory during that window.
    pub fn bind(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        }
        // A stale socket left by a crashed daemon must not block a fresh bind.
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Self { listener, path })
    }

    pub fn local_path(&self) -> &Path {
        &self.path
    }

    /// Accepts connections until the listener itself errors. Each connection
    /// gets its own handshake and its own rule-4 accounting, so one slow
    /// client cannot stall another.
    ///
    /// `session` decides what an accepted connection is: it is handed the
    /// preamble, if the caller sent one, and answers with the handshake and the
    /// handler for THAT connection. A closure rather than one config for the
    /// listener's life because scope arrives per connection — a caller relayed
    /// from ssh may hold less than the owning user at this socket does, and a
    /// constant could only ever describe one of them. `None` refuses the
    /// connection.
    pub async fn serve<H, F>(&self, session: F) -> std::io::Result<()>
    where
        H: Handler,
        F: Fn(Option<SessionPreamble>) -> Option<(HandshakeConfig, H)> + Send + Sync + 'static,
    {
        self.serve_with_pid(move |preamble, _pid| session(preamble)).await
    }

    /// `serve`, with the connecting process's pid handed to `session` too,
    /// when the kernel says (`peer_pid`). The daemon reads it to tell which
    /// pane a local caller runs in, whatever it claims to be (ov-494).
    pub async fn serve_with_pid<H, F>(&self, session: F) -> std::io::Result<()>
    where
        H: Handler,
        F: Fn(Option<SessionPreamble>, Option<i32>) -> Option<(HandshakeConfig, H)> + Send + Sync + 'static,
    {
        let session = Arc::new(session);
        loop {
            let (stream, _addr) = self.listener.accept().await?;
            let session = session.clone();
            let pid = peer_pid(&stream);
            tokio::spawn(async move {
                let (mut read_half, write_half) = stream.into_split();
                let opening = match read_opening(&mut read_half).await {
                    Ok(opening) => opening,
                    Err(err) => {
                        tracing::debug!(error = %err, "connection closed before the protocol began");
                        return;
                    }
                };
                let (preamble, buffered) = match opening {
                    Opening::Preamble(preamble, rest) => (Some(preamble), rest),
                    Opening::Frames(rest) => (None, rest),
                };

                // Refused in silence, and that is not a gap: the only thing that
                // sends a preamble is this daemon's own stdio relay, which
                // validates its arguments and exits with a message on stderr
                // before it ever connects. Anything else reaching here wrote the
                // line by hand.
                // Only a device's relayed session names itself; a local
                // caller (the Mac app, a pane) does not, and is not logged.
                let device = preamble.as_ref().map(|p| (p.client.clone().unwrap_or_else(|| "-".into()), p.scope.clone()));
                let Some((cfg, handler)) = session(preamble, pid) else {
                    tracing::warn!("refused a session whose preamble named no scope this daemon has");
                    return;
                };

                let reader = std::io::Cursor::new(buffered).chain(read_half);
                let mut conn = Connection::new(reader, write_half);
                if let Some((client, scope)) = &device {
                    tracing::info!(%client, %scope, "a device session began");
                }
                let served = serve_connection(&mut conn, &cfg, &handler).await;
                if let Err(err) = &served {
                    tracing::debug!(error = %err, "connection closed");
                }
                if let Some((client, _)) = &device {
                    log_session_end(client, &served);
                }
            });
        }
    }
}

/// Say, at INFO, that a device's session ended and why: the client id and the
/// reason word, never a message, a key or a token. A browser's failed sign-in
/// and a phone's reach this the same way, so this is where the next
/// "the connection didn't work" gets its cause (ov-852).
fn log_session_end(client: &str, served: &Result<(), crate::ConnectionError>) {
    let reason = match served {
        Ok(()) => "ended",
        Err(err) => err.word(),
    };
    tracing::info!(%client, %reason, "a device session ended");
}

/// The process on the other end of a local connection, as the kernel says.
#[cfg(target_os = "macos")]
pub fn peer_pid(stream: &tokio::net::UnixStream) -> Option<i32> {
    use std::os::fd::AsRawFd;

    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    // SAFETY: a valid borrowed fd, an out-parameter of the size the option
    // documents, and its length passed by pointer as getsockopt requires.
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            &mut pid as *mut libc::pid_t as *mut libc::c_void,
            &mut len,
        )
    };
    (rc == 0 && pid > 0).then_some(pid)
}

/// The process on the other end of a local connection, as the kernel says.
#[cfg(target_os = "linux")]
pub fn peer_pid(stream: &tokio::net::UnixStream) -> Option<i32> {
    stream.peer_cred().ok().and_then(|c| c.pid()).filter(|p| *p > 0)
}

/// No way to ask on this system.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn peer_pid(_stream: &tokio::net::UnixStream) -> Option<i32> {
    None
}

impl UnixListenerServer {
    /// Accept connections only to refuse each one with `err`, until the
    /// listener itself errors.
    ///
    /// A daemon that cannot serve still holds the socket this way, rather than
    /// exiting. Exiting would leave a supervisor restarting it into the same
    /// refusal every few seconds, and every client finding no socket at all,
    /// which says nothing. This says the reason to each of them.
    ///
    /// The preamble is read and ignored: who is asking changes nothing about
    /// the answer.
    pub async fn refuse_every(&self, err: farcooler_core::DomainError) -> std::io::Result<()> {
        loop {
            let (stream, _addr) = self.listener.accept().await?;
            let err = err.clone();
            tokio::spawn(async move {
                let (mut read_half, write_half) = stream.into_split();
                let buffered = match read_opening(&mut read_half).await {
                    Ok(Opening::Preamble(_, rest) | Opening::Frames(rest)) => rest,
                    Err(e) => {
                        tracing::debug!(error = %e, "connection closed before the protocol began");
                        return;
                    }
                };
                let reader = std::io::Cursor::new(buffered).chain(read_half);
                if let Err(e) = refuse(reader, write_half, err).await {
                    tracing::debug!(error = %e, "connection closed before it could be refused");
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[tokio::test]
    async fn socket_and_parent_directory_are_owner_only() {
        let dir = tempdir().unwrap();
        let sock_path = dir.path().join("nested").join("farcooler.sock");
        let server = UnixListenerServer::bind(&sock_path).unwrap();

        let meta = std::fs::metadata(server.local_path()).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600, "socket must be mode 0600");

        let parent_meta = std::fs::metadata(sock_path.parent().unwrap()).unwrap();
        assert_eq!(parent_meta.permissions().mode() & 0o777, 0o700, "parent dir must be user-only");
    }

    #[tokio::test]
    async fn rebinding_removes_a_stale_socket() {
        let dir = tempdir().unwrap();
        let sock_path = dir.path().join("farcooler.sock");
        let first = UnixListenerServer::bind(&sock_path).unwrap();
        // Simulate a crash: drop the listener but leave the file behind.
        drop(first);
        assert!(sock_path.exists(), "the socket file itself outlives the listener");

        let second = UnixListenerServer::bind(&sock_path);
        assert!(second.is_ok(), "a stale socket file must not block a fresh bind");
    }

    /// A handler that is never asked anything: the handshake fails first.
    struct NoHandler;
    impl crate::Handler for NoHandler {
        fn peer(&self) -> crate::Peer {
            crate::Peer { client_id: None, scope: farcooler_protocol::v1::Scope::Read }
        }
        async fn handle(&self, _req: farcooler_protocol::v1::Request) -> farcooler_protocol::v1::Response {
            unreachable!("the handshake fails first")
        }
    }

    /// Collects what the log says, to read it back.
    #[derive(Clone, Default)]
    struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
        type Writer = Capture;
        fn make_writer(&'a self) -> Capture {
            self.clone()
        }
    }

    /// A device whose handshake fails leaves its client id and the reason in
    /// the log at INFO (ov-852); a key or a token never does.
    #[tokio::test]
    async fn a_failed_device_handshake_logs_its_client_and_reason() {
        use tokio::io::AsyncWriteExt;

        let capture = Capture::default();
        let _log = tracing::subscriber::set_default(
            tracing_subscriber::fmt().with_writer(capture.clone()).with_ansi(false).with_max_level(tracing::Level::INFO).finish(),
        );
        let dir = tempdir().unwrap();
        let sock_path = dir.path().join("farcooler.sock");
        let server = UnixListenerServer::bind(&sock_path).unwrap();
        let serving = tokio::spawn(async move {
            server
                .serve(|_| Some((HandshakeConfig { daemon_version: "t".into() }, NoHandler)))
                .await
        });
        let mut stream = tokio::net::UnixStream::connect(&sock_path).await.unwrap();
        // A device's preamble, then bytes that are no ClientHello.
        stream.write_all(b"farcooler-session read browser-7\n").await.unwrap();
        stream.write_all(&[0, 0, 0, 3, 1, 2, 3]).await.unwrap();
        for _ in 0..200 {
            if String::from_utf8_lossy(&capture.0.lock().unwrap()).contains("ended") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        serving.abort();
        let log = String::from_utf8_lossy(&capture.0.lock().unwrap()).to_string();
        let ended = log.lines().find(|l| l.contains("a device session ended")).unwrap_or_else(|| panic!("no end line in {log:?}"));
        assert!(ended.contains("INFO") && ended.contains("browser-7"), "{ended}");
        assert!(ended.contains("reason=") && !ended.contains("reason=ended"), "{ended}");
        assert!(log.contains("a device session began"), "{log:?}");
    }

    /// A daemon that cannot serve says why to whoever connects, with a
    /// preamble or without one, and the client keeps the reason. Before
    /// `Client::over` read a refusal, this arrived as `NoHello`, which every
    /// caller words as "nothing is installed there".
    #[tokio::test]
    async fn a_refusing_daemon_tells_every_client_why() {
        use tokio::io::AsyncWriteExt;

        use crate::{Client, ClientError};

        let dir = tempdir().unwrap();
        let sock_path = dir.path().join("farcooler.sock");
        let server = UnixListenerServer::bind(&sock_path).unwrap();
        let serving = tokio::spawn(async move {
            server.refuse_every(farcooler_core::DomainError::NewerData).await
        });

        for preamble in [None, Some("farcooler-session read phone-7\n")] {
            let mut stream = tokio::net::UnixStream::connect(&sock_path).await.unwrap();
            if let Some(line) = preamble {
                stream.write_all(line.as_bytes()).await.unwrap();
            }
            let (read, write) = stream.into_split();
            let err = Client::over(read, write, "test", "0").await.err().expect("refused");
            match err {
                ClientError::Daemon { code, retryable, message, what } => {
                    assert_eq!(
                        code,
                        farcooler_protocol::v1::ErrorCode::VersionIncompatible as i32
                    );
                    assert!(!retryable);
                    assert_eq!(
                        message,
                        "This runner's data was written by a newer Far Cooler. Update Far Cooler to use it."
                    );
                    assert_eq!(what, "newer_data");
                }
                other => panic!("expected the refusal, got {other:?}"),
            }
        }
        serving.abort();
    }
}
