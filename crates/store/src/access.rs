//! Access follows the person (ov-856, slice 1 of `reports/ov-856/design.md`):
//! the records only. Nothing here is read by the runner's enforcement yet, so
//! a device's line and its calls are what they were.
//!
//! - **`owner_devices`** is the positive binding for the runner's owner: the
//!   devices that read as the owner's today (bound to no member, not
//!   released). `state` is `confirmed`, or `check` for a legacy teammate's
//!   line (R3): unbound, but added by a member's or a released device, which
//!   People lists under "Check These Devices" until the owner confirms it.
//! - **`member_access`** is a member's access: `admin`, `member` or
//!   `viewer`. The owner has none: the owner is the Unix account.
//! - **`device_caps`** only ever lowers a device below its person's access:
//!   `viewer` (line `read`) or `member` (line `control`). The migration
//!   writes one for every device whose line is below its person's ceiling,
//!   the owner's included, so the person's access and the cap together
//!   reproduce every line (B1).
//! - **`step_up_keys`** holds the keys a device signs admin actions with,
//!   `trusted` or `proposed`. Empty until the step-up slice.
//!
//! The migration into them ran once (`meta.access_migrated`), under the
//! fence lock, in the daemon (`access_migration`): the lines are where the
//! per-device scopes live.

use rusqlite::{OptionalExtension, params};

use crate::error::map_err;
use crate::store::Store;
use farcooler_core::{DomainError, Result};

/// Four new tables, none of them keyed into a table old code writes except
/// `member_access`, whose rows go with their member by cascade.
///
/// `Older::Welcome`: a build from before it never names them, and enforces
/// per device as it always did.
pub(crate) fn migration_0066_person_access(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        "CREATE TABLE owner_devices (
             client_id TEXT PRIMARY KEY NOT NULL,
             state TEXT NOT NULL DEFAULT 'confirmed' CHECK (state IN ('confirmed', 'check'))
         );
         CREATE TABLE member_access (
             handle TEXT PRIMARY KEY NOT NULL REFERENCES members(handle) ON DELETE CASCADE,
             access TEXT NOT NULL CHECK (access IN ('admin', 'member', 'viewer'))
         );
         CREATE TABLE device_caps (
             client_id TEXT PRIMARY KEY NOT NULL,
             cap TEXT NOT NULL CHECK (cap IN ('viewer', 'member'))
         );
         CREATE TABLE step_up_keys (
             client_id TEXT PRIMARY KEY NOT NULL,
             alg TEXT NOT NULL,
             pubkey BLOB NOT NULL,
             state TEXT NOT NULL CHECK (state IN ('trusted', 'proposed')),
             registered_at INTEGER NOT NULL,
             sign_count INTEGER NOT NULL DEFAULT 0
         );",
    )
}

/// What a person may do on this runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Access {
    Viewer,
    Member,
    Admin,
    Owner,
}

impl Access {
    pub fn as_str(self) -> &'static str {
        match self {
            Access::Viewer => "viewer",
            Access::Member => "member",
            Access::Admin => "admin",
            Access::Owner => "owner",
        }
    }

    /// The access a wire word names. Never `owner`: that is not a word
    /// anyone can be given.
    pub fn parse(word: &str) -> Option<Access> {
        Access::from_stored(word)
    }

    pub(crate) fn from_stored(word: &str) -> Option<Access> {
        Some(match word {
            "viewer" => Access::Viewer,
            "member" => Access::Member,
            "admin" => Access::Admin,
            _ => return None,
        })
    }
}

/// A per-device ceiling below its person's access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cap {
    Viewer,
    Member,
}

impl Cap {
    pub fn as_str(self) -> &'static str {
        match self {
            Cap::Viewer => "viewer",
            Cap::Member => "member",
        }
    }

    /// The cap a wire word names.
    pub fn parse(word: &str) -> Option<Cap> {
        Cap::from_stored(word)
    }

    pub(crate) fn from_stored(word: &str) -> Option<Cap> {
        match word {
            "viewer" => Some(Cap::Viewer),
            "member" => Some(Cap::Member),
            _ => None,
        }
    }
}

/// Whether an owner device is settled or waits on the owner's say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerDeviceState {
    Confirmed,
    Check,
}

/// What the one-time migration writes, worked out by the daemon from the
/// lines and the store.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccessPlan {
    pub owner_devices: Vec<(String, OwnerDeviceState)>,
    pub member_access: Vec<(String, Access)>,
    pub caps: Vec<(String, Cap)>,
}

/// One row of the read-only People view's data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAccess {
    pub owner_devices: Vec<(String, OwnerDeviceState)>,
    pub member_access: Vec<(String, Access)>,
    pub caps: Vec<(String, Cap)>,
    /// `(client_id, state)` of every step-up key.
    pub step_up_keys: Vec<(String, String)>,
}

const MIGRATED: &str = "access_migrated";

impl Store {
    /// Whether the access migration has run.
    pub fn access_migrated(&self) -> Result<bool> {
        let conn = self.conn();
        crate::migrate::read_meta_u32(&conn, MIGRATED).map(|v| v.is_some())
    }

    /// Write `plan` and mark the migration done, in one transaction. A no-op
    /// answering `false` when it already ran, so a restart never overwrites
    /// what a later change decided.
    pub fn apply_access_plan(&self, plan: &AccessPlan) -> Result<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        if crate::migrate::read_meta_u32(&tx, MIGRATED)?.is_some() {
            return Ok(false);
        }
        // The first run derives the tables from the lines alone (the slice 1
        // review's SF1): a binding written before it, by an enroll of this
        // build, is re-derived rather than trusted.
        for table in ["owner_devices", "member_access", "device_caps"] {
            tx.execute(&format!("DELETE FROM {table}"), []).map_err(map_err)?;
        }
        for (device, state) in &plan.owner_devices {
            let state = if *state == OwnerDeviceState::Check { "check" } else { "confirmed" };
            tx.execute("INSERT OR IGNORE INTO owner_devices (client_id, state) VALUES (?1, ?2)", params![device, state])
                .map_err(map_err)?;
        }
        for (handle, access) in &plan.member_access {
            if *access == Access::Owner {
                return Err(DomainError::InvalidArgument { what: "access" });
            }
            tx.execute("INSERT OR IGNORE INTO member_access (handle, access) VALUES (?1, ?2)", params![handle, access.as_str()])
                .map_err(map_err)?;
        }
        for (device, cap) in &plan.caps {
            tx.execute("INSERT OR IGNORE INTO device_caps (client_id, cap) VALUES (?1, ?2)", params![device, cap.as_str()])
                .map_err(map_err)?;
        }
        tx.execute("INSERT INTO meta (key, value) VALUES (?1, '1')", [MIGRATED]).map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        Ok(true)
    }

    /// `handle`'s stored access, or `None` when they have none (a member
    /// from before the migration ran).
    pub fn member_access(&self, handle: &str) -> Result<Option<Access>> {
        let stored: Option<String> = self
            .conn()
            .query_row("SELECT access FROM member_access WHERE handle = ?1", [handle], |r| r.get(0))
            .optional()
            .map_err(map_err)?;
        Ok(stored.as_deref().and_then(Access::from_stored))
    }

    /// `device`'s stored cap, if it has one.
    pub fn device_cap(&self, device: &str) -> Result<Option<Cap>> {
        let stored: Option<String> =
            self.conn().query_row("SELECT cap FROM device_caps WHERE client_id = ?1", [device], |r| r.get(0)).optional().map_err(map_err)?;
        Ok(stored.as_deref().and_then(Cap::from_stored))
    }

    /// Everything the access tables hold, sorted.
    pub fn stored_access(&self) -> Result<StoredAccess> {
        let conn = self.conn();
        let pairs = |sql: &str| -> Result<Vec<(String, String)>> {
            let mut stmt = conn.prepare(sql).map_err(map_err)?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(map_err)?;
            rows.collect::<rusqlite::Result<_>>().map_err(map_err)
        };
        Ok(StoredAccess {
            owner_devices: pairs("SELECT client_id, state FROM owner_devices ORDER BY client_id")?
                .into_iter()
                .map(|(d, s)| (d, if s == "check" { OwnerDeviceState::Check } else { OwnerDeviceState::Confirmed }))
                .collect(),
            member_access: pairs("SELECT handle, access FROM member_access ORDER BY handle")?
                .into_iter()
                .filter_map(|(h, a)| Some((h, Access::from_stored(&a)?)))
                .collect(),
            caps: pairs("SELECT client_id, cap FROM device_caps ORDER BY client_id")?
                .into_iter()
                .filter_map(|(d, c)| Some((d, Cap::from_stored(&c)?)))
                .collect(),
            step_up_keys: pairs("SELECT client_id, state FROM step_up_keys ORDER BY client_id")?,
        })
    }
}

#[cfg(test)]
#[path = "access_tests.rs"]
mod tests;
