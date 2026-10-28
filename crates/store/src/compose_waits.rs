//! Composed messages typed into claude's box whose Enter waits out a turn
//! (ov-808), kept across a restart as held messages are (ov-816).
//!
//! A row is one wait, as the daemon's `answer_wake::compose::pending` writes
//! it: what was typed, where claude will say it took it, and when the wait
//! began, as JSON this crate never reads into. The daemon writes the row
//! before the wait begins and deletes it when the wait ends, however it
//! ends; a row a restart finds is a wait cut short, which the daemon picks
//! up again. A pane's rows go with it by cascade.

use rusqlite::params;
use uuid::Uuid;

use farcooler_core::Result;

use crate::error::map_err;
use crate::models::{get_uuid, uuid_blob};
use crate::store::Store;
use crate::tasks::now_millis;

/// One new table, so a build from before it reads every table it knows
/// exactly as it did. One that runs meanwhile keeps no wait across a restart,
/// as before, and never reads the rows.
pub(crate) fn migration_0062_compose_waits(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        CREATE TABLE compose_waits (
            id BLOB PRIMARY KEY,
            terminal_id BLOB NOT NULL REFERENCES terminals(id) ON DELETE CASCADE,
            wait TEXT NOT NULL,
            kept_at INTEGER NOT NULL
        );
        "#,
    )
}

/// One kept wait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeWait {
    pub id: Uuid,
    pub terminal: Uuid,
    /// The daemon's own record of it, as it wrote it.
    pub wait: String,
    /// Unix milliseconds.
    pub kept_at: i64,
}

impl Store {
    /// Keep the wait `id` for `terminal`.
    pub fn keep_compose_wait(&self, id: Uuid, terminal: Uuid, wait: &str) -> Result<()> {
        self.conn()
            .execute(
                "INSERT INTO compose_waits (id, terminal_id, wait, kept_at) VALUES (?1, ?2, ?3, ?4)",
                params![uuid_blob(id), uuid_blob(terminal), wait, now_millis()],
            )
            .map(|_| ())
            .map_err(map_err)
    }

    /// Every wait kept, oldest first.
    pub fn compose_waits(&self) -> Result<Vec<ComposeWait>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id, terminal_id, wait, kept_at FROM compose_waits ORDER BY kept_at, rowid").map_err(map_err)?;
        let rows = stmt
            .query_map([], |r| Ok(ComposeWait { id: get_uuid(r, 0)?, terminal: get_uuid(r, 1)?, wait: r.get(2)?, kept_at: r.get(3)? }))
            .map_err(map_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_err)?;
        Ok(rows)
    }

    /// The wait `id` is over: gone.
    pub fn drop_compose_wait(&self, id: Uuid) -> Result<()> {
        self.conn().execute("DELETE FROM compose_waits WHERE id = ?1", params![uuid_blob(id)]).map(|_| ()).map_err(map_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The migration is the 53rd (0062) and `Welcome`.
    #[test]
    fn the_migration_is_welcome() {
        let it = &crate::migrate::MIGRATIONS[52];
        assert!(std::ptr::fn_addr_eq(it.0, migration_0062_compose_waits as fn(&rusqlite::Transaction) -> rusqlite::Result<()>));
        assert_eq!(it.1, crate::compat::Older::Welcome);
        assert_eq!(crate::migrate::CURRENT_SCHEMA_VERSION, 55);
    }

    /// Kept in order, read back as written, dropped one by one, and gone
    /// with their pane.
    #[test]
    fn waits_are_kept_until_dropped_or_their_pane_goes() {
        let store = Store::open_in_memory().unwrap();
        let host = Uuid::now_v7();
        let root = store.create_repository_root(host, "/r", 1_000).unwrap();
        let repo = store.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();
        let ws = store.create_worktree(repo.id, "feature/x", "/r", false).unwrap();
        let pane = store.create_terminal(ws.id, "Agent", "claude", farcooler_protocol::v1::TerminalIntent::Running, 120, 40).unwrap();
        assert_eq!(store.compose_waits().unwrap(), []);
        let (first, second) = (Uuid::now_v7(), Uuid::now_v7());
        store.keep_compose_wait(first, pane.id, r#"{"one":1}"#).unwrap();
        store.keep_compose_wait(second, pane.id, r#"{"two":2}"#).unwrap();
        let kept: Vec<_> = store.compose_waits().unwrap().into_iter().map(|w| (w.id, w.terminal, w.wait)).collect();
        assert_eq!(kept, [(first, pane.id, r#"{"one":1}"#.to_string()), (second, pane.id, r#"{"two":2}"#.to_string())]);
        store.drop_compose_wait(first).unwrap();
        assert_eq!(store.compose_waits().unwrap().into_iter().map(|w| w.id).collect::<Vec<_>>(), [second]);
        store.delete_terminal(pane.id, pane.resource_version).unwrap();
        assert_eq!(store.compose_waits().unwrap(), []);
    }
}
