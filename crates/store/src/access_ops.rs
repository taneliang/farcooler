//! What changes the access tables after the migration (ov-856, slice 2): the
//! positive bindings an enroll writes, the rows a revoke clears, a person's
//! access and a device's cap, and the generation each of those bumps.
//!
//! - **A bind is insert-only.** An id with any row left (a binding, a cap, a
//!   step-up key, a mark, a proposal) is refused `device_taken`: a later
//!   enroll of that id must not inherit an earlier person or key (S5).
//! - **A revoke clears the rows it owns for the id** in one transaction
//!   ([`Store::clear_device_rows`]): its owner binding, cap, step-up key,
//!   pending-teammate mark and enroller. It calls it inside the same fence lock
//!   hold that removed the line. **The two older marks stay**, as they did
//!   before: `member_devices` (a revoked device is still in its member's list
//!   until someone edits it) and `released_devices` (a removed member's device
//!   stays refused and stays swept). Both still count as rows left over, so a
//!   new line under such an id is refused until the mark is gone.
//! - **`access_generation` is per person** (the owner is the empty string):
//!   a change to their access, to the cap of a device of theirs, or the
//!   revoke of a device that held a cap or a step-up key, bumps it in the
//!   same transaction. A request that decided something under an older
//!   generation is refused `access_changed` when it reaches the lock (S2).
//!   Adding a device does not bump it: that changes nobody's authority, and
//!   two people pairing at once must not refuse each other. A device moved to
//!   another person is caught by re-reading the person, not the generation.
//!   Kept in `meta`, so it adds no table an older build must know of.

use std::collections::BTreeSet;

use rusqlite::{OptionalExtension, params};

use crate::access::{Access, AccessPlan, Cap, OwnerDeviceState};
use crate::error::map_err;
use crate::store::Store;
use farcooler_core::{DomainError, Result};

/// The tables a revoke clears for the id it removes.
const CLEARED_TABLES: [&str; 5] = ["owner_devices", "device_caps", "step_up_keys", "pending_teammates", "device_enrollers"];

/// The two marks older than this a revoke leaves, as it always has.
const KEPT_MARKS: [&str; 2] = ["member_devices", "released_devices"];

/// Every table that can hold something about one device id.
const DEVICE_TABLES: [&str; 7] = [CLEARED_TABLES[0], KEPT_MARKS[0], CLEARED_TABLES[1], CLEARED_TABLES[2], CLEARED_TABLES[3], KEPT_MARKS[1], CLEARED_TABLES[4]];

fn generation_key(person: &str) -> String {
    format!("access_gen:{person}")
}

/// Bump `person`'s generation inside `tx`.
fn bump_in(tx: &rusqlite::Transaction, person: &str) -> Result<()> {
    tx.execute(
        "INSERT INTO meta (key, value) VALUES (?1, '1') ON CONFLICT(key) DO UPDATE SET value = CAST(value AS INTEGER) + 1",
        [generation_key(person)],
    )
    .map_err(map_err)?;
    Ok(())
}

/// Who `device` is bound to in `tx`: a member's handle, the empty string for
/// the owner's positive binding, or `None` for nobody's.
fn holder_in(tx: &rusqlite::Transaction, device: &str) -> Result<Option<String>> {
    let member: Option<String> =
        tx.query_row("SELECT handle FROM member_devices WHERE client_id = ?1", [device], |r| r.get(0)).optional().map_err(map_err)?;
    if member.is_some() {
        return Ok(member);
    }
    let owner: bool = tx
        .query_row("SELECT EXISTS(SELECT 1 FROM owner_devices WHERE client_id = ?1)", [device], |r| r.get(0))
        .map_err(map_err)?;
    Ok(owner.then(String::new))
}

/// Called by `set_member_releasing` once `devices` are newly bound to a
/// member: the owner's positive binding for each goes (it is theirs now), and
/// so does the cap it was given against the owner's ceiling, since the caller
/// works the cap out again against theirs (`person_access::record_binding`).
pub(crate) fn rebound_in(tx: &rusqlite::Transaction, devices: &[String]) -> Result<()> {
    for device in devices {
        if tx.execute("DELETE FROM owner_devices WHERE client_id = ?1", [device]).map_err(map_err)? > 0 {
            tx.execute("DELETE FROM device_caps WHERE client_id = ?1", [device]).map_err(map_err)?;
        }
    }
    Ok(())
}

impl Store {
    /// `person`'s generation: the owner's is `""`, a member's their handle.
    /// Zero until something about them has changed.
    pub fn access_generation(&self, person: &str) -> Result<u32> {
        let conn = self.conn();
        Ok(crate::migrate::read_meta_u32(&conn, &generation_key(person))?.unwrap_or(0))
    }

    /// Whether any table holds something about `device`.
    pub fn device_has_rows(&self, device: &str) -> Result<bool> {
        let conn = self.conn();
        for table in DEVICE_TABLES {
            let found: bool = conn
                .query_row(&format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE client_id = ?1)"), [device], |r| r.get(0))
                .map_err(map_err)?;
            if found {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether the tables this slice added hold something about `device`: an
    /// owner binding, a cap or a step-up key. A member's enroll supersedes the
    /// older marks itself (`record_member_enrolled` unbinds and releases the id
    /// first), so only these are left over for it.
    pub fn device_has_access_rows(&self, device: &str) -> Result<bool> {
        let conn = self.conn();
        for table in ["owner_devices", "device_caps", "step_up_keys"] {
            let found: bool = conn
                .query_row(&format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE client_id = ?1)"), [device], |r| r.get(0))
                .map_err(map_err)?;
            if found {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Bind a new line's `device` to the runner's owner, insert-only: refused
    /// `device_taken`, writing nothing, when anything about the id is left.
    /// `cap` is the line's, when it is below the owner's ceiling, so the
    /// access and the cap together reproduce it.
    pub fn bind_owner_device(&self, device: &str, cap: Option<Cap>) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        for table in DEVICE_TABLES {
            let found: bool = tx
                .query_row(&format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE client_id = ?1)"), [device], |r| r.get(0))
                .map_err(map_err)?;
            if found {
                return Err(DomainError::Conflict { what: "device_taken" });
            }
        }
        tx.execute("INSERT INTO owner_devices (client_id, state) VALUES (?1, 'confirmed')", [device]).map_err(map_err)?;
        if let Some(cap) = cap {
            tx.execute("INSERT INTO device_caps (client_id, cap) VALUES (?1, ?2)", params![device, cap.as_str()]).map_err(map_err)?;
        }
        tx.commit().map_err(map_err)
    }

    /// A cap for a device just bound to a member, only if it has none: the
    /// line is below their ceiling. Not a change of anyone's authority, so
    /// no generation moves.
    pub fn cap_new_device(&self, device: &str, cap: Cap) -> Result<()> {
        self.conn()
            .execute("INSERT OR IGNORE INTO device_caps (client_id, cap) VALUES (?1, ?2)", params![device, cap.as_str()])
            .map_err(map_err)?;
        Ok(())
    }

    /// The rows a revoke owns about `device`, gone in one transaction: its
    /// owner binding, cap, step-up key, pending-teammate mark and enroller.
    /// Who added it, whom it added, and the two older marks
    /// ([`KEPT_MARKS`]) stay. Its person's generation moves when it held a
    /// cap or a key.
    pub fn clear_device_rows(&self, device: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let holder = holder_in(&tx, device)?;
        let mut authority = false;
        for table in CLEARED_TABLES {
            let gone = tx.execute(&format!("DELETE FROM {table} WHERE client_id = ?1"), [device]).map_err(map_err)?;
            authority |= gone > 0 && (table == "device_caps" || table == "step_up_keys");
        }
        if let (Some(person), true) = (holder, authority) {
            bump_in(&tx, &person)?;
        }
        tx.commit().map_err(map_err)
    }

    /// A member's access, and what it was (`None` when they had none). Never
    /// the owner's: the owner is the Unix account. Bumps their generation.
    pub fn set_member_access(&self, handle: &str, access: Access) -> Result<Option<Access>> {
        if access == Access::Owner {
            return Err(DomainError::InvalidArgument { what: "access" });
        }
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let known: bool =
            tx.query_row("SELECT EXISTS(SELECT 1 FROM members WHERE handle = ?1)", [handle], |r| r.get(0)).map_err(map_err)?;
        if !known {
            return Err(DomainError::NotFound);
        }
        let before: Option<String> =
            tx.query_row("SELECT access FROM member_access WHERE handle = ?1", [handle], |r| r.get(0)).optional().map_err(map_err)?;
        tx.execute(
            "INSERT INTO member_access (handle, access) VALUES (?1, ?2) ON CONFLICT(handle) DO UPDATE SET access = excluded.access",
            params![handle, access.as_str()],
        )
        .map_err(map_err)?;
        bump_in(&tx, handle)?;
        tx.commit().map_err(map_err)?;
        Ok(before.as_deref().and_then(Access::from_stored))
    }

    /// Give `handle` `access` only if they have none yet: a member added after
    /// the migration. Reports whether it wrote.
    pub fn ensure_member_access(&self, handle: &str, access: Access) -> Result<bool> {
        if access == Access::Owner {
            return Err(DomainError::InvalidArgument { what: "access" });
        }
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let wrote = tx
            .execute(
                "INSERT OR IGNORE INTO member_access (handle, access) SELECT ?1, ?2 WHERE EXISTS (SELECT 1 FROM members WHERE handle = ?1)",
                params![handle, access.as_str()],
            )
            .map_err(map_err)?
            > 0;
        tx.commit().map_err(map_err)?;
        Ok(wrote)
    }

    /// A device's cap, `None` to clear it, answering what it was. Bumps the
    /// generation of whoever the device is bound to.
    pub fn set_device_cap(&self, device: &str, cap: Option<Cap>) -> Result<Option<Cap>> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let before: Option<String> =
            tx.query_row("SELECT cap FROM device_caps WHERE client_id = ?1", [device], |r| r.get(0)).optional().map_err(map_err)?;
        match cap {
            Some(cap) => tx
                .execute(
                    "INSERT INTO device_caps (client_id, cap) VALUES (?1, ?2) ON CONFLICT(client_id) DO UPDATE SET cap = excluded.cap",
                    params![device, cap.as_str()],
                )
                .map_err(map_err)?,
            None => tx.execute("DELETE FROM device_caps WHERE client_id = ?1", [device]).map_err(map_err)?,
        };
        if let Some(person) = holder_in(&tx, device)? {
            bump_in(&tx, &person)?;
        }
        tx.commit().map_err(map_err)?;
        Ok(before.as_deref().and_then(Cap::from_stored))
    }

    /// Every released device id.
    pub fn released_all(&self) -> Result<BTreeSet<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT client_id FROM released_devices").map_err(map_err)?;
        let rows = stmt.query_map([], |r| r.get(0)).map_err(map_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(map_err)
    }

    /// Bring the tables in line with the lines on a later start (the slice 1
    /// review's SF1), from the `plan` the daemon worked out under the fence
    /// lock. Adds what is missing and drops what no line backs; never changes
    /// a row that has a say already: an access, a cap or a state a later
    /// decision wrote. `live` is every Key A device with a line. A cap on a
    /// device whose person changed stays: only a person's cap lowers a line.
    pub fn refresh_access(&self, plan: &AccessPlan, live: &BTreeSet<String>) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let owners: BTreeSet<&str> = plan.owner_devices.iter().map(|(d, _)| d.as_str()).collect();
        let stale: Vec<String> = {
            let mut stmt = tx.prepare("SELECT client_id FROM owner_devices").map_err(map_err)?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(map_err)?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)?
        };
        for device in stale.iter().filter(|d| !owners.contains(d.as_str())) {
            tx.execute("DELETE FROM owner_devices WHERE client_id = ?1", [device]).map_err(map_err)?;
        }
        for (device, state) in &plan.owner_devices {
            let state = if *state == OwnerDeviceState::Check { "check" } else { "confirmed" };
            tx.execute("INSERT OR IGNORE INTO owner_devices (client_id, state) VALUES (?1, ?2)", params![device, state]).map_err(map_err)?;
        }
        for (handle, access) in &plan.member_access {
            tx.execute("INSERT OR IGNORE INTO member_access (handle, access) VALUES (?1, ?2)", params![handle, access.as_str()])
                .map_err(map_err)?;
        }
        let capped: Vec<String> = {
            let mut stmt = tx.prepare("SELECT client_id FROM device_caps").map_err(map_err)?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(map_err)?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)?
        };
        for device in capped.iter().filter(|d| !live.contains(d.as_str())) {
            tx.execute("DELETE FROM device_caps WHERE client_id = ?1", [device]).map_err(map_err)?;
        }
        for (device, cap) in &plan.caps {
            tx.execute("INSERT OR IGNORE INTO device_caps (client_id, cap) VALUES (?1, ?2)", params![device, cap.as_str()]).map_err(map_err)?;
        }
        tx.commit().map_err(map_err)
    }
}

#[cfg(test)]
#[path = "access_ops_tests.rs"]
mod tests;
