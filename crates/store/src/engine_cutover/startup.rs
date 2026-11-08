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
    inspect_against(conn, CURRENT_SCHEMA_VERSION)
}

/// `inspect` for a build whose schema is `current`. A database below it is
/// admitted only if it is a RELEASED install: past the cutover, stamped by the
/// build that wrote it with exactly the floor that build owed it. Anything
/// else below `current` (no authority table, a legacy, frozen or committed
/// authority, a missing or hand-moved floor) still goes to the explicit
/// cutover or recovery. Admission never migrates; `Store::open` does, after.
pub(crate) fn inspect_against(conn: &Connection, current: u32) -> Result<Admission> {
    let has = |name: &str| -> Result<bool> {
        Ok(conn.query_row("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1", [name], |_| Ok(())).optional()?.is_some())
    };
    if !has("meta")? { return refuse("existing database has no schema marker; not a fresh install"); }
    let meta = |key: &str| -> Result<Option<u32>> {
        let raw: Option<String> = conn.query_row("SELECT value FROM meta WHERE key=?1", [key], |r| r.get(0)).optional()?;
        Ok(raw.and_then(|s| s.parse().ok()))
    };
    let Some(version) = meta("schema_version")? else { return refuse("existing database has no schema version; not a fresh install") };
    if version > current { return Ok(Admission::Newer); }
    let owed_floor = if version == current { COMPATIBLE_DOWN_TO } else { crate::compat::floor_at(version) };
    if version < current && !has("engine_cutover_authority")? { return refuse("pre-cutover schema requires the explicit authenticated cutover"); }
    if meta("compatible_down_to")? != Some(owed_floor) { return refuse("compatibility floor is missing or permissive; recovery required"); }
    if !has("engine_cutover_authority")? { return refuse("authority table missing at current schema"); }
    let phase: String = conn.query_row("SELECT phase FROM engine_cutover_authority WHERE singleton=1", [], |r| r.get(0))
        .map_err(|_| Error::Refused("authority row unreadable".into()))?;
    match phase.as_str() {
        "released" => Ok(Admission::Existing),
        "legacy" if version == current => Ok(Admission::Existing),
        "legacy" => refuse("pre-cutover schema requires the explicit authenticated cutover"),
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
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON; PRAGMA fullfsync=ON; PRAGMA checkpoint_fullfsync=ON;")?;
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

/// Whether `terminal` is still `workspace`'s orchestrator on the runner's own
/// login, as the database says now (ov-974): its role, no member login and no
/// rep on the pane, and no member leading the workspace. Read-only, for a
/// recovery process, which holds no Store; nothing else can write these while
/// recovery holds the file, so this guards an edited or restored file.
pub fn owners_orchestrator(db: &Path, terminal: uuid::Uuid, workspace: uuid::Uuid) -> Result<bool> {
    let conn = readonly(db)?;
    let (t, w) = (terminal.as_bytes().to_vec(), workspace.as_bytes().to_vec());
    let role: Option<(i64, Option<Vec<u8>>)> =
        conn.query_row("SELECT role, workspace_id FROM terminals WHERE id = ?1", [&t], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    let Some((role, Some(home))) = role else { return Ok(false) };
    if role != crate::models::TerminalRole::Orchestrator.as_i64() { return Ok(false); }
    let none = |sql: &str, id: &Vec<u8>| -> Result<bool> { Ok(conn.query_row(sql, [id], |_| Ok(())).optional()?.is_none()) };
    Ok(home == w
        && none("SELECT 1 FROM terminal_logins WHERE terminal_id = ?1", &t)?
        && none("SELECT 1 FROM reps WHERE terminal_id = ?1", &t)?
        && none("SELECT 1 FROM workspace_leads WHERE workspace_id = ?1", &w)?)
}

/// Whether the owner's grant `answer` still admits cutover `attempt` at
/// `now_ms` (ov-974), as the database says now, read-only for a recovery
/// process: the owner's own answer, said from the app or their phone
/// (`said_from`), no older than `max_age_ms`, and spent by the runner on this
/// attempt and no other, a spend given back not counting (`cutover_grant` in
/// the daemon writes these keys).
pub fn grant_holds(db: &Path, answer: uuid::Uuid, attempt: &str, now_ms: i64, max_age_ms: i64) -> Result<bool> {
    let conn = readonly(db)?;
    let row: Option<(Vec<u8>, String, String, i64, String)> = conn
        .query_row("SELECT task_id, kind, actor, at, extra FROM task_notes WHERE id = ?1", [answer.as_bytes().to_vec()], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .optional()?;
    let Some((task, kind, actor, at, extra)) = row else { return Ok(false) };
    let extra: serde_json::Value = serde_json::from_str(&extra).unwrap_or_default();
    let said = matches!(extra.get("said_from").and_then(|v| v.as_str()), Some("app" | "device"));
    if kind != "answer" || actor != "user" || !said || at > now_ms || now_ms - at > max_age_ms {
        return Ok(false);
    }
    let id = answer.to_string();
    let mut notes = conn.prepare("SELECT extra FROM task_notes WHERE task_id = ?1 AND actor = 'runner' ORDER BY at, rowid")?;
    let extras: Vec<serde_json::Value> = notes
        .query_map([task], |r| r.get::<_, String>(0))?
        .filter_map(|e| e.ok())
        .filter_map(|e| serde_json::from_str::<serde_json::Value>(&e).ok())
        .collect();
    let by = |key: &str| -> Vec<String> {
        extras.iter()
            .filter_map(|e| e.get(key))
            .filter(|s| s.get("answer").and_then(|a| a.as_str()) == Some(id.as_str()))
            .map(|s| s.get("attempt").and_then(|a| a.as_str()).unwrap_or_default().to_string())
            .collect()
    };
    // A spend a failed hold gave back (`cutover_grant_returned`) is no spend.
    let mut returned = by("cutover_grant_returned");
    let spent_on: Vec<String> = by("cutover_grant").into_iter().filter(|attempt| match returned.iter().position(|r| r == attempt) {
        Some(at) => { returned.remove(at); false }
        None => true,
    }).collect();
    Ok(spent_on == [attempt])
}
