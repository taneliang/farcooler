//! Prefer launchd for standalone runner startup when the job is loaded.

use std::io;
use std::path::{Path, PathBuf};

pub const LABEL: &str = "com.farcooler.daemon.canary.standalone";
pub const INSTALL_LOCK: &str = "activate.lock";

pub fn install_root(home: &Path) -> PathBuf {
    home.join(".local/share/farcooler-runner-canary")
}

/// Whether this account's runner is the standalone one: the installer's
/// release pointer is there. `FARCOOLER_HOME` selects another runtime.
pub fn is_installed() -> bool {
    farcooler_protocol::CHANNEL == farcooler_protocol::Channel::Canary
        && std::env::var_os("FARCOOLER_HOME").is_none()
        && std::env::var_os("HOME")
            .is_some_and(|home| install_root(Path::new(&home)).join("current").is_symlink())
}

/// True when launchd owns startup. An updater holding the exclusive lock
/// will start the job itself, so clients can wait for the socket. If the job
/// has yet to load, clients can use their ordinary on-demand startup.
#[cfg(target_os = "macos")]
pub fn start_if_installed() -> io::Result<bool> {
    use std::fs::{self, File};
    use std::process::Command;

    use rustix::fs::{FlockOperation, flock};
    use rustix::io::Errno;

    if farcooler_protocol::CHANNEL != farcooler_protocol::Channel::Canary
        || std::env::var_os("FARCOOLER_HOME").is_some()
    {
        return Ok(false);
    }
    let Some(home) = std::env::var_os("HOME") else { return Ok(false) };
    let root = install_root(Path::new(&home));
    let Ok(releases) = fs::canonicalize(root.join("releases")) else { return Ok(false) };
    let exe = fs::canonicalize(std::env::current_exe()?)?;
    if exe.parent().and_then(Path::parent) != Some(releases.as_path()) {
        return Ok(false);
    }

    let lock = File::open(root.join(INSTALL_LOCK))?;
    match flock(&lock, FlockOperation::NonBlockingLockShared) {
        Err(err) if err == Errno::WOULDBLOCK => return Ok(true),
        Err(err) => return Err(err.into()),
        Ok(()) => {}
    }
    // SAFETY: geteuid has no preconditions.
    let uid = unsafe { libc::geteuid() };
    let output = Command::new("/bin/launchctl")
        .args(["kickstart", &format!("user/{uid}/{LABEL}")]).output()?;
    kickstart_result(output)
}

#[cfg(target_os = "macos")]
fn kickstart_result(output: std::process::Output) -> io::Result<bool> {
    match output.status.code() {
        Some(0) => Ok(true),
        // sshd can create the user domain before the first Aqua login loads
        // the LaunchAgent. launchctl reports "Could not find service" (113).
        Some(113) => Ok(false),
        _ => Err(io::Error::other(format!(
            "Could not start the standalone LaunchAgent: {}",
            String::from_utf8_lossy(&output.stderr).trim(),
        ))),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn start_if_installed() -> io::Result<bool> {
    Ok(false)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::kickstart_result;
    use std::os::unix::process::ExitStatusExt;
    use std::process::{ExitStatus, Output};

    fn output(status: ExitStatus, stderr: &str) -> Output {
        Output { status, stdout: Vec::new(), stderr: stderr.as_bytes().to_vec() }
    }

    #[test]
    fn only_an_unloaded_launch_agent_allows_on_demand_startup() {
        assert!(kickstart_result(output(ExitStatus::from_raw(0), "")).unwrap());
        assert!(!kickstart_result(output(ExitStatus::from_raw(113 << 8), "Could not find service")).unwrap());
        let denied = kickstart_result(output(ExitStatus::from_raw(1 << 8), "Operation not permitted"));
        assert!(denied.unwrap_err().to_string().contains("Operation not permitted"));
        assert!(kickstart_result(output(ExitStatus::from_raw(libc::SIGTERM), "")).is_err());
    }
}
