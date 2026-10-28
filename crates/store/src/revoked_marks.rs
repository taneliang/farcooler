//! Which devices were revoked and not enrolled again since (ov-832, the
//! ov-826 review's S2).
//!
//! A revoke marks its device under the fence lock (`Sessions::mark_revoked`),
//! and every connection the device opens after that starts closed, until a
//! real enrollment write of that id lifts the mark (`Sessions::readmit`). The
//! mark lived only in memory, so a daemon restart forgot it. A revoked Mac's
//! ssh master, still alive (`ControlPersist`), could then open a fresh
//! channel, get an unmarked session, and `client.enroll` its own line back.
//! Kept here, the mark outlives the process, and the daemon loads it at open.

use rusqlite::params;

use crate::error::map_err;
use crate::store::Store;
use farcooler_core::Result;

/// One new table.
///
/// `Older::Welcome`: a build from before it never names the table, and
/// keeps its marks in memory as it always did.
pub(crate) fn migration_0064_revoked_marks(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        "CREATE TABLE revoked_marks (
             client_id TEXT PRIMARY KEY NOT NULL,
             revoked_at INTEGER NOT NULL
         );",
    )
}

impl Store {
    /// Keep `client_id`'s revoked mark until [`Store::lift_revoked_mark`].
    /// Marking it again keeps one row.
    pub fn keep_revoked_mark(&self, client_id: &str) -> Result<()> {
        self.conn()
            .execute(
                "INSERT INTO revoked_marks (client_id, revoked_at) VALUES (?1, ?2)
                 ON CONFLICT(client_id) DO UPDATE SET revoked_at = excluded.revoked_at",
                params![client_id, crate::tasks::now_millis()],
            )
            .map(drop)
            .map_err(map_err)
    }

    /// `client_id` was enrolled again: its mark goes.
    pub fn lift_revoked_mark(&self, client_id: &str) -> Result<()> {
        self.conn().execute("DELETE FROM revoked_marks WHERE client_id = ?1", [client_id]).map(drop).map_err(map_err)
    }

    /// Every device still marked revoked, sorted.
    pub fn revoked_marks(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT client_id FROM revoked_marks ORDER BY client_id").map_err(map_err)?;
        let rows = stmt.query_map([], |r| r.get(0)).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<String>>>().map_err(map_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mark_is_kept_until_it_is_lifted() {
        let store = Store::open_in_memory().unwrap();
        store.keep_revoked_mark("phone").unwrap();
        store.keep_revoked_mark("phone").unwrap();
        store.keep_revoked_mark("mac").unwrap();
        assert_eq!(store.revoked_marks().unwrap(), vec!["mac".to_string(), "phone".to_string()]);
        store.lift_revoked_mark("phone").unwrap();
        assert_eq!(store.revoked_marks().unwrap(), vec!["mac".to_string()]);
    }

    /// 0064 (ov-832) is the 55th and `Welcome`.
    #[test]
    fn the_revoked_marks_migration_is_welcome() {
        let it = &crate::migrate::MIGRATIONS[54];
        assert!(std::ptr::fn_addr_eq(it.0, migration_0064_revoked_marks as fn(&rusqlite::Transaction) -> rusqlite::Result<()>));
        assert_eq!(it.1, crate::compat::Older::Welcome);
    }
}
