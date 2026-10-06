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
//! `None` means this system has no in-process answer, or the walk found
//! nothing at all (a sandbox hiding the table). The caller then falls back to
//! `ps`, which is slower and always right.

/// One row in `ps`'s column order. `foreground` is `ps`'s `+`: in the
/// foreground process group of the controlling terminal.
fn line(pid: i32, ppid: i32, pgid: i32, tty: &str, foreground: bool, args: &str) -> String {
    // A newline inside an argument would split one row into two.
    let args: String = args.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let args = if args.trim().is_empty() { "-" } else { args.trim() };
    format!("{pid} {ppid} {pgid} {tty} {} {args}\n", if foreground { "S+" } else { "S" })
}

/// Every process on the host as `ps -axo pid=,ppid=,pgid=,tty=,stat=,args=`
/// would print it, or `None` when this system can't be read in-process.
pub fn snapshot() -> Option<String> {
    let text = walk()?;
    (!text.is_empty()).then_some(text)
}

#[cfg(target_os = "macos")]
fn walk() -> Option<String> {
    unsafe extern "C" {
        // In libSystem; the `libc` crate doesn't declare it.
        fn devname_r(
            dev: libc::dev_t,
            kind: libc::mode_t,
            buf: *mut libc::c_char,
            len: libc::c_int,
        ) -> *mut libc::c_char;
    }
    // `proc_bsdinfo.pbi_flags`, `PROC_FLAG_CTTY` in `<sys/proc_info.h>`: the
    // process has a controlling terminal. (`PROC_FLAG_CONTROLT` is 0x80, and
    // `PROC_FLAG_SLEADER` 0x20; read off a live pane's session leader, 0x...f0.)
    const PROC_FLAG_CTTY: u32 = 0x40;
    const NODEV: u32 = u32::MAX;

    // SAFETY: a null buffer of size 0 asks only for the count.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return None;
    }
    // Room for processes that start between the two calls.
    let mut pids = vec![0 as libc::pid_t; count as usize + 256];
    // SAFETY: the buffer holds `len * 4` bytes and the kernel writes at most that.
    let got = unsafe {
        libc::proc_listallpids(pids.as_mut_ptr().cast(), (pids.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int)
    };
    if got <= 0 {
        return None;
    }
    pids.truncate(got as usize);

    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let mut out = String::new();
    let mut ttys: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    for pid in pids.into_iter().filter(|&p| p > 0) {
        let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        // SAFETY: a zeroed `proc_bsdinfo` of exactly `size` bytes.
        let written = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, info.as_mut_ptr().cast(), size) };
        if written != size {
            continue;
        }
        // SAFETY: fully written by the call above.
        let info = unsafe { info.assume_init() };
        let tty = if info.e_tdev == NODEV {
            "??".to_string()
        } else {
            ttys.entry(info.e_tdev)
                .or_insert_with(|| {
                    let mut buf = [0 as libc::c_char; 64];
                    // SAFETY: `buf` is 64 bytes and `devname_r` writes a NUL-terminated name into at most `len`.
                    let name = unsafe { devname_r(info.e_tdev as libc::dev_t, libc::S_IFCHR, buf.as_mut_ptr(), 64) };
                    if name.is_null() {
                        return "??".to_string();
                    }
                    // SAFETY: NUL-terminated by `devname_r`.
                    unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned()
                })
                .clone()
        };
        let pgid = info.pbi_pgid as i32;
        let foreground = tty != "??" && info.pbi_flags & PROC_FLAG_CTTY != 0 && info.e_tpgid == info.pbi_pgid;
        // Arguments only where they are read: `foreground::parse` labels a pane
        // from its foreground rows and from nothing else.
        let args = if foreground { procargs(pid).unwrap_or_else(|| comm(&info.pbi_comm)) } else { String::new() };
        out.push_str(&line(pid, info.pbi_ppid as i32, pgid, &tty, foreground, &args));
    }
    Some(out)
}

#[cfg(target_os = "macos")]
fn comm(raw: &[libc::c_char]) -> String {
    let bytes: Vec<u8> = raw.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
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
    fn a_row_is_one_line_in_ps_column_order() {
        assert_eq!(line(5, 1, 5, "ttys1", true, "a\nb"), "5 1 5 ttys1 S+ a b\n");
        assert_eq!(line(6, 5, 6, "??", false, ""), "6 5 6 ?? S -\n");
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
