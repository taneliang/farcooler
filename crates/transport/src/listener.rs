//! Rule 1: the daemon opens no network listener. This is the only
//! network-adjacent entry point it has, and it is a filesystem-permissioned
//! Unix socket, never a TCP port.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::UnixListener;

use crate::Handler;
use crate::connection::{Connection, HandshakeConfig, serve_connection};

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
        let session = Arc::new(session);
        loop {
            let (stream, _addr) = self.listener.accept().await?;
            let session = session.clone();
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
                let Some((cfg, handler)) = session(preamble) else {
                    tracing::warn!("refused a session whose preamble named no scope this daemon has");
                    return;
                };

                let reader = std::io::Cursor::new(buffered).chain(read_half);
                let mut conn = Connection::new(reader, write_half);
                if let Err(err) = serve_connection(&mut conn, &cfg, &handler).await {
                    tracing::debug!(error = %err, "connection closed");
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
}
