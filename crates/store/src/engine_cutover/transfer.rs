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
use super::{JoinedOwners, Preflight, Result, Error, boundary::reach, refuse, startup::{self, artifact_dir}, preflight};
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
const PRE_LIFT_DB: &str = "pre-lift.db";
const PRE_LIFT: &str = "pre-lift.json";
/// The registry a source is lifted to. Its prefix of `MIGRATIONS` is the one
/// the pinned `SOURCE60_SCHEMA_HASH` describes.
const LIFT_TO: u32 = 60;

pub(super) fn sync_dir(dir: &Path) -> Result<()> { File::open(dir)?.sync_all()?; Ok(()) }
fn sync_parent(db: &Path) -> Result<()> { if let Some(parent) = db.parent() { sync_dir(parent)?; } Ok(()) }
/// `torn` names the boundary where a crash leaves half of this file behind.
pub(super) fn publish(dir: &Path, name: &str, bytes: &[u8], torn: &'static str) -> Result<()> {
    let mut f = OpenOptions::new().write(true).create_new(true).open(dir.join(name))?;
    let (first, rest) = bytes.split_at(bytes.len() / 2);
    f.write_all(first)?;
    reach(torn);
    f.write_all(rest)?;
    f.sync_all()?;
    sync_dir(dir)
}
pub(super) fn db_hash(db: &Path) -> Result<String> { Ok(farcooler_workflow::digest(&fs::read(db)?)) }
pub(super) fn refuse_sidecars(db: &Path) -> Result<()> {
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut p = db.as_os_str().to_owned();
        p.push(suffix);
        if fs::symlink_metadata(PathBuf::from(p)).is_ok() { return refuse("residual WAL/SHM/journal; baseline would be incomplete"); }
    }
    Ok(())
}
/// A journal with no header yet is not hot: its writer died before any page
/// reached the database, as a process killed mid-transaction leaves it, whatever
/// its length. SQLite neither replays nor removes one, and it would refuse the
/// transfer forever. Under the proof, nothing else can own it. It is kept beside
/// the database as `-journal.inert-N`, a name SQLite ignores.
pub(super) fn clear_inert_journal(db: &Path) -> Result<()> {
    startup::settle_journal(db)?;
    let mut name = db.as_os_str().to_owned();
    name.push("-journal");
    let journal = PathBuf::from(name);
    if fs::metadata(&journal).is_ok_and(|m| m.is_file()) {
        let mut first = [0u8; 1];
        let read = File::open(&journal)?.read(&mut first)?;
        if read == 0 || first[0] == 0 {
            // Moved aside, not deleted: after a power cut with the drive cache out
            // of order this is the only trace of what the writer was doing.
            let kept = (1..).map(|n| { let mut name = db.as_os_str().to_owned(); name.push(format!("-journal.inert-{n}")); PathBuf::from(name) })
                .find(|p| fs::symlink_metadata(p).is_err()).expect("an unused name");
            fs::rename(&journal, &kept)?;
            sync_parent(db)?;
        }
    }
    refuse_sidecars(db)
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PreLift { attempt: String, from: u32, to: u32, db_hash: String }

fn schema_below_60(db: &Path) -> Option<u32> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let version = crate::migrate::read_schema_version(&conn).ok()?;
    (1..LIFT_TO).contains(&version).then_some(version)
}
/// The pinned lift pragmas: what an ordinary open sets before it migrates.
fn lift_connection(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON; PRAGMA fullfsync=ON; PRAGMA checkpoint_fullfsync=ON;")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(conn)
}
/// `boundary` names the crash point inside the transaction, for the real file
/// only; the scratch copy passes none.
fn lift(conn: &mut Connection, from: u32, boundary: Option<&'static str>) -> Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    crate::migrate::apply_between(&tx, from, LIFT_TO).map_err(|e| Error::Refused(e.to_string()))?;
    // What the registry-60 build stamped: it needs a build that has its tables.
    tx.execute("INSERT INTO meta (key, value) VALUES ('compatible_down_to', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value", [LIFT_TO.to_string()])?;
    if let Some(name) = boundary { reach(name); }
    tx.commit()?;
    Ok(())
}
/// Bring a source below registry 60 up to exactly 60, so it meets the pinned
/// registry-60 catalog check like any other. The order keeps the original
/// whole at every step:
///
/// 1. The lift runs on a scratch copy first and is judged by `preflight`. A
///    drifted catalog is refused here, with the real file never opened for writing.
/// 2. The original bytes are copied, synced and hashed into `pre-lift.db`.
/// 3. Only then does the real file lift, in one transaction.
///
/// `pre-lift.db` goes wherever the artifact directory goes (Recover retires it,
/// Finish carries it). Recover returns the lifted file, not the original: the
/// original is put back by hand from `pre-lift.db`.
fn lift_below_60(db: &Path, dir: &Path, held: bool, attempt: &str) -> Result<bool> {
    let Some(from) = schema_below_60(db) else { return Ok(false) };
    let mut scratch = db.as_os_str().to_owned();
    scratch.push(".lift-check");
    let scratch = PathBuf::from(scratch);
    let _ = fs::remove_file(&scratch);
    fs::copy(db, &scratch)?;
    let judged = (|| -> Result<()> {
        let mut conn = lift_connection(&scratch)?;
        lift(&mut conn, from, None)?;
        preflight(&conn)?;
        Ok(())
    })();
    let _ = fs::remove_file(&scratch);
    judged?;
    if !held { create_artifacts(dir)?; }
    reach("lift.before-copy");
    let bytes = fs::read(db)?;
    let mut copy = OpenOptions::new().write(true).create_new(true).open(dir.join(PRE_LIFT_DB))?;
    copy.write_all(&bytes)?;
    copy.sync_all()?;
    drop(copy);
    let kept = db_hash(&dir.join(PRE_LIFT_DB))?;
    if kept != farcooler_workflow::digest(&bytes) { return refuse("pre-lift copy differs from the source"); }
    reach("lift.after-copy");
    // Evidence only: nothing reads it back to decide anything, so a tear is harmless.
    publish(dir, PRE_LIFT, &serde_json::to_vec(&PreLift { attempt: attempt.into(), from, to: LIFT_TO, db_hash: kept })?, "lift.torn-note")?;
    reach("lift.after-note");
    let mut conn = lift_connection(db)?;
    lift(&mut conn, from, Some("lift.in-lift"))?;
    drop(conn);
    reach("lift.after-lift");
    Ok(true)
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
    reach("hold.before");
    create_artifacts(&dir)?;
    publish(&dir, HOLD, &serde_json::to_vec(&Hold { attempt: attempt.into() })?, "hold.torn")?;
    sync_parent(db)?;
    reach("hold.after");
    Ok(())
}
/// The attempt a runner is in: its baseline's once Prepare has run, its hold
/// mark's before then, and `None` when it is in neither.
pub fn attempt_in_hand(db: &Path) -> Option<String> {
    if let Ok((baseline, _)) = validate_artifacts(db) { return Some(baseline.attempt); }
    let hold: Hold = serde_json::from_slice(&fs::read(artifact_dir(db).join(HOLD)).ok()?).ok()?;
    Some(hold.attempt)
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
    let lifted = lift_below_60(db, &dir, hold.is_some(), owners.attempt())?;
    let (root, database) = location(db)?;
    let meta = fs::metadata(db)?;
    // Open before SQLite and closed after its commit: closing any descriptor on
    // this file drops every POSIX lock the process holds on it, SQLite's too.
    let mut source = File::open(db)?;
    let mut conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON; PRAGMA fullfsync=ON; PRAGMA checkpoint_fullfsync=ON;")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let plan = preflight(&tx)?;
    if plan.registry >= 66 && super::authority(&tx)?.phase != "legacy" { return refuse("source authority was not legacy"); }
    let resources = super::resource_generations(&tx)?;
    let mut bytes = Vec::new();
    source.seek(SeekFrom::Start(0))?;
    source.read_to_end(&mut bytes)?;
    if hold.is_none() && !lifted { create_artifacts(&dir)?; }
    reach("begin.before-baseline-db");
    // Original, closed, byte-identical copy; synced before the intent exists.
    let mut copy = OpenOptions::new().write(true).create_new(true).open(dir.join(BASELINE_DB))?;
    copy.write_all(&bytes)?;
    copy.sync_all()?;
    let baseline = Baseline { attempt: owners.attempt().into(), root, database, identity: (meta.dev(), meta.ino()),
        registry: plan.registry, floor: crate::compat::read_compatible_down_to(&tx).map_err(|e| Error::Refused(e.to_string()))?,
        catalog_hash: farcooler_workflow::digest(&plan.raw.objects), db_hash: farcooler_workflow::digest(&bytes),
        raw_hash: plan.raw.hash(), owners_hash: farcooler_workflow::digest(&owners.receipt()?), resources };
    if db_hash(&dir.join(BASELINE_DB))? != baseline.db_hash { return refuse("baseline copy differs from source"); }
    reach("begin.after-baseline-db");
    publish(&dir, BASELINE, &serde_json::to_vec(&baseline)?, "begin.torn-baseline")?;
    reach("begin.before-intent");
    let intent = Intent { attempt: baseline.attempt.clone(), baseline_hash: farcooler_workflow::digest(&baseline) };
    publish(&dir, INTENT, &serde_json::to_vec(&intent)?, "begin.torn-intent")?;
    sync_parent(db)?;
    reach("begin.after-intent");
    #[cfg(test)]
    if let Some(hook) = AFTER_CAPTURE.with(|h| h.borrow_mut().take()) { hook(); }
    // Atomic upgrade: DDL + version + refusing floor + frozen attempt together.
    freeze(&tx, plan.registry, &baseline)?;
    reach("begin.in-upgrade");
    tx.commit()?;
    reach("begin.after-upgrade");
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
            reach("recover.before-rename");
            fs::rename(&restore, db)?;
            sync_parent(db)?;
            reach("recover.after-rename");
            let evidence = retire(db, "abandoned")?;
            reach("recover.after-retire");
            Ok(Recovered { restored: true, evidence })
        }
        Some(_) => refuse("committed authority recovers forward only"),
    }
}
/// The database's frozen attempt must be the one these artifacts describe.
pub(super) fn frozen_as(conn: &Connection, baseline: &Baseline) -> Result<()> {
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
    super::closing::record_up_to(db, &baseline, "resolved")?;
    let dir = artifact_dir(db);
    let mut done = dir.as_os_str().to_owned();
    done.push(format!("-resolved-{}", baseline.attempt));
    let done = PathBuf::from(done);
    reach("finish.before-rename");
    fs::rename(&dir, &done)?;
    sync_parent(db)?;
    reach("finish.after-rename");
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
