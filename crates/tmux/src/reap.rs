//! Take a tmux server down by its process id, with every wait bounded.
//!
//! Tests start real tmux servers and tear them down in `Drop`. A bare
//! `tmux kill-server` there trusts the server to answer. On 3 Oct a test
//! server and its `kill-server` client spun at about 92% CPU each for 15.5
//! hours: SIGTERM did nothing, SIGKILL ended it. So teardown asks politely
//! for a bounded time, then ends the process by PID: SIGTERM, then SIGKILL.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long `kill-server`, or any other question put to the server, may take.
const ASK: Duration = Duration::from_secs(3);
/// How long the server gets to exit after each signal before the next one.
const GRACE: Duration = Duration::from_secs(2);

/// Run `cmd` to completion, but for no longer than `limit`; a command that
/// outlasts it is killed. `None` on a timeout, a spawn failure or a failure
/// exit; otherwise its stdout.
fn run_bounded(mut cmd: Command, limit: Duration) -> Option<String> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut out = String::new();
                if let Some(mut pipe) = child.stdout.take() {
                    std::io::Read::read_to_string(&mut pipe, &mut out).ok()?;
                }
                return status.success().then_some(out);
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// Where tmux puts the socket for `-L <name>`: `$TMUX_TMPDIR` (or `/tmp`),
/// then `tmux-<uid>`.
fn socket_path(socket: &str, tmpdir: Option<&std::path::Path>) -> PathBuf {
    let base = tmpdir
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("TMUX_TMPDIR").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    // SAFETY: getuid has no preconditions and cannot fail.
    base.join(format!("tmux-{}", unsafe { libc::getuid() })).join(socket)
}

/// The server's PID. Asked of the server first; a hung one does not answer,
/// so the fallback is whoever has the socket open.
fn server_pid(tmux: &std::path::Path, socket: &str, tmpdir: Option<&std::path::Path>, waits: Waits) -> Option<i32> {
    let mut ask = Command::new(tmux);
    with_tmpdir(&mut ask, tmpdir);
    ask.args(["-L", socket, "display-message", "-p", "#{pid}"]);
    if let Some(pid) = run_bounded(ask, waits.ask).and_then(|s| s.trim().parse().ok()) {
        return Some(pid);
    }
    // Canonical, because lsof matches the resolved name: `/tmp` is a link to
    // `/private/tmp` on macOS and a bare `/tmp/...` finds nothing.
    let path = socket_path(socket, tmpdir).canonicalize().ok()?;
    let mut lsof = Command::new("lsof");
    lsof.arg("-t").arg("--").arg(path);
    run_bounded(lsof, ASK)?.lines().next()?.trim().parse().ok()
}

fn with_tmpdir(cmd: &mut Command, tmpdir: Option<&std::path::Path>) {
    if let Some(dir) = tmpdir {
        cmd.env("TMUX_TMPDIR", dir);
    }
}

fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

fn wait_dead(pid: i32, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    while alive(pid) {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}

/// End the tmux server on `-L <socket>`, and do not return while it lives.
///
/// Blocking, because `Drop` cannot await. Asks the server to `kill-server`
/// for up to 3 s, then SIGTERMs its PID, then SIGKILLs it, 2 s apart. A
/// server that survives SIGKILL panics (unless the thread already is), so a
/// leak fails the test that made it. No server on that socket is not an error.
pub fn reap_server(socket: &str) {
    reap_server_in(socket, None);
}

/// The same, for a server started with its own `TMUX_TMPDIR`.
///
/// A test that sets `TMUX_TMPDIR` on the daemon child only (not on itself)
/// must say where the socket is, or the lookup falls to `/tmp`, finds no
/// server, and the real one outlives the test (ov-985).
pub fn reap_server_in(socket: &str, tmpdir: Option<&std::path::Path>) {
    reap_within(socket, tmpdir, Waits { ask: ASK, grace: GRACE });
}

/// How long the server is asked, and how long it gets after each signal.
#[derive(Clone, Copy)]
struct Waits {
    ask: Duration,
    grace: Duration,
}

/// `reap_server_in` with its waits given: a test of a server that answers
/// nothing spends every one of them, so it passes shorter ones (ov-993).
fn reap_within(socket: &str, tmpdir: Option<&std::path::Path>, waits: Waits) {
    let Some(tmux) = farcooler_core::programs::find("tmux") else { return };
    let pid = server_pid(&tmux, socket, tmpdir, waits);
    let mut kill = Command::new(&tmux);
    with_tmpdir(&mut kill, tmpdir);
    kill.args(["-L", socket, "kill-server"]);
    let _ = run_bounded(kill, waits.ask);
    let Some(pid) = pid else { return };
    if wait_dead(pid, waits.grace) {
        return;
    }
    for signal in [libc::SIGTERM, libc::SIGKILL] {
        // SAFETY: a signal to a PID we read from this socket's own server.
        unsafe { libc::kill(pid, signal) };
        if wait_dead(pid, waits.grace) {
            return;
        }
    }
    if !std::thread::panicking() {
        panic!("tmux server {pid} on {socket} survived SIGKILL");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Start a real server on a fresh socket, or `None` without tmux.
    fn started(tag: &str) -> Option<String> {
        let real = farcooler_core::programs::find("tmux")?;
        let socket = format!("reap-{tag}-{}", std::process::id());
        let ok = Command::new(&real)
            .args(["-L", &socket, "-f", "/dev/null", "new-session", "-d", "-s", "x", "sleep 600"])
            .status()
            .ok()?;
        ok.success().then_some(socket)
    }

    /// The server's PID, read back from the server.
    fn pid_of(socket: &str) -> i32 {
        let tmux = farcooler_core::programs::find("tmux").unwrap();
        let out = Command::new(tmux).args(["-L", socket, "display-message", "-p", "#{pid}"]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
    }

    #[test]
    fn a_running_server_is_gone_when_reap_returns() {
        let Some(socket) = started("plain") else { return };
        let pid = pid_of(&socket);
        reap_server(&socket);
        assert!(!alive(pid), "server {pid} still running after reap_server");
    }

    #[test]
    fn a_server_that_ignores_kill_server_is_ended_by_pid() {
        let Some(socket) = started("stopped") else { return };
        let pid = pid_of(&socket);
        // A stopped server answers nothing, as the hung one did: its clients
        // wait forever, and SIGTERM stays pending until it is continued.
        // SAFETY: SIGSTOP to the server this test just started.
        unsafe { libc::kill(pid, libc::SIGSTOP) };
        let started = Instant::now();
        // Every wait is spent here, so shorter ones than production's 3 s
        // asks and 2 s graces: the path through them is the same. `lsof`,
        // the fallback that finds the PID, keeps `ASK`.
        let waits = Waits { ask: Duration::from_millis(500), grace: Duration::from_millis(300) };
        reap_within(&socket, None, waits);
        assert!(!alive(pid), "a stopped server {pid} outlived reap_server");
        assert!(started.elapsed() < Duration::from_secs(20), "teardown was not bounded: {:?}", started.elapsed());
    }

    #[test]
    fn no_server_is_not_an_error() {
        reap_server("reap-never-started");
    }
}
