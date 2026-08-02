//! Reaching the daemon — locally, or on another machine.
//!
//! Durable state has exactly one owner. Before this existed, every `overnight`
//! invocation opened the database itself, which meant two commands running at
//! once were two authorities on the same file — and it made a second client,
//! which is the entire point of the product, impossible.
//!
//! Local and remote produce the SAME `Link`, because they are the same
//! protocol: one over a Unix socket, one over ssh's stdin and stdout. Every
//! command above this file is written once and works against either, which is
//! the property that keeps a remote host from being a second-class citizen with
//! its own subtly different behaviour.
//!
//! Auto-start is local-only and deliberate: the first thing a user does is run
//! a command, not install a service. Two commands racing to start the daemon is
//! harmless, because it probes for a live socket before binding.

use std::path::PathBuf;
use std::time::Duration;

use overnight_protocol::v1::{Request, request, result};
use overnight_transport::{Client, ClientError};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::process::Child;

type Reader = Box<dyn AsyncRead + Unpin + Send>;
type Writer = Box<dyn AsyncWrite + Unpin + Send>;

/// A connection to a daemon, however it was reached.
pub struct Link {
    client: Client<Reader, Writer>,
    /// The ssh process, for a remote link. Held so the session lives exactly as
    /// long as the link does and is killed on drop rather than leaking.
    _ssh: Option<Child>,
}

impl Link {
    pub async fn call(
        &mut self,
        request: Request,
    ) -> Result<overnight_protocol::v1::Result, ClientError> {
        self.client.call(request).await
    }

    /// Block until the daemon pushes something.
    pub async fn next_event(&mut self) -> Result<overnight_protocol::v1::Event, ClientError> {
        self.client.next_event().await
    }
}

/// Connect to a daemon: the local one, or `target`'s over ssh.
pub async fn connect_to(target: Option<&str>) -> Result<Link, Box<dyn std::error::Error>> {
    match target {
        Some(host) => {
            let remote = crate::remote::connect(host).await?;
            Ok(Link { client: remote.client, _ssh: Some(remote.child) })
        }
        None => connect().await,
    }
}

/// Connect to the local daemon, starting it if nothing answers.
pub async fn connect() -> Result<Link, Box<dyn std::error::Error>> {
    let socket = overnight_daemon::paths::socket_path()?;

    match dial(&socket).await {
        Ok(link) => return Ok(link),
        Err(ClientError::Connect(_)) => {}
        // A failure for any reason OTHER than not reaching the socket is a real
        // problem — a version mismatch, say. Starting a second daemon would not
        // fix it and would bury the message.
        Err(other) => return Err(Box::new(other)),
    }

    spawn_daemon()?;
    wait_for(&socket).await
}

async fn dial(socket: &std::path::Path) -> Result<Link, ClientError> {
    let stream = tokio::net::UnixStream::connect(socket).await.map_err(ClientError::Connect)?;
    let (read, write) = stream.into_split();
    let client = Client::over(
        Box::new(read) as Reader,
        Box::new(write) as Writer,
        "overnight-cli",
        env!("CARGO_PKG_VERSION"),
    )
    .await?;
    Ok(Link { client, _ssh: None })
}

fn spawn_daemon() -> Result<(), Box<dyn std::error::Error>> {
    let binary = daemon_binary()?;
    tracing::debug!(binary = %binary.display(), "starting the daemon");

    std::process::Command::new(&binary)
        // Detached: the daemon outlives the command that started it, which is
        // the whole point of a daemon. Its output goes nowhere rather than
        // interleaving with this command's own.
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", binary.display()))?;
    Ok(())
}

/// Where `overnightd` is.
///
/// Beside this executable first, because that is how both the app bundle and a
/// cargo target directory lay them out, and it guarantees the daemon matches
/// the CLI that started it. A mismatched pair is exactly the failure the
/// version handshake exists to catch, and it is better not to create one.
fn daemon_binary() -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Ok(explicit) = std::env::var("OVERNIGHTD_BIN") {
        return Ok(PathBuf::from(explicit));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let sibling = dir.join("overnightd");
        if sibling.is_file() {
            return Ok(sibling);
        }
    }
    Ok(PathBuf::from("overnightd"))
}

/// Poll until the daemon is listening.
///
/// Bounded: a daemon that cannot start must produce an error, not a command
/// that hangs forever waiting for it.
async fn wait_for(socket: &std::path::Path) -> Result<Link, Box<dyn std::error::Error>> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut last: Option<ClientError> = None;

    while std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(40)).await;
        match dial(socket).await {
            Ok(link) => return Ok(link),
            Err(e) => last = Some(e),
        }
    }

    Err(match last {
        Some(e) => format!("the daemon did not come up within 5s: {e}").into(),
        None => "the daemon did not come up within 5s".into(),
    })
}

/// A request with no payload.
pub fn req(method: &str) -> Request {
    overnight_transport::request(method)
}

/// A request addressed at one resource.
pub fn req_for(method: &str, target: uuid::Uuid) -> Request {
    let mut r = req(method);
    r.target_resource_id = Some(bytes::Bytes::copy_from_slice(target.as_bytes()));
    r
}

/// Attach a payload.
pub fn with(mut r: Request, payload: request::Payload) -> Request {
    r.payload = Some(payload);
    r
}

/// Unwrap a result into the variant the caller expects.
///
/// A daemon that answered with the wrong variant is a protocol bug, and saying
/// so beats a panic or a silent default.
pub fn expect_value(
    value: Option<result::Value>,
    what: &str,
) -> Result<result::Value, Box<dyn std::error::Error>> {
    value.ok_or_else(|| format!("the daemon returned no {what}").into())
}
