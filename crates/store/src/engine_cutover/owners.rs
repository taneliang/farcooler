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
//! A process that predates this lock takes no part in it, so the proof also
//! asks the kernel who has the database file open and refuses while anyone else
//! does: such a process would otherwise keep writing to the inode Recover
//! replaces. One that opens the file after this reads the refusing floor.
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

/// Every other process with `db` open, however it got there. A scan that cannot
/// answer is an error, never an empty list. Nothing here signals anyone.
#[cfg(target_os = "macos")]
pub fn other_holders(db: &Path) -> Result<Vec<i32>> {
    use std::os::unix::ffi::OsStrExt;
    const PROC_ALL_PIDS: u32 = 1;
    const EXCLUDE_EVTONLY: u32 = 2;
    unsafe extern "C" {
        fn proc_listpidspath(kind: u32, info: u32, path: *const libc::c_char, flags: u32, buffer: *mut libc::c_void, size: libc::c_int) -> libc::c_int;
    }
    let path = std::ffi::CString::new(db.as_os_str().as_bytes()).map_err(|_| Error::Refused("database path holds a NUL".into()))?;
    let mut pids = vec![0i32; 4096];
    let size = (pids.len() * size_of::<i32>()) as libc::c_int;
    // SAFETY: `path` is a NUL-terminated string and `pids` has `size` writable bytes, both for the whole call.
    let used = unsafe { proc_listpidspath(PROC_ALL_PIDS, 0, path.as_ptr(), EXCLUDE_EVTONLY, pids.as_mut_ptr().cast(), size) };
    if used < 0 { return Err(Error::Files(std::io::Error::last_os_error())); }
    if used >= size { return refuse("too many processes have this database open to name"); }
    pids.truncate(used as usize / size_of::<i32>());
    pids.retain(|&pid| pid != std::process::id() as i32);
    Ok(pids)
}
/// `/proc`, by device and inode. Another user's process can't be read, and the
/// database is private to this one.
#[cfg(not(target_os = "macos"))]
pub fn other_holders(db: &Path) -> Result<Vec<i32>> {
    let wanted = std::fs::metadata(db)?;
    let mut found = Vec::new();
    for process in std::fs::read_dir("/proc")? {
        let process = process?;
        let Some(pid) = process.file_name().to_str().and_then(|n| n.parse::<i32>().ok()) else { continue };
        if pid == std::process::id() as i32 { continue; }
        let Ok(descriptors) = std::fs::read_dir(process.path().join("fd")) else { continue };
        for descriptor in descriptors.flatten() {
            if std::fs::metadata(descriptor.path()).is_ok_and(|m| (m.dev(), m.ino()) == (wanted.dev(), wanted.ino())) { found.push(pid); break; }
        }
    }
    Ok(found)
}

/// `other_holders`, given a moment. A file indexer's read, or a child between
/// its fork and exec, is gone at once; a process that stays is a holder.
fn settled_holders(db: &Path) -> Result<Vec<i32>> {
    let mut others = other_holders(db)?;
    for _ in 0..10 {
        if others.is_empty() { break; }
        std::thread::sleep(std::time::Duration::from_millis(50));
        others = other_holders(db)?;
    }
    Ok(others)
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
        let others = settled_holders(db)?;
        if !others.is_empty() { return Err(Error::Refused(format!("process {others:?} has this database open; no proof issued"))); }
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
