//! `farcoolerd --stdio` ends with its session.
//!
//! The relay pumps bytes between sshd and the running daemon. When the daemon
//! closes the connection (a revoke does, `sessions::close_device`), the relay
//! must exit, so sshd closes the channel and the device sees its session end.
//! It used to return from `main` instead, and the runtime's drop waited on the
//! blocking thread parked in a `read` of stdin until ssh closed it: the relay
//! lingered, the device's call waited for an answer that never came, and CI
//! failed `a_real_sshd_forces_the_scope` one run in a few (ov-856).
//!
//! No sshd here: a listener on the daemon's socket path that hangs up, and a
//! relay whose stdin stays open, as a live ssh session's does.

use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::net::UnixListener;
use tokio::process::Command;

#[tokio::test]
async fn the_relay_exits_when_the_daemon_hangs_up_though_stdin_is_open() {
    let home = tempfile::Builder::new().prefix("fc").tempdir().expect("tempdir");
    let socket = home.path().join("farcoolerd.sock");
    let listener = UnixListener::bind(&socket).expect("bind the daemon's socket path");
    let hang_up = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("the relay connects");
        let mut stream = stream;
        // The preamble line, then a pause long enough for the relay to be
        // parked reading its stdin (a session hung up on at once never reads
        // it, and ends cleanly), then close.
        let mut byte = [0u8; 1];
        let _ = stream.read(&mut byte).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        drop(stream);
    });

    let mut relay = Command::new(env!("CARGO_BIN_EXE_farcoolerd"))
        .args(["--stdio", "--client", "phone-7", "--scope", "read"])
        .env("FARCOOLER_HOME", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn the relay");
    // Held open for the whole test, as ssh holds it for a live session.
    let _stdin = relay.stdin.take();

    let exited = tokio::time::timeout(Duration::from_secs(10), relay.wait()).await;
    hang_up.await.expect("the listener ran");
    let status = exited.expect("the relay is still running 10 s after the daemon hung up, so ssh never closes the device's session").expect("wait");
    assert!(status.success(), "{status:?}");
}
