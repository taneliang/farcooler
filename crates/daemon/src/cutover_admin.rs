//! Operations behind the single `workflow.cutover` method. A transfer runs only
//! in a recovery process, one that started no writer, under the kernel proof
//! that no other process has the database open (`engine_cutover::owners`).
//! Nothing a caller says is quiescence. The live daemon can only inspect, or
//! drain and stop so that the next start is that process.
use std::path::{Path, PathBuf};
use farcooler_protocol::workflow_cutover::Cutover;
use farcooler_store::engine_cutover::{JoinedOwners, boundary, closing, owners, startup, transfer};
use serde_json::{Value, json};

/// Set by a live Prepare, which stops the daemon whether or not it held the
/// runner: launchd and systemd restart only a failure, so the stop exits with
/// `rpc::recovery::RESTART_EXIT` and the supervisor starts what comes next
/// (recovery after a hold, ordinary service after a refusal).
pub static STOPPED_FOR_PREPARE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Debug)]
pub struct Refused(pub String);
impl std::fmt::Display for Refused { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.0) } }
impl Refused {
    /// The refusal as a sentence for the person at the CLI: the named holders
    /// when a process has the file open, which is what they can act on, and
    /// otherwise a pointer to `inspect`. The raw text is internal prose and
    /// stays in the daemon log.
    pub fn said(&self) -> String {
        if self.0.contains("has this database open") {
            let named = self.0.split_once("process [").and_then(|(_, r)| r.split_once(']')).map(|(pids, _)| pids.trim().to_string()).filter(|p| !p.is_empty());
            return match named {
                Some(pids) => format!("Another process (pid {pids}) has this runner's database open. Close it, then try again."),
                None => "Another process has this runner's database open. Close it, then try again.".into(),
            };
        }
        "That upgrade step was refused. Run `farcooler workflow-cutover inspect` to see where the upgrade stands.".into()
    }
}
fn refuse<E: std::fmt::Display>(e: E) -> Refused { Refused(e.to_string()) }

pub fn database(root: &Path) -> PathBuf { root.join(crate::cutover_gate::DATABASE) }

/// Writes nothing, a crashed writer's journal included, which it only reports.
pub fn inspect(root: &Path) -> Value {
    let db = database(root);
    let admission = match startup::readonly_admission(&db) { Ok(a) => format!("{a:?}"), Err(e) => format!("refused: {e}") };
    let authority = startup::read_authority(&db).ok().flatten();
    let baseline = transfer::validate_artifacts(&db).ok().map(|(b, _)| json!({"attempt": b.attempt, "registry": b.registry, "raw_hash": b.raw_hash}));
    // The attempt in hand, held or prepared: a Prepare under any other is
    // refused, so the Mac's Finish Upgrade prepares under this one (ov-984).
    let attempt = transfer::attempt_in_hand(&db);
    json!({"schema": 1, "ordinary_startup": admission, "artifacts": startup::artifact_dir(&db).exists(), "attempt": attempt,
        "crash_journal": startup::crash_journal(&db), "authority": authority, "baseline": baseline,
        "open_elsewhere": owners::other_holders(&db).ok()})
}

fn attempt_of(db: &Path) -> Result<String, Refused> { Ok(transfer::validate_artifacts(db).map_err(refuse)?.0.attempt) }

/// Capture the baseline and intent, then upgrade and freeze, in this process.
pub fn prepare(root: &Path, attempt: &str) -> Result<Value, Refused> {
    let db = database(root);
    let owners = JoinedOwners::acquire(&db, attempt).map_err(refuse)?;
    let (store, plan) = transfer::begin(&db, owners).map_err(refuse)?;
    let authority = store.engine_authority().map_err(refuse)?;
    Ok(json!({"schema": 1, "prepared": attempt, "phase": authority.phase, "raw_hash": plan.raw.hash()}))
}

/// End an uncommitted attempt: the database goes back to its original bytes,
/// or was never changed. Committed state refuses and recovers forward.
pub fn recover(root: &Path) -> Result<Value, Refused> {
    let db = database(root);
    let owners = JoinedOwners::acquire(&db, "recover").map_err(refuse)?;
    let recovered = transfer::recover(&db, owners).map_err(refuse)?;
    Ok(json!({"schema": 1, "restored": recovered.restored, "evidence": recovered.evidence}))
}

/// Take the closed backup and map the frozen attempt, under one proof. Past this
/// the attempt recovers forward only.
/// Commit, naming who asked (`caller_note`) in the journal first, so even a
/// Commit that then refuses or crashes leaves its requester on record.
pub fn commit(root: &Path, caller_note: &str) -> Result<Value, Refused> {
    let db = database(root);
    closing::record_caller(&db, caller_note).map_err(refuse)?;
    let owners = JoinedOwners::acquire(&db, &attempt_of(&db)?).map_err(refuse)?;
    let committed = closing::commit(&db, owners).map_err(refuse)?;
    Ok(json!({"schema": 1, "committed": committed.attempt, "resumed": committed.resumed, "backup": committed.backup}))
}

pub fn verify(root: &Path) -> Result<Value, Refused> {
    let db = database(root);
    let owners = JoinedOwners::acquire(&db, &attempt_of(&db)?).map_err(refuse)?;
    let store = startup::open_recovery(&db, &owners).map_err(refuse)?;
    let (baseline, plan) = transfer::bound_plan(&db, &store).map_err(refuse)?;
    closing::held(&db, &store, &baseline).map_err(refuse)?;
    store.engine_verify(&plan).map_err(refuse)?;
    boundary::reach("verify.after");
    closing::record_up_to(&db, &baseline, "verified").map_err(refuse)?;
    Ok(json!({"schema": 1, "verified": baseline.attempt}))
}

pub fn release(root: &Path) -> Result<Value, Refused> {
    let db = database(root);
    let owners = JoinedOwners::acquire(&db, &attempt_of(&db)?).map_err(refuse)?;
    let store = startup::open_recovery(&db, &owners).map_err(refuse)?;
    let (baseline, plan) = transfer::bound_plan(&db, &store).map_err(refuse)?;
    closing::held(&db, &store, &baseline).map_err(refuse)?;
    store.engine_release(&plan, owners).map_err(refuse)?;
    closing::record_up_to(&db, &baseline, "released").map_err(refuse)?;
    Ok(json!({"schema": 1, "released": baseline.attempt}))
}

pub fn finish(root: &Path) -> Result<Value, Refused> {
    let db = database(root);
    let owners = JoinedOwners::acquire(&db, &attempt_of(&db)?).map_err(refuse)?;
    let store = startup::open_recovery(&db, &owners).map_err(refuse)?;
    let kept = transfer::finish(&db, &store, owners).map_err(refuse)?;
    Ok(json!({"schema": 1, "evidence": kept}))
}

/// The live daemon's Prepare. It closes admission and joins what it runs, then
/// marks the attempt. The caller stops the daemon whatever this returns, since
/// a closed gate never reopens in this process: with the mark the next start
/// serves recovery, and without it ordinary service. A writer that would not
/// join gets no mark.
pub async fn hold(svc: &crate::service::Service, attempt: &str, still_allowed: impl FnOnce() -> Result<(), Refused>) -> Result<Value, Refused> {
    let db = database(svc.root_dir());
    let drained = svc.drain_writers().await;
    still_allowed()?;
    drained.map_err(Refused)?;
    transfer::hold(&db, attempt).map_err(refuse)?;
    Ok(json!({"schema": 1, "held": attempt, "next": "run prepare again once the daemon has restarted into recovery"}))
}

/// The ops a recovery process (no Service) answers.
pub fn recovery(root: &Path, op: &Cutover, caller_note: &str) -> Result<Value, Refused> {
    match op {
        Cutover::Inspect => Ok(inspect(root)),
        Cutover::Prepare { attempt } => prepare(root, attempt),
        Cutover::Recover => recover(root),
        Cutover::Commit => commit(root, caller_note),
        Cutover::Verify => verify(root),
        Cutover::Release => release(root),
        Cutover::Finish => finish(root),
    }
}
