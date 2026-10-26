//! A member's role on the runner, and the people log (ov-757, slice 1 of
//! `reports/ov-753/design.md`).
//!
//! - **A role** is Member, everyone's by default, or Maintainer, which the
//!   owner gives someone trusted with the workspaces' rules: a Maintainer
//!   edits any workspace's charter, renames a workspace, sets its prefix,
//!   and sets the settings that don't act as the owner (ov-784). A runner
//!   where nobody is a Maintainer behaves exactly as before. The owner and
//!   each workspace's lead are not roles: the owner is the runner's own
//!   account, and the lead is `workspace_leads`.
//! - **The people log** is who changed what about the people on the runner,
//!   and when: a role given or taken, a member added, their devices changed,
//!   a member removed, a workspace's charter edited through the runner, a
//!   workspace renamed or re-prefixed, or its lead handed on (ov-784). One
//!   row per change, written in the change's own transaction where the
//!   change is the store's, so a change is never made without its row.
//!
//! Who may change a role is the daemon's (`people::gate`): the owner alone.

use std::collections::HashMap;

use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use farcooler_core::{DomainError, Result};

use crate::error::map_err;
use crate::models::{get_uuid, uuid_blob};
use crate::store::Store;
use crate::tasks::now_millis;

/// A defaulted column on 0046's `members`, and one new table.
///
/// `Older::Welcome`: a build from before it never names either. It adds a
/// member as `member`, the default, keeps a role when it renames someone
/// (its upsert names only the name, account and email), and reads every
/// member as it did: a Maintainer is a member to it, which is all a member
/// was, and it has no charter write for one to make.
pub(crate) fn migration_0060_member_roles(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        ALTER TABLE members ADD COLUMN role TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('member', 'maintainer'));
        CREATE TABLE people_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            at INTEGER NOT NULL,
            -- Who made the change: `user` for the owner, `member:<handle>`.
            actor TEXT NOT NULL,
            -- Whom it's about; empty for a charter edit.
            handle TEXT NOT NULL,
            -- The workspace, for a charter edit. No key into workspaces: the
            -- record outlives a workspace deleted.
            workspace_id BLOB,
            kind TEXT NOT NULL,
            detail TEXT NOT NULL DEFAULT '{}'
        );
        "#,
    )
}

/// What a member may do beyond what every member may.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Role {
    /// Every member, by default: today's rulings, unchanged.
    #[default]
    Member,
    /// Edits any workspace's charter (`workspace.set_charter`), renames a
    /// workspace and sets its prefix and the settings that don't act as the
    /// owner (ov-784).
    Maintainer,
}

impl Role {
    /// The word stored and sent: `member` or `maintainer`.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Member => "member",
            Role::Maintainer => "maintainer",
        }
    }

    /// The role `word` names, or `None` for none.
    pub fn parse(word: &str) -> Option<Role> {
        match word {
            "member" => Some(Role::Member),
            "maintainer" => Some(Role::Maintainer),
            _ => None,
        }
    }
}

/// One line of the people log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeopleEvent {
    pub id: i64,
    pub at: i64,
    pub actor: String,
    pub handle: String,
    pub workspace: Option<Uuid>,
    /// `role`, `added`, `devices`, `removed`, `revoked`, `revoke_failed`,
    /// `charter`, `rename`, `prefix` or `lead`.
    pub kind: String,
    /// A JSON object: a role change's, rename's, prefix's or lead's `from`
    /// and `to`, a device change's `devices`, a charter edit's `sha256`.
    pub detail: String,
}

/// A change to the people log, as its writer gives it.
#[derive(Debug, Clone, Copy)]
pub struct NewPeopleEvent<'a> {
    pub actor: &'a str,
    pub handle: &'a str,
    pub workspace: Option<Uuid>,
    pub kind: &'a str,
    pub detail: &'a serde_json::Value,
}

fn insert_event(conn: &rusqlite::Connection, e: NewPeopleEvent) -> Result<()> {
    conn.execute(
        "INSERT INTO people_events (at, actor, handle, workspace_id, kind, detail) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![now_millis(), e.actor, e.handle, e.workspace.map(uuid_blob), e.kind, e.detail.to_string()],
    )
    .map_err(map_err)?;
    Ok(())
}

impl Store {
    /// `handle`'s role. `NotFound` for nobody by that handle.
    pub fn member_role(&self, handle: &str) -> Result<Role> {
        let word: Option<String> = self
            .conn()
            .query_row("SELECT role FROM members WHERE handle = ?1", [handle], |r| r.get(0))
            .optional()
            .map_err(map_err)?;
        Ok(stored(&word.ok_or(DomainError::NotFound)?))
    }

    /// Every member's role, by handle.
    pub fn member_roles(&self) -> Result<HashMap<String, Role>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT handle, role FROM members").map_err(map_err)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(map_err)?;
        rows.map(|row| row.map(|(handle, word)| (handle, stored(&word))).map_err(map_err)).collect()
    }

    /// Give `handle` `role`, written to the people log as `actor`'s in the
    /// same transaction. Answers the role they had. Setting the role they
    /// have writes nothing. `NotFound` for nobody by that handle.
    pub fn set_member_role(&self, handle: &str, role: Role, actor: &str) -> Result<Role> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let had: Option<String> = tx
            .query_row("SELECT role FROM members WHERE handle = ?1", [handle], |r| r.get(0))
            .optional()
            .map_err(map_err)?;
        let had = stored(&had.ok_or(DomainError::NotFound)?);
        if had == role {
            return Ok(had);
        }
        tx.execute("UPDATE members SET role = ?1 WHERE handle = ?2", params![role.as_str(), handle]).map_err(map_err)?;
        let detail = serde_json::json!({ "from": had.as_str(), "to": role.as_str() });
        insert_event(&tx, NewPeopleEvent { actor, handle, workspace: None, kind: "role", detail: &detail })?;
        tx.commit().map_err(map_err)?;
        Ok(had)
    }

    /// Add a line to the people log, for a change made elsewhere.
    pub fn record_people_event(&self, event: NewPeopleEvent) -> Result<()> {
        insert_event(&self.conn(), event)
    }

    /// The people log, newest first, `limit` lines at most.
    pub fn people_events(&self, limit: u32) -> Result<Vec<PeopleEvent>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT id, at, actor, handle, workspace_id, kind, detail FROM people_events
                 ORDER BY id DESC LIMIT ?1",
            )
            .map_err(map_err)?;
        let rows = stmt
            .query_map([limit], |r| {
                let workspace = match r.get_ref(4)? {
                    rusqlite::types::ValueRef::Null => None,
                    _ => Some(get_uuid(r, 4)?),
                };
                Ok(PeopleEvent {
                    id: r.get(0)?,
                    at: r.get(1)?,
                    actor: r.get(2)?,
                    handle: r.get(3)?,
                    workspace,
                    kind: r.get(5)?,
                    detail: r.get(6)?,
                })
            })
            .map_err(map_err)?;
        rows.map(|row| row.map_err(map_err)).collect()
    }
}

/// A stored role word as a `Role`: the column's check allows only the two,
/// so anything else is read as the lesser.
fn stored(word: &str) -> Role {
    Role::parse(word).unwrap_or(Role::Member)
}

#[cfg(test)]
#[path = "member_roles_tests.rs"]
mod tests;
