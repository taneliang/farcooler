//! A runner below registry 60 is lifted to 60 inside Prepare, and no further,
//! then meets the pinned registry-60 catalog check. Raw fixtures are the real
//! migration history stopped at 28, 45 and 57; nothing here is hand-written SQL
//! standing in for a schema.
use super::*;
use super::startup_gate::bytes;
use startup::artifact_dir;

fn below(dir: &std::path::Path, registry: u32) -> std::path::PathBuf {
    let path = dir.join("db.sqlite");
    crate::testing::write_source_below_60(&path, registry);
    path
}
fn rows(path: &std::path::Path, table: &str) -> i64 {
    Connection::open(path).unwrap().query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)).unwrap()
}

#[test]
fn a_runner_below_60_is_lifted_to_exactly_60_with_its_original_bytes_kept() {
    for registry in [28u32, 45, 57] {
        let tmp = tempfile::tempdir().unwrap();
        let path = below(tmp.path(), registry);
        let original = bytes(&path);
        let tasks = rows(&path, "tasks");
        assert!(tasks >= 2, "registry {registry} fixture carries tasks");
        let (store, plan) = transfer::begin(&path, join_db(&path, "lift")).unwrap();
        assert_eq!(plan.registry, 60, "the lift stops at 60, not at {registry}+n");
        let dir = artifact_dir(&path);
        assert_eq!(bytes(&dir.join("pre-lift.db")), original, "registry {registry}: the original is kept whole");
        let (baseline, _) = transfer::validate_artifacts(&path).unwrap();
        assert_eq!((baseline.registry, baseline.floor), (60, Some(60)));
        assert_ne!(bytes(&dir.join("baseline.db")), original, "the baseline is the lifted file");
        assert_eq!(store.conn().query_row("SELECT count(*) FROM tasks", [], |r| r.get::<_, i64>(0)).unwrap(), tasks);
        drop(store);
        // The original opens at its own registry, so it really is recoverable.
        let kept = Connection::open(dir.join("pre-lift.db")).unwrap();
        let version: String = kept.query_row("SELECT value FROM meta WHERE key='schema_version'", [], |r| r.get(0)).unwrap();
        assert_eq!(version, registry.to_string());
    }
}

#[test]
fn a_lifted_runner_goes_on_to_commit_and_verify() {
    let tmp = tempfile::tempdir().unwrap();
    let path = below(tmp.path(), 45);
    let (store, plan) = transfer::begin(&path, join_db(&path, "lift")).unwrap();
    let evidence = proof(&store);
    store.engine_commit_for_test(&plan, &evidence, "lift").unwrap();
    store.engine_verify(&plan).unwrap();
}

#[test]
fn recover_after_a_lift_returns_the_lifted_file_and_keeps_the_original_as_evidence() {
    let tmp = tempfile::tempdir().unwrap();
    let path = below(tmp.path(), 57);
    let original = bytes(&path);
    let (store, _) = transfer::begin(&path, join_db(&path, "lift")).unwrap();
    drop(store);
    let lifted = bytes(&artifact_dir(&path).join("baseline.db"));
    let recovered = transfer::recover(&path, join_db(&path, "out")).unwrap();
    assert!(recovered.restored);
    assert_eq!(bytes(&path), lifted);
    assert_eq!(bytes(&recovered.evidence.join("pre-lift.db")), original);
    // A second Prepare starts from 60 and lifts nothing more.
    let (store, plan) = transfer::begin(&path, join_db(&path, "again")).unwrap();
    assert_eq!(plan.registry, 60);
    assert!(!artifact_dir(&path).join("pre-lift.db").exists());
    drop(store);
}

#[test]
fn a_drifted_catalog_below_60_is_refused_before_anything_is_written() {
    for registry in [28u32, 45, 57] {
        let tmp = tempfile::tempdir().unwrap();
        let path = below(tmp.path(), registry);
        Connection::open(&path).unwrap().execute_batch("CREATE TABLE stray(x)").unwrap();
        let before = bytes(&path);
        let Err(error) = transfer::begin(&path, join_db(&path, "drift")) else { panic!("registry {registry}: a drifted catalog was admitted") };
        let error = error.to_string();
        assert!(error.contains("pinned"), "registry {registry}: {error}");
        assert_eq!(bytes(&path), before, "registry {registry}: the file is untouched");
        assert!(!artifact_dir(&path).exists(), "registry {registry}: no artifact");
        let mut scratch = path.as_os_str().to_owned();
        scratch.push(".lift-check");
        assert!(!std::path::Path::new(&scratch).exists(), "registry {registry}: no scratch copy left");
    }
}
