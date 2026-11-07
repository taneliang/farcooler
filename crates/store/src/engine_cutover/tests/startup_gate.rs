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
    assert_eq!((schema.version, schema.compatible_down_to), (67, Some(67)), "refusing floor lands with the version");
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
