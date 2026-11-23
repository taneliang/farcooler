//! Interlock between the local runner's landing effects and its replacement.
//! Both sides leave the lock files in the runner home. The locks themselves
//! disappear when their holders exit, including after a crash.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use rustix::fs::{FlockOperation, flock};

/// Hold from admission until the effect's result has been recorded.
pub(crate) fn try_landing(home: &Path) -> io::Result<Option<File>> {
    // Hold admission across the effect-lock probe so a waiter rules out
    // even short-lived shared effect locks from rejected admissions.
    let Some(_admission) = try_lock(home, WAITER_LOCK, FlockOperation::NonBlockingLockExclusive)? else {
        return Ok(None);
    };
    try_lock(home, EFFECT_LOCK, FlockOperation::NonBlockingLockShared)
}

/// Keep new effects out while a waiting update drains the admitted ones.
pub fn announce_update(home: &Path) -> io::Result<File> {
    let file = open_lock(home, WAITER_LOCK)?;
    // Updates share this lock, so one waiter exiting cannot cancel another's
    // announcement. Its only exclusive holder is the brief admission check.
    loop {
        match flock(&file, FlockOperation::LockShared) {
            Ok(()) => return Ok(file),
            Err(rustix::io::Errno::INTR) => continue,
            Err(err) => return Err(err.into()),
        }
    }
}

/// Probe for active landings or reserve the runner through replacement.
pub fn try_update(home: &Path) -> io::Result<Option<File>> {
    try_lock(home, EFFECT_LOCK, FlockOperation::NonBlockingLockExclusive)
}

const EFFECT_LOCK: &str = "landing-update.lock";
const WAITER_LOCK: &str = "landing-update-waiters.lock";

fn open_lock(home: &Path, name: &str) -> io::Result<File> {
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false)
        .mode(0o600).custom_flags(libc::O_NOFOLLOW).open(home.join(name))?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(io::Error::other("invalid landing update lock"));
    }
    Ok(file)
}

fn try_lock(home: &Path, name: &str, operation: FlockOperation) -> io::Result<Option<File>> {
    let file = open_lock(home, name)?;
    match flock(&file, operation) {
        Ok(()) => Ok(Some(file)),
        Err(err) if err == rustix::io::Errno::WOULDBLOCK => Ok(None),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::{announce_update, try_landing, try_update};

    #[test]
    fn a_waiting_update_gets_the_lock_before_a_second_landing() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let first = try_landing(home).unwrap().unwrap();
        let waiting = announce_update(home).unwrap();
        assert!(try_update(home).unwrap().is_none());
        assert!(try_landing(home).unwrap().is_none(), "a second landing cannot join the first");
        drop(first);

        assert!(try_landing(home).unwrap().is_none(), "a second landing cannot take the gap");
        let update = try_update(home).unwrap().unwrap();
        drop(waiting);
        assert!(try_landing(home).unwrap().is_none(), "replacement is in progress");
        drop(update);

        assert!(try_landing(home).unwrap().is_some());
    }
}
