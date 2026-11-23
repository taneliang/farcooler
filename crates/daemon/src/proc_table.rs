//! The host's process table, read in this process instead of through `ps`.
//!
//! `foreground::read` used to spawn `ps -axo pid=,ppid=,pgid=,tty=,stat=,args=`
//! on every sampling tick. On a Mac running several daemons and a loaded
//! process table that one spawn cost 40-70% of a core, a second, per daemon.
//! The kernel already answers the same question to a process that asks it
//! directly: libproc on macOS, `/proc` on Linux. So this walk produces the
//! very text `ps` printed, and `foreground::parse` reads it unchanged. The
//! two sources cannot disagree about what a row means, because there is one
//! reader.
//!
//! One difference from `ps`: a foreground process whose argv can't be read
//! (a `sudo` it may not inspect, a zombie) prints empty args here, where `ps`
//! prints `(comm)`. `foreground::parse` skips a row with no args, so the pane
//! keeps tmux's `pane_current_command` for it instead of a label.
//!
//! `None` means this system has no in-process answer, or the walk found
//! nothing at all (a sandbox hiding the table). A timed-out read also returns
//! no data.

/// One row in `ps`'s column order. `foreground` is `ps`'s `+`: in the
/// foreground process group of the controlling terminal.
fn line(pid: i32, ppid: i32, pgid: i32, tty: &str, foreground: bool, args: &str) -> String {
    // A newline inside an argument would split one row into two.
    let args: String = args.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    // A foreground row with no argv stays empty, so `parse` skips it as it
    // skips one `ps` printed empty, and the pane keeps tmux's name for it.
    let args = if args.trim().is_empty() && !foreground { "-" } else { args.trim() };
    format!("{pid} {ppid} {pgid} {tty} {} {args}\n", if foreground { "S+" } else { "S" })
}

/// Every process on the host as `ps -axo pid=,ppid=,pgid=,tty=,stat=,args=`
/// would print it, or `None` when this system can't be read in-process.
pub fn snapshot() -> Option<String> {
    crate::process_reads::LABELS.read(|| sample(true)).flatten()
}

pub(crate) async fn foreground_snapshot() -> Option<Option<String>> {
    crate::process_reads::LABELS.read_async(|| sample(true)).await
}

/// Process relationships for authorization and signal delivery. Even a
/// foreground process contributes only its topology here.
pub(crate) fn topology() -> Option<String> {
    crate::process_reads::TOPOLOGY.read(|| sample(false)).flatten()
}

pub(crate) async fn topology_async() -> Option<String> {
    crate::process_reads::TOPOLOGY.read_async(|| sample(false)).await.flatten()
}

fn sample(arguments: bool) -> Option<String> {
    let text = walk(arguments)?;
    (!text.is_empty()).then_some(text)
}

pub(crate) fn process_state(pid: i32) -> ProcessState {
    crate::process_reads::DETAILS.read(move || read_state(pid)).unwrap_or(ProcessState::Unknown)
}

// A successful signal only requests a stop. Confirm the kernel observed it before acknowledging a
// pause: otherwise a shell handling its child's exit can still execute after the caller sees Paused.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessState { Running, Stopped, Gone, Unknown }

pub(crate) fn stopped_or_gone(pid: i32) -> bool {
    matches!(process_state(pid), ProcessState::Stopped | ProcessState::Gone)
}

#[cfg(target_os = "macos")]
fn read_state(pid: i32) -> ProcessState {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as i32;
    let got = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size) };
    if got == size {
        return match info.pbi_status {
            libc::SSTOP => ProcessState::Stopped,
            libc::SZOMB => ProcessState::Gone,
            _ => ProcessState::Running,
        };
    }
    if got <= 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
        ProcessState::Gone
    } else { ProcessState::Unknown }
}
#[cfg(not(target_os = "macos"))]
fn read_state(pid: i32) -> ProcessState {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat.rsplit_once(')').and_then(|(_, s)| s.trim_start().chars().next())
            .map_or(ProcessState::Unknown, |state| match state {
                'T' | 't' => ProcessState::Stopped,
                'Z' | 'X' => ProcessState::Gone,
                _ => ProcessState::Running,
            }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ProcessState::Gone,
        Err(_) => ProcessState::Unknown,
    }
}

/// One `struct kinfo_proc` on 64-bit macOS (`<sys/sysctl.h>`), and the offsets
/// this reads from it. The `libc` crate has no definition. They were read off
/// the kernel's own answer against `ps`'s for every process on a live host
/// (all 979 agreed), and the layout has been fixed since the 64-bit ABI.
#[cfg(target_os = "macos")]
mod kinfo {
    pub const SIZE: usize = 648;
    pub const P_START: usize = 0;
    pub const P_FLAG: usize = 32;
    pub const P_PID: usize = 40;
    pub const E_PPID: usize = 560;
    pub const E_PGID: usize = 564;
    pub const E_TDEV: usize = 572;
    pub const E_TPGID: usize = 576;
    /// `p_flag`: the process has a controlling terminal.
    pub const P_CONTROLT: i32 = 0x2;
}

/// A retained child can already be a zombie when its owner receipt is written.
/// KERN_PROC_PID includes that child, whereas proc_pidinfo only finds live processes.
#[cfg(target_os = "macos")]
pub(crate) fn owner_start_micros(pid: i32) -> Option<i64> {
    crate::process_reads::DETAILS.read(move || {
        let mut mib = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_PID, pid];
        let mut record = [0u8; kinfo::SIZE];
        let mut size = record.len();
        // SAFETY: sysctl receives valid MIB, output buffer, and in/out length pointers.
        let result = unsafe { libc::sysctl(mib.as_mut_ptr(), 4, record.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0) };
        if result != 0 || size != record.len() || i32::from_ne_bytes(record[kinfo::P_PID..][..4].try_into().ok()?) != pid {
            return None;
        }
        let start = &record[kinfo::P_START..];
        let seconds = i64::from_ne_bytes(start[..8].try_into().ok()?);
        let micros = i32::from_ne_bytes(start[8..12].try_into().ok()?);
        if seconds <= 0 || !(0..1_000_000).contains(&micros) { return None; }
        seconds.checked_mul(1_000_000)?.checked_add(i64::from(micros))
    }).flatten()
}

#[cfg(not(target_os = "macos"))]
pub(crate) use crate::claude_registry::process_incarnation as owner_start_micros;

/// The name `ps` prints for a terminal's device number (`ttys012`).
///
/// `devname_r` finds it by walking `/dev` with an `lstat` per entry, which on a
/// host with a few thousand ptys cost more than everything else in the walk
/// put together (profiled: 1,851 of 2,026 samples). A pty slave is major 16,
/// named by its minor, so that case is arithmetic; the rest (the console and
/// the like) ask `devname_r` once per walk.
#[cfg(target_os = "macos")]
fn tty_name(tdev: i32) -> String {
    unsafe extern "C" {
        // In libSystem; the `libc` crate doesn't declare it.
        fn devname_r(
            dev: libc::dev_t,
            kind: libc::mode_t,
            buf: *mut libc::c_char,
            len: libc::c_int,
        ) -> *mut libc::c_char;
    }
    if (tdev >> 24) & 0xff == 16 {
        return format!("ttys{:03}", tdev & 0xff_ffff);
    }
    let mut name = [0 as libc::c_char; 64];
    // SAFETY: `name` is 64 bytes and `devname_r` writes a NUL-terminated name into at most `len`.
    let found = unsafe { devname_r(tdev as libc::dev_t, libc::S_IFCHR, name.as_mut_ptr(), 64) };
    if found.is_null() {
        return "??".to_string();
    }
    // SAFETY: NUL-terminated by `devname_r`.
    unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) }.to_string_lossy().into_owned()
}

/// Every process, from the one `sysctl(KERN_PROC_ALL)` that `ps` itself starts
/// with. Per-pid `proc_pidinfo` costs about ten times as much per process on
/// a loaded host; this is a single call.
#[cfg(target_os = "macos")]
fn walk(arguments: bool) -> Option<String> {
    const NODEV: i32 = -1;

    let mut mib = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_ALL];
    let mut buf: Vec<u8> = Vec::new();
    // The table can grow between asking its size and reading it (ENOMEM):
    // ask again, a few times, and then give up to `ps`.
    let mut filled = None;
    for _ in 0..4 {
        let mut size: libc::size_t = 0;
        // SAFETY: a null buffer asks only for the size.
        let rc = unsafe { libc::sysctl(mib.as_mut_ptr(), 3, std::ptr::null_mut(), &mut size, std::ptr::null_mut(), 0) };
        if rc != 0 || size == 0 {
            return None;
        }
        size += size / 8 + kinfo::SIZE * 16;
        buf.resize(size, 0);
        // SAFETY: `buf` holds `size` bytes and the kernel writes at most that.
        let rc = unsafe { libc::sysctl(mib.as_mut_ptr(), 3, buf.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0) };
        if rc == 0 {
            filled = Some(size);
            break;
        }
    }
    let size = filled?;
    if size % kinfo::SIZE != 0 {
        // A layout this code doesn't know: refuse rather than misread it.
        return None;
    }
    let int = |rec: &[u8], at: usize| i32::from_ne_bytes([rec[at], rec[at + 1], rec[at + 2], rec[at + 3]]);

    let mut out = String::new();
    let mut ttys: std::collections::HashMap<i32, String> = std::collections::HashMap::new();
    let (me, my_parent) = (std::process::id() as i32, unsafe { libc::getppid() });
    let mut saw_myself = false;
    for rec in buf[..size].chunks_exact(kinfo::SIZE) {
        let pid = int(rec, kinfo::P_PID);
        if pid <= 0 {
            continue;
        }
        let (ppid, pgid, tdev, tpgid) =
            (int(rec, kinfo::E_PPID), int(rec, kinfo::E_PGID), int(rec, kinfo::E_TDEV), int(rec, kinfo::E_TPGID));
        saw_myself |= pid == me && ppid == my_parent;
        let tty = if tdev == NODEV { "??".to_string() } else { ttys.entry(tdev).or_insert_with(|| tty_name(tdev)).clone() };
        let foreground = tty != "??" && int(rec, kinfo::P_FLAG) & kinfo::P_CONTROLT != 0 && pgid == tpgid;
        // Arguments only where they are read: `foreground::parse` labels a pane
        // from its foreground rows and from nothing else.
        let args = if foreground && arguments {
            #[cfg(test)]
            stall_foreground_read(pid);
            procargs(pid).unwrap_or_default()
        } else { String::new() };
        out.push_str(&line(pid, ppid, pgid, &tty, foreground, &args));
    }
    // This process must be in the table, under its own parent: if it isn't,
    // the layout is not the one the offsets were read from.
    saw_myself.then_some(out)
}

/// One process's executable and `argv` joined by spaces, read now, whatever
/// its terminal (ov-620, `caller_seat`): the snapshot reads a command line
/// only for a foreground process, and a caller's ancestors seldom are. `None`
/// when it can't be read (gone, or another user's).
pub fn command_of(pid: i32) -> Option<(String, String)> {
    crate::process_reads::ARGUMENTS.read(move || read_command_of(pid)).flatten()
}

#[cfg(target_os = "macos")]
fn read_command_of(pid: i32) -> Option<(String, String)> {
    let buf = procargs2(pid)?;
    let path = buf.get(4..)?.split(|&b| b == 0).next().map(|p| String::from_utf8_lossy(p).into_owned()).unwrap_or_default();
    Some((path, parse_procargs2(&buf)?))
}

/// The same on Linux: `/proc/<pid>/exe` and `cmdline`. A node program can
/// rewrite its `cmdline` (`process.title`); the executable link still names
/// the binary.
#[cfg(target_os = "linux")]
fn read_command_of(pid: i32) -> Option<(String, String)> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let args: Vec<String> = raw.split(|&b| b == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).into_owned()).collect();
    let args = args.join(" ");
    let exe = std::fs::read_link(format!("/proc/{pid}/exe")).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    (!args.trim().is_empty() || !exe.is_empty()).then_some((exe, args))
}

/// No in-process reader elsewhere: never known.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn read_command_of(_pid: i32) -> Option<(String, String)> {
    None
}

/// One process's executable, by path, for `caller_seat` (ov-649). macOS
/// answers for any user's process, root's `sshd` included; Linux only for
/// this user's, whose `/proc/<pid>/exe` it may read.
pub fn path_of(pid: i32) -> Option<String> {
    crate::process_reads::DETAILS.read(move || read_path_of(pid)).flatten()
}

#[cfg(target_os = "macos")]
fn read_path_of(pid: i32) -> Option<String> {
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: `buf` holds the size passed, and the kernel writes at most that.
    let len = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    (len > 0).then(|| String::from_utf8_lossy(&buf[..len as usize]).into_owned())
}

/// The same on Linux.
#[cfg(target_os = "linux")]
fn read_path_of(pid: i32) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/exe")).ok().map(|p| p.to_string_lossy().into_owned())
}

/// No in-process reader elsewhere: never known.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn read_path_of(_pid: i32) -> Option<String> {
    None
}

/// The user one process runs as, whoever's it is: `proc_pidinfo`'s short
/// BSD record, which it gives for root's processes too.
pub fn uid_of(pid: i32) -> Option<u32> {
    crate::process_reads::DETAILS.read(move || read_uid_of(pid)).flatten()
}

#[cfg(target_os = "macos")]
fn read_uid_of(pid: i32) -> Option<u32> {
    // SAFETY: a plain C struct of integers, for which all zeroes is valid.
    let mut info: libc::proc_bsdshortinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdshortinfo>() as libc::c_int;
    // SAFETY: `info` is `size` bytes, and the kernel writes at most that.
    let got = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDT_SHORTBSDINFO, 0, (&raw mut info).cast(), size) };
    (got == size).then_some(info.pbsi_uid)
}

/// The same on Linux: the owner of `/proc/<pid>`, which is its effective uid.
#[cfg(target_os = "linux")]
fn read_uid_of(pid: i32) -> Option<u32> {
    use std::os::unix::fs::MetadataExt as _;
    std::fs::metadata(format!("/proc/{pid}")).ok().map(|m| m.uid())
}

/// No in-process reader elsewhere: never known.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn read_uid_of(_pid: i32) -> Option<u32> {
    None
}

/// One process's program name, whoever's it is: its executable's file name
/// on macOS, and on Linux the kernel's `comm`, which any user may read (an
/// `sshd` session's `exe` is root's, or not dumpable).
#[cfg(target_os = "macos")]
pub fn name_of(pid: i32) -> Option<String> {
    path_of(pid).and_then(|p| std::path::Path::new(&p).file_name().map(|n| n.to_string_lossy().into_owned()))
}

/// The same on Linux.
#[cfg(target_os = "linux")]
pub fn name_of(pid: i32) -> Option<String> {
    crate::process_reads::DETAILS.read(move || read_name_of(pid)).flatten()
}

#[cfg(target_os = "linux")]
fn read_name_of(pid: i32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/comm")).ok().map(|c| c.trim_end().to_string()).filter(|c| !c.is_empty())
}

/// No in-process reader elsewhere: never known.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn name_of(_pid: i32) -> Option<String> {
    None
}

/// `argv` joined by spaces, from the kernel's `KERN_PROCARGS2`, as `ps` reads it.
#[cfg(target_os = "macos")]
fn procargs(pid: i32) -> Option<String> {
    parse_procargs2(&procargs2(pid)?)
}

/// The kernel's `KERN_PROCARGS2` buffer for `pid`.
#[cfg(target_os = "macos")]
fn procargs2(pid: i32) -> Option<Vec<u8>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut size: libc::size_t = 0;
    // SAFETY: a null buffer asks for the size.
    let rc = unsafe { libc::sysctl(mib.as_mut_ptr(), 3, std::ptr::null_mut(), &mut size, std::ptr::null_mut(), 0) };
    if rc != 0 || size < 4 {
        return None;
    }
    // A little slack: the arguments can grow between the two calls.
    size += 4096;
    let mut buf = vec![0u8; size];
    // SAFETY: `buf` holds `size` bytes.
    let rc = unsafe { libc::sysctl(mib.as_mut_ptr(), 3, buf.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0) };
    if rc != 0 {
        return None;
    }
    buf.truncate(size);
    Some(buf)
}

/// Temp roots an older Python heavy taker could have selected. Read only the
/// three tempfile settings and resolve relative paths against the taker's cwd.
pub(crate) fn python_temp_roots(pid: i32) -> Option<Vec<std::path::PathBuf>> {
    crate::process_reads::ARGUMENTS.read(move || read_python_temp_roots(pid))
}

fn read_python_temp_roots(pid: i32) -> Vec<std::path::PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    let Some(exe) = read_path_of(pid) else { return Vec::new() };
    if !Path::new(&exe).file_name().and_then(|s| s.to_str()).is_some_and(|s| s.to_ascii_lowercase().starts_with("python")) {
        return Vec::new();
    }
    let Some(env) = process_environment(pid) else { return Vec::new() };
    let mut cwd = None;
    env.split(|&b| b == 0).filter_map(|entry| {
        let (key, value) = entry.split_at(entry.iter().position(|&b| b == b'=')?);
        let value = &value[1..];
        if !matches!(key, b"TMPDIR" | b"TEMP" | b"TMP") || value.is_empty() { return None; }
        let dir = Path::new(std::ffi::OsStr::from_bytes(value));
        if dir.is_absolute() { return Some(dir.to_path_buf()); }
        Some(cwd.get_or_insert_with(|| process_cwd(pid)).as_ref()?.join(dir))
    }).collect()
}

#[cfg(target_os = "macos")]
fn process_environment(pid: i32) -> Option<Vec<u8>> {
    let buf = procargs2(pid)?;
    let (argc, argv) = procargs_payload(&buf)?;
    let env = argv.splitn(argc + 1, |&b| b == 0).nth(argc)?;
    Some(env.to_vec())
}

#[cfg(target_os = "linux")]
fn process_environment(pid: i32) -> Option<Vec<u8>> {
    std::fs::read(format!("/proc/{pid}/environ")).ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn process_environment(_pid: i32) -> Option<Vec<u8>> { None }

#[cfg(target_os = "macos")]
fn process_cwd(pid: i32) -> Option<std::path::PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as i32;
    let got = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDVNODEPATHINFO, 0, (&raw mut info).cast(), size) };
    if got != size { return None; }
    let path: Vec<u8> = info.pvi_cdir.vip_path.as_flattened().iter().take_while(|&&c| c != 0)
        .map(|&c| c as u8).collect();
    (!path.is_empty()).then(|| std::ffi::OsString::from_vec(path).into())
}

#[cfg(target_os = "linux")]
fn process_cwd(pid: i32) -> Option<std::path::PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn process_cwd(_pid: i32) -> Option<std::path::PathBuf> { None }

/// `KERN_PROCARGS2`'s layout: an `i32` argc, the executable path, NUL
/// padding, then argc NUL-terminated arguments, then the environment.
#[cfg(any(target_os = "macos", test))]
fn procargs_payload(buf: &[u8]) -> Option<(usize, &[u8])> {
    let argc = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?);
    if argc <= 0 {
        return None;
    }
    let mut rest = &buf[4..];
    let end = rest.iter().position(|&b| b == 0)?;
    rest = &rest[end..];
    let start = rest.iter().position(|&b| b != 0)?;
    Some((argc as usize, &rest[start..]))
}

#[cfg(any(target_os = "macos", test))]
fn parse_procargs2(buf: &[u8]) -> Option<String> {
    let (argc, argv) = procargs_payload(buf)?;
    let args: Vec<String> = argv
        .split(|&b| b == 0)
        .take(argc)
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
    let joined = args.join(" ");
    (!joined.trim().is_empty()).then_some(joined)
}

#[cfg(target_os = "linux")]
fn walk(arguments: bool) -> Option<String> {
    let mut out = String::new();
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<i32>().ok()) else { continue };
        // Gone between the listing and the read: not an error, just not there.
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { continue };
        let Some(s) = parse_linux_stat(&stat) else { continue };
        let tty = linux_tty(s.tty_nr);
        let foreground = tty != "??" && s.tpgid > 0 && s.tpgid == s.pgrp;
        let args = if foreground && arguments {
            #[cfg(test)]
            stall_foreground_read(pid);
            std::fs::read(format!("/proc/{pid}/cmdline"))
                .ok()
                .map(|raw| {
                    let text: Vec<String> =
                        raw.split(|&b| b == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).into_owned()).collect();
                    text.join(" ")
                })
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| format!("[{}]", s.comm))
        } else {
            String::new()
        };
        out.push_str(&line(pid, s.ppid, s.pgrp, &tty, foreground, &args));
    }
    Some(out)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn walk(_arguments: bool) -> Option<String> {
    None
}

/// The fields of `/proc/<pid>/stat` this reads.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Debug, PartialEq, Eq)]
struct LinuxStat {
    comm: String,
    ppid: i32,
    pgrp: i32,
    tty_nr: u32,
    tpgid: i32,
}

/// `pid (comm) S ppid pgrp session tty_nr tpgid ...`. `comm` can hold spaces
/// and parentheses, so the fields after it are found from the LAST `)`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_linux_stat(text: &str) -> Option<LinuxStat> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let mut fields = text.get(close + 1..)?.split_whitespace();
    let _state = fields.next()?;
    let ppid = fields.next()?.parse().ok()?;
    let pgrp = fields.next()?.parse().ok()?;
    let _session = fields.next()?;
    let tty_nr = fields.next()?.parse().ok()?;
    let tpgid = fields.next()?.parse().ok()?;
    Some(LinuxStat { comm: text.get(open + 1..close)?.to_string(), ppid, pgrp, tty_nr, tpgid })
}

/// The name `ps` prints for a `tty_nr`: `pts/3`, `tty1`, or `??` for none.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn linux_tty(tty_nr: u32) -> String {
    if tty_nr == 0 {
        return "??".to_string();
    }
    let major = (tty_nr >> 8) & 0xfff;
    let minor = (tty_nr & 0xff) | ((tty_nr >> 12) & 0xfff00);
    match major {
        136..=143 => format!("pts/{}", minor + (major - 136) * 256),
        4 => format!("tty{minor}"),
        _ => "??".to_string(),
    }
}

#[cfg(test)]
pub(crate) struct StalledRead {
    pid: i32,
    pub entered: tokio::sync::Notify,
    release: std::sync::Condvar,
    released: std::sync::Mutex<bool>,
}

#[cfg(test)]
static STALLED_READ: std::sync::Mutex<Option<std::sync::Arc<StalledRead>>> = std::sync::Mutex::new(None);

#[cfg(test)]
impl StalledRead {
    pub(crate) fn install(pid: i32) -> std::sync::Arc<Self> {
        let read = std::sync::Arc::new(Self { pid, entered: Default::default(), release: Default::default(), released: Default::default() });
        *STALLED_READ.lock().unwrap() = Some(read.clone());
        read
    }

    pub(crate) fn release(&self) {
        *STALLED_READ.lock().unwrap() = None;
        *self.released.lock().unwrap() = true;
        self.release.notify_all();
    }
}

#[cfg(test)]
fn stall_foreground_read(pid: i32) {
    let read = STALLED_READ.lock().unwrap().clone();
    if let Some(read) = read.filter(|read| read.pid == pid) {
        read.entered.notify_one();
        let _guard = read.release.wait_while(read.released.lock().unwrap(), |released| !*released).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_linux_stat_line_is_read_from_the_last_paren() {
        let s = parse_linux_stat("42 (we (ird) name) S 7 42 42 34819 42 4194560 1 2 3").unwrap();
        assert_eq!(
            s,
            LinuxStat { comm: "we (ird) name".into(), ppid: 7, pgrp: 42, tty_nr: 34819, tpgid: 42 }
        );
        assert_eq!(linux_tty(34819), "pts/3");
        assert_eq!(linux_tty(1025), "tty1");
        assert_eq!(linux_tty(0), "??");
    }

    #[test]
    fn procargs2_is_argv_without_the_path_or_environment() {
        let mut buf = 3i32.to_ne_bytes().to_vec();
        buf.extend(b"/usr/bin/pnpm\0\0\0pnpm\0dev\0--port\0PATH=/bin\0HOME=/h\0");
        assert_eq!(parse_procargs2(&buf).as_deref(), Some("pnpm dev --port"));
        assert_eq!(parse_procargs2(&0i32.to_ne_bytes()), None);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn a_pty_slave_is_named_by_its_minor() {
        // `ps -o tdev` printed 16/59 for ttys059, read off a live pane.
        assert_eq!(tty_name(0x1000_003b), "ttys059");
        assert_eq!(tty_name(0x1000_0000), "ttys000");
    }

    /// `tty_name` computes a pty's name from major 16. This stats a real pty
    /// slave, so a kernel that moved the major fails here and not silently.
    #[test]
    #[cfg(target_os = "macos")]
    fn a_pty_slave_has_the_major_the_name_rule_assumes() {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: plain libc pty setup; return values are checked.
        let (master, name) = unsafe {
            let master = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
            assert!(master >= 0 && libc::grantpt(master) == 0 && libc::unlockpt(master) == 0);
            let name = std::ffi::CStr::from_ptr(libc::ptsname(master)).to_str().unwrap().to_string();
            (master, name)
        };
        let rdev = std::fs::metadata(&name).expect("the pty slave").rdev() as i32;
        // SAFETY: closing the descriptor opened above.
        unsafe { libc::close(master) };
        assert_eq!((rdev >> 24) & 0xff, 16, "{name} is not major 16");
        assert_eq!(tty_name(rdev), name.trim_start_matches("/dev/"));
    }

    #[test]
    fn a_row_is_one_line_in_ps_column_order() {
        assert_eq!(line(5, 1, 5, "ttys1", true, "a\nb"), "5 1 5 ttys1 S+ a b\n");
        assert_eq!(line(6, 5, 6, "??", false, ""), "6 5 6 ?? S -\n");
        assert_eq!(line(7, 5, 7, "ttys1", true, ""), "7 5 7 ttys1 S+ \n");
    }

    /// This process's executable is where it was run from, and pid 1, another
    /// user's, still has a name (`caller_seat` reads root's `sshd` so).
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn a_process_is_named_by_its_executable_whoever_runs_it() {
        let me = std::process::id() as i32;
        let exe = std::fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
        assert_eq!(path_of(me).map(std::path::PathBuf::from), Some(exe.clone()));
        let named = name_of(me).expect("this process's name");
        assert!(exe.file_name().unwrap().to_string_lossy().starts_with(&named), "{named} for {}", exe.display());
        assert!(name_of(1).is_some_and(|n| !n.is_empty()), "pid 1 has a name");
        assert_eq!(name_of(i32::MAX - 7), None, "a pid no process has");
    }

    /// The user a process runs as, this one's and root's pid 1 alike.
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn a_process_is_owned_by_the_user_it_runs_as() {
        // SAFETY: plain libc getter.
        assert_eq!(uid_of(std::process::id() as i32), Some(unsafe { libc::geteuid() }));
        assert_eq!(uid_of(1), Some(0), "pid 1 is root's");
        assert_eq!(uid_of(i32::MAX - 7), None, "a pid no process has");
    }

    /// The whole table, read in this process, holds this process under its own
    /// parent. (Linux and macOS both; elsewhere there is no answer and the
    /// `ps` fallback serves.)
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn the_snapshot_holds_this_process() {
        let text = snapshot().expect("this system can be read in-process");
        let me = std::process::id() as i32;
        let ppid = unsafe { libc::getppid() };
        let found = text.lines().any(|l| {
            let mut f = l.split_whitespace().filter_map(|w| w.parse::<i32>().ok());
            f.next() == Some(me) && f.next() == Some(ppid)
        });
        assert!(found, "no row for pid {me} under {ppid}");
    }
}
