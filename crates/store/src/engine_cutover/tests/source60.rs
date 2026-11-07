//! The finite raw schema60 path, from a registry-prefix fixture seeded with
//! synthetic literals. Nothing here opens the file ordinarily before capture.
use super::*;
use super::startup_gate::bytes;
use startup::{artifact_dir, open_recovery};

fn put(conn: &Connection, table: &str, given: &[(&str, rusqlite::types::Value)]) {
    use rusqlite::types::Value;
    let mut columns: Vec<String> = given.iter().map(|(c, _)| (*c).to_owned()).collect();
    let mut values: Vec<Value> = given.iter().map(|(_, v)| v.clone()).collect();
    let info: Vec<(String, String, bool, bool)> = conn.prepare(&format!("PRAGMA table_info({table})")).unwrap()
        .query_map([], |r| Ok((r.get(1)?, r.get(2)?, r.get::<_, i64>(3)? == 1, r.get::<_, Option<String>>(4)?.is_some()))).unwrap().map(|r| r.unwrap()).collect();
    for (name, kind, required, defaulted) in info {
        if !required || defaulted || columns.contains(&name) { continue; }
        values.push(match kind.as_str() { "INTEGER" => Value::Integer(0), "TEXT" => Value::Text(String::new()), _ => Value::Blob(vec![0; 16]) });
        columns.push(name);
    }
    let marks = vec!["?"; columns.len()].join(",");
    conn.execute(&format!("INSERT INTO \"{table}\"({}) VALUES({marks})", columns.join(",")), rusqlite::params_from_iter(values)).unwrap();
}
fn id(n: u8) -> rusqlite::types::Value { rusqlite::types::Value::Blob(vec![n; 16]) }
fn text(s: &str) -> rusqlite::types::Value { rusqlite::types::Value::Text(s.into()) }
fn int(n: i64) -> rusqlite::types::Value { rusqlite::types::Value::Integer(n) }

/// Registry prefix 60 with invalid wait bytes, value-type distinctions, equal-time
/// notes, a deleted high allocator id, edges, policy, budget and check-in history.
pub(super) fn source60(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("db.sqlite");
    let mut conn = Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys=OFF; PRAGMA recursive_triggers=ON; CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);").unwrap();
    crate::migrate::migrate_only_to(&mut conn, 60);
    crate::compat::stamp_compatible_down_to(&conn).unwrap();
    conn.execute("UPDATE meta SET value='60' WHERE key='compatible_down_to'", []).unwrap();
    put(&conn, "repository_roots", &[("id", id(1)), ("path", text("resources/repo"))]);
    put(&conn, "repositories", &[("id", id(2)), ("repository_root_id", id(1)), ("canonical_git_dir", text("resources/repo/.git"))]);
    put(&conn, "workspaces", &[("id", id(3)), ("repository_id", id(2)), ("name", text("Main")), ("is_main", int(1))]);
    for (n, key, status) in [(4u8, "ov-1", "in_progress"), (5, "ov-2", "done"), (6, "ov-3", "not-a-status")] {
        put(&conn, "tasks", &[("id", id(n)), ("workspace_id", id(3)), ("repository_id", id(2)), ("key", text(key)), ("title", text("空\nmultiline")), ("status", text(status)), ("status_since", int(n as i64))]);
    }
    conn.execute("UPDATE tasks SET wait_kind='\u{1}bad', wait_rank=x'00ff' WHERE key='ov-3'", []).unwrap();
    use rusqlite::types::Value;
    for (n, at, extra) in [(7u8, int(1000), Value::Text("{}".into())), (8, int(1000), Value::Blob(vec![0, 255])), (9, Value::Real(1.5), Value::Text(String::new()))] {
        put(&conn, "task_notes", &[("id", id(n)), ("task_id", id(4)), ("kind", text("decision")), ("actor", text("user")), ("at", at), ("body", text("")), ("extra", extra)]);
    }
    put(&conn, "task_blocks", &[("task_id", id(4)), ("blocked_by", id(5)), ("reason", text("edge"))]);
    put(&conn, "task_reads", &[("task_id", id(4))]);
    put(&conn, "task_reads", &[("task_id", id(5))]);
    put(&conn, "ticket_lines", &[("task_id", id(4)), ("stage", text("waiting")), ("stack_parent", id(5)), ("same_branch", id(5))]);
    put(&conn, "ticket_lines", &[("task_id", id(5)), ("stage", text("landed"))]);
    put(&conn, "plan_budgets", &[("id", id(10)), ("workspace_id", id(3)), ("task_id", id(4)), ("tokens", int(5))]);
    put(&conn, "check_in_schedules", &[("workspace_id", id(3)), ("task_id", id(4)), ("authorizer_source", text("local_person")), ("state", text("enabled"))]);
    put(&conn, "check_in_occurrences", &[("id", id(11)), ("workspace_id", id(3)), ("task_id", id(4)), ("authorizer_source", text("local_person")), ("state", text("uncertain"))]);
    put(&conn, "check_in_enter_fences", &[("workspace_id", id(3)), ("task_id", id(4)), ("occurrence_id", id(11)), ("attempt_id", id(12))]);
    put(&conn, "people_events", &[("id", int(900)), ("actor", text("user"))]);
    conn.execute("DELETE FROM people_events", []).unwrap();
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
