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
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok().and_then(linux_cwd)
}

/// What `/proc/<pid>/cwd` links to, as a working directory.
///
/// A directory removed while the process stood in it reads as its old path
/// with ` (deleted)` on the end. That's evidence of nothing: matched as a
/// path, `/r/.worktrees/x (deleted)` isn't inside `x` but is inside `/r`, so
/// a process left in a removed worktree would read as working in the main
/// checkout. So it's no working directory at all, and neither is a real
/// directory someone named that, which the link can't tell apart. A pure
/// function, so a test on any system can reach it.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn linux_cwd(link: PathBuf) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;

    (!link.as_os_str().as_bytes().ends_with(b" (deleted)")).then_some(link)
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

    /// Linux's link for a directory that was removed is no directory; any
    /// other link is the directory, a name with `deleted` in it included.
    #[test]
    fn a_removed_directory_is_no_working_directory() {
        assert_eq!(linux_cwd(PathBuf::from("/r/.worktrees/x (deleted)")), None);
        assert_eq!(linux_cwd(PathBuf::from("/r/.worktrees/x")), Some(PathBuf::from("/r/.worktrees/x")));
        assert_eq!(linux_cwd(PathBuf::from("/r/deleted")), Some(PathBuf::from("/r/deleted")));
        assert_eq!(linux_cwd(PathBuf::from("/r/(deleted)")), Some(PathBuf::from("/r/(deleted)")));
    }

    #[test]
    fn a_pid_nobody_has_is_nothing() {
        assert_eq!(cwd_of(i32::MAX), None);
    }
}
