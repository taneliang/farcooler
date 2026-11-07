//! Read-only admission gate that every ordinary production open runs BEFORE any
//! migration, compatibility stamp, wait sweep or helper. It never writes.
//! A surviving transfer artifact, a pre-cutover schema, a frozen authority or a
//! permissive/missing floor all refuse ordinary startup and route to the
//! explicit authenticated cutover/recovery entry; absence of a marker is never
//! proof that an existing file is a fresh install. The one write it can cause is
//! SQLite rolling back a crashed writer's journal, which restores committed
//! bytes and is what any read of the file would do.
use std::path::{Path, PathBuf};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use super::{Authority, Error, JoinedOwners, Result, refuse};
use crate::compat::COMPATIBLE_DOWN_TO;
use crate::migrate::CURRENT_SCHEMA_VERSION;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    /// Database file and every transfer artifact proven absent.
    Fresh,
    /// Existing current-schema database whose authority admits engine work.
    Existing,
    /// Written by a newer build; the compatibility gate decides.
    Newer,
}

/// Directory holding the raw baseline and attempt intent for `db`.
pub fn artifact_dir(db: &Path) -> PathBuf {
    let mut name = db.as_os_str().to_owned();
    name.push(".cutover");
    PathBuf::from(name)
}

const BUSY: std::time::Duration = std::time::Duration::from_secs(5);
pub fn crash_journal(db: &Path) -> bool {
    let mut name = db.as_os_str().to_owned();
    name.push("-journal");
    std::fs::symlink_metadata(PathBuf::from(name)).is_ok()
}
/// A read-only connection cannot roll back a crashed writer's journal and fails
/// on one, so a writable connection reads once to let SQLite do it. A journal
/// a live writer holds is not hot, and this leaves it alone.
pub(crate) fn settle_journal(db: &Path) -> Result<()> {
    if !crash_journal(db) { return Ok(()); }
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.busy_timeout(BUSY)?;
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))?;
    Ok(())
}
fn readonly(db: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| Error::Refused("existing database cannot be inspected read-only".into()))?;
    conn.busy_timeout(BUSY)?;
    Ok(conn)
}
/// The gate's answer without touching the file: what `inspect` reports.
pub fn readonly_admission(db: &Path) -> Result<Admission> {
    if std::fs::symlink_metadata(artifact_dir(db)).is_ok() {
        return refuse("cutover transfer artifacts exist; use authenticated recovery");
    }
    match std::fs::symlink_metadata(db) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Admission::Fresh),
        Err(e) => return Err(e.into()),
        Ok(m) if !m.is_file() => return refuse("database path is not a regular file"),
        Ok(_) => {}
    }
    inspect(&readonly(db)?)
}
pub fn ordinary_admission(db: &Path) -> Result<Admission> {
    if std::fs::symlink_metadata(artifact_dir(db)).is_err() { settle_journal(db)?; }
    readonly_admission(db)
}
/// The authority row as it stands, `None` before the table exists. Read-only.
pub fn read_authority(db: &Path) -> Result<Option<Authority>> {
    let conn = readonly(db)?;
    if conn.query_row("SELECT 1 FROM sqlite_master WHERE name='engine_cutover_authority'", [], |_| Ok(())).optional()?.is_none() { return Ok(None); }
    Ok(Some(super::authority(&conn)?))
}

pub(crate) fn inspect(conn: &Connection) -> Result<Admission> {
    let has = |name: &str| -> Result<bool> {
        Ok(conn.query_row("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1", [name], |_| Ok(())).optional()?.is_some())
    };
    if !has("meta")? { return refuse("existing database has no schema marker; not a fresh install"); }
    let meta = |key: &str| -> Result<Option<u32>> {
        let raw: Option<String> = conn.query_row("SELECT value FROM meta WHERE key=?1", [key], |r| r.get(0)).optional()?;
        Ok(raw.and_then(|s| s.parse().ok()))
    };
    let Some(version) = meta("schema_version")? else { return refuse("existing database has no schema version; not a fresh install") };
    if version > CURRENT_SCHEMA_VERSION { return Ok(Admission::Newer); }
    if version < CURRENT_SCHEMA_VERSION { return refuse("pre-cutover schema requires the explicit authenticated cutover"); }
    if meta("compatible_down_to")? != Some(COMPATIBLE_DOWN_TO) { return refuse("compatibility floor is missing or permissive; recovery required"); }
    if !has("engine_cutover_authority")? { return refuse("authority table missing at current schema"); }
    let phase: String = conn.query_row("SELECT phase FROM engine_cutover_authority WHERE singleton=1", [], |r| r.get(0))
        .map_err(|_| Error::Refused("authority row unreadable".into()))?;
    match phase.as_str() {
        "legacy" | "released" => Ok(Admission::Existing),
        _ => refuse("cutover is frozen or committed; unverified authority is not admitted"),
    }
}

/// Authenticated admin/recovery open: reads existing current-schema truth and
/// NEVER migrates, stamps, sweeps or falls back to the ordinary open. Frozen,
/// quiesced and committed states are reachable here, for the owner of `proof`.
pub fn open_recovery(db: &Path, proof: &JoinedOwners) -> Result<crate::Store> {
    if !proof.binds(db) { return refuse("join proof was issued for another database"); }
    settle_journal(db)?;
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON;")?;
    let version: Option<String> = conn.query_row("SELECT value FROM meta WHERE key='schema_version'", [], |r| r.get(0)).optional()?;
    if version.and_then(|v| v.parse::<u32>().ok()) != Some(CURRENT_SCHEMA_VERSION) {
        return refuse("recovery open needs the current schema; run the explicit transfer");
    }
    crate::Store::from_engine_cutover_connection(conn, None).map_err(|e| Error::Refused(e.to_string()))
}

/// The ordinary Store open's gate, with a loud log and an opaque domain error.
/// Run under the shared owner lock it returns, so a transfer cannot begin
/// between this answer and the migration that relies on it.
pub(crate) fn admit_or_refuse(db: &Path) -> farcooler_core::Result<std::fs::File> {
    let refused = |e: Error| { tracing::warn!(error=%e, "ordinary open refused before normalization"); farcooler_core::DomainError::CutoverHeld };
    let lock = super::owners::share(db).map_err(refused)?;
    ordinary_admission(db).map_err(refused)?;
    Ok(lock)
}
