//! Who owns a database file, proved by the kernel instead of by an inventory.
//! Every ordinary `Store::open` holds a shared `flock` on `<db>.owners` for the
//! Store's life. `JoinedOwners::acquire` takes it exclusively without blocking,
//! so it succeeds only when every gated Store in every process (daemon, stream,
//! standalone stdio) has closed, however it ended: the kernel drops the lock on
//! exit and crash, so there is no stale record to disbelieve and no PID to
//! signal. A holder blocks the proof; nothing is ever killed to get it. The
//! proof owns the lock, has no other constructor and no `Clone` or serde, and
//! each cutover stage consumes it, so a stage after a reopen must prove afresh.
//! `Quiescence` remains evidence for the rehearsal's permit, never authority.
//! A process that predates this lock is invisible here: capture and upgrade
//! share one write transaction, and conservation catches its later writes.
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use super::{Error, Result, refuse};

pub fn lock_path(db: &Path) -> PathBuf {
    let mut name = db.as_os_str().to_owned();
    name.push(".owners");
    PathBuf::from(name)
}
fn open_lock(db: &Path) -> Result<File> {
    Ok(OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600)
        .custom_flags(libc::O_NOFOLLOW).open(lock_path(db))?)
}
/// `flock` without waiting; false when another holder conflicts.
fn try_flock(file: &File, kind: i32) -> Result<bool> {
    // SAFETY: `file` owns a descriptor open for the whole call.
    if unsafe { libc::flock(file.as_raw_fd(), kind | libc::LOCK_NB) } == 0 { return Ok(true); }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::WouldBlock { Ok(false) } else { Err(Error::Files(error)) }
}
/// The shared hold of one ordinary Store; refused while recovery owns the file.
pub(crate) fn share(db: &Path) -> Result<File> {
    let file = open_lock(db)?;
    if try_flock(&file, libc::LOCK_SH)? { Ok(file) } else { refuse("cutover recovery owns this database") }
}

/// One stage's proof that nothing else has this database open.
#[derive(Debug)]
pub struct JoinedOwners { attempt: String, identity: Option<(u64, u64)>, lock: Option<File> }
impl JoinedOwners {
    pub fn acquire(db: &Path, attempt: &str) -> Result<Self> {
        if attempt.is_empty() { return refuse("empty attempt"); }
        let lock = open_lock(db)?;
        if !try_flock(&lock, libc::LOCK_EX)? { return refuse("another process has this database open; no proof issued"); }
        // Read under the lock, so this names the file the lock protects.
        let meta = std::fs::metadata(db)?;
        Ok(Self { attempt: attempt.into(), identity: Some((meta.dev(), meta.ino())), lock: Some(lock) })
    }
    pub fn attempt(&self) -> &str { &self.attempt }
    /// What this proof rests on, for the attempt's durable record.
    pub(crate) fn receipt(&self) -> Result<String> {
        let lock = self.lock.as_ref().map(|f| f.metadata().map(|m| (m.dev(), m.ino()))).transpose()?;
        Ok(serde_json::to_string(&serde_json::json!({"attempt": self.attempt, "database": self.identity, "lock": lock, "process": std::process::id()}))?)
    }
    /// The proof binds the exact database file it was issued for.
    pub(crate) fn binds(&self, db: &Path) -> bool {
        let Some(identity) = self.identity else { return true };
        std::fs::metadata(db).map(|m| (m.dev(), m.ino()) == identity).unwrap_or(false)
    }
    /// File-backed stores must be the file this proof was issued for.
    pub(crate) fn check_store(&self, conn: &rusqlite::Connection) -> Result<()> {
        match conn.path() {
            Some(p) if !p.is_empty() && !self.binds(Path::new(p)) => refuse("join proof was issued for another database"),
            _ => Ok(()),
        }
    }
    pub(crate) fn into_lock(self) -> Option<File> { self.lock }
    /// An in-memory fixture has no file to lock.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn unit(attempt: &str) -> Self { Self { attempt: attempt.into(), identity: None, lock: None } }
}
