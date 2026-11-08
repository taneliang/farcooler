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

    /// Who added `client_id` and when, in milliseconds: `("", at)` for this
    /// runner's socket. `None` for a device enrolled before ov-838 recorded it.
    pub fn added_by_of(&self, client_id: &str) -> Result<Option<(String, i64)>> {
        self.conn()
            .query_row("SELECT added_by, added_at FROM device_added_by WHERE client_id = ?1", [client_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .map_err(map_err)
    }

    /// Record how `client_id` was paired (ov-982), replacing what an earlier
    /// enrollment of the id said. Kept in `meta` beside its enroller, so no
    /// migration lands for one word per device.
    pub fn record_paired_via(&self, client_id: &str, via: PairedVia) -> Result<()> {
        self.conn()
            .execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![paired_via_key(client_id), via.as_str()],
            )
            .map(drop)
            .map_err(map_err)
    }

    /// How `client_id` was paired, or `None` for a device paired before
    /// ov-982 recorded it.
    pub fn paired_via(&self, client_id: &str) -> Result<Option<PairedVia>> {
        let said: Option<String> =
            self.conn().query_row("SELECT value FROM meta WHERE key = ?1", [paired_via_key(client_id)], |r| r.get(0)).optional().map_err(map_err)?;
        Ok(said.as_deref().and_then(PairedVia::parse))
    }

    /// Record the model `client_id` said at pairing (ov-989), replacing an earlier
    /// enrollment's. Empty forgets it. In `meta`, like how it was paired.
    pub fn record_device_model(&self, client_id: &str, model: &str) -> Result<()> {
        let conn = self.conn();
        if model.is_empty() {
            return conn.execute("DELETE FROM meta WHERE key = ?1", [device_model_key(client_id)]).map(drop).map_err(map_err);
        }
        conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![device_model_key(client_id), model],
        )
        .map(drop)
        .map_err(map_err)
    }

    /// The model `client_id` said at pairing, or empty.
    pub fn device_model(&self, client_id: &str) -> Result<String> {
        let said: Option<String> =
            self.conn().query_row("SELECT value FROM meta WHERE key = ?1", [device_model_key(client_id)], |r| r.get(0)).optional().map_err(map_err)?;
        Ok(said.unwrap_or_default())
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

/// How a device was paired (ov-982), for the owner deciding whether to vouch
/// for it: through the Far Cooler app's own pairing on this runner's Mac, by
/// another device of theirs, or from a terminal on the runner (the CLI's
/// `client enroll`, or anything else on the socket that isn't the app).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairedVia {
    App,
    Device,
    Terminal,
}

impl PairedVia {
    pub fn as_str(self) -> &'static str {
        match self {
            PairedVia::App => "app",
            PairedVia::Device => "device",
            PairedVia::Terminal => "terminal",
        }
    }

    pub fn parse(word: &str) -> Option<PairedVia> {
        Some(match word {
            "app" => PairedVia::App,
            "device" => PairedVia::Device,
            "terminal" => PairedVia::Terminal,
            _ => return None,
        })
    }
}

fn device_model_key(client_id: &str) -> String {
    format!("device_model:{client_id}")
}

/// A model a device said, as one short printable line (ov-989): control and
/// format characters (a bidi override included) dropped, runs of space made
/// one, at most 40 characters. It is shown to the owner deciding whether to
/// vouch, so it can't carry a line break or reorder what's beside it.
pub fn clean_model(said: &str) -> String {
    let kept: String = said.chars().map(|c| if c.is_whitespace() { ' ' } else { c }).filter(|c| !c.is_control() && !is_format(*c)).collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(40).collect::<String>().trim().to_string()
}

fn is_format(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}

fn paired_via_key(client_id: &str) -> String {
    format!("device_paired_via:{client_id}")
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
