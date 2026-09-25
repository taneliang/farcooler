//! Fixtures for other crates' tests. Built only under `cfg(test)` or the
//! `testing` feature, which the daemon enables from its dev-dependencies.

use std::path::Path;

use rusqlite::{Connection, params};
use uuid::Uuid;

use crate::models::uuid_blob;

/// Writes a database file at `path` exactly as a schema-11 runner left it:
/// one repository, `repository`, named `overnight`, registered before the
/// board, so it has no task key prefix; and on it one task, `task`, keyed
/// `-1`. Opening it with `Store::open` runs migration 12 on it.
pub fn write_prefixless_board_at_schema_11(path: &Path, repository: Uuid, task: Uuid) {
    let mut conn = Connection::open(path).unwrap();
    conn.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);").unwrap();
    crate::migrate::migrate_only_to(&mut conn, 11);
    let (root, host) = (uuid_blob(Uuid::now_v7()), uuid_blob(Uuid::now_v7()));
    conn.execute("INSERT INTO repository_roots VALUES (?1, ?2, '/r', 0, 1)", params![root, host]).unwrap();
    conn.execute(
        "INSERT INTO repositories VALUES (?1, ?2, ?3, 'overnight', '/r/.git', '', 1, '')",
        params![uuid_blob(repository), host, root],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tasks (id, repository_id, key, title, status, status_since, created_at, resource_version)
         VALUES (?1, ?2, '-1', 'first', 'backlog', 0, 0, 1)",
        params![uuid_blob(task), uuid_blob(repository)],
    )
    .unwrap();
}
