//! Interlock between the local runner's landing effects and its replacement.
//! Both sides open the same file in the runner home and leave it in place.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use rustix::fs::{FlockOperation, flock};

/// Hold from admission until the effect's result has been recorded.
pub(crate) fn try_landing(home: &Path) -> io::Result<Option<File>> {
    try_lock(home, FlockOperation::NonBlockingLockShared)
}

/// Probe for active landings or reserve the runner through replacement.
pub fn try_update(home: &Path) -> io::Result<Option<File>> {
    try_lock(home, FlockOperation::NonBlockingLockExclusive)
}

fn try_lock(home: &Path, operation: FlockOperation) -> io::Result<Option<File>> {
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false)
        .mode(0o600).custom_flags(libc::O_NOFOLLOW).open(home.join("landing-update.lock"))?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(io::Error::other("invalid landing update lock"));
    }
    match flock(&file, operation) {
        Ok(()) => Ok(Some(file)),
        Err(err) if err == rustix::io::Errno::WOULDBLOCK => Ok(None),
        Err(err) => Err(err.into()),
    }
}
