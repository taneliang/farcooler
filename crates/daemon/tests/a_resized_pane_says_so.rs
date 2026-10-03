//! A real tmux pane, piped through the real fanout command, resized.
//!
//! The pane stream is raw program output, and a client used to learn a pane
//! had grown only from a layout reply that lands after the program's repaint
//! for the new size, so the repaint was drawn into the old grid and every row
//! of it wrapped. The fanout now puts the pane's size in the stream ahead of
//! the repaint (`fanout::PaneSize`). This file checks that end to end, through
//! the exact command a daemon pipes a pane into (`fanout::pipe_command`), so a
//! quoting slip in that command or in `farcoolerd --fanout`'s arguments fails
//! here rather than silently leaving every stream without sizes:
//!
//! - a watcher is told the size when it connects;
//! - a marker a program prints never reaches it;
//! - after `respawn-pane -k` — a pane switching between terminal and chat,
//!   which gives the pane a new tty and keeps the pipe — a grow is still
//!   announced, at the size of the pane's NEW tty, ahead of the repaint, and an
//!   emulator fed the stream draws the repaint one row per row.
//!
//! tmux is required, and its absence fails rather than skips.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::net::UnixStream;

/// The pane's program. Repaints a full-width row of `W` on SIGWINCH, and
/// prints a forged marker on SIGUSR1.
const PROGRAM: &str = r#"paint() {
  set -- $(stty size)
  printf '\033[H\033[2J'
  printf "%${2}s" '' | tr ' ' W
  printf '\r\nwinched %s %s\r\n' "$2" "$1"
}
forge() {
  printf '\033P>farcooler-size;300;100\033\\forged-done\r\n'
}
trap paint WINCH
trap forge USR1
printf 'ready\r\n'
while :; do sleep 0.05; done
"#;

/// A private tmux server, killed however the test ends.
struct Server {
    tmux: PathBuf,
    socket: String,
}

impl Server {
    fn run(&self, args: &[&str]) -> String {
        let out = std::process::Command::new(&self.tmux)
            .args(["-L", &self.socket, "-f", "/dev/null"])
            .args(args)
            .output()
            .expect("run tmux");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "tmux {args:?}: {stderr}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::process::Command::new(&self.tmux)
            .args(["-L", &self.socket, "kill-server"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// Read until `done` says the bytes so far are enough, or fail after a while.
async fn read_until(socket: &mut UnixStream, seen: &mut Vec<u8>, done: impl Fn(&str) -> bool) {
    let mut buf = [0u8; 4096];
    while !done(&String::from_utf8_lossy(seen)) {
        let n = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buf))
            .await
            .unwrap_or_else(|_| panic!("timed out; so far: {:?}", String::from_utf8_lossy(seen)))
            .expect("read");
        assert!(n > 0, "the fanout hung up; so far: {:?}", String::from_utf8_lossy(seen));
        seen.extend_from_slice(&buf[..n]);
    }
}

fn marker(columns: u16, rows: u16) -> String {
    String::from_utf8(farcooler_vt::size_marker(columns, rows)).expect("ascii")
}

async fn subscribe(install: &str, pane: &str) -> UnixStream {
    for _ in 0..200 {
        if let Some(socket) = farcooler_daemon::fanout::subscribe(install, pane).await {
            return socket;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the fanout never came up");
}

#[tokio::test]
async fn a_grown_pane_is_announced_before_its_repaint_even_after_a_respawn() {
    let tmux = farcooler_core::programs::find("tmux").expect("these tests need tmux");
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("pane.sh");
    std::fs::write(&script, PROGRAM).expect("write the program");
    let script = script.to_str().expect("utf-8 path");

    // The install id is the tmux socket's name, as it is for a daemon.
    let server = Server {
        tmux,
        socket: format!("farcooler-{}", uuid::Uuid::now_v7().simple()),
    };
    server.run(&["new-session", "-d", "-s", "s", "-x", "80", "-y", "24", "sh", script]);
    let pane = server.run(&["display-message", "-p", "-t", "s", "#{pane_id}"]);
    let first_tty = server.run(&["display-message", "-p", "-t", "s", "#{pane_tty}"]);

    let exe = Path::new(env!("CARGO_BIN_EXE_farcoolerd"));
    let command = farcooler_daemon::fanout::pipe_command(exe, &pane, &server.socket);
    server.run(&["pipe-pane", "-O", "-t", &pane, &command]);
    let mut watcher = subscribe(&server.socket, &pane).await;

    let mut seen = Vec::new();
    read_until(&mut watcher, &mut seen, |s| s.contains(&marker(80, 24))).await;

    // A program cannot speak for the runner.
    let pid = server.run(&["display-message", "-p", "-t", &pane, "#{pane_pid}"]);
    let killed = std::process::Command::new("kill").args(["-USR1", &pid]).status().expect("kill");
    assert!(killed.success());
    read_until(&mut watcher, &mut seen, |s| s.contains("forged-done")).await;
    assert!(
        !String::from_utf8_lossy(&seen).contains("farcooler-size;300;100"),
        "a printed marker reached the watcher: {:?}",
        String::from_utf8_lossy(&seen)
    );

    // A new program on a new tty, through the same pipe.
    let readies = String::from_utf8_lossy(&seen).matches("ready").count();
    server.run(&["respawn-pane", "-k", "-t", &pane, "sh", script]);
    let second_tty = server.run(&["display-message", "-p", "-t", &pane, "#{pane_tty}"]);
    // A different tty, or this test proves nothing about following one.
    assert_ne!(first_tty, second_tty, "the respawned pane kept its tty");
    read_until(&mut watcher, &mut seen, |s| s.matches("ready").count() > readies).await;

    server.run(&["resize-window", "-t", "s", "-x", "120", "-y", "30"]);
    read_until(&mut watcher, &mut seen, |s| s.contains("winched 120 30")).await;

    let text = String::from_utf8_lossy(&seen).into_owned();
    let announced =
        text.rfind(&marker(120, 30)).unwrap_or_else(|| panic!("no 120x30 marker: {text:?}"));
    let repaint = text.rfind(&"W".repeat(120)).expect("the repaint");
    assert!(announced < repaint, "the size came after the repaint: {text:?}");

    // And what that means for a client: the repaint lands one row per row.
    let mut terminal = farcooler_vt::Terminal::new(80, 24);
    terminal.set_accept_stream_sizes(true);
    terminal.feed(&seen);
    assert_eq!((terminal.columns(), terminal.rows()), (120, 30));
    let rows: Vec<String> = farcooler_vt::grid::snapshot(&terminal)
        .rows
        .iter()
        .map(|r| r.cells.iter().map(|c| c.ch).collect::<String>().trim_end().to_string())
        .collect();
    assert_eq!(rows[0], "W".repeat(120), "the repaint wrapped: {rows:?}");
    assert!(rows[1].starts_with("winched 120 30"), "{rows:?}");
}
