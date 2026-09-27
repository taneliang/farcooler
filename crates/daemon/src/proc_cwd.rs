//! A process's working directory, read from the kernel.
//!
//! Claiming's weakest signal (`claims::scan`) asks where each process under a
//! pane is working. `ps` can't say: it has no column for it on macOS. `lsof -d
//! cwd` can, but it's a process of its own, and the watcher already spends one
//! `ps` and one `lsof` a second. This is one syscall per pid.
//!
//! The kernel's answer is already resolved: `/tmp/x` comes back as
//! `/private/tmp/x` on macOS, which is how a worktree path is compared anyway.

use std::path::PathBuf;

/// Where `pid` is working, or `None` for a pid that's gone, isn't ours to
/// read, or runs on a system this doesn't know how to ask.
#[cfg(target_os = "macos")]
pub fn cwd_of(pid: i32) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;

    let mut info = std::mem::MaybeUninit::<libc::proc_vnodepathinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
    // SAFETY: the buffer is a zeroed `proc_vnodepathinfo` of exactly the size
    // passed, and the kernel writes at most that many bytes into it.
    let written = unsafe {
        libc::proc_pidinfo(pid, libc::PROC_PIDVNODEPATHINFO, 0, info.as_mut_ptr().cast(), size)
    };
    if written != size {
        return None;
    }
    // SAFETY: fully written by the call above, and zeroed before it besides.
    let info = unsafe { info.assume_init() };
    // `vip_path` is `char[MAXPATHLEN]`, which libc spells as 32 rows of 32.
    let bytes: Vec<u8> = info.pvi_cdir.vip_path.iter().flatten().map(|&c| c as u8).collect();
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(&bytes[..end])))
}

/// Where `pid` is working, or `None` for a pid that's gone or isn't ours to
/// read.
#[cfg(target_os = "linux")]
pub fn cwd_of(pid: i32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

/// Nothing to ask on this system, so nothing is ever seen.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn cwd_of(_pid: i32) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This process's own directory, which the test runner set, compared in
    /// the kernel's spelling of it.
    #[test]
    fn a_process_is_where_the_kernel_says_it_is() {
        let here = std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap();
        assert_eq!(cwd_of(std::process::id() as i32), Some(here));
    }

    /// A child moved somewhere else is read there, not where its parent is.
    #[test]
    fn a_child_is_read_where_it_went() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .current_dir(dir.path())
            .spawn()
            .unwrap();
        let seen = cwd_of(child.id() as i32);
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(seen, Some(std::fs::canonicalize(dir.path()).unwrap()));
    }

    #[test]
    fn a_pid_nobody_has_is_nothing() {
        assert_eq!(cwd_of(i32::MAX), None);
    }
}
