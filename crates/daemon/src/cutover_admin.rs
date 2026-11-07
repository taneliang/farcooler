//! Operations behind the single `workflow.cutover` method. Each works from
//! database truth, the original baseline and a fresh in-process join: nothing a
//! caller says is quiescence, and commit is not offered (it needs the
//! closed-backup permit). The recovery daemon and the live daemon share these.
use std::path::{Path, PathBuf};
use farcooler_protocol::workflow_cutover::Cutover;
use farcooler_store::engine_cutover::{self, startup, transfer};
use serde_json::{Value, json};

#[derive(Debug)]
pub struct Refused(pub String);
impl std::fmt::Display for Refused { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.0) } }
fn refuse<E: std::fmt::Display>(e: E) -> Refused { Refused(e.to_string()) }

pub fn database(root: &Path) -> PathBuf { root.join(crate::cutover_gate::DATABASE) }

/// Read-only: what a person needs to know to choose the next step.
pub fn inspect(root: &Path) -> Value {
    let db = database(root);
    let artifacts = startup::artifact_dir(&db).exists();
    let admission = match startup::ordinary_admission(&db) { Ok(a) => format!("{a:?}"), Err(e) => format!("refused: {e}") };
    let authority = startup::open_recovery(&db).ok().and_then(|s| s.engine_authority().ok());
    let baseline = transfer::validate_artifacts(&db).ok().map(|(b, _)| json!({"attempt": b.attempt, "registry": b.registry, "raw_hash": b.raw_hash}));
    json!({"schema": 1, "ordinary_startup": admission, "artifacts": artifacts, "authority": authority, "baseline": baseline})
}

fn baseline_plan(db: &Path) -> Result<(transfer::Baseline, engine_cutover::Preflight), Refused> { transfer::baseline_plan(db).map_err(refuse) }

pub fn verify(root: &Path) -> Result<Value, Refused> {
    let db = database(root);
    let (baseline, plan) = baseline_plan(&db)?;
    let store = startup::open_recovery(&db).map_err(refuse)?;
    store.engine_verify(&plan).map_err(refuse)?;
    Ok(json!({"schema": 1, "verified": baseline.attempt}))
}

/// Release under a recovery process that has started no writers of its own.
pub fn release(root: &Path) -> Result<Value, Refused> {
    let db = database(root);
    let (baseline, plan) = baseline_plan(&db)?;
    let store = startup::open_recovery(&db).map_err(refuse)?;
    let protected = store.engine_protected_resources().map_err(refuse)?;
    let owners = crate::writer_join::recovery_proof(&baseline.attempt, &db, &protected).map_err(Refused)?;
    store.engine_release(&plan, &owners).map_err(refuse)?;
    Ok(json!({"schema": 1, "released": baseline.attempt}))
}

pub fn finish(root: &Path) -> Result<Value, Refused> {
    let db = database(root);
    let store = startup::open_recovery(&db).map_err(refuse)?;
    let kept = transfer::finish(&db, &store).map_err(refuse)?;
    Ok(json!({"schema": 1, "evidence": kept}))
}

/// Live daemon only: join its writers, capture the baseline and upgrade+freeze.
pub async fn prepare(svc: &crate::service::Service, attempt: &str) -> Result<Value, Refused> {
    let db = database(svc.root_dir());
    let protected = svc.store.engine_protected_resources().map_err(refuse)?;
    let owners = svc.join_writers(attempt, &db, &protected).await.map_err(Refused)?;
    let (store, plan) = transfer::begin(&db, &owners).map_err(refuse)?;
    let authority = store.engine_authority().map_err(refuse)?;
    Ok(json!({"schema": 1, "prepared": attempt, "phase": authority.phase, "raw_hash": plan.raw.hash(),
        "restart": "this daemon's writers are closed; restart it to continue in recovery"}))
}

/// The ops a recovery process (no Service) may run.
pub fn recovery(root: &Path, op: &Cutover) -> Result<Value, Refused> {
    match op {
        Cutover::Inspect => Ok(inspect(root)),
        Cutover::Verify => verify(root),
        Cutover::Release => release(root),
        Cutover::Finish => finish(root),
        Cutover::Prepare { .. } => Err(Refused("prepare runs in the live daemon, before restart".into())),
    }
}
