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
//! - after a respawn — a pane switching between terminal and chat, which gives
//!   the pane a new pty and keeps the pipe — a grow is still announced, at the
//!   size of the pane's new pty, ahead of the repaint, and an emulator fed the
//!   stream draws the repaint one row per row.
//!
//! Whether that new pty has a new path is up to the OS: macOS hands out the
//! lowest free one, so a respawn lands on a new path when the old pty is
//! still open as the new one is made, and on the same path when it isn't. So
//! both are forced rather than left to the host. One holds the old tty open
//! across the respawn so the pane must move, then lets another terminal of the
//! same size take the old number before the pane writes a byte: the path the
//! pipe started on still reads the size last announced, so only asking tmux
//! about the pane (`fanout::PaneSize`) finds the move. Another frees the old
//! pty first and respawns onto it. A third respawns a program that ignores
//! the hangup: it lives on, holding the old tty at the size last announced,
//! so only the tty being hung up says the pane has gone.
//!
//! tmux is required, and its absence fails rather than skips.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::net::UnixStream;

/// The pane's program. Repaints a full-width row of `W` on SIGWINCH, and
/// prints a forged marker on SIGUSR1. Says `ready` when it starts, or, given
/// `quiet`, not until SIGUSR2. Given `stubborn`, ignores SIGHUP, as does each
/// `sleep` it starts.
const PROGRAM: &str = r#"paint() {
  set -- $(stty size)
  printf '\033[H\033[2J'
  printf "%${2}s" '' | tr ' ' W
  printf '\r\nwinched %s %s\r\n' "$2" "$1"
}
forge() {
  printf '\033P>farcooler-size;300;100\033\\forged-done\r\n'
}
greet() {
  printf 'ready\r\n'
}
trap paint WINCH
trap forge USR1
trap greet USR2
[ "$1" = stubborn ] && trap '' HUP
[ "$1" = quiet ] || greet
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

/// Where a respawn puts the pane.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Respawn {
    /// A new tty, and the old one's number then taken by another terminal
    /// of the same size before the pane writes a byte, so the path the pipe
    /// started on still reads the size last announced, from a stranger.
    OntoANewTty,
    /// The same tty: the old pty freed first, so the new one takes its number.
    OntoTheSameTty,
    /// A new tty, the old program ignoring the hangup and living on: its pid
    /// still exists, and the old tty still reads the size last announced.
    PastAProgramThatIgnoresTheHangup,
}

/// One respawn at a time. Each takes or frees pty numbers on purpose, and
/// another test's respawn taking the number one is waiting for would fail it.
static RESPAWNING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn a_grown_pane_is_announced_before_its_repaint_after_a_respawn_onto_a_new_tty() {
    let _one = RESPAWNING.lock().await;
    grown_after_a_respawn(Respawn::OntoANewTty).await;
}

#[tokio::test]
async fn a_grown_pane_is_announced_before_its_repaint_after_a_respawn_onto_the_same_tty() {
    let _one = RESPAWNING.lock().await;
    grown_after_a_respawn(Respawn::OntoTheSameTty).await;
}

#[tokio::test]
async fn a_grown_pane_is_announced_before_its_repaint_after_a_respawn_past_a_program_that_ignores_the_hangup() {
    let _one = RESPAWNING.lock().await;
    grown_after_a_respawn(Respawn::PastAProgramThatIgnoresTheHangup).await;
}

/// A process the test left running, killed however the test ends.
struct Lingering(libc::pid_t);

impl Drop for Lingering {
    fn drop(&mut self) {
        // SAFETY: a plain signal to a pid this test saw outlive its pane.
        unsafe { libc::kill(self.0, libc::SIGKILL) };
    }
}

/// Open `tty` without making it anyone's controlling terminal. Held, it keeps
/// the pty's number from being handed out again.
fn hold(tty: &str) -> std::fs::File {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK)
        .open(tty)
        .unwrap_or_else(|e| panic!("hold {tty}: {e}"))
}

async fn grown_after_a_respawn(how: Respawn) {
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
    let first = if how == Respawn::PastAProgramThatIgnoresTheHangup { "stubborn" } else { "" };
    server.run(&["new-session", "-d", "-s", "s", "-x", "80", "-y", "24", "sh", script, first]);
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

    // A new program through the same pipe, on the tty `how` says.
    let readies = String::from_utf8_lossy(&seen).matches("ready").count();
    let mut _lingering = None;
    let _old_tty = match how {
        Respawn::OntoANewTty => {
            let held = hold(&first_tty);
            server.run(&["respawn-pane", "-k", "-t", &pane, "sh", script, "quiet"]);
            let second_tty = server.run(&["display-message", "-p", "-t", &pane, "#{pane_tty}"]);
            assert_ne!(first_tty, second_tty, "a held tty was handed out again");
            drop(held);
            let stranger = take(&first_tty, 80, 24);
            let pid = server.run(&["display-message", "-p", "-t", &pane, "#{pane_pid}"]);
            let told = std::process::Command::new("kill").args(["-USR2", &pid]).status().expect("kill");
            assert!(told.success());
            Some(stranger)
        }
        Respawn::OntoTheSameTty => {
            respawn_onto_the_same_tty(&server, &pane, script).await;
            None
        }
        Respawn::PastAProgramThatIgnoresTheHangup => {
            let old: libc::pid_t = pid.parse().expect("a pid");
            server.run(&["respawn-pane", "-k", "-t", &pane, "sh", script]);
            // SAFETY: signal 0 only asks whether the process exists.
            let lives = unsafe { libc::kill(old, 0) } == 0;
            if lives {
                _lingering = Some(Lingering(old));
            }
            assert!(lives, "the old program did not outlive the respawn");
            let second_tty = server.run(&["display-message", "-p", "-t", &pane, "#{pane_tty}"]);
            assert_ne!(first_tty, second_tty, "a tty still held was handed out again");
            None
        }
    };
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

/// Respawn `pane` onto the tty it has now. The program is ended and the pane
/// left dead, so the old pty is closed before the new one is opened and the
/// lowest free number is its own again — unless something else on the host
/// opened a pty in between, so it is tried a few times.
async fn respawn_onto_the_same_tty(server: &Server, pane: &str, script: &str) {
    server.run(&["set-option", "-w", "-t", pane, "remain-on-exit", "on"]);
    for _ in 0..5 {
        let before = server.run(&["display-message", "-p", "-t", pane, "#{pane_tty}"]);
        let pid = server.run(&["display-message", "-p", "-t", pane, "#{pane_pid}"]);
        let killed = std::process::Command::new("kill").args(["-KILL", &pid]).status().expect("kill");
        assert!(killed.success());
        let mut dead = false;
        for _ in 0..500 {
            if server.run(&["display-message", "-p", "-t", pane, "#{pane_dead}"]) == "1" {
                dead = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(dead, "the pane never died");
        server.run(&["respawn-pane", "-t", pane, "sh", script]);
        if server.run(&["display-message", "-p", "-t", pane, "#{pane_tty}"]) == before {
            return;
        }
    }
    panic!("a respawned pane never landed on its old tty");
}

/// A pty of our own at `tty`, sized `columns` by `rows`: the terminal anyone
/// might open next, on the number a respawned pane let go of. Ptys are handed
/// out lowest number first, so every one below it is taken on the way and let
/// go once it is found.
fn take(tty: &str, columns: u16, rows: u16) -> (std::os::fd::OwnedFd, std::fs::File) {
    use std::os::fd::{FromRawFd, OwnedFd};
    let mut on_the_way = Vec::new();
    for _ in 0..256 {
        // SAFETY: plain libc calls on a descriptor this function owns; the
        // name `ptsname` returns is copied out before any other pty call.
        let (master, name) = unsafe {
            let fd = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
            assert!(fd >= 0, "posix_openpt: {}", std::io::Error::last_os_error());
            let master = OwnedFd::from_raw_fd(fd);
            assert_eq!(libc::grantpt(fd), 0, "grantpt");
            assert_eq!(libc::unlockpt(fd), 0, "unlockpt");
            let name = libc::ptsname(fd);
            assert!(!name.is_null(), "ptsname");
            (master, std::ffi::CStr::from_ptr(name).to_string_lossy().into_owned())
        };
        if name == tty {
            // Sized through its terminal side: macOS refuses it on the master.
            let size = rustix::termios::Winsize { ws_row: rows, ws_col: columns, ws_xpixel: 0, ws_ypixel: 0 };
            let terminal = hold(tty);
            rustix::termios::tcsetwinsize(&terminal, size).expect("size the stranger");
            return (master, terminal);
        }
        on_the_way.push(master);
    }
    panic!("{tty} was never handed out again");
}
