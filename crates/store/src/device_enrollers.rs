//! Which member's device enrolled a device (ov-719, the ov-716 review's
//! finding 1).
//!
//! A device bound to nobody and never released is the runner's owner
//! (`people`). Before this, a device a member admin enrolled was exactly
//! that: one call gave a member's device a second device with the owner's
//! say, which outlived the member's removal. So a device a member's device
//! enrolls is released (nobody's, reads only) from the moment it's written
//! until a runner admin binds it to a member (`Store::set_member_in` takes
//! it off `released_devices`), and its enroller is kept here, so removing
//! that member revokes and releases what they enrolled and nobody was bound
//! to.
//!
//! The owner's enrollments, from the local socket or the owner's own
//! device, are never recorded: they're the owner's, as before.

use rusqlite::params;

use crate::error::map_err;
use crate::store::Store;
use farcooler_core::Result;

/// One new table. A row goes with its member by cascade, so a handle used
/// again later inherits nothing.
///
/// `Older::Welcome`: a build from before it never names the table. It
/// still reads a device a member enrolled as released until bound, because
/// that mark is a `released_devices` row, 0046's own table; only removing
/// the member revokes nothing more than it did.
pub(crate) fn migration_0058_device_enrollers(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        "CREATE TABLE device_enrollers (
             client_id TEXT PRIMARY KEY NOT NULL,
             handle TEXT NOT NULL REFERENCES members(handle) ON DELETE CASCADE,
             enrolled_at INTEGER NOT NULL
         );",
    )
}

impl Store {
    /// Record that `handle`'s device enrolled `client_id`, and release it
    /// unless it's bound to a member already (a teammate's device written
    /// again keeps its member). One transaction: a device is never written
    /// down as enrolled by a member and still read as the owner's.
    pub fn record_member_enrolled(&self, client_id: &str, handle: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        tx.execute(
            "INSERT INTO device_enrollers (client_id, handle, enrolled_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(client_id) DO UPDATE SET handle = excluded.handle, enrolled_at = excluded.enrolled_at",
            params![client_id, handle, crate::tasks::now_millis()],
        )
        .map_err(map_err)?;
        tx.execute(
            "INSERT OR IGNORE INTO released_devices (client_id)
             SELECT ?1 WHERE NOT EXISTS (SELECT 1 FROM member_devices WHERE client_id = ?1)",
            [client_id],
        )
        .map_err(map_err)?;
        tx.commit().map_err(map_err)
    }

    /// The devices `handle`'s devices enrolled that no member is bound to,
    /// sorted: what removing them revokes beside their own.
    pub fn unbound_enrolled_by(&self, handle: &str) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT e.client_id FROM device_enrollers e
                 WHERE e.handle = ?1 AND NOT EXISTS (SELECT 1 FROM member_devices d WHERE d.client_id = e.client_id)
                 ORDER BY e.client_id",
            )
            .map_err(map_err)?;
        let rows = stmt.query_map([handle], |r| r.get(0)).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<String>>>().map_err(map_err)
    }

    /// Whether `client_id` reads as the owner's device: bound to no member
    /// and not released. True too of an id nothing enrolled, which is
    /// the owner's should it ever be (`people::person_of`).
    pub fn device_is_owners(&self, client_id: &str) -> Result<bool> {
        Ok(self.member_of_device(client_id)?.is_none() && !self.device_released(client_id)?)
    }
}

#[cfg(test)]
#[path = "device_enrollers_tests.rs"]
mod tests;
