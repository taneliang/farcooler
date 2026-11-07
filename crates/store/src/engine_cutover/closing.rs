//! Commit on a real install: the closed backup and the mapping, in one proof.
//! The backup is the frozen database as it stands just before the mapping, in
//! the attempt's own artifact directory, so ordinary startup stays refused while
//! it exists and `finish` carries it into the kept evidence. It is compared
//! through an independent restore before the permit exists. A permit lives only
//! in memory, under the kernel proof that made it, so a crash can never reuse
//! one: another Commit takes a new number and the old copy stays as evidence.
//! Database truth outranks the journal, which only records what it has said.
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use super::{JoinedOwners, Manifest, Preflight, Result, boundary::reach, preflight, refuse, startup::{self, artifact_dir}};
use super::rehearsal::{CommitPermit, JournalEntry};
use super::transfer::{self, Baseline};

const STAGES: [&str; 4] = ["committed", "verified", "released", "resolved"];

/// What a closed backup holds, published beside it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot { pub attempt: String, pub number: u32, pub db_hash: String, pub length: u64, pub mode: u32, pub manifest: String, pub protected: BTreeSet<String> }

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Committed { pub attempt: String, pub resumed: bool, pub backup: Option<PathBuf> }

fn journal_dir(db: &Path) -> Result<PathBuf> {
    let dir = artifact_dir(db).join("journal");
    if let Err(e) = fs::create_dir(&dir) && e.kind() != std::io::ErrorKind::AlreadyExists { return Err(e.into()); }
    Ok(dir)
}
/// Append `name` once. A repeat keeps the first record: the journal only ever
/// grows, and a crash between a database commit and this line repeats it.
fn record(db: &Path, baseline: &Baseline, name: &str, stage: &str) -> Result<()> {
    let dir = journal_dir(db)?;
    let file = format!("{name}.json");
    if fs::symlink_metadata(dir.join(&file)).is_ok() { return Ok(()); }
    let entry = JournalEntry { attempt: baseline.attempt.clone(), stage: stage.into(), evidence: baseline.raw_hash.clone() };
    transfer::publish(&dir, &file, &serde_json::to_vec(&entry)?, "journal.torn")
}
/// Who asked for a Commit, as the daemon saw them: pid, terminal and session.
/// Evidence for the owner reading the journal afterward, never authority; the
/// walk that admitted the caller is a guardrail, and this makes an unplanned
/// Commit visible. One entry per request, so a repeat is seen too.
pub fn record_caller(db: &Path, caller: &str) -> Result<()> {
    let (baseline, _) = transfer::validate_artifacts(db)?;
    let dir = journal_dir(db)?;
    let entry = JournalEntry { attempt: baseline.attempt, stage: "commit-requested".into(), evidence: caller.into() };
    for n in 1..=1000 {
        let file = format!("commit-requested-{n}.json");
        if fs::symlink_metadata(dir.join(&file)).is_ok() { continue; }
        // Not `publish`: that one names a crash boundary, and this entry is
        // evidence only. A torn one costs a line of history, not a decision.
        let mut f = fs::OpenOptions::new().write(true).create_new(true).open(dir.join(&file))?;
        std::io::Write::write_all(&mut f, &serde_json::to_vec(&entry)?)?;
        f.sync_all()?;
        return transfer::sync_dir(&dir);
    }
    refuse("too many commit requests recorded")
}
/// Record `stage` and any earlier stage a crash left unrecorded.
pub fn record_up_to(db: &Path, baseline: &Baseline, stage: &str) -> Result<()> {
    let Some(last) = STAGES.iter().position(|s| *s == stage) else { return refuse("unknown journal stage") };
    for stage in &STAGES[..=last] { record(db, baseline, stage, stage)?; }
    Ok(())
}

fn closed_dir(dir: &Path) -> Result<(u32, PathBuf)> {
    for number in 1.. {
        let path = dir.join(format!("closed-{number}"));
        match fs::create_dir(&path) {
            Ok(()) => {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
                transfer::sync_dir(dir)?;
                return Ok((number, path));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
    }
    refuse("no unused backup number")
}
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut copy = OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
    copy.write_all(bytes)?;
    copy.sync_all()?;
    Ok(())
}
/// No Store, sidecar or lock on the file, and a clean integrity check.
fn closed(db: &Path) -> Result<()> {
    transfer::refuse_sidecars(db)?;
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.busy_timeout(std::time::Duration::ZERO)?;
    conn.execute_batch("BEGIN EXCLUSIVE;")?;
    let check: String = conn.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
    if check != "ok" { return refuse("closed database integrity check failed"); }
    conn.execute_batch("ROLLBACK;")?;
    conn.close().map_err(|(_, e)| super::Error::Storage(e))?;
    transfer::refuse_sidecars(db)
}
fn manifest_of(db: &Path) -> Result<(String, Preflight, BTreeSet<String>)> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let plan = preflight(&conn)?;
    let protected = super::required_resources(&conn)?;
    Ok((Manifest::capture(&conn)?.hash(), plan, protected))
}
/// Copy the closed database, then restore that copy somewhere else and compare
/// it to what the plan and the live file say. Returns the permit's backup hash.
fn back_up(db: &Path, plan: &Preflight, attempt: &str, protected: &BTreeSet<String>) -> Result<(PathBuf, String)> {
    closed(db)?;
    let (manifest, _, _) = manifest_of(db)?;
    let (number, dir) = closed_dir(&artifact_dir(db))?;
    reach("commit.before-copy");
    let bytes = fs::read(db)?;
    write_private(&dir.join("db"), &bytes)?;
    reach("commit.after-copy");
    let snapshot = Snapshot { attempt: attempt.into(), number, db_hash: farcooler_workflow::digest(&bytes), length: bytes.len() as u64,
        mode: fs::metadata(db)?.permissions().mode() & 0o777, manifest, protected: protected.clone() };
    transfer::publish(&dir, "SNAPSHOT.json", &serde_json::to_vec(&snapshot)?, "commit.torn-snapshot")?;
    reach("commit.after-snapshot");
    let restore = dir.join("restore-check");
    write_private(&restore, &fs::read(dir.join("db"))?)?;
    let (restored, restored_plan, restored_resources) = manifest_of(&restore)?;
    if transfer::db_hash(&restore)? != snapshot.db_hash || restored != snapshot.manifest { return refuse("independent restore differs from the live database"); }
    if !plan.raw.conserved_by(&restored_plan.raw) || restored_resources != *protected { return refuse("backup baseline or resources differ from the plan"); }
    fs::remove_file(&restore)?;
    transfer::sync_dir(&dir)?;
    reach("commit.after-restore");
    if transfer::db_hash(db)? != snapshot.db_hash { return refuse("database changed during the backup"); }
    Ok((dir, farcooler_workflow::digest(&snapshot)))
}

/// Back up and map one frozen attempt. Holds `owners` from before the copy until
/// the commit, so nothing can come between the backup and what it proves. On a
/// database that already committed this attempt it only finishes the journal.
pub fn commit(db: &Path, owners: JoinedOwners) -> Result<Committed> {
    if !owners.binds(db) { return refuse("join proof was issued for another database"); }
    let (baseline, _) = transfer::validate_artifacts(db)?;
    if owners.attempt() != baseline.attempt { return refuse("join proof belongs to another attempt"); }
    let (plan, protected) = {
        let store = startup::open_recovery(db, &owners)?;
        transfer::frozen_as(&store.conn(), &baseline)?;
        let authority = store.engine_authority()?;
        if matches!(authority.phase.as_str(), "committed" | "released") && authority.manifest.as_deref() == Some(baseline.raw_hash.as_str()) {
            record_up_to(db, &baseline, "committed")?;
            return Ok(Committed { attempt: baseline.attempt, resumed: true, backup: None });
        }
        if authority.phase != "quiesced" { return refuse("cutover not quiesced"); }
        let (_, plan) = transfer::bound_plan(db, &store)?;
        // Release refuses this forever, and Recover refuses after a commit, so
        // here is the last place it has an exit.
        if plan.quarantines.iter().any(|q| q.resource == "schema:unknown") { return refuse("unreviewed source schema cannot commit; recover instead"); }
        (plan, store.engine_protected_resources()?)
    };
    transfer::clear_inert_journal(db)?;
    let (backup, backup_hash) = back_up(db, &plan, owners.attempt(), &protected)?;
    let permit = CommitPermit::closed(&plan, &owners, backup_hash.clone(), protected)?;
    let number = backup.file_name().and_then(|n| n.to_str()).unwrap_or("closed").to_owned();
    record(db, &baseline, &format!("backed_up-{number}"), &backup_hash)?;
    reach("commit.after-permit");
    let store = startup::open_recovery(db, &owners)?;
    store.engine_commit(&plan, owners, &permit)?;
    drop(store);
    record_up_to(db, &baseline, "committed")?;
    reach("commit.after-journal");
    Ok(Committed { attempt: baseline.attempt, resumed: false, backup: Some(backup) })
}

/// The closed backup this attempt's commit named, still whole. A lost or changed
/// backup refuses and repairs nothing.
pub fn held(db: &Path, store: &crate::Store, baseline: &Baseline) -> Result<PathBuf> {
    let recorded: String = store.conn().query_row("SELECT proof FROM engine_cutover_attempts WHERE id=?1", [&baseline.attempt], |r| r.get(0))?;
    let (_, backup): (serde_json::Value, String) = serde_json::from_str(&recorded)?;
    for entry in fs::read_dir(artifact_dir(db))? {
        let dir = entry?.path();
        if !dir.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("closed-")) { continue; }
        let Ok(raw) = fs::read(dir.join("SNAPSHOT.json")) else { continue };
        let Ok(snapshot) = serde_json::from_slice::<Snapshot>(&raw) else { continue };
        if farcooler_workflow::digest(&snapshot) != backup { continue; }
        if transfer::db_hash(&dir.join("db")).ok().as_deref() != Some(snapshot.db_hash.as_str()) { return refuse("closed backup changed or truncated"); }
        return Ok(dir);
    }
    refuse("the attempt's closed backup is missing")
}
