//! Which device added each device (ov-838, the ov-828 review's N1).
//!
//! `device_enrollers` keeps a member's enrollments for their removal; this
//! keeps every enrollment's enroller, by device: the client id of the device
//! whose `client.enroll` wrote the line, or the empty string for this
//! runner's own socket. With it:
//!
//! - revoking a lost phone can name the devices it added, and those they
//!   added, and take them all in one step (`client.revoke` `with_added`);
//! - a device below `host_admin` adds only so many devices a day
//!   ([`Store::added_by_since`]);
//! - binding a device to a member takes along what it added while it read
//!   as the owner's (`Store::set_member_releasing`), so a teammate's line
//!   from before ov-828 can't leave the owner's devices behind it.
//!
//! A row outlives its device's revoke, so a chain through a revoked device
//! is still a chain. Enrolling the id again replaces it.

use rusqlite::{OptionalExtension, params};

use crate::error::map_err;
use crate::store::Store;
use farcooler_core::Result;

/// One new table.
///
/// `Older::Welcome`: a build from before it never names the table, and
/// records no enroller, as it never did.
pub(crate) fn migration_0065_device_added_by(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        "CREATE TABLE device_added_by (
             client_id TEXT PRIMARY KEY NOT NULL,
             added_by TEXT NOT NULL,
             added_at INTEGER NOT NULL
         );
         CREATE INDEX device_added_by_enroller ON device_added_by(added_by, added_at);",
    )
}

impl Store {
    /// `added_by` (a device id, or empty for this runner's socket) added
    /// `client_id`, now.
    pub fn record_added_by(&self, client_id: &str, added_by: &str) -> Result<()> {
        self.record_added_by_at(client_id, added_by, crate::tasks::now_millis())
    }

    /// [`Store::record_added_by`], at `at` milliseconds.
    pub fn record_added_by_at(&self, client_id: &str, added_by: &str, at: i64) -> Result<()> {
        self.conn()
            .execute(
                "INSERT INTO device_added_by (client_id, added_by, added_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(client_id) DO UPDATE SET added_by = excluded.added_by, added_at = excluded.added_at",
                params![client_id, added_by, at],
            )
            .map(drop)
            .map_err(map_err)
    }

    /// How many devices `added_by` added at or after `since` milliseconds.
    pub fn added_by_since(&self, added_by: &str, since: i64) -> Result<u32> {
        self.conn()
            .query_row(
                "SELECT COUNT(*) FROM device_added_by WHERE added_by = ?1 AND added_at >= ?2",
                params![added_by, since],
                |r| r.get(0),
            )
            .map_err(map_err)
    }

    /// Every device's enroller, as `(device, added_by)`, sorted by device.
    pub fn added_by_all(&self) -> Result<Vec<(String, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT client_id, added_by FROM device_added_by ORDER BY client_id").map_err(map_err)?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<(String, String)>>>().map_err(map_err)
    }

    /// Bind `client_id` to `handle` as a device their own `control` device
    /// added for themselves (ov-838, R-88): their browser, theirs from its
    /// write, so removing them revokes it as their other devices. One
    /// transaction. Refused once `handle` is no member, and `device_taken`
    /// when another member holds the id, writing nothing either way.
    pub fn record_member_device_added(&self, client_id: &str, handle: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let holder: Option<String> = tx
            .query_row("SELECT handle FROM member_devices WHERE client_id = ?1", [client_id], |r| r.get(0))
            .optional()
            .map_err(map_err)?;
        if holder.as_deref().is_some_and(|h| h != handle) {
            return Err(farcooler_core::DomainError::Conflict { what: "device_taken" });
        }
        tx.execute(
            "INSERT INTO member_devices (client_id, handle) VALUES (?1, ?2) ON CONFLICT(client_id) DO NOTHING",
            params![client_id, handle],
        )
        .map_err(map_err)?;
        tx.execute("DELETE FROM released_devices WHERE client_id = ?1", [client_id]).map_err(map_err)?;
        tx.execute("DELETE FROM pending_teammates WHERE client_id = ?1", [client_id]).map_err(map_err)?;
        tx.execute("DELETE FROM device_enrollers WHERE client_id = ?1", [client_id]).map_err(map_err)?;
        tx.commit().map_err(map_err)
    }

    /// What a revoke of `client_id` leaves behind that's about it alone
    /// (the ov-828 re-review's R4): a pending teammate's row, and a member's
    /// enrollment waiting on its bind. Its line is gone, so neither has
    /// anything left to wait for. Who added it, and whom it added, stay.
    pub fn forget_revoked_device(&self, client_id: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        tx.execute("DELETE FROM pending_teammates WHERE client_id = ?1", [client_id]).map_err(map_err)?;
        tx.execute("DELETE FROM device_enrollers WHERE client_id = ?1", [client_id]).map_err(map_err)?;
        tx.commit().map_err(map_err)
    }
}

/// The devices `root` added, and those they added, without end: every id
/// reached through `rows` (`(device, added_by)`), never `root` itself.
/// Sorted, each once, whatever cycle the rows hold.
pub fn added_under(rows: &[(String, String)], root: &str) -> Vec<String> {
    let mut found: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut frontier = vec![root.to_string()];
    while let Some(by) = frontier.pop() {
        for (device, added_by) in rows {
            if *added_by == by && device != root && found.insert(device.clone()) {
                frontier.push(device.clone());
            }
        }
    }
    found.into_iter().collect()
}

/// In `tx`: the devices `added_by` added, and those they added, that read
/// as the owner's (bound to no member, not released), each recorded as
/// `handle`'s enrollment and released until bound, as a member's own
/// enrollment is (ov-719). For `set_member_releasing`, once `added_by` is
/// bound to `handle`.
pub(crate) fn hand_over_added_in(tx: &rusqlite::Transaction, added_by: &str, handle: &str) -> Result<Vec<String>> {
    let rows: Vec<(String, String)> = {
        let mut stmt = tx.prepare("SELECT client_id, added_by FROM device_added_by").map_err(map_err)?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(map_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(map_err)?
    };
    let mut handed = Vec::new();
    for device in added_under(&rows, added_by) {
        let owners: bool = tx
            .query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM member_devices WHERE client_id = ?1)
                    AND NOT EXISTS(SELECT 1 FROM released_devices WHERE client_id = ?1)",
                [&device],
                |r| r.get(0),
            )
            .map_err(map_err)?;
        if !owners {
            continue;
        }
        tx.execute(
            "INSERT INTO device_enrollers (client_id, handle, enrolled_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(client_id) DO UPDATE SET handle = excluded.handle, enrolled_at = excluded.enrolled_at",
            params![device, handle, crate::tasks::now_millis()],
        )
        .map_err(map_err)?;
        tx.execute("INSERT OR IGNORE INTO released_devices (client_id) VALUES (?1)", [&device]).map_err(map_err)?;
        handed.push(device);
    }
    Ok(handed)
}

#[cfg(test)]
#[path = "device_added_by_tests.rs"]
mod tests;
