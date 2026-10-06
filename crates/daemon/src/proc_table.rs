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
//! nothing at all (a sandbox hiding the table). The caller then falls back to
//! `ps`, which is slower and always right.

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
    let text = walk()?;
    (!text.is_empty()).then_some(text)
}

/// One `struct kinfo_proc` on 64-bit macOS (`<sys/sysctl.h>`), and the offsets
/// this reads from it. The `libc` crate has no definition. They were read off
/// the kernel's own answer against `ps`'s for every process on a live host
/// (all 979 agreed), and the layout has been fixed since the 64-bit ABI.
#[cfg(target_os = "macos")]
mod kinfo {
    pub const SIZE: usize = 648;
    pub const P_FLAG: usize = 32;
    pub const P_PID: usize = 40;
    pub const E_PPID: usize = 560;
    pub const E_PGID: usize = 564;
    pub const E_TDEV: usize = 572;
    pub const E_TPGID: usize = 576;
    /// `p_flag`: the process has a controlling terminal.
    pub const P_CONTROLT: i32 = 0x2;
}

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
fn walk() -> Option<String> {
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
        let args = if foreground { procargs(pid).unwrap_or_default() } else { String::new() };
        out.push_str(&line(pid, ppid, pgid, &tty, foreground, &args));
    }
    // This process must be in the table, under its own parent: if it isn't,
    // the layout is not the one the offsets were read from.
    saw_myself.then_some(out)
}

/// `argv` joined by spaces, from the kernel's `KERN_PROCARGS2`, as `ps` reads it.
#[cfg(target_os = "macos")]
fn procargs(pid: i32) -> Option<String> {
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
    parse_procargs2(&buf)
}

/// `KERN_PROCARGS2`'s layout: an `i32` argc, the executable path, NUL
/// padding, then argc NUL-terminated arguments, then the environment.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_procargs2(buf: &[u8]) -> Option<String> {
    let argc = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?);
    if argc <= 0 {
        return None;
    }
    let mut rest = &buf[4..];
    let end = rest.iter().position(|&b| b == 0)?;
    rest = &rest[end..];
    let start = rest.iter().position(|&b| b != 0)?;
    rest = &rest[start..];
    let args: Vec<String> = rest
        .split(|&b| b == 0)
        .take(argc as usize)
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
    let joined = args.join(" ");
    (!joined.trim().is_empty()).then_some(joined)
}

#[cfg(target_os = "linux")]
fn walk() -> Option<String> {
    let mut out = String::new();
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<i32>().ok()) else { continue };
        // Gone between the listing and the read: not an error, just not there.
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { continue };
        let Some(s) = parse_linux_stat(&stat) else { continue };
        let tty = linux_tty(s.tty_nr);
        let foreground = tty != "??" && s.tpgid > 0 && s.tpgid == s.pgrp;
        let args = if foreground {
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
fn walk() -> Option<String> {
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
