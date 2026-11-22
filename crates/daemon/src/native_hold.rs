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
//! The daemon never signals the agent. It asks the hold over `<socket>` to `pause`, `stop`, `cont` or `end`. The hold reads
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
//! `unsettled` and keeps its socket, so the daemon refuses to settle. On macOS a descendant is known by the kernel's
//! record of who is responsible for it (the hold, which a relay starts that way), so the hold need not have seen its
//! parents; on Linux the hold is a child subreaper. See reports/ov-1330/design.md.
//!
//! Requests are answered one at a time, in the order their clients connected, so a `stop` asked before a `cont` is
//! done before it even when its words arrive later (a landing gate tells and does not wait).

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
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
const PAUSE_ACK: [u8; 1] = *b"1";

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
struct Held {
    leader: Leader,
    reaped: Option<i32>,
    lineage: Lineage,
    gate_stop: bool,
}

/// The only signal call in the native engine (ov-1330, enforced by `the_native_engine_signals_only_its_own_group`).
/// Nothing once the leader is reaped: its number may belong to someone else from then on.
fn signal_group(held: &Held, signal: i32) -> bool {
    if held.reaped.is_some() || held.leader.0 <= 1 { return false; }
    // SAFETY: `killpg` takes no pointers. The group is pinned by the unreaped leader (module docs).
    unsafe { libc::killpg(held.leader.0, signal) == 0 }
}

unsafe extern "C" {
    fn farcooler_native_hold_install_signals();
    fn farcooler_native_hold_hangup_pending(clear: bool) -> bool;
}

/// Set in the environment of a hold that a relay started (macOS), and removed before the agent starts.
#[cfg(target_os = "macos")]
const RELAYED: &str = "FARCOOLER_NATIVE_HOLD_RELAYED";
/// Debug builds only: the hold learns no lineage, so the tests can show that responsibility alone finds a process
/// whose parent the hold never saw (macOS). Release builds never read it.
const BLIND: &str = "FARCOOLER_NATIVE_HOLD_BLIND_FOR_TEST";

/// Run the hold: listen on `socket`, start `argv` in its own group, and answer until the group has ended.
pub fn serve(socket: &Path, argv: &[String]) -> i32 {
    // macOS: the hold must be the process the kernel holds responsible for everything the agent starts (see
    // `responsibility`). tmux started this process, so it is not; it starts the real hold as a disclaimed child and
    // relays to it. A relayed hold that still is not responsible for itself cannot prove its agent ended: refuse.
    #[cfg(target_os = "macos")]
    if !responsibility::is_own() {
        if std::env::var_os(RELAYED).is_some() {
            eprintln!("{FLAG}: macOS did not make the hold responsible for its agent; it could not prove the agent ended");
            return 125;
        }
        return relay();
    }
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
    // SAFETY: this dedicated hold process owns its signal dispositions. The callback only stores to a lock-free atomic.
    unsafe { farcooler_native_hold_install_signals(); }
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
    let held = Arc::new(Mutex::new(Held { leader: Leader(leader), reaped: None, lineage, gate_stop: false }));
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
        // SAFETY: reads the C shim's lock-free flag without clearing it, so an unsettled hold retries cleanup.
        if unsafe { farcooler_native_hold_hangup_pending(false) } || exited(leader) {
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

/// The pane's process on macOS: start this same hold again as a child that disclaims responsibility, so the kernel
/// records that child as responsible for itself and everything it starts, then wait for it and exit with its status.
/// A hangup from tmux is passed on to it, and that is the relay's only signal: its target is the pid `posix_spawn`
/// returned, which the relay has not reaped, so it is still that child.
#[cfg(target_os = "macos")]
fn relay() -> i32 {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let Some(disclaim) = responsibility::disclaim() else {
        eprintln!("{FLAG}: this macOS cannot make the hold responsible for its agent");
        return 125;
    };
    let Ok(exe) = std::env::current_exe() else { eprintln!("{FLAG}: cannot find this program"); return 125; };
    let cstr = |bytes: &[u8]| CString::new(bytes).ok();
    let Some(path) = cstr(exe.as_os_str().as_bytes()) else { return 125 };
    let Some(args) = std::env::args_os().map(|a| cstr(a.as_bytes())).collect::<Option<Vec<_>>>() else { return 125 };
    let Some(mut env) = std::env::vars_os().filter(|(k, _)| k != RELAYED)
        .map(|(k, v)| cstr(&[k.as_bytes(), b"=", v.as_bytes()].concat())).collect::<Option<Vec<_>>>() else { return 125 };
    env.extend(cstr(format!("{RELAYED}=1").as_bytes()));
    let argv: Vec<*mut libc::c_char> = args.iter().map(|a| a.as_ptr().cast_mut()).chain([std::ptr::null_mut()]).collect();
    let envp: Vec<*mut libc::c_char> = env.iter().map(|e| e.as_ptr().cast_mut()).chain([std::ptr::null_mut()]).collect();
    // SAFETY: this dedicated relay process owns its signal dispositions. The callback only stores to a lock-free atomic.
    unsafe { farcooler_native_hold_install_signals(); }
    let mut hold: libc::pid_t = 0;
    // SAFETY: `attr` is initialised before use and destroyed after; every pointer outlives the call.
    let spawned = unsafe {
        let mut attr: libc::posix_spawnattr_t = std::ptr::null_mut();
        if libc::posix_spawnattr_init(&mut attr) != 0 { return 125; }
        let set = disclaim(&mut attr, 1);
        let spawned = if set == 0 { libc::posix_spawn(&mut hold, path.as_ptr(), std::ptr::null(), &attr, argv.as_ptr(), envp.as_ptr()) } else { set };
        libc::posix_spawnattr_destroy(&mut attr);
        spawned
    };
    if spawned != 0 { eprintln!("{FLAG}: cannot start the hold: {}", std::io::Error::from_raw_os_error(spawned)); return 125; }
    loop {
        let mut status = 0;
        // SAFETY: `status` is a valid int.
        if unsafe { libc::waitpid(hold, &mut status, libc::WNOHANG) } == hold {
            return if libc::WIFEXITED(status) { libc::WEXITSTATUS(status) } else { 128 + libc::WTERMSIG(status) };
        }
        // SAFETY: reads and clears the C shim's lock-free flag in one atomic operation.
        if unsafe { farcooler_native_hold_hangup_pending(true) } {
            // SAFETY: `kill` takes no pointers. `hold` is our unreaped child (above), so the pid is still it.
            unsafe { libc::kill(hold, libc::SIGHUP); }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// macOS responsibility (private API in libSystem, looked up at run time). Every process has a *responsible* process,
/// which its children inherit at `fork`, and keep through `exec`, `setsid`, `setpgid` and being reparented to launchd
/// once their parent exits. A process started with responsibility disclaimed is responsible for itself. So once the
/// hold is responsible for itself, everything the agent starts names the hold as responsible, however it leaves the
/// group and whether or not the hold ever saw its parent; only a process that itself disclaims, through this same
/// private API, stops naming it. The kernel answers the hold's own pid only while the hold is alive, and the hold is
/// alive whenever it asks, so a pid in the answer is the hold's and no one else's.
#[cfg(target_os = "macos")]
mod responsibility {
    type Disclaim = unsafe extern "C" fn(*mut libc::posix_spawnattr_t, libc::c_int) -> libc::c_int;
    type Responsible = unsafe extern "C" fn(libc::pid_t) -> libc::pid_t;
    /// `RTLD_DEFAULT` in `<dlfcn.h>` on Apple platforms.
    const DEFAULT: *mut libc::c_void = -2isize as *mut libc::c_void;

    fn symbol(name: &std::ffi::CStr) -> Option<*mut libc::c_void> {
        // SAFETY: `name` is NUL-terminated; `dlsym` only reads it.
        let found = unsafe { libc::dlsym(DEFAULT, name.as_ptr()) };
        (!found.is_null()).then_some(found)
    }
    pub(super) fn disclaim() -> Option<Disclaim> {
        // SAFETY: the symbol has this signature in every macOS that exports it.
        symbol(c"responsibility_spawnattrs_setdisclaim").map(|f| unsafe { std::mem::transmute::<*mut libc::c_void, Disclaim>(f) })
    }
    fn responsible() -> Option<Responsible> {
        static FOUND: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
        let found = (*FOUND.get_or_init(|| symbol(c"responsibility_get_pid_responsible_for_pid").map(|f| f as usize)))?;
        // SAFETY: as above.
        Some(unsafe { std::mem::transmute::<usize, Responsible>(found) })
    }
    /// The process responsible for `pid`, or `None` when it can't be read (no such process, another user's, or a
    /// macOS without the API).
    pub(super) fn of(pid: i32) -> Option<i32> {
        // SAFETY: takes and returns plain integers.
        let answer = unsafe { responsible()?(pid) };
        (answer > 0).then_some(answer)
    }
    /// Whether this process is responsible for itself.
    pub(super) fn is_own() -> bool {
        let me = std::process::id() as i32;
        of(me) == Some(me)
    }
}

fn lock(held: &Mutex<Held>) -> std::sync::MutexGuard<'_, Held> { held.lock().unwrap_or_else(std::sync::PoisonError::into_inner) }

/// Remove the socket, only now that the group is empty, and exit with the agent's status.
fn finish(socket: &Path, code: i32) -> ! {
    let _ = std::fs::remove_file(socket);
    std::process::exit(code)
}

/// One request and reply per connection. A manual pause also takes an acknowledgement.
fn answer(stream: UnixStream, held: &Mutex<Held>, socket: &Path) {
    if !same_user(&stream) { return; }
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut line = String::new();
    let mut reader = BufReader::new(&stream);
    if reader.read_line(&mut line).is_err() { return; }
    let mut held = lock(held);
    if line.trim() == "group" && held.reaped.is_none() {
        let _ = writeln!(&stream, "{}", held.leader.0);
        return;
    }
    let reply = match line.trim() {
        "ready" if held.reaped.is_none() => "ready",
        "stopped" if held.reaped.is_none() => if group_stopped(&held) { "stopped" } else { "unsettled" },
        "pause" if held.reaped.is_none() => return answer_pause(&stream, &mut reader, &held),
        "stop" if held.reaped.is_none() => {
            held.gate_stop = true;
            if stop(&held) { "stopped" } else { "unsettled" }
        }
        "cont" if held.reaped.is_none() => {
            held.gate_stop = false;
            signal_group(&held, libc::SIGCONT);
            "continued"
        }
        "end" => match end(&mut held, STRAY_WAIT) {
            Ended::Settled(code) => {
                // Gone before the answer, so whoever reads `ended` also finds the socket gone.
                let _ = std::fs::remove_file(socket);
                let _ = (&stream).write_all(b"ended\n");
                finish(socket, code);
            }
            Ended::Unsettled => "unsettled",
        },
        "ready" | "pause" | "stop" | "stopped" | "cont" | "group" => "ended",
        _ => "unknown",
    };
    let _ = (&stream).write_all(format!("{reply}\n").as_bytes());
}

/// A hand pause is provisional until the caller confirms it on the same socket. Finish any rollback before
/// processing the next connection, and leave an earlier landing gate's stop in effect.
fn answer_pause(mut stream: &UnixStream, reader: &mut BufReader<&UnixStream>, held: &Held) {
    use std::io::Read;
    use std::os::fd::AsRawFd;

    let mut ack = [0u8];
    // EOF means this caller left while queued. Peeking keeps the normal acknowledgement read intact.
    // SAFETY: ack is writable for one byte; the stream keeps its descriptor open.
    if unsafe { libc::recv(stream.as_raw_fd(), ack.as_mut_ptr().cast(), 1, libc::MSG_PEEK | libc::MSG_DONTWAIT) } == 0 {
        return;
    }
    let stopped = stop(held);
    let reply = if stopped { b"stopped\n".as_slice() } else { b"unsettled\n" };
    let committed = stream.write_all(reply).is_ok() && stopped
        && reader.read_exact(&mut ack).is_ok() && ack == PAUSE_ACK;
    if !committed && !held.gate_stop {
        signal_group(held, libc::SIGCONT);
    }
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
    #[cfg(target_os = "macos")]
    command.env_remove(RELAYED);
    command.env_remove(BLIND);
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

fn group_stopped(held: &Held) -> bool {
    let leader = held.leader.0;
    rows().is_some_and(|rows| rows.iter().filter(|row| row.pgid == leader).all(|row| row.stopped))
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
///
/// On macOS a process whose parent exited before the hold ever saw it alive has a parent id the hold never learned,
/// so lineage alone would miss it. There `responsible` covers it: a live process that names the hold as responsible
/// for it (see `responsibility`) is the agent's, whatever the hold saw of its parents. Lineage still counts what
/// disclaimed responsibility, if the hold saw its parent.
struct Lineage { me: u64, known: HashSet<u64>, blind: bool }

/// A live process: its pid, group, identity and its parent's identity (unique ids on macOS, pids on Linux), and
/// whether it names the hold as responsible for it (macOS; always false on Linux).
struct Row { pid: i32, pgid: i32, id: u64, parent: u64, responsible: bool, stopped: bool }

/// What one look found alive: members of the agent's group (the leader among them), and descendants outside it.
struct Look { group: usize, strays: usize }

impl Lineage {
    const LEARNS: bool = cfg!(target_os = "macos");

    /// `None` when the hold can't learn its own identity: it could then follow nothing.
    fn new() -> Option<Self> {
        let me = own_id()?;
        // Only macOS learns, so only there is there anything to be blind to.
        let blind = Self::LEARNS && cfg!(debug_assertions) && std::env::var_os(BLIND).is_some();
        Some(Self { me, known: HashSet::from([me]), blind })
    }

    /// Look once, or `None` when the table can't be read whole. A look that misses a process can only see less of
    /// the lineage, never someone else's process as ours: an identity is learned only from its parent's.
    fn look(&mut self, leader: i32) -> Option<Look> {
        let rows = rows()?;
        if !Self::LEARNS { self.known = HashSet::from([self.me]); }
        while !self.blind {
            let before = self.known.len();
            for row in &rows { if self.known.contains(&row.parent) { self.known.insert(row.id); } }
            if self.known.len() == before { break; }
        }
        let me = std::process::id() as i32;
        let mut look = Look { group: 0, strays: 0 };
        for row in rows.iter().filter(|row| row.pid != me) {
            if row.pgid == leader { look.group += 1; } else if row.responsible || self.known.contains(&row.id) { look.strays += 1; }
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
        let responsible = pid != me && responsibility::of(pid) == Some(me);
        rows.push(Row { pid, pgid: info.pbi_pgid as i32, id: ids.id, parent: ids.parent, responsible,
            stopped: info.pbi_status == libc::SSTOP });
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
        rows.push(Row { pid, pgid, id: u64::from(pid as u32), parent: u64::from(ppid), responsible: false, stopped: matches!(state, "T" | "t") });
    }
    complete.then_some(rows)
}

/// What a hold said.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    /// Its one-word reply.
    Said(String),
    /// Its socket is gone: the hold ended its group and exited (or never started, which the caller rules out).
    Gone,
    /// Its socket exists, but no complete answer arrived before the caller stopped waiting.
    Silent,
}

/// The pinned group ID is used to observe heavy-queue membership. Only the hold signals the group.
pub(crate) fn group(socket: &Path) -> std::io::Result<Option<i32>> {
    match ask_blocking(socket, "group", Duration::from_millis(100)) {
        Answer::Gone => Ok(None),
        Answer::Said(word) if word == "ended" => Ok(None),
        Answer::Said(word) => word.parse::<i32>().ok().filter(|&pid| pid > 1).map(Some)
            .ok_or_else(|| std::io::Error::other("invalid native hold group")),
        Answer::Silent => Err(std::io::Error::other("native hold did not report its group")),
    }
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

/// Request a manual pause. Timing out or dropping this future closes the socket, so the hold discards or
/// rolls back the unconfirmed stop before taking another request.
pub(crate) async fn pause(socket: &Path, wait: Duration) -> Answer {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let request = async {
        let mut stream = BufReader::new(tokio::net::UnixStream::connect(socket).await?);
        stream.write_all(b"pause\n").await?;
        let mut line = String::new();
        if stream.read_line(&mut line).await? == 0 { return Ok(None); }
        let word = line.trim();
        if word == "stopped" { stream.write_all(&PAUSE_ACK).await?; }
        Ok::<_, std::io::Error>(Some(Answer::Said(word.to_owned())))
    };
    match tokio::time::timeout(wait, request).await {
        Ok(Ok(Some(answer))) => answer,
        _ if !socket.exists() => Answer::Gone,
        _ => Answer::Silent,
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

#[cfg(test)]
#[path = "../../../test/daemon_binary.rs"]
pub(crate) mod test_binary;
