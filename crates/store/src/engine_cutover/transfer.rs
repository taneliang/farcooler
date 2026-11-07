//! C1 durable pre-upgrade barrier: under one write transaction, capture the raw
//! source, publish the recoverable baseline and exclusive intent, then upgrade
//! (remaining DDL + version + refusing floor + bound frozen attempt). No writer
//! can land between capture and upgrade. Nothing here recaptures upgraded rows
//! as raw, falls back to an ordinary open, or takes a marker for revocation of
//! a writer's descriptors. An uncommitted attempt has one exit, `recover`.
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use super::{JoinedOwners, Preflight, Result, Error, refuse, startup::{self, artifact_dir}, preflight};
use crate::Store;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Baseline {
    pub attempt: String,
    pub root: PathBuf,
    pub database: String,
    pub identity: (u64, u64),
    pub registry: u32,
    pub floor: Option<u32>,
    pub catalog_hash: String,
    pub db_hash: String,
    pub raw_hash: String,
    pub owners_hash: String,
    pub resources: BTreeMap<String, Option<i64>>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Intent { pub attempt: String, pub baseline_hash: String }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Hold { attempt: String }

const BASELINE_DB: &str = "baseline.db";
const BASELINE: &str = "baseline.json";
const INTENT: &str = "intent.json";
const HOLD: &str = "hold.json";
const REPLAY_DB: &str = "replay.db";
const RESTORE_DB: &str = "restore.db";

fn sync_dir(dir: &Path) -> Result<()> { File::open(dir)?.sync_all()?; Ok(()) }
fn sync_parent(db: &Path) -> Result<()> { if let Some(parent) = db.parent() { sync_dir(parent)?; } Ok(()) }
fn publish(dir: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    let mut f = OpenOptions::new().write(true).create_new(true).open(dir.join(name))?;
    f.write_all(bytes)?;
    f.sync_all()?;
    sync_dir(dir)
}
fn db_hash(db: &Path) -> Result<String> { Ok(farcooler_workflow::digest(&fs::read(db)?)) }
fn refuse_sidecars(db: &Path) -> Result<()> {
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut p = db.as_os_str().to_owned();
        p.push(suffix);
        if fs::symlink_metadata(PathBuf::from(p)).is_ok() { return refuse("residual WAL/SHM/journal; baseline would be incomplete"); }
    }
    Ok(())
}
/// An empty journal is not hot: SQLite neither replays nor removes it, and it
/// would refuse the transfer forever. Under the proof, nothing else can own it.
fn clear_inert_journal(db: &Path) -> Result<()> {
    startup::settle_journal(db)?;
    let mut name = db.as_os_str().to_owned();
    name.push("-journal");
    let journal = PathBuf::from(name);
    if fs::metadata(&journal).is_ok_and(|m| m.is_file() && m.len() == 0) { fs::remove_file(&journal)?; }
    refuse_sidecars(db)
}
fn frozen_id(attempt: &str) -> String { format!("frozen:{attempt}") }
fn create_artifacts(dir: &Path) -> Result<()> {
    fs::create_dir(dir).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => Error::Refused("transfer artifacts already exist; recover instead".into()),
        _ => Error::Files(e),
    })
}

/// The live daemon's half: mark `attempt` so the next start serves recovery
/// instead of ordinary service. Nothing is captured here; that needs a process
/// with no writers, which is what the restart makes.
pub fn hold(db: &Path, attempt: &str) -> Result<()> {
    // No authority table is a source below 67; a database past legacy has nothing to hold.
    if startup::read_authority(db)?.is_some_and(|a| a.phase != "legacy") { return refuse("only a legacy database can be held for cutover"); }
    let dir = artifact_dir(db);
    create_artifacts(&dir)?;
    publish(&dir, HOLD, &serde_json::to_vec(&Hold { attempt: attempt.into() })?)?;
    sync_parent(db)
}
fn held(dir: &Path) -> Result<Option<Hold>> {
    if fs::symlink_metadata(dir).is_err() { return Ok(None); }
    match fs::read(dir.join(HOLD)) {
        Ok(raw) => Ok(Some(serde_json::from_slice(&raw)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => refuse("transfer artifacts already exist; recover instead"),
        Err(e) => Err(e.into()),
    }
}

/// Validate the published baseline+intent, and that they are this database's:
/// a copied root with its artifacts, or a replaced file, binds to nothing.
pub fn validate_artifacts(db: &Path) -> Result<(Baseline, Intent)> {
    let dir = artifact_dir(db);
    let baseline: Baseline = serde_json::from_slice(&fs::read(dir.join(BASELINE))?)?;
    let intent: Intent = serde_json::from_slice(&fs::read(dir.join(INTENT))?)?;
    if intent.attempt != baseline.attempt || intent.baseline_hash != farcooler_workflow::digest(&baseline) {
        return refuse("intent does not reference this baseline");
    }
    if db_hash(&dir.join(BASELINE_DB))? != baseline.db_hash { return refuse("baseline database changed or truncated"); }
    let (root, database) = location(db)?;
    let meta = fs::metadata(db)?;
    if baseline.root != root || baseline.database != database || baseline.identity != (meta.dev(), meta.ino()) {
        return refuse("baseline belongs to another root or database file");
    }
    Ok((baseline, intent))
}
fn location(db: &Path) -> Result<(PathBuf, String)> {
    let Some(name) = db.file_name().and_then(|n| n.to_str()) else { return refuse("database has no file name") };
    let parent = db.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    Ok((parent.canonicalize()?, name.to_owned()))
}

/// The one upgrade: replayed verbatim by `recover` to prove what it produced.
fn freeze(tx: &Transaction<'_>, registry: u32, baseline: &Baseline) -> Result<()> {
    crate::migrate::apply_remaining(tx, registry).map_err(|e| Error::Refused(e.to_string()))?;
    crate::compat::stamp_compatible_down_to(tx).map_err(|e| Error::Refused(e.to_string()))?;
    let changed = tx.execute("UPDATE engine_cutover_authority SET phase='quiesced',manifest=?1 WHERE singleton=1 AND phase='legacy'", [&baseline.raw_hash])?;
    if changed != 1 { return refuse("source authority was not legacy"); }
    tx.execute("INSERT INTO engine_cutover_attempts VALUES(?1,?2,?3)", rusqlite::params![frozen_id(&baseline.attempt), baseline.raw_hash, baseline.owners_hash])?;
    Ok(())
}
#[cfg(test)]
thread_local! { pub(super) static AFTER_CAPTURE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) }; }

/// Capture, publish and upgrade. Returns the frozen Store, which keeps the
/// owner lock, and the plan from the ORIGINAL source. A refusal before capture
/// leaves no artifact; a failure after it leaves the intent for `recover`.
pub fn begin(db: &Path, owners: JoinedOwners) -> Result<(Store, Preflight)> {
    if !owners.binds(db) { return refuse("join proof was issued for another database"); }
    clear_inert_journal(db)?;
    let dir = artifact_dir(db);
    let hold = held(&dir)?;
    if hold.as_ref().is_some_and(|h| h.attempt != owners.attempt()) { return refuse("another attempt holds this database"); }
    let (root, database) = location(db)?;
    let meta = fs::metadata(db)?;
    // Open before SQLite and closed after its commit: closing any descriptor on
    // this file drops every POSIX lock the process holds on it, SQLite's too.
    let mut source = File::open(db)?;
    let mut conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON;")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let plan = preflight(&tx)?;
    if plan.registry >= 66 && super::authority(&tx)?.phase != "legacy" { return refuse("source authority was not legacy"); }
    let resources = super::resource_generations(&tx)?;
    let mut bytes = Vec::new();
    source.seek(SeekFrom::Start(0))?;
    source.read_to_end(&mut bytes)?;
    if hold.is_none() { create_artifacts(&dir)?; }
    // Original, closed, byte-identical copy; synced before the intent exists.
    let mut copy = OpenOptions::new().write(true).create_new(true).open(dir.join(BASELINE_DB))?;
    copy.write_all(&bytes)?;
    copy.sync_all()?;
    let baseline = Baseline { attempt: owners.attempt().into(), root, database, identity: (meta.dev(), meta.ino()),
        registry: plan.registry, floor: crate::compat::read_compatible_down_to(&tx).map_err(|e| Error::Refused(e.to_string()))?,
        catalog_hash: farcooler_workflow::digest(&plan.raw.objects), db_hash: farcooler_workflow::digest(&bytes),
        raw_hash: plan.raw.hash(), owners_hash: farcooler_workflow::digest(&owners.receipt()?), resources };
    if db_hash(&dir.join(BASELINE_DB))? != baseline.db_hash { return refuse("baseline copy differs from source"); }
    publish(&dir, BASELINE, &serde_json::to_vec(&baseline)?)?;
    let intent = Intent { attempt: baseline.attempt.clone(), baseline_hash: farcooler_workflow::digest(&baseline) };
    publish(&dir, INTENT, &serde_json::to_vec(&intent)?)?;
    sync_parent(db)?;
    #[cfg(test)]
    if let Some(hook) = AFTER_CAPTURE.with(|h| h.borrow_mut().take()) { hook(); }
    // Atomic upgrade: DDL + version + refusing floor + frozen attempt together.
    freeze(&tx, plan.registry, &baseline)?;
    tx.commit()?;
    drop(source);
    let store = Store::from_engine_cutover_connection(conn, owners.into_lock()).map_err(|e| Error::Refused(e.to_string()))?;
    Ok((store, plan))
}

/// What `freeze` makes of the baseline, on a scratch copy of it.
fn replayed(dir: &Path, baseline: &Baseline) -> Result<String> {
    let path = dir.join(REPLAY_DB);
    fs::copy(dir.join(BASELINE_DB), &path)?;
    let mut conn = Connection::open(&path)?;
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON;")?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    freeze(&tx, baseline.registry, baseline)?;
    tx.commit()?;
    let hash = super::Manifest::capture(&conn)?.hash();
    drop(conn);
    fs::remove_file(&path)?;
    Ok(hash)
}
/// Keep the artifacts as evidence under a name ordinary startup ignores.
fn retire(db: &Path, label: &str) -> Result<PathBuf> {
    let dir = artifact_dir(db);
    let done = (1..).map(|n| { let mut name = dir.as_os_str().to_owned(); name.push(format!("-{label}-{n}")); PathBuf::from(name) })
        .find(|p| fs::symlink_metadata(p).is_err()).expect("an unused name");
    fs::rename(&dir, &done)?;
    sync_parent(db)?;
    Ok(done)
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Recovered { pub restored: bool, pub evidence: PathBuf }

/// The exit from an uncommitted attempt. Upgrade and freeze commit together, so
/// a database not frozen was never upgraded and the artifacts are simply
/// retired. A frozen one is returned to the original bytes, only if it is still
/// exactly what the upgrade made: a write since then is data this must not
/// lose, so it refuses and leaves everything. Committed state is forward-only.
pub fn recover(db: &Path, owners: JoinedOwners) -> Result<Recovered> {
    if !owners.binds(db) { return refuse("join proof was issued for another database"); }
    let dir = artifact_dir(db);
    if fs::symlink_metadata(&dir).is_err() { return refuse("no attempt to recover"); }
    // Before any byte moves: a journal left beside restored bytes would replay.
    clear_inert_journal(db)?;
    // A hold alone moved nothing, in whatever phase the database has since reached.
    if fs::symlink_metadata(dir.join(BASELINE)).is_err() && fs::symlink_metadata(dir.join(INTENT)).is_err() {
        return Ok(Recovered { restored: false, evidence: retire(db, "abandoned")? });
    }
    match startup::read_authority(db)?.map(|a| a.phase).as_deref() {
        None | Some("legacy") => Ok(Recovered { restored: false, evidence: retire(db, "abandoned")? }),
        Some("quiesced") => {
            let (baseline, _) = validate_artifacts(db)?;
            let current = {
                let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
                frozen_as(&conn, &baseline)?;
                super::Manifest::capture(&conn)?.hash()
            };
            if current != replayed(&dir, &baseline)? { return refuse("database changed since the upgrade; keeping it for review"); }
            let restore = dir.join(RESTORE_DB);
            let mut copy = OpenOptions::new().write(true).create(true).truncate(true).open(&restore)?;
            copy.write_all(&fs::read(dir.join(BASELINE_DB))?)?;
            copy.sync_all()?;
            copy.set_permissions(fs::metadata(db)?.permissions())?;
            drop(copy);
            if db_hash(&restore)? != baseline.db_hash { return refuse("restore copy differs from baseline"); }
            fs::rename(&restore, db)?;
            sync_parent(db)?;
            Ok(Recovered { restored: true, evidence: retire(db, "abandoned")? })
        }
        Some(_) => refuse("committed authority recovers forward only"),
    }
}
/// The database's frozen attempt must be the one these artifacts describe.
fn frozen_as(conn: &Connection, baseline: &Baseline) -> Result<()> {
    let manifest: Option<String> = conn.query_row("SELECT manifest FROM engine_cutover_attempts WHERE id=?1", [frozen_id(&baseline.attempt)], |r| r.get(0)).optional()?;
    if manifest.as_deref() != Some(baseline.raw_hash.as_str()) { return refuse("database was frozen by another attempt"); }
    Ok(())
}

/// Terminal disposition: only after DB truth shows this attempt's baseline was
/// verified and released does the intent stop blocking ordinary startup. The
/// evidence directory is renamed, never deleted, and never reused.
pub fn finish(db: &Path, store: &Store, owners: JoinedOwners) -> Result<PathBuf> {
    if !owners.binds(db) { return refuse("join proof was issued for another database"); }
    let (baseline, _) = validate_artifacts(db)?;
    frozen_as(&store.conn(), &baseline)?;
    let authority = store.engine_authority()?;
    if authority.phase != "released" || authority.manifest.as_deref() != Some(baseline.raw_hash.as_str()) {
        return refuse("attempt is not verified and released against its baseline");
    }
    let dir = artifact_dir(db);
    let mut done = dir.as_os_str().to_owned();
    done.push(format!("-resolved-{}", baseline.attempt));
    let done = PathBuf::from(done);
    fs::rename(&dir, &done)?;
    sync_parent(db)?;
    Ok(done)
}

/// The plan recomputed from the ORIGINAL baseline copy, after validating the
/// artifacts; recovery never recaptures upgraded rows as the raw source.
pub fn baseline_plan(db: &Path) -> Result<(Baseline, Preflight)> {
    let (baseline, _) = validate_artifacts(db)?;
    let ro = Connection::open_with_flags(artifact_dir(db).join(BASELINE_DB), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let plan = preflight(&ro)?;
    if plan.raw.hash() != baseline.raw_hash { return refuse("baseline no longer matches its recorded raw hash"); }
    Ok((baseline, plan))
}
/// `baseline_plan`, and that `store` holds this attempt over these resources.
pub fn bound_plan(db: &Path, store: &Store) -> Result<(Baseline, Preflight)> {
    let (baseline, plan) = baseline_plan(db)?;
    frozen_as(&store.conn(), &baseline)?;
    if !baseline.resources.keys().eq(store.engine_protected_resources()?.iter()) { return refuse("protected resources changed since the baseline"); }
    Ok((baseline, plan))
}
