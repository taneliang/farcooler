//! The finite raw schema60 path, from a registry-prefix fixture seeded with
//! synthetic literals. Nothing here opens the file ordinarily before capture.
use super::*;
use super::startup_gate::bytes;
use startup::{artifact_dir, open_recovery};

/// The one trusted raw schema60 fixture: `testing::write_source60`.
pub(super) fn source60(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("db.sqlite");
    crate::testing::write_source60(&path);
    path
}
fn sequence(conn: &Connection) -> i64 { conn.query_row("SELECT seq FROM sqlite_sequence WHERE name='people_events'", [], |r| r.get(0)).unwrap() }

fn committed(dir: &std::path::Path, attempt: &str) -> (std::path::PathBuf, Store, Preflight) {
    let path = source60(dir);
    let (store, plan) = transfer::begin(&path, join_db(&path, attempt)).unwrap();
    let evidence = proof(&store);
    store.engine_commit_for_test(&plan, &evidence, attempt).unwrap();
    (path, store, plan)
}

#[test]
fn source60_transfer_freezes_atomically_and_recover_returns_the_exact_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let path = source60(tmp.path());
    let original = bytes(&path);
    let raw = preflight(&Connection::open(&path).unwrap()).unwrap();
    assert_eq!(raw.registry, 60);
    let (store, plan) = transfer::begin(&path, join_db(&path, "raw60")).unwrap();
    assert_eq!(plan.raw, raw.raw, "the plan is the raw source, not the upgraded rows");
    let schema = store.schema().unwrap();
    assert_eq!((schema.version, schema.compatible_down_to), (67, Some(67)));
    assert_eq!(store.engine_authority().unwrap().phase, "quiesced");
    assert_eq!(bytes(&artifact_dir(&path).join("baseline.db")), original);
    let (baseline, _) = transfer::validate_artifacts(&path).unwrap();
    assert_eq!((baseline.registry, baseline.floor), (60, Some(60)));
    drop(store);
    assert!(Store::open(&path).is_err());
    assert!(transfer::recover(&path, join_db(&path, "out")).unwrap().restored);
    assert_eq!(bytes(&path), original);
    assert!(!artifact_dir(&path).exists());
}
#[test]
fn source60_unknown_catalog_refuses_before_any_artifact() {
    let tmp = tempfile::tempdir().unwrap();
    let path = source60(tmp.path());
    Connection::open(&path).unwrap().execute_batch("CREATE TABLE stray(x)").unwrap();
    let before = bytes(&path);
    assert!(transfer::begin(&path, join_db(&path, "a")).is_err());
    assert_eq!(bytes(&path), before);
    assert!(!artifact_dir(&path).exists());
}
#[test]
fn source60_conserves_every_field_through_commit_and_release_and_invents_no_row() {
    let tmp = tempfile::tempdir().unwrap();
    let (path, store, plan) = committed(tmp.path(), "raw60");
    store.engine_verify(&plan).unwrap();
    drop(store);
    let owners = join_db(&path, "raw60");
    let store = open_recovery(&path, &owners).unwrap();
    let (baseline, bound) = transfer::bound_plan(&path, &store).unwrap();
    assert_eq!((baseline.attempt.as_str(), bound.raw.hash()), ("raw60", plan.raw.hash()));
    store.engine_release(&bound, owners).unwrap();
    let conn = store.conn();
    assert_eq!(sequence(&conn), 900, "the allocator's high-water mark survives");
    let quarantined: i64 = conn.query_row("SELECT count(*) FROM engine_cutover_quarantine WHERE resource LIKE 'task:%'", [], |r| r.get(0)).unwrap();
    assert!(quarantined >= 2, "invalid wait and live policy stay quarantined, got {quarantined}");
    let invented: i64 = conn.query_row("SELECT (SELECT count(*) FROM workflow_states)+(SELECT count(*) FROM workflow_events)+(SELECT count(*) FROM workflow_outbox)", [], |r| r.get(0)).unwrap();
    assert_eq!(invented, 0, "raw legacy state mints no engine event");
    let wait: Vec<u8> = conn.query_row("SELECT wait_rank FROM tasks WHERE key='ov-3'", [], |r| r.get(0)).unwrap();
    assert_eq!(wait, [0, 255]);
    let at: f64 = conn.query_row("SELECT at FROM task_notes WHERE id=?1", [vec![9u8; 16]], |r| r.get(0)).unwrap();
    assert_eq!(at, 1.5);
    let extra: Vec<u8> = conn.query_row("SELECT extra FROM task_notes WHERE id=?1", [vec![8u8; 16]], |r| r.get(0)).unwrap();
    assert_eq!(extra, [0, 255]);
}
#[test]
fn source60_verify_catches_field_rowid_allocator_and_relationship_damage() {
    for damage in ["UPDATE tasks SET title='same count' WHERE key='ov-1'",
        "UPDATE task_reads SET rowid=rowid+100 WHERE rowid=1",
        "UPDATE sqlite_sequence SET seq=0 WHERE name='people_events'",
        "DELETE FROM task_reads WHERE rowid=2",
        "CREATE TABLE stray(x)"] {
        let tmp = tempfile::tempdir().unwrap();
        let (_path, store, plan) = committed(tmp.path(), "damage");
        store.engine_verify(&plan).expect("clean before the damage");
        store.conn().execute_batch(damage).unwrap();
        assert!(store.engine_verify(&plan).is_err(), "{damage}");
    }
}
