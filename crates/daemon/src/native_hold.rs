//! The hold: the one process allowed to signal a native agent (ov-1330).
//!
//! A native agent used to be ended by finding its processes afterwards, by tty and pid, and proving they were still
//! the agent's. Six reviews of ov-1319 and ov-1325 each found a way that proof could be stale (a reused tty, a reused
//! pid, a parent-child edge read across two incarnations of one pid) and a signal could reach someone else.
//!
//! So nothing is found any more. tmux runs `farcoolerd --native-hold <socket> -- <command>` in the agent's pane. The
//! hold starts `<command>` as its own child, in a new process group led by that child, and never reaps the child
//! until the group is empty. A pid is not reused while its process is unreaped, nor while it is in use as a process
//! group id, on Linux or macOS, so from `fork` until the hold reaps the child, `killpg(leader, _)` reaches this group
//! and no other. Every signal the engine sends a native agent is that call, made here, in [`signal_group`].
//!
//! The daemon never signals the agent. It asks the hold over `<socket>` to `stop`, `cont` or `end`. The hold reads
//! the process table only to wait (is anything left in the group, is all of it stopped); while the leader is
//! unreaped a row with its group id is one of ours, so a stale read can only make it wait longer.
//!
//! When the agent exits by itself, or tmux hangs the pane up, the hold ends the group just as `end` does, removes its
//! socket and exits with the agent's status. The socket is removed only after the group is empty, so a socket that
//! is gone is a hold that finished its work.
//!
//! Out of reach: a process that leaves the group (`setsid`, `setpgid`). `end` sends `SIGTERM` first and waits a
//! second, so an agent can end what it started itself (as their parent it can do so without a race), then kills.
//! See reports/ov-1330/design.md.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The `farcoolerd` flag that runs a hold.
pub const FLAG: &str = "--native-hold";

/// How long `end` lets the agent end its own work after `SIGTERM` before the group is killed.
const GRACE: Duration = Duration::from_secs(1);

/// `farcoolerd --native-hold <socket> -- <program> [args…]`, if that is how this process was started: run the hold
/// and return the exit status for `main`.
pub fn from_args() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some(FLAG) { return None; }
    let (Some(socket), Some("--")) = (args.get(2), args.get(3).map(String::as_str)) else {
        eprintln!("usage: farcoolerd {FLAG} <socket> -- <program> [args…]");
        return Some(125);
    };
    if args.len() < 5 { eprintln!("{FLAG}: no program to hold"); return Some(125); }
    Some(serve(Path::new(socket), &args[4..]))
}

/// The pane command that runs `command` (a `/bin/sh -c` script) under a hold listening on `socket`.
pub fn pane_command(hold: &Path, socket: &Path, script: &str) -> String {
    use farcooler_core::shell::shell_quote as q;
    format!("exec {} {FLAG} {} -- /bin/sh -c {}", q(&hold.to_string_lossy()), q(&socket.to_string_lossy()), q(script))
}

/// The pid `fork` returned to the hold. Only `spawn` makes one, and the hold never reaps it except in `end`, under
/// the same lock as every signal, so while a `Leader` is reachable through an unreaped [`Held`] its pid and group id
/// are this agent's.
struct Leader(i32);
struct Held { leader: Leader, reaped: Option<i32> }

/// The only signal call in the native engine (ov-1330, enforced by `the_native_engine_signals_only_its_own_group`).
/// Nothing once the leader is reaped: its number may belong to someone else from then on.
fn signal_group(held: &Held, signal: i32) -> bool {
    if held.reaped.is_some() || held.leader.0 <= 1 { return false; }
    // SAFETY: `killpg` takes no pointers. The group is pinned by the unreaped leader (module docs).
    unsafe { libc::killpg(held.leader.0, signal) == 0 }
}

static HANGUP: AtomicBool = AtomicBool::new(false);
extern "C" fn on_hangup(_: libc::c_int) { HANGUP.store(true, Ordering::SeqCst); }

/// Run the hold: listen on `socket`, start `argv` in its own group, and answer until the group has ended.
pub fn serve(socket: &Path, argv: &[String]) -> i32 {
    // Owner-only from the moment it exists; restored before the agent inherits the mask.
    let mask = unsafe { libc::umask(0o177) };
    let listener = UnixListener::bind(socket);
    unsafe { libc::umask(mask); }
    let listener = match listener {
        Ok(listener) => listener,
        Err(error) => { eprintln!("{FLAG}: cannot listen on {}: {error}", socket.display()); return 125; }
    };
    for signal in [libc::SIGHUP, libc::SIGTERM] {
        // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
        unsafe { libc::signal(signal, on_hangup as *const () as libc::sighandler_t); }
    }
    let leader = match spawn(argv) {
        Ok(pid) => pid,
        Err(error) => {
            eprintln!("{FLAG}: cannot start {}: {error}", argv[0]);
            let _ = std::fs::remove_file(socket);
            return 127;
        }
    };
    let held = Arc::new(Mutex::new(Held { leader: Leader(leader), reaped: None }));
    let socket: PathBuf = socket.to_path_buf();
    {
        let (held, socket) = (held.clone(), socket.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (held, socket) = (held.clone(), socket.clone());
                std::thread::spawn(move || answer(stream, &held, &socket));
            }
        });
    }
    loop {
        if HANGUP.load(Ordering::SeqCst) || exited(leader) {
            let mut held = lock(&held);
            let code = end(&mut held);
            finish(&socket, code);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn lock(held: &Mutex<Held>) -> std::sync::MutexGuard<'_, Held> { held.lock().unwrap_or_else(std::sync::PoisonError::into_inner) }

/// Remove the socket, only now that the group is empty, and exit with the agent's status.
fn finish(socket: &Path, code: i32) -> ! {
    let _ = std::fs::remove_file(socket);
    std::process::exit(code)
}

/// One request: a line naming the operation, answered with one line.
fn answer(stream: UnixStream, held: &Mutex<Held>, socket: &Path) {
    if !same_user(&stream) { return; }
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() { return; }
    let mut held = lock(held);
    let reply = match line.trim() {
        "ready" if held.reaped.is_none() => "ready",
        "stopped" if held.reaped.is_none() => if group_stopped(&held) { "stopped" } else { "unsettled" },
        "stop" if held.reaped.is_none() => if stop(&held) { "stopped" } else { "unsettled" },
        "cont" if held.reaped.is_none() => { signal_group(&held, libc::SIGCONT); "continued" }
        "end" => {
            let code = end(&mut held);
            // Gone before the answer, so whoever reads `ended` also finds the socket gone.
            let _ = std::fs::remove_file(socket);
            let _ = (&stream).write_all(b"ended\n");
            finish(socket, code);
        }
        "ready" | "stop" | "stopped" | "cont" => "ended",
        _ => "unknown",
    };
    let _ = (&stream).write_all(format!("{reply}\n").as_bytes());
}

#[cfg(target_os = "macos")]
fn same_user(stream: &UnixStream) -> bool {
    use std::os::fd::AsRawFd;
    let (mut uid, mut gid) = (0, 0);
    unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) == 0 && uid == libc::geteuid() }
}
#[cfg(not(target_os = "macos"))]
fn same_user(stream: &UnixStream) -> bool {
    use std::os::fd::AsRawFd;
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    unsafe {
        libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&raw mut cred).cast(), &mut len) == 0
            && cred.uid == libc::geteuid()
    }
}

/// Start `argv` as the leader of a new process group, in the terminal's foreground.
fn spawn(argv: &[String]) -> std::io::Result<i32> {
    use std::os::unix::process::CommandExt;
    let mut command = std::process::Command::new(&argv[0]);
    command.args(&argv[1..]);
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) != 0 { return Err(std::io::Error::last_os_error()); }
            // A background group may not take the terminal unless SIGTTOU is ignored; the default is restored for
            // the agent. No terminal (a test's pipe) is not an error: there is nothing to be in front of.
            libc::signal(libc::SIGTTOU, libc::SIG_IGN);
            libc::tcsetpgrp(0, libc::getpid());
            libc::signal(libc::SIGTTOU, libc::SIG_DFL);
            libc::signal(libc::SIGHUP, libc::SIG_DFL);
            libc::signal(libc::SIGTERM, libc::SIG_DFL);
            Ok(())
        });
    }
    // `spawn` returns after the child has exec'd, so the group exists before anything can be asked of it.
    let child = command.spawn()?;
    Ok(child.id() as i32)
}

/// Whether the leader has exited, without reaping it.
fn exited(leader: i32) -> bool {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // SAFETY: `info` is a valid siginfo_t; WNOWAIT leaves the child waitable, so its pid stays pinned.
    let got = unsafe { libc::waitid(libc::P_PID, leader as libc::id_t, &mut info, libc::WEXITED | libc::WNOHANG | libc::WNOWAIT) };
    // macOS answers a stopped child here too, though only WEXITED was asked for: a stop is not an exit.
    got == 0 && info.si_signo == libc::SIGCHLD && matches!(info.si_code, libc::CLD_EXITED | libc::CLD_KILLED | libc::CLD_DUMPED)
}

/// Every process in the leader's group but the leader, or `None` when the table can't be read whole.
fn members(leader: i32) -> Option<Vec<i32>> {
    let table = crate::proc_table::snapshot()?;
    let me = std::process::id() as i32;
    let mut found = Vec::new();
    let mut complete = false;
    for row in table.lines() {
        let mut columns = row.split_whitespace().map(str::parse::<i32>);
        let (Some(Ok(pid)), Some(Ok(_)), Some(Ok(pgid))) = (columns.next(), columns.next(), columns.next()) else { continue };
        complete |= pid == me;
        if pgid == leader && pid != leader { found.push(pid); }
    }
    complete.then_some(found)
}

fn group_stopped(held: &Held) -> bool {
    let leader = held.leader.0;
    crate::proc_table::stopped_or_gone(leader)
        && members(leader).is_some_and(|rest| rest.iter().all(|&pid| crate::proc_table::stopped_or_gone(pid)))
}

/// Stop the group and wait until the kernel has stopped every member: a stop only requests it.
fn stop(held: &Held) -> bool {
    for _ in 0..100 {
        signal_group(held, libc::SIGSTOP);
        if group_stopped(held) { return true; }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// End the group and reap the leader: SIGTERM, a grace for the agent to end its own work, then SIGKILL until two
/// reads of the table in a row find no member left. Returns the agent's exit status, as a shell reports it.
fn end(held: &mut Held) -> i32 {
    if let Some(code) = held.reaped { return code; }
    let leader = held.leader.0;
    signal_group(held, libc::SIGCONT);
    signal_group(held, libc::SIGTERM);
    let started = Instant::now();
    let mut empty = 0;
    while empty < 2 {
        if started.elapsed() >= GRACE { signal_group(held, libc::SIGKILL); }
        empty = match members(leader) { Some(rest) if rest.is_empty() && exited(leader) => empty + 1, _ => 0 };
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut status = 0;
    // SAFETY: `status` is a valid int. The leader has exited (above), so this returns at once.
    unsafe { libc::waitpid(leader, &mut status, 0); }
    let code = if libc::WIFEXITED(status) { libc::WEXITSTATUS(status) } else { 128 + libc::WTERMSIG(status) };
    held.reaped = Some(code);
    code
}

// A successful signal only requests a stop. Confirm the kernel observed it. Observation only: what this reads is
// never signalled.
#[cfg(target_os = "macos")]
pub(crate) fn stopped(pid: i32) -> bool {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as i32;
    let got = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size) };
    (got == size && matches!(info.pbi_status, libc::SSTOP | libc::SZOMB))
        || (got <= 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH))
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn stopped(pid: i32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat.rsplit_once(')').and_then(|(_, s)| s.trim_start().chars().next())
            .is_some_and(|state| matches!(state, 'T' | 't' | 'Z' | 'X')),
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}

/// What a hold said.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    /// Its one-word reply.
    Said(String),
    /// Its socket is gone: the hold ended its group and exited (or never started, which the caller rules out).
    Gone,
    /// Its socket is there and nobody answers: the hold was killed before it could end its group.
    Silent,
}

/// Ask the hold at `socket` to do `op`, waiting up to `wait` for its answer. Blocking; see [`ask`].
pub(crate) fn ask_blocking(socket: &Path, op: &str, wait: Duration) -> Answer {
    let gone = || if socket.exists() { Answer::Silent } else { Answer::Gone };
    let Ok(mut stream) = UnixStream::connect(socket) else { return gone() };
    let _ = stream.set_read_timeout(Some(wait));
    let _ = stream.set_write_timeout(Some(wait));
    if stream.write_all(format!("{op}\n").as_bytes()).is_err() { return gone(); }
    let mut line = String::new();
    match BufReader::new(&stream).read_line(&mut line) {
        Ok(n) if n > 0 => Answer::Said(line.trim().to_owned()),
        // Ended between our connect and its reply: the socket is removed before it exits.
        _ => gone(),
    }
}

/// Ask without waiting for the answer: a landing gate pausing builders holds a lock, and the hold acts on a request
/// it has read whether or not anyone stays for the reply.
pub(crate) fn tell(socket: &Path, op: &str) {
    if let Ok(mut stream) = UnixStream::connect(socket) {
        let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
        let _ = stream.write_all(format!("{op}\n").as_bytes());
    }
}

/// [`ask_blocking`] off the async runtime's threads.
pub(crate) async fn ask(socket: &Path, op: &'static str, wait: Duration) -> Answer {
    let socket = socket.to_path_buf();
    tokio::task::spawn_blocking(move || ask_blocking(&socket, op, wait)).await.unwrap_or(Answer::Silent)
}

#[cfg(test)]
#[path = "native_hold_tests.rs"]
mod tests;
