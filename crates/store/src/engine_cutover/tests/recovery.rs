//! The ownership proof, the capture boundary, and the exit from every
//! uncommitted state a transfer can leave.
use super::*;
use super::startup_gate::{bytes, source65};
use startup::{artifact_dir, open_recovery, ordinary_admission, readonly_admission};

#[test]
fn owner_lock_any_open_store_blocks_the_proof_and_the_proof_blocks_ordinary_opens() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("db.sqlite");
    let first = Store::open(&path).unwrap();
    let second = Store::open(&path).unwrap();
    assert!(JoinedOwners::acquire(&path, "a").is_err(), "an open Store is an owner");
    drop(first);
    assert!(JoinedOwners::acquire(&path, "a").is_err(), "the other Store still is");
    drop(second);
    let held = JoinedOwners::acquire(&path, "a").unwrap();
    assert!(Store::open(&path).is_err(), "recovery owns the file");
    assert!(JoinedOwners::acquire(&path, "b").is_err(), "one proof at a time; none can be minted beside it");
    drop(held);
    drop(Store::open(&path).unwrap());
}
#[test]
fn owner_lock_proof_refuses_a_replaced_file_and_a_stage_of_another_attempt() {
    let tmp = tempfile::tempdir().unwrap();
    let path = source65(tmp.path());
    let owners = join_db(&path, "a");
    std::fs::rename(&path, tmp.path().join("moved.sqlite")).unwrap();
    std::fs::copy(tmp.path().join("moved.sqlite"), &path).unwrap();
    assert!(transfer::begin(&path, owners).is_err(), "the proof names the file it was issued for");
    let (store, plan) = transfer::begin(&path, join_db(&path, "b")).unwrap();
    let evidence = proof(&store);
    assert!(store.engine_commit_for_test(&plan, &evidence, "other").is_err(), "the frozen attempt is b");
    assert!(store.engine_release(&plan, JoinedOwners::unit("other")).is_err());
    assert!(store.engine_quiesce(JoinedOwners::unit("other")).is_err());
}
#[test]
fn transfer_no_writer_lands_between_capture_and_upgrade() {
    let tmp = tempfile::tempdir().unwrap();
    let path = source65(tmp.path());
    let writer = Connection::open(&path).unwrap();
    writer.busy_timeout(std::time::Duration::ZERO).unwrap();
    let hit = std::rc::Rc::new(std::cell::Cell::new(None));
    let seen = hit.clone();
    transfer::AFTER_CAPTURE.with(|hook| *hook.borrow_mut() = Some(Box::new(move || {
        seen.set(Some(writer.execute("INSERT INTO meta VALUES('late','row')", []).is_err()));
    })));
    let (store, plan) = transfer::begin(&path, join_db(&path, "race")).unwrap();
    assert_eq!(hit.get(), Some(true), "the hook ran and the write was refused");
    let present: i64 = store.conn().query_row("SELECT count(*) FROM meta WHERE key='late'", [], |r| r.get(0)).unwrap();
    assert_eq!(present, 0);
    let baseline = Connection::open(artifact_dir(&path).join("baseline.db")).unwrap();
    assert_eq!(preflight(&baseline).unwrap().raw, plan.raw);
}
#[test]
fn transfer_refuses_a_held_write_lock_without_leaving_artifacts() {
    let tmp = tempfile::tempdir().unwrap();
    let path = source65(tmp.path());
    let writer = Connection::open(&path).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(transfer::begin(&path, join_db(&path, "busy")).is_err());
    assert!(!artifact_dir(&path).exists());
}
#[test]
fn recover_returns_a_frozen_attempt_to_the_original_bytes_and_a_new_attempt_can_start() {
    let tmp = tempfile::tempdir().unwrap();
    let path = source65(tmp.path());
    let original = bytes(&path);
    drop(transfer::begin(&path, join_db(&path, "one")).unwrap());
    let recovered = transfer::recover(&path, join_db(&path, "out")).unwrap();
    assert!(recovered.restored && recovered.evidence.join("baseline.db").exists());
    assert_eq!(bytes(&path), original);
    assert!(!artifact_dir(&path).exists());
    assert!(ordinary_admission(&path).is_err(), "still pre-cutover: recovery stays reachable");
    drop(transfer::begin(&path, join_db(&path, "two")).unwrap());
    assert_eq!(transfer::validate_artifacts(&path).unwrap().0.attempt, "two");
    assert!(transfer::recover(&path, join_db(&path, "out")).unwrap().evidence.ends_with("db.sqlite.cutover-abandoned-2"));
}
#[test]
fn recover_returns_a_legacy_current_database_to_ordinary_service() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("db.sqlite");
    drop(Store::open(&path).unwrap());
    let original = bytes(&path);
    transfer::hold(&path, "held").unwrap();
    assert!(Store::open(&path).is_err(), "a hold refuses ordinary startup");
    assert!(transfer::begin(&path, join_db(&path, "other")).is_err(), "only the held attempt may begin");
    drop(transfer::begin(&path, join_db(&path, "held")).unwrap());
    assert!(transfer::recover(&path, join_db(&path, "out")).unwrap().restored);
    assert_eq!(bytes(&path), original);
    drop(Store::open(&path).unwrap());
}
#[test]
fn recover_retires_a_hold_or_attempt_that_never_upgraded() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("db.sqlite");
    drop(Store::open(&path).unwrap());
    transfer::hold(&path, "held").unwrap();
    assert!(!transfer::recover(&path, join_db(&path, "out")).unwrap().restored);
    drop(Store::open(&path).unwrap());
    let tmp = tempfile::tempdir().unwrap();
    let path = source65(tmp.path());
    // A colliding object fails the upgrade after the intent is durable.
    Connection::open(&path).unwrap().execute_batch("CREATE TABLE engine_cutover_workspaces(x)").unwrap();
    let before = bytes(&path);
    assert!(transfer::begin(&path, join_db(&path, "att")).is_err());
    assert!(ordinary_admission(&path).is_err());
    assert!(!transfer::recover(&path, join_db(&path, "out")).unwrap().restored);
    assert_eq!(bytes(&path), before);
    assert!(!artifact_dir(&path).exists());
}
#[test]
fn recover_clears_partial_or_torn_artifacts_only_when_nothing_was_frozen() {
    let tmp = tempfile::tempdir().unwrap();
    let path = source65(tmp.path());
    let original = bytes(&path);
    std::fs::create_dir(artifact_dir(&path)).unwrap();
    std::fs::write(artifact_dir(&path).join("baseline.db"), b"torn").unwrap();
    assert!(ordinary_admission(&path).is_err());
    assert!(!transfer::recover(&path, join_db(&path, "out")).unwrap().restored);
    assert_eq!(bytes(&path), original);
    drop(transfer::begin(&path, join_db(&path, "a")).unwrap());
    let intent = artifact_dir(&path).join("intent.json");
    std::fs::remove_file(&intent).unwrap();
    assert!(transfer::recover(&path, join_db(&path, "out")).is_err(), "frozen with a lost intent stays blocked");
    assert!(artifact_dir(&path).join("baseline.db").exists(), "and refusing moved nothing");
}
#[test]
fn recover_refuses_committed_truth_and_a_database_written_since_the_upgrade() {
    let tmp = tempfile::tempdir().unwrap();
    let path = source65(tmp.path());
    drop(transfer::begin(&path, join_db(&path, "a")).unwrap());
    let upgraded = bytes(&path);
    // meta is not fenced: a restore would lose this write.
    let store = open_recovery(&path, &join_db(&path, "a")).unwrap();
    store.conn().execute("INSERT INTO meta VALUES('written-after','keep me')", []).unwrap();
    drop(store);
    let changed = bytes(&path);
    assert_ne!(changed, upgraded);
    assert!(transfer::recover(&path, join_db(&path, "out")).is_err());
    assert_eq!(bytes(&path), changed, "the later write survives");
    let store = open_recovery(&path, &join_db(&path, "a")).unwrap();
    store.conn().execute("DELETE FROM meta WHERE key='written-after'", []).unwrap();
    store.conn().execute("UPDATE engine_cutover_authority SET phase='committed'", []).unwrap();
    drop(store);
    assert!(transfer::recover(&path, join_db(&path, "out")).is_err(), "committed recovers forward only");
    assert!(artifact_dir(&path).exists());
}
#[test]
fn baseline_binds_root_file_attempt_floor_catalog_and_resources() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let path = source65(&root);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys=OFF; INSERT INTO workflow_resource_generations(resource,generation) VALUES('repo:x',3); INSERT INTO workflow_resources VALUES('repo:x','w','i','r',0,3);").unwrap();
    drop(conn);
    drop(transfer::begin(&path, join_db(&path, "a")).unwrap());
    let (baseline, _) = transfer::validate_artifacts(&path).unwrap();
    assert_eq!(baseline.root, root.canonicalize().unwrap());
    assert_eq!((baseline.database.as_str(), baseline.attempt.as_str(), baseline.registry), ("db.sqlite", "a", 65));
    assert_eq!(baseline.resources.get("repo:x"), Some(&Some(3)));
    assert!(baseline.owners_hash.starts_with("wf1:") && baseline.catalog_hash.starts_with("wf1:"));
    let attempt: String = Connection::open(&path).unwrap().query_row("SELECT id FROM engine_cutover_attempts", [], |r| r.get(0)).unwrap();
    assert_eq!(attempt, "frozen:a", "the attempt is frozen in the same transaction as the upgrade");

    // A copied root with its artifacts is not the original.
    let copy = tmp.path().join("copy");
    std::fs::create_dir_all(copy.join("db.sqlite.cutover")).unwrap();
    std::fs::copy(&path, copy.join("db.sqlite")).unwrap();
    for name in ["baseline.db", "baseline.json", "intent.json"] {
        std::fs::copy(artifact_dir(&path).join(name), copy.join("db.sqlite.cutover").join(name)).unwrap();
    }
    assert!(transfer::validate_artifacts(&copy.join("db.sqlite")).is_err());
    // Nor is a file swapped in at the original path.
    let swapped = tmp.path().join("swapped");
    std::fs::copy(&path, &swapped).unwrap();
    std::fs::rename(&swapped, &path).unwrap();
    assert!(transfer::validate_artifacts(&path).is_err());
}
#[test]
fn baseline_of_another_attempt_is_not_this_databases() {
    let tmp = tempfile::tempdir().unwrap();
    let path = source65(tmp.path());
    drop(transfer::begin(&path, join_db(&path, "a")).unwrap());
    let dir = artifact_dir(&path);
    let (mut baseline, _) = transfer::validate_artifacts(&path).unwrap();
    baseline.attempt = "b".into();
    let intent = transfer::Intent { attempt: "b".into(), baseline_hash: farcooler_workflow::digest(&baseline) };
    std::fs::write(dir.join("baseline.json"), serde_json::to_vec(&baseline).unwrap()).unwrap();
    std::fs::write(dir.join("intent.json"), serde_json::to_vec(&intent).unwrap()).unwrap();
    assert!(transfer::validate_artifacts(&path).is_ok(), "the forged files agree with each other");
    let owners = join_db(&path, "b");
    let store = open_recovery(&path, &owners).unwrap();
    assert!(transfer::bound_plan(&path, &store).is_err(), "the database says attempt a");
    assert!(transfer::finish(&path, &store, owners).is_err());
    drop(store);
    assert!(transfer::recover(&path, join_db(&path, "out")).is_err());
}
/// A crash image: the database and journal as a writer left them mid-transaction.
fn crashed(dir: &std::path::Path) -> (std::path::PathBuf, i64) {
    let live = dir.join("live.sqlite");
    drop(Store::open(&live).unwrap());
    let conn = Connection::open(&live).unwrap();
    conn.execute_batch("PRAGMA cache_size=1; CREATE TABLE IF NOT EXISTS bulk(x); BEGIN IMMEDIATE;").unwrap();
    let committed: i64 = conn.query_row("SELECT count(*) FROM meta", [], |r| r.get(0)).unwrap();
    for n in 0..4000 { conn.execute("INSERT INTO meta VALUES(?1,hex(randomblob(256)))", [format!("uncommitted-{n}")]).unwrap(); }
    let path = dir.join("db.sqlite");
    std::fs::copy(&live, &path).unwrap();
    std::fs::copy(dir.join("live.sqlite-journal"), dir.join("db.sqlite-journal")).unwrap();
    conn.execute_batch("ROLLBACK").unwrap();
    (path, committed)
}
#[test]
fn hot_journal_is_rolled_back_by_ordinary_admission_and_only_reported_by_inspection() {
    let tmp = tempfile::tempdir().unwrap();
    let (path, committed) = crashed(tmp.path());
    let journal = tmp.path().join("db.sqlite-journal");
    assert!(readonly_admission(&path).is_err(), "a read-only connection cannot roll it back");
    assert!(journal.exists(), "and inspecting leaves it");
    assert!(startup::crash_journal(&path));
    assert_eq!(ordinary_admission(&path).unwrap(), startup::Admission::Existing);
    assert!(!journal.exists());
    let store = Store::open(&path).unwrap();
    let rows: i64 = store.conn().query_row("SELECT count(*) FROM meta", [], |r| r.get(0)).unwrap();
    assert_eq!(rows, committed, "the crashed transaction is gone");
}
#[test]
fn hot_journal_never_lands_on_restored_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let path = source65(tmp.path());
    let original = bytes(&path);
    drop(transfer::begin(&path, join_db(&path, "a")).unwrap());
    // A leftover journal must be settled or refused, never kept beside the restore.
    std::fs::write(tmp.path().join("db.sqlite-journal"), b"").unwrap();
    assert!(transfer::recover(&path, join_db(&path, "out")).is_ok());
    assert_eq!(bytes(&path), original);
    assert!(!tmp.path().join("db.sqlite-journal").exists());
}
#[test]
fn hold_refuses_a_database_that_is_not_legacy_and_recover_retires_a_stranded_hold() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("db.sqlite");
    drop(Store::open(&path).unwrap());
    Connection::open(&path).unwrap().execute("UPDATE engine_cutover_authority SET phase='released'", []).unwrap();
    assert!(transfer::hold(&path, "held").is_err(), "a released database is not held");
    assert!(!artifact_dir(&path).exists(), "a refused hold leaves no mark");
    // A hold already written before the phase moved on is still an exit.
    std::fs::create_dir(artifact_dir(&path)).unwrap();
    std::fs::write(artifact_dir(&path).join("hold.json"), br#"{"attempt":"held"}"#).unwrap();
    assert!(!transfer::recover(&path, join_db(&path, "out")).unwrap().restored);
    assert!(!artifact_dir(&path).exists());
}
