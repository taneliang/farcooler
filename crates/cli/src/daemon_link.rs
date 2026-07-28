//! Reaching the daemon, and starting it if it is not there.
//!
//! Durable state has exactly one owner. Before this existed, every `overnight`
//! invocation opened the database itself, which meant two commands running at
//! once were two authorities on the same file — and it made a second client,
//! which is the entire point of the product, impossible.
//!
//! Auto-start is here rather than left to launchd because the first thing a
//! user does is run a command, not install a service. A command that answers
//! "the daemon is not running" and stops is a worse product than one that
//! starts it. Two commands racing to start it is harmless: the daemon probes
//! for a live socket before binding and exits quietly if it loses.

use std::path::PathBuf;
use std::time::Duration;

use overnight_protocol::v1::{Request, request, result};
use overnight_transport::{Client, ClientError};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

pub type Link = Client<OwnedReadHalf, OwnedWriteHalf>;

/// Connect, starting the daemon if nothing answers.
pub async fn connect() -> Result<Link, Box<dyn std::error::Error>> {
    let socket = overnight_daemon::paths::socket_path()?;

    match Client::connect(&socket, "overnight-cli", env!("CARGO_PKG_VERSION")).await {
        Ok(client) => return Ok(client),
        Err(ClientError::Connect(_)) => {}
        // A connect that failed for any reason OTHER than not reaching the
        // socket is a real problem — a version mismatch, say. Starting a second
        // daemon would not fix it and would bury the message.
        Err(other) => return Err(Box::new(other)),
    }

    spawn_daemon(&socket)?;
    wait_for(&socket).await
}

fn spawn_daemon(socket: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
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

    let _ = socket;
    Ok(())
}

/// Where `overnightd` is.
///
/// Beside this executable first, because that is how both the app bundle and a
/// cargo target directory lay them out, and it guarantees the daemon matches
/// the CLI that started it. A mismatched pair is exactly the failure the
/// version handshake exists to catch, and it is better not to create it.
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
    // Fall back to PATH.
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
        match Client::connect(socket, "overnight-cli", env!("CARGO_PKG_VERSION")).await {
            Ok(client) => return Ok(client),
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
