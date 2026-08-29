//! Spawning a real `farcoolerd`, shared by the tests that need one.
//!
//! Shared rather than copied because the interesting part is not the spawn: it
//! is the tmux server the daemon starts and the rule for reaping it. A second
//! copy of that rule is one to get quietly wrong, and getting it wrong leaks a
//! tmux server per test run.

// Each test binary uses a different subset of this, and the ones it does not
// use are not dead code — they are code the other binary uses.
#![allow(dead_code)]

use farcooler_transport::Client;
use tokio::process::{ChildStdin, ChildStdout, Command};

/// A spawned daemon, and the tmux server it will have started.
///
/// `kill_on_drop` ends the daemon and nothing else. The daemon's private tmux
/// server is a separate process, on a socket named after this runtime
/// directory's install id — so the moment the directory is deleted, nothing on
/// the machine can work out what that socket was called, and the server stays
/// up until the machine is restarted. One per tmux-using test, every run.
///
/// They are not idle passengers. tmux is single-threaded per server, and the
/// crowd that accumulated this way was measured slowing `capture-pane` against
/// the real fleet from 50ms to 740ms.
///
/// The same guard `rpc_over_socket.rs`'s `Harness` already carries, for the same
/// reason; this file never got one.
pub struct DaemonChild {
    pub child: tokio::process::Child,
    pub home: std::path::PathBuf,
}

impl Drop for DaemonChild {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        // Read rather than recomputed: how an install id becomes a socket name
        // is the daemon's rule, and a second copy of it here would be one to
        // get quietly wrong.
        let Ok(install) = std::fs::read_to_string(self.home.join("install-id")) else { return };
        let socket = format!("farcooler-{}", install.trim());
        let Some(tmux) = farcooler_core::programs::find("tmux") else { return };
        let _ = std::process::Command::new(tmux)
            .args(["-L", &socket, "kill-server"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// Spawn the daemon in stdio mode against a private runtime directory.
pub async fn spawn(dir: &std::path::Path) -> (DaemonChild, Client<ChildStdout, ChildStdin>) {
    spawn_with(dir, &[]).await
}

/// The same, with extra arguments — the ones a forced command in
/// `authorized_keys` would carry.
pub async fn spawn_with(
    dir: &std::path::Path,
    extra: &[&str],
) -> (DaemonChild, Client<ChildStdout, ChildStdin>) {
    spawn_command(stdio_command(dir, extra), dir).await
}

/// The same, with extra environment.
///
/// For a test that has to put its own `gh` ahead of the real one on `PATH`.
/// The daemon resolves that binary through the environment it was STARTED
/// with, and setting it in the test process would not do: the environment is
/// process-global and this suite runs in parallel, so a test that changed it
/// would change it under every other test in the binary. That is the same trap
/// `Harness` documents about `FARCOOLER_HOME`.
pub async fn spawn_with_env(
    dir: &std::path::Path,
    env: &[(&str, &str)],
) -> (DaemonChild, Client<ChildStdout, ChildStdin>) {
    let mut command = stdio_command(dir, &[]);
    for (key, value) in env {
        command.env(key, value);
    }
    spawn_command(command, dir).await
}

/// Spawn a prepared invocation and complete the handshake over its pipes.
async fn spawn_command(
    mut command: Command,
    dir: &std::path::Path,
) -> (DaemonChild, Client<ChildStdout, ChildStdin>) {
    let mut child = command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn farcoolerd --stdio");

    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let client = Client::over(stdout, stdin, "test-client", "0.0.0")
        .await
        .expect("handshake over stdio");
    (DaemonChild { child, home: dir.to_path_buf() }, client)
}

/// The stdio invocation, before its pipes are decided.
///
/// Separate so a test can spawn one it expects to REFUSE the session and read
/// the exit status, which the handshaking helpers above cannot do.
pub fn stdio_command(dir: &std::path::Path, extra: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_farcoolerd"));
    command
        .arg("--stdio")
        .args(extra)
        .env("FARCOOLER_HOME", dir)
        // Deliberately noisy: if any of this reaches stdout the handshake
        // breaks, which is exactly what `stdio_transport.rs` exists to catch.
        .env("RUST_LOG", "debug");
    command
}

/// A daemon holding the socket for this runtime directory, up and accepting.
///
/// Polled rather than slept on: binding happens after the service opens and the
/// inventory is taken, which is fast on an idle machine and not on a busy one,
/// and a fixed wait would be flaky in exactly the direction that wastes an hour.
pub async fn listening_daemon(dir: &std::path::Path) -> DaemonChild {
    let child = Command::new(env!("CARGO_BIN_EXE_farcoolerd"))
        .env("FARCOOLER_HOME", dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn farcoolerd");
    let daemon = DaemonChild { child, home: dir.to_path_buf() };

    let socket = dir.join("farcoolerd.sock");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        if tokio::net::UnixStream::connect(&socket).await.is_ok() {
            return daemon;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("no daemon ever accepted on {}", socket.display());
}
