//! Reaching the daemon — locally, or on another machine.
//!
//! Durable state has exactly one owner. Before this existed, every `farcooler`
//! invocation opened the database itself, which meant two commands running at
//! once were two authorities on the same file — and it made a second client,
//! which is the entire point of the product, impossible.
//!
//! Local and remote produce the SAME `Link`, because they are the same
//! protocol: one over a Unix socket, one over ssh's stdin and stdout. Every
//! command above this file is written once and works against either, which is
//! the property that keeps a remote host from being a second-class citizen with
//! its own subtly different behavior.
//!
//! Auto-start is local-only and deliberate: the first thing a user does is run
//! a command, not install a service. Two commands racing to start the daemon is
//! harmless, because it probes for a live socket before binding.

use std::path::PathBuf;
use std::time::Duration;

use farcooler_protocol::v1::{Request, request, result};
use farcooler_transport::{Client, ClientError};
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
    /// The process on the other end, for a local link. See `peer_pid`.
    peer: Option<i32>,
}

impl Link {
    pub async fn call(
        &mut self,
        request: Request,
    ) -> Result<farcooler_protocol::v1::Result, ClientError> {
        self.client.call(request).await
    }

    /// Block until the daemon pushes something.
    pub async fn next_event(&mut self) -> Result<farcooler_protocol::v1::Event, ClientError> {
        self.client.next_event().await
    }

    /// Which source the daemon on the other end was built from.
    ///
    /// Free: it arrives in the handshake, so asking costs no round trip.
    pub fn daemon_build(&self) -> &str {
        &self.client.server_hello().daemon_version
    }

    /// The transport underneath, for callers reusing shared request-building
    /// code from `farcooler_client::actions` instead of building requests
    /// inline.
    pub fn client_mut(&mut self) -> &mut Client<Reader, Writer> {
        &mut self.client
    }
}

/// Connect to a daemon: the local one, or `target`'s over ssh.
pub async fn connect_to(target: Option<&str>) -> Result<Link, Box<dyn std::error::Error>> {
    match target {
        Some(host) => {
            let remote = crate::remote::connect(host).await?;
            Ok(Link { client: remote.client, _ssh: Some(remote.child), peer: None })
        }
        None => connect().await,
    }
}

/// Connect to the local daemon, starting it if nothing answers.
pub async fn connect() -> Result<Link, Box<dyn std::error::Error>> {
    let socket = farcooler_daemon::paths::socket_path()?;

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
    let peer = peer_pid(&stream);
    let (read, write) = stream.into_split();
    let client = Client::over(
        Box::new(read) as Reader,
        Box::new(write) as Writer,
        "farcooler-cli",
        env!("CARGO_PKG_VERSION"),
    )
    .await?;
    Ok(Link { client, _ssh: None, peer })
}

/// Which process is on the other end of this socket.
///
/// The kernel answers, so it is not a guess: no pidfile to go stale, no name to
/// match, no way to signal something that merely used to be the daemon. That
/// matters because the one thing this pid is used for is `SIGTERM`, and the
/// caller reaching for it is replacing a daemon too old to know how to stop
/// itself — which is precisely the daemon that cannot be asked politely.
#[cfg(target_os = "macos")]
fn peer_pid(stream: &tokio::net::UnixStream) -> Option<i32> {
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

#[cfg(target_os = "linux")]
fn peer_pid(stream: &tokio::net::UnixStream) -> Option<i32> {
    stream.peer_cred().ok().and_then(|c| c.pid()).filter(|p| *p > 0)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn peer_pid(_stream: &tokio::net::UnixStream) -> Option<i32> {
    None
}

/// Connect to the local daemon only if one is already there.
///
/// The difference from `connect` is the whole point: this one never starts a
/// daemon, so "nothing is running" comes back as an answer rather than being
/// quietly fixed. Anything managing the daemon's lifecycle has to be able to
/// ask that question without changing it.
pub async fn connect_existing() -> Result<Option<Link>, Box<dyn std::error::Error>> {
    let socket = farcooler_daemon::paths::socket_path()?;
    match dial(&socket).await {
        Ok(link) => Ok(Some(link)),
        Err(ClientError::Connect(_)) => Ok(None),
        Err(other) => Err(Box::new(other)),
    }
}

/// What `ensure_local` had to do to leave a matching daemon running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ensured {
    /// One was already running, built from this same source.
    Unchanged,
    /// Nothing was listening, so one was started.
    Started,
    /// One was running from different source; it was stopped and replaced.
    Replaced,
}

impl Ensured {
    pub fn as_str(self) -> &'static str {
        match self {
            Ensured::Unchanged => "unchanged",
            Ensured::Started => "started",
            Ensured::Replaced => "replaced",
        }
    }
}

/// Leave this machine running a daemon built from the same source as this CLI.
///
/// The Mac app calls this at launch, which is what makes "the app owns the
/// local daemon" true rather than aspirational. Before it existed, a daemon
/// started by yesterday's build kept the socket for as long as it stayed alive
/// — through every rebuild, every reinstall — and the only sign was a MISMATCH
/// line in `farcooler status` that nothing was obliged to read. Two components
/// built from different source behave like two different programs, and the
/// symptom is a bug you already fixed still happening.
///
/// Replacing costs nothing that matters: terminals are tmux windows and outlive
/// this entirely, agent shims reconnect on the next start, and durable state is
/// committed to SQLite per call.
pub async fn ensure_local() -> Result<(Ensured, String), Box<dyn std::error::Error>> {
    let socket = farcooler_daemon::paths::socket_path()?;

    let mut link = match dial(&socket).await {
        Ok(link) => link,
        Err(ClientError::Connect(_)) => {
            spawn_daemon()?;
            let started = wait_for(&socket).await?;
            return Ok((Ensured::Started, started.daemon_build().to_string()));
        }
        Err(other) => return Err(Box::new(other)),
    };

    if link.daemon_build() == farcooler_protocol::BUILD {
        return Ok((Ensured::Unchanged, link.daemon_build().to_string()));
    }

    let running = link.daemon_build().to_string();
    tracing::info!(running, ours = farcooler_protocol::BUILD, "replacing the local daemon");
    stop(&mut link).await?;
    drop(link);
    wait_until_gone(&socket).await?;

    spawn_daemon()?;
    let started = wait_for(&socket).await?;
    let build = started.daemon_build().to_string();
    if build != farcooler_protocol::BUILD {
        // The daemon beside this CLI is not the daemon this CLI was built with,
        // so replacing it again would loop forever. Say which two, and stop.
        return Err(format!(
            "started {}, which was built from different source than this CLI ({})",
            build,
            farcooler_protocol::BUILD
        )
        .into());
    }
    Ok((Ensured::Replaced, build))
}

/// Ask the daemon to stop; failing that, tell the kernel to ask it.
///
/// The polite request only works on a daemon new enough to know the method, and
/// the daemon being replaced is by definition an older one — so the fallback is
/// not an edge case, it is the whole upgrade path from every build that shipped
/// before `daemon.shutdown` existed. SIGTERM lands on the same orderly stop the
/// method triggers: the socket is unlinked and nothing is killed.
pub async fn stop(link: &mut Link) -> Result<(), Box<dyn std::error::Error>> {
    let peer = link.peer;
    match link.call(req("daemon.shutdown")).await {
        Ok(_) => return Ok(()),
        Err(e) => tracing::debug!(error = %e, "the daemon would not stop on request"),
    }

    let Some(pid) = peer else {
        return Err("the daemon is too old to stop on request and did not identify itself".into());
    };
    // SAFETY: a pid the kernel gave us for this socket's peer, and SIGTERM,
    // which this daemon handles as a clean shutdown.
    if unsafe { libc::kill(pid, libc::SIGTERM) } != 0 {
        return Err(format!("could not signal the daemon (pid {pid})").into());
    }
    Ok(())
}

/// Poll until nothing answers the socket.
async fn wait_until_gone(socket: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if tokio::net::UnixStream::connect(socket).await.is_err() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    Err("the running daemon did not stop within 5s".into())
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

/// Where `farcoolerd` is.
///
/// Beside this executable first, because that is how both the app bundle and a
/// cargo target directory lay them out, and it guarantees the daemon matches
/// the CLI that started it. A mismatched pair is exactly the failure the
/// version handshake exists to catch, and it is better not to create one.
fn daemon_binary() -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Ok(explicit) = std::env::var("FARCOOLERD_BIN") {
        return Ok(PathBuf::from(explicit));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let sibling = dir.join("farcoolerd");
        if sibling.is_file() {
            return Ok(sibling);
        }
    }
    Ok(PathBuf::from("farcoolerd"))
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
    farcooler_transport::request(method)
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
