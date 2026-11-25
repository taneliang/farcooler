//! Locks for Git processes launched by a workflow Provision effect.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::{fs::OpenOptionsExt, process::CommandExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use farcooler_core::{DomainError, Result};

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

pub(super) fn track(cmd: &mut std::process::Command, args: &[&str]) -> Result<Option<File>> {
    let writer = PROVISION_WRITERS.try_with(|path| {
        try_provision_lock(path).map_err(|error| {
            tracing::warn!(%error, ?args, "could not take the provisioning Git lock");
            DomainError::OperationFailed
        })
    }).ok().transpose()?;
    if let Some(file) = &writer {
        inherit_provision_lock(cmd, file);
    }
    Ok(writer)
}

pub(super) async fn after_timeout(writer: Option<File>) {
    // Dropping the future requests a kill. Give an owned provisioning
    // writer a short chance to exit before any cleanup tries its lock.
    if writer.is_some() {
        drop(writer);
        let _ = tokio::time::timeout(Duration::from_millis(100), async {
            while PROVISION_WRITERS.with(|path| try_provision_lock(path))
                .is_err_and(|error| error.kind() == std::io::ErrorKind::WouldBlock)
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await;
    }
}
