use super::*;
use startup::{Admission, artifact_dir, open_recovery, ordinary_admission};

pub(super) fn source65(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("db.sqlite");
    let mut conn = Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON; CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);").unwrap();
    crate::migrate::migrate_only_to(&mut conn, 65);
    path
}
pub(super) fn bytes(p: &std::path::Path) -> Vec<u8> { std::fs::read(p).unwrap() }

#[test]
fn startup_gate_refuses_pre_cutover_schema_without_writing_a_byte() {
    let tmp = tempfile::tempdir().unwrap();
    let path = source65(tmp.path());
    let before = bytes(&path);
    assert!(ordinary_admission(&path).is_err());
    assert!(Store::open(&path).is_err());
    assert_eq!(before, bytes(&path), "ordinary refusal must not migrate, stamp or sweep");
    assert!(!artifact_dir(&path).exists());
}
#[test]
fn startup_gate_fresh_requires_proven_absence_and_empty_files_are_not_fresh() {
    let tmp = tempfile::tempdir().unwrap();
    let absent = tmp.path().join("new.sqlite");
    assert_eq!(ordinary_admission(&absent).unwrap(), Admission::Fresh);
    drop(Store::open(&absent).unwrap());
    assert_eq!(ordinary_admission(&absent).unwrap(), Admission::Existing);
    let empty = tmp.path().join("empty.sqlite");
    std::fs::write(&empty, b"").unwrap();
    assert!(ordinary_admission(&empty).is_err(), "an existing empty file is not a fresh install");
    let orphan = tmp.path().join("orphan.sqlite");
    std::fs::create_dir(artifact_dir(&orphan)).unwrap();
    assert!(ordinary_admission(&orphan).is_err(), "orphan artifacts block even an absent database");
}
#[test]
fn startup_gate_refuses_missing_floor_and_frozen_authority_but_recovery_reaches_them() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("db.sqlite");
    drop(Store::open(&path).unwrap());
    Connection::open(&path).unwrap().execute("DELETE FROM meta WHERE key='compatible_down_to'", []).unwrap();
    let before = bytes(&path);
    assert!(Store::open(&path).is_err(), "a missing floor is inconsistent state, not permission to stamp");
    assert_eq!(before, bytes(&path));
    Connection::open(&path).unwrap().execute("INSERT INTO meta VALUES('compatible_down_to',?1)", [crate::compat::COMPATIBLE_DOWN_TO.to_string()]).unwrap();
    Connection::open(&path).unwrap().execute("UPDATE engine_cutover_authority SET phase='quiesced'", []).unwrap();
    assert!(Store::open(&path).is_err());
    let owners = join_db(&path, "gate");
    let store = open_recovery(&path, &owners).unwrap();
    assert_eq!(store.engine_authority().unwrap().phase, "quiesced");
}
#[test]
fn transfer_publishes_raw_baseline_and_intent_then_upgrades_floor_and_freeze_atomically() {
    let tmp = tempfile::tempdir().unwrap();
    let path = source65(tmp.path());
    let original = bytes(&path);
    let (store, plan) = transfer::begin(&path, join_db(&path, "att-1")).unwrap();
    let authority = store.engine_authority().unwrap();
    assert_eq!((authority.phase.as_str(), authority.manifest.as_deref()), ("quiesced", Some(plan.raw.hash().as_str())));
    let schema = store.schema().unwrap();
    assert_eq!((schema.version, schema.compatible_down_to), (68, Some(68)), "refusing floor lands with the version");
    assert_eq!(bytes(&artifact_dir(&path).join("baseline.db")), original, "baseline is the untouched original");
    let (baseline, intent) = transfer::validate_artifacts(&path).unwrap();
    assert_eq!((baseline.registry, baseline.raw_hash.as_str()), (65, plan.raw.hash().as_str()));
    assert_eq!(intent.attempt, "att-1");
    drop(store);
    assert!(Store::open(&path).is_err(), "intent and frozen authority both refuse ordinary startup");
    assert!(transfer::begin(&path, join_db(&path, "att-2")).is_err(), "no second transfer or recapture");
    assert_eq!(bytes(&artifact_dir(&path).join("baseline.db")), original);
}
#[test]
fn transfer_upgrade_failure_rolls_back_whole_and_keeps_the_intent() {
    let tmp = tempfile::tempdir().unwrap();
    let path = source65(tmp.path());
    // A colliding object makes the cutover migration fail inside the transaction.
    Connection::open(&path).unwrap().execute_batch("CREATE TABLE engine_cutover_workspaces(x)").unwrap();
    let before = bytes(&path);
    assert!(transfer::begin(&path, join_db(&path, "att")).is_err());
    assert_eq!(before, bytes(&path), "version, floor, catalog and authority rolled back together");
    assert!(transfer::validate_artifacts(&path).is_ok(), "durable intent survives for recovery");
    assert!(ordinary_admission(&path).is_err());
}
#[test]
fn transfer_baseline_tamper_and_foreign_proof_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let path = source65(tmp.path());
    let other = tmp.path().join("other.sqlite");
    std::fs::copy(&path, &other).unwrap();
    assert!(transfer::begin(&path, join_db(&other, "att")).is_err(), "proof issued for another file");
    assert!(!artifact_dir(&path).exists(), "refused before any artifact");
    transfer::begin(&path, join_db(&path, "att")).unwrap();
    std::fs::OpenOptions::new().append(true).open(artifact_dir(&path).join("baseline.db")).unwrap().write_all(b"x").unwrap();
    assert!(transfer::validate_artifacts(&path).is_err());
}
use std::io::Write;

/// Fixture writer for `reports/ov-940/old-reader.py`, which names the directory.
#[test]
#[ignore = "run by reports/ov-940/old-reader.py"]
fn old_reader_fixture_writes_source_and_begun_databases() {
    let out = std::path::PathBuf::from(std::env::var_os("OV940_OLD_READER_DIR").expect("the script names a private directory"));
    let source = super::source60::source60(&out);
    std::fs::rename(&source, out.join("v60.sqlite")).unwrap();
    let begun = out.join("begun");
    std::fs::create_dir(&begun).unwrap();
    let path = super::source60::source60(&begun);
    drop(transfer::begin(&path, join_db(&path, "old-reader")).unwrap());
    std::fs::rename(&path, out.join("begun.sqlite")).unwrap();
}

#[test]
fn transfer_finish_needs_a_released_attempt_and_keeps_the_evidence() {
    let tmp = tempfile::tempdir().unwrap();
    let path = source65(tmp.path());
    let (store, _plan) = transfer::begin(&path, join_db(&path, "fin")).unwrap();
    drop(store);
    let owners = join_db(&path, "fin");
    let store = open_recovery(&path, &owners).unwrap();
    assert!(transfer::finish(&path, &store, owners).is_err(), "quiesced is not released");
    store.conn().execute("UPDATE engine_cutover_authority SET phase='released'", []).unwrap();
    drop(store);
    let owners = join_db(&path, "fin");
    let store = open_recovery(&path, &owners).unwrap();
    let kept = transfer::finish(&path, &store, owners).unwrap();
    assert!(kept.join("baseline.db").exists() && !artifact_dir(&path).exists());
    drop(store);
    assert!(ordinary_admission(&path).is_ok());
    let owners = join_db(&path, "fin");
    let store = open_recovery(&path, &owners).unwrap();
    assert!(transfer::finish(&path, &store, owners).is_err(), "no second disposition");
}

/// SQLite's default sync on macOS is a plain `fsync`, which can leave a commit
/// in the drive's cache. The cutover's database commits must be as strong as
/// its artifact files (`sync_all` is `F_FULLFSYNC` there).
#[test]
fn cutover_connections_sync_fully() {
    let tmp = tempfile::tempdir().unwrap();
    let path = source65(tmp.path());
    let full = |store: &crate::Store| -> (i64, i64) {
        let conn = store.conn();
        (conn.query_row("PRAGMA fullfsync", [], |r| r.get(0)).unwrap(), conn.query_row("PRAGMA checkpoint_fullfsync", [], |r| r.get(0)).unwrap())
    };
    let (begun, _) = transfer::begin(&path, join_db(&path, "a")).unwrap();
    assert_eq!(full(&begun), (1, 1), "the upgrade's own connection");
    drop(begun);
    let owners = join_db(&path, "a");
    let reopened = open_recovery(&path, &owners).unwrap();
    assert_eq!(full(&reopened), (1, 1), "and every recovery open after it");
}

/// A database exactly as the build one migration back left it, at `phase`,
/// with the floor that build stamped unless `floor` says otherwise.
fn one_behind(dir: &std::path::Path, phase: &str, floor: Option<u32>) -> std::path::PathBuf {
    let path = dir.join("db.sqlite");
    let behind = crate::migrate::CURRENT_SCHEMA_VERSION - 1;
    let mut conn = Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON; CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);").unwrap();
    crate::migrate::migrate_only_to(&mut conn, behind);
    if let Some(f) = floor { conn.execute("INSERT INTO meta(key,value) VALUES('compatible_down_to',?1)", [f.to_string()]).unwrap(); }
    conn.execute("UPDATE engine_cutover_authority SET phase=?1 WHERE singleton=1", [phase]).unwrap();
    path
}
fn behind_floor() -> u32 { crate::compat::floor_at(crate::migrate::CURRENT_SCHEMA_VERSION - 1) }

#[test]
fn a_released_database_one_migration_behind_goes_forward_and_keeps_its_authority() {
    let tmp = tempfile::tempdir().unwrap();
    let path = one_behind(tmp.path(), "released", Some(behind_floor()));
    assert_eq!(ordinary_admission(&path).unwrap(), Admission::Existing);
    let store = Store::open(&path).expect("a released install must migrate forward, not strand");
    let conn = store.conn();
    let version: String = conn.query_row("SELECT value FROM meta WHERE key='schema_version'", [], |r| r.get(0)).unwrap();
    assert_eq!(version, crate::migrate::CURRENT_SCHEMA_VERSION.to_string());
    let phase: String = conn.query_row("SELECT phase FROM engine_cutover_authority", [], |r| r.get(0)).unwrap();
    assert_eq!(phase, "released");
    let floor: String = conn.query_row("SELECT value FROM meta WHERE key='compatible_down_to'", [], |r| r.get(0)).unwrap();
    assert_eq!(floor, crate::compat::COMPATIBLE_DOWN_TO.to_string());
}
#[test]
fn a_database_one_migration_behind_that_is_not_released_or_has_a_wrong_floor_stays_refused_untouched() {
    for (phase, floor) in [("legacy", Some(behind_floor())), ("quiesced", Some(behind_floor())), ("committed", Some(behind_floor())),
        ("released", None), ("released", Some(behind_floor() + 1)), ("released", Some(behind_floor() - 1))] {
        let tmp = tempfile::tempdir().unwrap();
        let path = one_behind(tmp.path(), phase, floor);
        let before = bytes(&path);
        assert!(ordinary_admission(&path).is_err(), "{phase} floor {floor:?} must stay refused");
        assert!(Store::open(&path).is_err(), "{phase} floor {floor:?} must not open");
        assert_eq!(before, bytes(&path), "{phase} floor {floor:?}: a refusal writes nothing");
    }
}
#[test]
fn a_build_with_a_later_migration_admits_released_installs_and_nothing_else() {
    let next = crate::migrate::CURRENT_SCHEMA_VERSION + 1;
    for (phase, admitted) in [("released", true), ("legacy", false), ("quiesced", false), ("committed", false)] {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("db.sqlite");
        drop(Store::open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        conn.execute("UPDATE engine_cutover_authority SET phase=?1", [phase]).unwrap();
        assert_eq!(startup::inspect_against(&conn, next).is_ok(), admitted, "{phase} at current-1 of a build one migration on");
        conn.execute("UPDATE meta SET value='1' WHERE key='compatible_down_to'", []).unwrap();
        assert!(startup::inspect_against(&conn, next).is_err(), "{phase}: a lowered floor is refused");
    }
}
