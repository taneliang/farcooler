//! Join proof for writer owners. `Quiescence` is serializable EVIDENCE only; the
//! authority-bearing value is `JoinedOwners`, which has no public constructor,
//! no `Clone`, and no serde. Only `OwnerJoin::join` issues it, and only after
//! this module itself drove every retained owner's close-and-join to success.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::os::unix::fs::MetadataExt;
use super::{Quiescence, StoppedWriter, Result, refuse};

/// A retained writer owner (task handle, owned child group, worker...). The
/// implementor must have held the actual handle since admission; stored PID or
/// PGID values are never an implementation.
pub trait OwnedWriter: Send {
    fn identity(&self) -> String;
    fn incarnation(&self) -> String;
    fn resources(&self) -> BTreeSet<String>;
    /// Close admission for this owner, drain it through its final effect and
    /// join it. Err on any unjoined, timed-out, panicked or unresolved owner.
    fn close_and_join(&mut self) -> std::result::Result<String, String>;
}

pub struct OwnerJoin { attempt: String, writers: Vec<Box<dyn OwnedWriter>> }
impl OwnerJoin {
    pub fn new(attempt: &str) -> Self { Self { attempt: attempt.into(), writers: Vec::new() } }
    pub fn retain(&mut self, writer: Box<dyn OwnedWriter>) { self.writers.push(writer); }
    /// Joins every owner (all are attempted, none skipped on an earlier error)
    /// and issues the proof only if all succeeded and the inventory is exactly
    /// complete for `db`'s protected resources.
    pub fn join(mut self, db: &Path, protected: &BTreeSet<String>) -> Result<JoinedOwners> {
        if self.attempt.is_empty() { return refuse("empty attempt"); }
        let required: BTreeSet<String> = super::REQUIRED_OWNERS.iter().map(|s| (*s).to_owned()).collect();
        let mut stopped = Vec::new();
        let mut failed = false;
        for w in &mut self.writers {
            match w.close_and_join() {
                Ok(receipt) if !receipt.is_empty() => stopped.push(StoppedWriter {
                    identity: w.identity(), incarnation: w.incarnation(), joined_receipt: receipt, resources: w.resources() }),
                _ => failed = true,
            }
        }
        if failed { return refuse("an owner did not join; no quiescence proof issued"); }
        let inventory: BTreeSet<String> = stopped.iter().map(|s| s.identity.clone()).collect();
        let evidence = Quiescence { inventory, stopped, protected_resources: protected.clone() };
        if !required.is_subset(&evidence.inventory) { return refuse("writer inventory incomplete; unknown owners block"); }
        evidence.validate()?;
        let meta = std::fs::metadata(db)?;
        Ok(JoinedOwners { attempt: self.attempt, db: db.to_path_buf(), identity: (meta.dev(), meta.ino()), evidence })
    }
}

/// One-attempt opaque proof. Deliberately not Clone/Serialize/Deserialize.
#[derive(Debug)]
pub struct JoinedOwners { attempt: String, db: PathBuf, identity: (u64, u64), evidence: Quiescence }
impl JoinedOwners {
    pub fn attempt(&self) -> &str { &self.attempt }
    pub fn evidence(&self) -> &Quiescence { &self.evidence }
    /// The proof binds the exact database file it was issued for.
    pub(crate) fn binds(&self, db: &Path) -> bool {
        #[cfg(test)]
        if self.db == Path::new("unit") { return true; }
        std::fs::metadata(db).map(|m| (m.dev(), m.ino()) == self.identity).unwrap_or(false) && !self.db.as_os_str().is_empty()
    }
    /// File-backed stores must be the file this proof was issued for.
    pub(crate) fn check_store(&self, conn: &rusqlite::Connection) -> Result<()> {
        match conn.path() {
            Some(p) if !p.is_empty() && !self.binds(Path::new(p)) => refuse("join proof was issued for another database"),
            _ => Ok(()),
        }
    }
    #[cfg(test)]
    pub(crate) fn unit(evidence: Quiescence, attempt: &str) -> Self {
        Self { attempt: attempt.into(), db: PathBuf::from("unit"), identity: (0, 0), evidence }
    }
}
