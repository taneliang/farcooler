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
//! A process that leaves the group (`setsid`, `setpgid`) is out of the hold's reach, and the hold never signals it.
//! It is not out of sight: [`Lineage`] follows every descendant of the hold, in the group or not. `end` sends
//! `SIGTERM` first and waits a second, so an agent can end what it started itself (as their parent it can do so
//! without a race), then kills the group. It answers `ended`, and removes its socket, only once nothing descended
//! from it is alive. A descendant that outlives the group is waited for, and if it is still working the hold answers
//! `unsettled` and keeps its socket, so the daemon refuses to settle. See reports/ov-1330/design.md.
//!
//! Requests are answered one at a time, in the order their clients connected, so a `stop` asked before a `cont` is
//! done before it even when its words arrive later (a landing gate tells and does not wait).

use std::collections::HashSet;
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
/// How long `end` waits, once the group is empty, for descendants that left it before it answers `unsettled`.
const STRAY_WAIT: Duration = Duration::from_secs(5);
/// How often the hold learns its lineage while the agent runs. Only macOS learns: see [`Lineage`].
const LEARN_EVERY: Duration = Duration::from_millis(100);

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
struct Held { leader: Leader, reaped: Option<i32>, lineage: Lineage }

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
    // An orphaned descendant is reparented to the hold, not to init, so it stays in sight (`Lineage`).
    #[cfg(target_os = "linux")]
    // SAFETY: `prctl` with integer arguments only.
    unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0); }
    for signal in [libc::SIGHUP, libc::SIGTERM] {
        // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
        unsafe { libc::signal(signal, on_hangup as *const () as libc::sighandler_t); }
    }
    let Some(mut lineage) = Lineage::new() else {
        eprintln!("{FLAG}: cannot read this process's own identity");
        let _ = std::fs::remove_file(socket);
        return 125;
    };
    let leader = match spawn(argv) {
        Ok(pid) => pid,
        Err(error) => {
            eprintln!("{FLAG}: cannot start {}: {error}", argv[0]);
            let _ = std::fs::remove_file(socket);
            return 127;
        }
    };
    lineage.look(leader);
    let held = Arc::new(Mutex::new(Held { leader: Leader(leader), reaped: None, lineage }));
    let socket: PathBuf = socket.to_path_buf();
    {
        let (held, socket) = (held.clone(), socket.clone());
        // One request at a time, on this thread, in the order the clients connected: a client that connected first
        // is answered first even if its words arrive after a later client's.
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() { answer(stream, &held, &socket); }
        });
    }
    let mut learned = Instant::now();
    loop {
        if HANGUP.load(Ordering::SeqCst) || exited(leader) {
            if let Ended::Settled(code) = end(&mut lock(&held), Duration::ZERO) { finish(&socket, code); }
            // Something that left the group is still alive. Keep the socket, so nobody takes its absence for an
            // ended agent, and look again.
            std::thread::sleep(Duration::from_millis(250));
            continue;
        }
        if Lineage::LEARNS && learned.elapsed() >= LEARN_EVERY {
            lock(&held).lineage.look(leader);
            learned = Instant::now();
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
        "end" => match end(&mut held, STRAY_WAIT) {
            Ended::Settled(code) => {
                // Gone before the answer, so whoever reads `ended` also finds the socket gone.
                let _ = std::fs::remove_file(socket);
                let _ = (&stream).write_all(b"ended\n");
                finish(socket, code);
            }
            Ended::Unsettled => "unsettled",
        },
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

/// What `end` proved.
enum Ended {
    /// The group is empty and nothing descended from the hold is alive; the leader is reaped. The agent's exit status,
    /// as a shell reports it.
    Settled(i32),
    /// The group is empty, but a descendant that left it was still alive `wait` later. The leader stays unreaped.
    Unsettled,
}

/// End the group, then wait for everything descended from the hold: SIGTERM, a grace for the agent to end its own
/// work, then SIGKILL until two looks in a row find no member of the group and no descendant alive. Only the group is
/// signalled. A descendant that left it is waited for, up to `wait` once the group is empty; if it is still alive then
/// the answer is `Unsettled`, and a later `end` looks again.
fn end(held: &mut Held, wait: Duration) -> Ended {
    if let Some(code) = held.reaped { return Ended::Settled(code); }
    let leader = held.leader.0;
    signal_group(held, libc::SIGCONT);
    signal_group(held, libc::SIGTERM);
    let started = Instant::now();
    let mut emptied: Option<Instant> = None;
    let mut clear = 0;
    while clear < 2 {
        if started.elapsed() >= GRACE { signal_group(held, libc::SIGKILL); }
        let look = held.lineage.look(leader);
        let group_gone = look.as_ref().is_some_and(|l| l.group == 0) && exited(leader);
        match look {
            Some(l) if group_gone && l.strays == 0 => clear += 1,
            Some(_) if group_gone => {
                clear = 0;
                if emptied.get_or_insert_with(Instant::now).elapsed() >= wait { return Ended::Unsettled; }
            }
            _ => clear = 0,
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut status = 0;
    // SAFETY: `status` is a valid int. The leader has exited (above), so this returns at once.
    unsafe { libc::waitpid(leader, &mut status, 0); }
    let code = if libc::WIFEXITED(status) { libc::WEXITSTATUS(status) } else { 128 + libc::WTERMSIG(status) };
    held.reaped = Some(code);
    Ended::Settled(code)
}

/// Everything descended from the hold, in the agent's group or not. Found by looking, so only ever waited for:
/// nothing a look finds is signalled.
///
/// macOS records, for every process, the kernel's unique id of the process that forked it (`p_puniqueid`), and keeps
/// it when the process is reparented to launchd. Unique ids are 64-bit and never reused, so an id once learned to be
/// ours stays ours, and `known` only grows; the hold learns while the agent runs, so a descendant whose parent has
/// since exited is still recognised. Linux has no such id, but there the hold is a child subreaper: an orphaned
/// descendant is reparented to the hold, never to init, so every live descendant has a chain of live parents back to
/// the hold, and each look starts afresh from the hold's pid (a pid is no identity across looks there).
struct Lineage { me: u64, known: HashSet<u64> }

/// A live process: its pid, group, identity and its parent's identity (unique ids on macOS, pids on Linux).
struct Row { pid: i32, pgid: i32, id: u64, parent: u64 }

/// What one look found alive: members of the agent's group (the leader among them), and descendants outside it.
struct Look { group: usize, strays: usize }

impl Lineage {
    const LEARNS: bool = cfg!(target_os = "macos");

    /// `None` when the hold can't learn its own identity: it could then follow nothing.
    fn new() -> Option<Self> {
        let me = own_id()?;
        Some(Self { me, known: HashSet::from([me]) })
    }

    /// Look once, or `None` when the table can't be read whole. A look that misses a process can only see less of
    /// the lineage, never someone else's process as ours: an identity is learned only from its parent's.
    fn look(&mut self, leader: i32) -> Option<Look> {
        let rows = rows()?;
        if !Self::LEARNS { self.known = HashSet::from([self.me]); }
        loop {
            let before = self.known.len();
            for row in &rows { if self.known.contains(&row.parent) { self.known.insert(row.id); } }
            if self.known.len() == before { break; }
        }
        let me = std::process::id() as i32;
        let mut look = Look { group: 0, strays: 0 };
        for row in rows.iter().filter(|row| row.pid != me) {
            if row.pgid == leader { look.group += 1; } else if self.known.contains(&row.id) { look.strays += 1; }
        }
        Some(look)
    }
}

/// `struct proc_uniqidentifierinfo` and its flavor (`<sys/proc_info.h>`, private headers; not in `libc`).
#[cfg(target_os = "macos")]
#[repr(C)]
struct UniqueIds { uuid: [u8; 16], id: u64, parent: u64, version: i32, original_parent_version: i32, reserved: [u64; 2] }
#[cfg(target_os = "macos")]
const PROC_PIDUNIQIDENTIFIERINFO: i32 = 17;

#[cfg(target_os = "macos")]
fn unique_ids(pid: i32) -> Option<UniqueIds> {
    let mut ids: UniqueIds = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<UniqueIds>() as i32;
    // SAFETY: `ids` is `size` bytes, and the kernel writes at most that.
    let got = unsafe { libc::proc_pidinfo(pid, PROC_PIDUNIQIDENTIFIERINFO, 0, (&raw mut ids).cast(), size) };
    (got == size).then_some(ids)
}

#[cfg(target_os = "macos")]
fn own_id() -> Option<u64> { unique_ids(std::process::id() as i32).map(|ids| ids.id).filter(|&id| id != 0) }
#[cfg(not(target_os = "macos"))]
fn own_id() -> Option<u64> { Some(u64::from(std::process::id())) }

/// Every live (not zombie) process, or `None` unless the read found the hold itself.
#[cfg(target_os = "macos")]
fn rows() -> Option<Vec<Row>> {
    let mut pids = vec![0i32; 4096];
    let count = loop {
        let bytes = (pids.len() * std::mem::size_of::<i32>()) as i32;
        // SAFETY: `pids` holds `bytes` bytes.
        let count = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
        if count <= 0 { return None; }
        if (count as usize) < pids.len() { break count as usize; }
        pids.resize(pids.len() * 2, 0);
    };
    let me = std::process::id() as i32;
    let mut complete = false;
    let mut rows = Vec::with_capacity(count);
    for &pid in pids[..count].iter().filter(|&&pid| pid > 0) {
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of_val(&info) as i32;
        // SAFETY: `info` is `size` bytes.
        if unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size) } != size { continue; }
        if info.pbi_status == libc::SZOMB { continue; }
        let Some(ids) = unique_ids(pid) else { continue };
        complete |= pid == me;
        rows.push(Row { pid, pgid: info.pbi_pgid as i32, id: ids.id, parent: ids.parent });
    }
    complete.then_some(rows)
}
#[cfg(not(target_os = "macos"))]
fn rows() -> Option<Vec<Row>> {
    let me = std::process::id() as i32;
    let mut complete = false;
    let mut rows = Vec::new();
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse::<i32>().ok()) else { continue };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else { continue };
        let Some((_, rest)) = stat.rsplit_once(')') else { continue };
        let mut fields = rest.split_whitespace();
        let (Some(state), Some(Ok(ppid)), Some(Ok(pgid))) =
            (fields.next(), fields.next().map(str::parse::<u32>), fields.next().map(str::parse::<i32>)) else { continue };
        if matches!(state, "Z" | "X" | "x") { continue; }
        complete |= pid == me;
        rows.push(Row { pid, pgid, id: u64::from(pid as u32), parent: u64::from(ppid) });
    }
    complete.then_some(rows)
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
