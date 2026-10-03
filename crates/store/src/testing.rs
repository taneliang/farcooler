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

/// A task's `edited_at`, which no `Task` field carries: the time of the
/// last revision that changed a field, or `None` if nothing has been
/// revised since the column existed. For a test that has to tell "this write
/// was dated" from "this write landed in the same millisecond as creation",
/// which `updated_at` alone cannot.
pub fn edited_at(store: &crate::Store, task: Uuid) -> Option<i64> {
    store
        .conn()
        .query_row("SELECT edited_at FROM tasks WHERE id = ?1", params![uuid_blob(task)], |r| r.get(0))
        .expect("the task exists")
}

/// Move every queued answer's `enqueued_at` back by `by_ms`, for a test of
/// what happens to an answer that has waited too long.
pub fn backdate_answer_wakes(store: &crate::Store, by_ms: i64) {
    store.conn().execute("UPDATE answer_wakes SET enqueued_at = enqueued_at - ?1", params![by_ms]).unwrap();
}

/// The store's own clock, for a test that sets a time relative to it.
pub fn now_millis() -> i64 {
    crate::tasks::now_millis()
}

/// A note on `task` as a newer build might write it: a kind and an actor
/// this build has no word for.
pub fn note_from_a_newer_build(store: &crate::Store, task: Uuid, kind: &str, actor: &str, body: &str) {
    store
        .conn()
        .execute(
            "INSERT INTO task_notes (id, task_id, kind, actor, at, body, extra) VALUES (?1, ?2, ?3, ?4, ?5, ?6, '{}')",
            params![uuid_blob(Uuid::now_v7()), uuid_blob(task), kind, actor, crate::tasks::now_millis(), body],
        )
        .unwrap();
}
