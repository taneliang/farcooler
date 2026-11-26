//! Locks for Git processes launched by a workflow Provision effect.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::{fs::OpenOptionsExt, process::CommandExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use farcooler_core::{DomainError, Result};
use tokio::time::Instant;

const LOCK_WAIT: Duration = Duration::from_secs(1);

tokio::task_local! {
    /// The receipt file for a provisioning effect, shared by its sequence of Git commands.
    pub(crate) static PROVISION_WRITERS: PathBuf;
}

/// Take a fresh lock on the effect's existing receipt. Another open of the
/// file conflicts while a Git process (or an inheriting helper) still holds it.
pub(crate) fn try_provision_lock(path: &Path) -> std::io::Result<File> {
    let file = OpenOptions::new().read(true).write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("invalid provisioning lock file"));
    }
    rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive)?;
    Ok(file)
}

/// Concurrent forks can briefly retain a close-on-exec copy of a lock.
/// Give those copies time to close before reporting a surviving writer.
pub(crate) async fn wait_provision_lock(path: &Path, deadline: Instant) -> std::io::Result<File> {
    let deadline = deadline.min(Instant::now() + LOCK_WAIT);
    loop {
        match try_provision_lock(path) {
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                tokio::time::sleep_until(deadline.min(Instant::now() + Duration::from_millis(10))).await;
            }
            result => return result,
        }
    }
}

/// Keep the lock across exec by clearing close-on-exec in the child alone.
/// Closing the daemon's copy, including on abrupt death, cannot unlock a
/// surviving Git command's copy. Never explicitly unlock this file.
fn inherit_provision_lock(cmd: &mut std::process::Command, file: &File) {
    let fd = file.as_raw_fd();
    // SAFETY: The caller keeps the file open through spawn. The child only
    // calls the async-signal-safe fcntl on its own descriptor table.
    unsafe {
        cmd.pre_exec(move || {
            if libc::fcntl(fd, libc::F_SETFD, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

pub(super) async fn track(cmd: &mut std::process::Command, args: &[&str], deadline: Instant) -> Result<Option<File>> {
    let Ok(path) = PROVISION_WRITERS.try_with(Clone::clone) else { return Ok(None) };
    let writer = wait_provision_lock(&path, deadline).await.map_err(|error| {
        tracing::warn!(%error, ?args, "could not take the provisioning Git lock");
        DomainError::OperationFailed
    })?;
    inherit_provision_lock(cmd, &writer);
    Ok(Some(writer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn provisioning_waits_for_a_briefly_retained_lock() {
        let receipt = tempfile::NamedTempFile::new().unwrap();
        let writer = try_provision_lock(receipt.path()).unwrap();
        let deadline = Instant::now() + Duration::from_millis(100);
        let still_held = wait_provision_lock(receipt.path(), Instant::now()).await.unwrap_err();
        assert_eq!(still_held.kind(), std::io::ErrorKind::WouldBlock);

        let (lock, ()) = tokio::join!(wait_provision_lock(receipt.path(), deadline), async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            drop(writer);
        });
        assert!(lock.is_ok());
    }
}
