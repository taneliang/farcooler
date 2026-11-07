//! Fixtures for other crates' tests. Built only under `cfg(test)` or the
//! `testing` feature, which the daemon enables from its dev-dependencies.

use std::path::Path;

use rusqlite::{Connection, params};
use uuid::Uuid;

use crate::models::uuid_blob;

/// An explicit legacy-fixture open: normalizes an old-schema file the way the
/// pre-cutover build did. Behind the `testing` feature only; production has no
/// flag, environment variable or argument that reaches this.
pub fn open_legacy_fixture(path: &Path) -> farcooler_core::Result<crate::Store> { crate::Store::open_normalizing(path) }

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

/// The same for every queued hold that ended.
pub fn backdate_hold_wakes(store: &crate::Store, by_ms: i64) {
    store.conn().execute("UPDATE hold_wakes SET enqueued_at = enqueued_at - ?1", params![by_ms]).unwrap();
}

/// The same for every queued message (ov-711).
pub fn backdate_message_wakes(store: &crate::Store, by_ms: i64) {
    store.conn().execute("UPDATE message_wakes SET enqueued_at = enqueued_at - ?1", params![by_ms]).unwrap();
}

/// The same for every held composed message (ov-468).
pub fn backdate_compose_holds(store: &crate::Store, by_ms: i64) {
    store.conn().execute("UPDATE compose_holds SET enqueued_at = enqueued_at - ?1", params![by_ms]).unwrap();
}

/// Rewrite every kept wait for an Enter (ov-816), as `change` makes it:
/// a wait some other runner's claude, or an older start, left.
pub fn rewrite_compose_waits(store: &crate::Store, change: impl Fn(&str) -> String) {
    for wait in store.compose_waits().unwrap() {
        store.conn().execute("UPDATE compose_waits SET wait = ?2 WHERE id = ?1", params![crate::models::uuid_blob(wait.id), change(&wait.wait)]).unwrap();
    }
}

/// Mark every held composed message claimed, as a restart mid-typing leaves it.
pub fn claim_compose_holds(store: &crate::Store) {
    store.conn().execute("UPDATE compose_holds SET claimed_at = 1", []).unwrap();
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

/// Remove the plan layer from a database: its themes, rulings (ov-304), the
/// line and its landings (ov-872), its history, budgets (ov-307) and CI reads,
/// children first. What deleting the experiment's migrations would leave
/// behind, for the drill that checks the board reads the same without it.
/// `agents` and `ticket_work` stay: they are the board's record of who works a
/// task (ov-213), as `task_workers` was.
pub fn drop_plan_layer(store: &crate::Store) {
    store
        .conn()
        .execute_batch(
            "DROP TABLE plan_budgets; DROP TABLE board_ci; DROP TABLE landing_tickets; DROP TABLE line_events;
             DROP TABLE landings; DROP TABLE ticket_lines;
             DROP TABLE board_ruling_tasks; DROP TABLE board_rulings;
             DROP TABLE board_theme_tasks; DROP TABLE board_themes;",
        )
        .unwrap();
}

/// Remove the pages from a database: both tables, children first. What
/// deleting their migration would leave behind (ov-269), for the drill that
/// checks the board and the plan read the same without them.
pub fn drop_pages(store: &crate::Store) {
    store.conn().execute_batch("DROP TABLE page_events; DROP TABLE board_pages;").unwrap();
}

/// Put a ticket in the landing stage with nothing else written, as a build
/// from before the collecting landing left it: on no landing, no event.
pub fn strand_in_landing(store: &crate::Store, task: uuid::Uuid) {
    store.conn().execute("UPDATE ticket_lines SET stage = 'landing' WHERE task_id = ?1", [task.as_bytes().to_vec()]).unwrap();
}

/// Forget every pane's recorded Claude login, as a pane from before they
/// were recorded (ov-583).
pub fn forget_terminal_logins(store: &crate::Store) {
    store.conn().execute("DELETE FROM terminal_logins", []).unwrap();
}

/// Make the members' own-login policy unreadable (ov-652 review 1, item 5):
/// a read of it fails as a damaged table would.
pub fn break_member_login_policy(store: &crate::Store) {
    store.conn().execute_batch("DROP TABLE member_login_policy").unwrap();
}

/// Make the kept revoked marks unwritable (ov-832): a lift of one fails as
/// a damaged table would.
pub fn break_revoked_marks(store: &crate::Store) {
    store.conn().execute_batch("DROP TABLE revoked_marks").unwrap();
}

/// Fixture-only: run one statement on the store's connection (no params).
pub fn exec_for_tests(store: &crate::Store, sql: &str) { store.conn().execute(sql, []).unwrap(); }

/// Fixture-only: a transfer driven to a released engine, for a daemon fixture
/// whose background tasks outlive its Service and so keep the owner lock. A
/// test of what follows a cutover, not of who may start one.
pub fn released_transfer_for_tests(db: &std::path::Path, attempt: &str) {
    use crate::engine_cutover::{JoinedOwners, transfer};
    let (store, _plan) = transfer::begin(db, JoinedOwners::unit(attempt)).unwrap();
    exec_for_tests(&store, "UPDATE engine_cutover_authority SET phase='released', incarnation=1");
    transfer::finish(db, &store, JoinedOwners::unit(attempt)).unwrap();
}

/// Fixture-only: the row count of `table`.
pub fn count_rows_for_tests(store: &crate::Store, table: &str) -> i64 {
    store.conn().query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)).unwrap()
}
