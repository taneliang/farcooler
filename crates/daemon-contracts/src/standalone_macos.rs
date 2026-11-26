//! Start the installed standalone runner via its owner, even over SSH.

use std::io;
use std::path::{Path, PathBuf};

pub const LABEL: &str = "com.farcooler.daemon.canary.standalone";
pub const INSTALL_LOCK: &str = "activate.lock";

pub fn install_root(home: &Path) -> PathBuf {
    home.join(".local/share/farcooler-runner-canary")
}

/// True when launchd owns startup. An updater holding the exclusive lock
/// will start the job itself, so clients can wait for the socket.
#[cfg(target_os = "macos")]
pub fn start_if_installed() -> io::Result<bool> {
    use std::fs::{self, File, TryLockError};
    use std::process::Command;

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
    match lock.try_lock_shared() {
        Err(TryLockError::WouldBlock) => return Ok(true),
        Err(TryLockError::Error(err)) => return Err(err),
        Ok(()) => {}
    }
    // SAFETY: geteuid has no preconditions.
    let uid = unsafe { libc::geteuid() };
    let output = Command::new("/bin/launchctl")
        .args(["kickstart", &format!("user/{uid}/{LABEL}")]).output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "Could not start the standalone LaunchAgent: {}",
            String::from_utf8_lossy(&output.stderr).trim(),
        )));
    }
    Ok(true)
}

#[cfg(not(target_os = "macos"))]
pub fn start_if_installed() -> io::Result<bool> {
    Ok(false)
}
