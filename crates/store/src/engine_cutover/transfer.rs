//! C1 durable pre-upgrade barrier: recoverable raw baseline -> exclusive intent
//! -> ONE transaction (remaining DDL + version + refusing floor + frozen
//! authority). Nothing here recaptures upgraded rows as raw, falls back to an
//! ordinary open, or treats a marker as revocation of a writer's descriptors:
//! the caller must already hold a `JoinedOwners` proof for this database file.
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use serde::{Deserialize, Serialize};
use super::{JoinedOwners, Preflight, Result, Error, refuse, startup::artifact_dir, preflight};
use crate::Store;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Baseline {
    pub attempt: String,
    pub registry: u32,
    pub db_hash: String,
    pub raw_hash: String,
    pub owners_hash: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Intent { pub attempt: String, pub baseline_hash: String }

const BASELINE_DB: &str = "baseline.db";
const BASELINE: &str = "baseline.json";
const INTENT: &str = "intent.json";

fn sync_dir(dir: &Path) -> Result<()> { File::open(dir)?.sync_all()?; Ok(()) }
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

/// Validate the published baseline+intent against the preserved original file.
pub fn validate_artifacts(db: &Path) -> Result<(Baseline, Intent)> {
    let dir = artifact_dir(db);
    let baseline: Baseline = serde_json::from_slice(&fs::read(dir.join(BASELINE))?)?;
    let intent: Intent = serde_json::from_slice(&fs::read(dir.join(INTENT))?)?;
    if intent.attempt != baseline.attempt || intent.baseline_hash != farcooler_workflow::digest(&baseline) {
        return refuse("intent does not reference this baseline");
    }
    if db_hash(&dir.join(BASELINE_DB))? != baseline.db_hash { return refuse("baseline database changed or truncated"); }
    Ok((baseline, intent))
}

/// Capture, publish and upgrade. Returns the frozen Store and the plan computed
/// from the ORIGINAL source. Fails closed at every step; an existing artifact
/// directory is never replaced (admin recovery re-validates it instead).
pub fn begin(db: &Path, owners: &JoinedOwners) -> Result<(Store, Preflight)> {
    if !owners.binds(db) { return refuse("join proof was issued for another database"); }
    owners.evidence().validate()?;
    refuse_sidecars(db)?;
    let dir = artifact_dir(db);
    fs::create_dir(&dir).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => Error::Refused("transfer artifacts already exist; recover instead".into()),
        _ => Error::Files(e),
    })?;
    let plan = {
        let ro = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        ro.execute_batch("PRAGMA query_only=ON;")?;
        preflight(&ro)?
    };
    // Original, closed, byte-identical copy; synced before the intent exists.
    let mut copy = OpenOptions::new().write(true).create_new(true).open(dir.join(BASELINE_DB))?;
    copy.write_all(&fs::read(db)?)?;
    copy.sync_all()?;
    let baseline = Baseline { attempt: owners.attempt().into(), registry: plan.registry, db_hash: db_hash(db)?,
        raw_hash: plan.raw.hash(), owners_hash: farcooler_workflow::digest(owners.evidence()) };
    if db_hash(&dir.join(BASELINE_DB))? != baseline.db_hash { return refuse("baseline copy differs from source"); }
    publish(&dir, BASELINE, &serde_json::to_vec(&baseline)?)?;
    let intent = Intent { attempt: baseline.attempt.clone(), baseline_hash: farcooler_workflow::digest(&baseline) };
    publish(&dir, INTENT, &serde_json::to_vec(&intent)?)?;
    if let Some(parent) = db.parent() { sync_dir(parent)?; }
    // Atomic upgrade: DDL + version + refusing floor + frozen authority together.
    let mut conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON;")?;
    if db_hash(db)? != baseline.db_hash { return refuse("source changed after baseline capture"); }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    crate::migrate::apply_remaining(&tx, plan.registry).map_err(|e| Error::Refused(e.to_string()))?;
    crate::compat::stamp_compatible_down_to(&tx).map_err(|e| Error::Refused(e.to_string()))?;
    let changed = tx.execute("UPDATE engine_cutover_authority SET phase='quiesced',manifest=?1 WHERE singleton=1 AND phase='legacy'", [&baseline.raw_hash])?;
    if changed != 1 { return refuse("source authority was not legacy"); }
    tx.commit()?;
    let store = Store::from_engine_cutover_connection(conn).map_err(|e| Error::Refused(e.to_string()))?;
    Ok((store, plan))
}

/// Terminal disposition: only after DB truth shows this attempt's baseline was
/// verified and released does the intent stop blocking ordinary startup. The
/// evidence directory is renamed, never deleted, and never reused.
pub fn finish(db: &Path, store: &Store) -> Result<PathBuf> {
    let (baseline, _) = validate_artifacts(db)?;
    let authority = store.engine_authority()?;
    if authority.phase != "released" || authority.manifest.as_deref() != Some(baseline.raw_hash.as_str()) {
        return refuse("attempt is not verified and released against its baseline");
    }
    let dir = artifact_dir(db);
    let mut done = dir.as_os_str().to_owned();
    done.push(format!("-resolved-{}", baseline.attempt));
    let done = PathBuf::from(done);
    fs::rename(&dir, &done)?;
    if let Some(parent) = db.parent() { sync_dir(parent)?; }
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
