//! Daemon composition: git worktree transactions, domain services, lifecycle.
pub mod agent_supervisor;
pub mod allowlist;
pub mod change_set;
pub mod enrollment;
pub mod fanout;
pub mod file_diff;
pub mod foreground;
pub mod fs_watch;
pub mod git;
pub mod layout;
pub mod log_join;
pub mod log_watch;
pub mod pastes;
pub mod paths;
pub mod push;
pub mod reconcile;
pub mod rpc;
pub mod review;
pub mod review_ops;
pub mod runtime;
pub mod service;
pub mod sessions;
pub mod stack;
pub mod session_discovery;
#[cfg(test)]
pub(crate) mod test_support;
pub mod watch;
pub mod wire;

/// This machine's name, for rejecting it as a pane title.
///
/// A plain shell leaves the hostname in the title, and a row called
/// `Mac.attlocal.net` says nothing about the pane.
///
/// Read through libc rather than by spawning `hostname`: this is wanted once
/// per sample tick, and a process spawn a second to learn a string that has not
/// changed since boot is a poor trade.
pub fn hostname() -> String {
    let mut buf = vec![0u8; 256];
    // SAFETY: `gethostname` writes at most `len` bytes into the buffer it is
    // given, and the buffer outlives the call.
    let ok = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) } == 0;
    if !ok {
        return String::new();
    }
    // Truncation leaves the result unterminated, so the NUL is not assumed.
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).trim().to_string()
}

#[cfg(test)]
mod tests {
    /// Against the program it replaced.
    ///
    /// An empty or wrong answer here is silent: `title::parse` would stop
    /// recognizing the hostname a plain shell leaves in its title, and every
    /// such pane would be named after the machine instead of what it runs.
    #[test]
    fn this_machine_knows_its_own_name() {
        let expected = std::process::Command::new("hostname")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        assert_eq!(super::hostname(), expected);
        assert!(!expected.is_empty(), "a machine with no name would break title matching");
    }
}
