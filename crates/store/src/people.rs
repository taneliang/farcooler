//! The people on a shared runner, each workspace's lead, and their
//! representatives (ov-494, phase 1 of `reports/ov-479/design.md`).
//!
//! - **A member** is a person who isn't the runner's owner: a handle, the name
//!   people read, and the enrolled devices (`client_id`s, from sshd's forced
//!   command) that are theirs. A device is one member's at most. The owner is
//!   nobody's row: the local socket, and any device never bound to anybody, is
//!   the owner, as every device the ceremony enrolls is today.
//! - **A released device** was a member's and isn't now: they were removed,
//!   or it was dropped from their list. It is nobody's, and never the owner's:
//!   removing someone must not give their phone the owner's say. Binding it
//!   to a member again takes it off this list.
//! - **A workspace's lead** (R-49) is the owner until a runner admin hands it
//!   to a member. One row per workspace at most, so there is exactly one lead:
//!   that member, or with no row, the owner. Removing the member gives the
//!   lead back to the owner, by cascade.
//! - **A rep** is the pane the runner bound to one member in one workspace,
//!   at most one each. Its messages are filed on the member's desk card in
//!   that workspace, which outlives any one rep pane.
//!
//! Who a caller is, is the daemon's to work out from its connection; this
//! module only keeps the records.

use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use farcooler_core::{DomainError, Result};

use crate::error::map_err;
use crate::models::{get_uuid, uuid_blob};
use crate::store::Store;
use crate::tasks::now_millis;

/// Six new tables, so a build from before them reads every table it knows
/// exactly as it did. One that runs meanwhile knows no members, leads or
/// reps: everyone is the owner, as before.
pub(crate) fn migration_0046_people(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        CREATE TABLE members (
            handle TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE member_devices (
            client_id TEXT PRIMARY KEY NOT NULL,
            handle TEXT NOT NULL REFERENCES members(handle) ON DELETE CASCADE
        );
        CREATE TABLE released_devices (
            client_id TEXT PRIMARY KEY NOT NULL
        );
        CREATE TABLE workspace_leads (
            workspace_id BLOB PRIMARY KEY NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            handle TEXT NOT NULL REFERENCES members(handle) ON DELETE CASCADE
        );
        CREATE TABLE rep_desks (
            workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            handle TEXT NOT NULL REFERENCES members(handle) ON DELETE CASCADE,
            task_id BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            PRIMARY KEY (workspace_id, handle)
        );
        CREATE TABLE reps (
            terminal_id BLOB PRIMARY KEY NOT NULL REFERENCES terminals(id) ON DELETE CASCADE,
            workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            handle TEXT NOT NULL REFERENCES members(handle) ON DELETE CASCADE,
            UNIQUE (workspace_id, handle)
        );
        "#,
    )
}

/// Two nullable columns on 0046's own `members` (ov-525): the team
/// directory's account id a member was added under by QR, and the email it
/// had then, plus an index that keeps one account to one member.
///
/// `Older::Welcome`: a build from before it never names either column, so it
/// reads members as it did, and every member it adds has neither, which the
/// index (only over rows that have one) never refuses.
pub(crate) fn migration_0050_member_accounts(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        ALTER TABLE members ADD COLUMN account TEXT NULL;
        ALTER TABLE members ADD COLUMN email TEXT NULL;
        CREATE UNIQUE INDEX members_by_account ON members(account) WHERE account IS NOT NULL;
        "#,
    )
}

/// [`Store::remove_member`]'s writes, in the caller's transaction.
pub(crate) fn remove_member_in(tx: &rusqlite::Transaction, handle: &str) -> Result<()> {
    // Their devices are no longer waiting on a first binding, whatever an
    // ov-719 build recorded: released now, they're swept (ov-783).
    tx.execute(
        "DELETE FROM device_enrollers WHERE client_id IN (SELECT client_id FROM member_devices WHERE handle = ?1)",
        [handle],
    )
    .map_err(map_err)?;
    tx.execute(
        "INSERT OR IGNORE INTO released_devices (client_id) SELECT client_id FROM member_devices WHERE handle = ?1",
        [handle],
    )
    .map_err(map_err)?;
    let removed = tx.execute("DELETE FROM members WHERE handle = ?1", [handle]).map_err(map_err)?;
    if removed == 0 {
        return Err(DomainError::NotFound);
    }
    Ok(())
}

/// One person on this runner who isn't its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// `alice`: what commands and `rep:<handle>` name them by.
    pub handle: String,
    /// "Alice Chen": what people read.
    pub name: String,
    /// Their enrolled devices, sorted.
    pub devices: Vec<String>,
    /// The team directory's account id they were added under (ov-525), or
    /// `None` for a member added by hand. What offboarding will find them by.
    pub account: Option<String>,
    /// That account's email when they were added: what People shows. A
    /// record of what the directory said, not a login.
    pub email: Option<String>,
}

/// Who a member is in the team directory, as `member.set` is told it. An
/// empty field leaves what's stored as it is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Directory<'a> {
    pub account: &'a str,
    pub email: &'a str,
}

/// What [`Store::set_member_releasing`] did: the member as written, the
/// devices it took off their list, and the devices they had before it, or
/// `None` when they're new.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberWrite {
    pub member: Member,
    pub released: Vec<String>,
    pub before: Option<Vec<String>>,
}

/// A rep pane: the member the runner bound it to, in a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rep {
    pub terminal: Uuid,
    pub workspace: Uuid,
    pub handle: String,
}

/// Handles the runner keeps for itself: `owner` names the owner in `workspace
/// lead`, and `orchestrator` is a message's other address.
const RESERVED: &[&str] = &["owner", "orchestrator", "rep", "runner", "manager", "user"];

/// A handle: 1 to 32 of lowercase ASCII letters, digits and `-`, starting
/// with a letter, and none the runner keeps (`RESERVED`).
pub fn valid_handle(handle: &str) -> bool {
    let mut chars = handle.chars();
    matches!(chars.next(), Some('a'..='z'))
        && handle.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !RESERVED.contains(&handle)
}

/// Whether `name` would read as one of the runner's own message tags
/// (`[from the orchestrator]`, `[from Alice's rep]`), which only the runner
/// may write.
fn mimics_a_tag(name: &str) -> bool {
    let lower = name.to_lowercase().replace('\u{2019}', "'");
    ["the orchestrator", "orchestrator", "the terminal", "far cooler", "an agent", "the runner"].contains(&lower.as_str())
        || lower.ends_with("'s rep")
}

impl Store {
    /// Add `handle`, or rename them, with exactly `devices` as theirs. Refused
    /// as `handle` for one `valid_handle` refuses, one for an empty,
    /// multi-line or bracketed one (`member_name`) (a message is tagged `[from <name>]`), and
    /// `device_taken` for a device another member holds, writing nothing.
    pub fn set_member(&self, handle: &str, name: &str, devices: &[String]) -> Result<Member> {
        self.set_member_in(handle, name, devices, Directory::default())
    }

    /// [`Store::set_member`], also recording who they are in the team
    /// directory (ov-525). An account is one member's at most, and a member's
    /// account never changes once set: refused as `account_taken` and
    /// `account_differs`, writing nothing. An empty field keeps what's there.
    pub fn set_member_in(&self, handle: &str, name: &str, devices: &[String], who: Directory) -> Result<Member> {
        self.set_member_releasing(handle, name, devices, who, None).map(|w| w.member)
    }

    /// [`Store::set_member_in`], answering too with the devices this call
    /// released, and the devices they had before it (`None` for someone new),
    /// both read in the same transaction, so a concurrent change to their list
    /// can't add one or hide one (ov-783). Sorted. The caller revokes them.
    ///
    /// With `read`, the devices the caller read as theirs, it's refused as
    /// `devices_changed`, writing nothing, unless their devices are exactly
    /// those, as a set, in this transaction (ov-786): a list built from an
    /// older read would release a device added since. A member not here yet
    /// has none. Without it, the list is replaced whatever it was.
    pub fn set_member_releasing(
        &self,
        handle: &str,
        name: &str,
        devices: &[String],
        who: Directory,
        read: Option<&[String]>,
    ) -> Result<MemberWrite> {
        let (account, email) = (who.account.trim(), who.email.trim());
        // Printable and short: both are only ever compared and shown.
        let printable = |s: &str, max: usize| s.len() <= max && !s.chars().any(|c| c.is_control() || c.is_whitespace());
        if !printable(account, 128) {
            return Err(DomainError::InvalidArgument { what: "member_account" });
        }
        if !printable(email, 254) || !(email.is_empty() || email.contains('@')) {
            return Err(DomainError::InvalidArgument { what: "member_email" });
        }
        if !valid_handle(handle) {
            return Err(DomainError::InvalidArgument { what: "handle" });
        }
        let name = name.trim();
        if name.is_empty()
            || name.chars().count() > 64
            || name.chars().any(|c| c.is_control() || c == '[' || c == ']')
            || mimics_a_tag(name)
        {
            return Err(DomainError::InvalidArgument { what: "member_name" });
        }
        let mut devices: Vec<String> = devices.iter().map(|d| d.trim().to_string()).filter(|d| !d.is_empty()).collect();
        devices.sort();
        devices.dedup();
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let had: Vec<String> = {
            let mut stmt = tx.prepare("SELECT client_id FROM member_devices WHERE handle = ?1 ORDER BY client_id").map_err(map_err)?;
            let rows = stmt.query_map([handle], |r| r.get(0)).map_err(map_err)?;
            rows.collect::<rusqlite::Result<_>>().map_err(map_err)?
        };
        let existed: bool = tx
            .query_row("SELECT EXISTS(SELECT 1 FROM members WHERE handle = ?1)", [handle], |r| r.get(0))
            .map_err(map_err)?;
        if let Some(read) = read {
            // Both sides trimmed, sorted and without repeats: compared as sets.
            let set = |ids: &mut Vec<&str>| {
                ids.retain(|d| !d.is_empty());
                ids.sort_unstable();
                ids.dedup();
            };
            let mut read: Vec<&str> = read.iter().map(|d| d.trim()).collect();
            let mut stored: Vec<&str> = had.iter().map(|d| d.trim()).collect();
            set(&mut read);
            set(&mut stored);
            if read != stored {
                return Err(DomainError::Conflict { what: "devices_changed" });
            }
        }
        for device in &devices {
            let holder: Option<String> = tx
                .query_row("SELECT handle FROM member_devices WHERE client_id = ?1", [device], |r| r.get(0))
                .optional()
                .map_err(map_err)?;
            if holder.is_some_and(|h| h != handle) {
                return Err(DomainError::Conflict { what: "device_taken" });
            }
        }
        if !account.is_empty() {
            let holder: Option<String> = tx
                .query_row("SELECT handle FROM members WHERE account = ?1", [account], |r| r.get(0))
                .optional()
                .map_err(map_err)?;
            if holder.is_some_and(|h| h != handle) {
                return Err(DomainError::Conflict { what: "account_taken" });
            }
            let theirs: Option<Option<String>> = tx
                .query_row("SELECT account FROM members WHERE handle = ?1", [handle], |r| r.get(0))
                .optional()
                .map_err(map_err)?;
            if theirs.flatten().is_some_and(|a| a != account) {
                return Err(DomainError::Conflict { what: "account_differs" });
            }
        }
        let some = |s: &str| (!s.is_empty()).then(|| s.to_string());
        tx.execute(
            "INSERT INTO members (handle, name, created_at, account, email) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(handle) DO UPDATE SET name = excluded.name,
                 account = COALESCE(excluded.account, members.account),
                 email = COALESCE(excluded.email, members.email)",
            params![handle, name, now_millis(), some(account), some(email)],
        )
        .map_err(map_err)?;
        let released: Vec<String> = had.iter().filter(|d| !devices.contains(d)).cloned().collect();
        // A device bound to a member, or taken off one, is no longer waiting
        // on its first binding (ov-719): who enrolled it stops mattering, and
        // released again it's one `released_to_sweep` names (ov-783).
        for device in devices.iter().chain(&released) {
            tx.execute("DELETE FROM device_enrollers WHERE client_id = ?1", [device]).map_err(map_err)?;
            tx.execute("DELETE FROM pending_teammates WHERE client_id = ?1", [device]).map_err(map_err)?;
        }
        // A device left off the list is released, never the owner's again.
        tx.execute(
            "INSERT OR IGNORE INTO released_devices (client_id) SELECT client_id FROM member_devices WHERE handle = ?1",
            [handle],
        )
        .map_err(map_err)?;
        tx.execute("DELETE FROM member_devices WHERE handle = ?1", [handle]).map_err(map_err)?;
        for device in &devices {
            tx.execute("INSERT INTO member_devices (client_id, handle) VALUES (?1, ?2)", params![device, handle])
                .map_err(map_err)?;
            tx.execute("DELETE FROM released_devices WHERE client_id = ?1", [device]).map_err(map_err)?;
        }
        // What a device newly theirs added while it read as the owner's is
        // theirs to answer for too (ov-838, the ov-828 re-review's R3): a
        // teammate's line from before ov-828 could grant as the owner.
        for device in devices.iter().filter(|d| !had.contains(d)) {
            crate::device_added_by::hand_over_added_in(&tx, device, handle)?;
        }
        let (account, email): (Option<String>, Option<String>) = tx
            .query_row("SELECT account, email FROM members WHERE handle = ?1", [handle], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        let member = Member { handle: handle.to_string(), name: name.to_string(), devices, account, email };
        Ok(MemberWrite { member, released, before: existed.then_some(had) })
    }

    /// Remove `handle`: their devices are released (nobody's, never the
    /// owner's), a workspace they led is led by the owner, and their desks and
    /// rep rows go. `NotFound` for nobody by that handle. Stopping their rep
    /// panes is the caller's.
    pub fn remove_member(&self, handle: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        remove_member_in(&tx, handle)?;
        tx.commit().map_err(map_err)?;
        Ok(())
    }

    /// Whether `client_id` was a member's and is nobody's now.
    pub fn device_released(&self, client_id: &str) -> Result<bool> {
        self.conn()
            .query_row("SELECT EXISTS(SELECT 1 FROM released_devices WHERE client_id = ?1)", [client_id], |r| r.get(0))
            .map_err(map_err)
    }

    /// The released devices that must have no line and no session (ov-783):
    /// each was a member's and was taken off their list, or went with them.
    /// Not one a member's device enrolled that's waiting on its first binding
    /// (`record_member_enrolled`), nor a teammate's the owner is adding
    /// (`record_teammate_pending`, ov-828): released too, each keeps its line. And
    /// never one bound to a member, whatever an older build or a hand edit
    /// left in `released_devices`. Sorted.
    pub fn released_to_sweep(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT r.client_id FROM released_devices r
                 WHERE NOT EXISTS (SELECT 1 FROM device_enrollers e WHERE e.client_id = r.client_id)
                   AND NOT EXISTS (SELECT 1 FROM pending_teammates t WHERE t.client_id = r.client_id)
                   AND NOT EXISTS (SELECT 1 FROM member_devices d WHERE d.client_id = r.client_id)
                 ORDER BY r.client_id",
            )
            .map_err(map_err)?;
        let rows = stmt.query_map([], |r| r.get(0)).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<String>>>().map_err(map_err)
    }

    /// Whether any member has a rep anywhere: the cheap check before working
    /// out which pane a local caller runs in.
    pub fn any_reps(&self) -> Result<bool> {
        self.conn().query_row("SELECT EXISTS(SELECT 1 FROM reps)", [], |r| r.get(0)).map_err(map_err)
    }

    /// Whether this runner has any member at all: the cheap check before
    /// refusing a caller no pane walk can place (ov-649).
    pub fn any_members(&self) -> Result<bool> {
        self.conn().query_row("SELECT EXISTS(SELECT 1 FROM members)", [], |r| r.get(0)).map_err(map_err)
    }

    /// Every member, by handle.
    pub fn members(&self) -> Result<Vec<Member>> {
        crate::read_counts::note_whole_table_read();
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT m.handle, m.name, d.client_id, m.account, m.email FROM members m
                 LEFT JOIN member_devices d ON d.handle = m.handle
                 ORDER BY m.handle, d.client_id",
            )
            .map_err(map_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(map_err)?;
        let mut members: Vec<Member> = Vec::new();
        for row in rows {
            let (handle, name, device, account, email) = row.map_err(map_err)?;
            if members.last().is_none_or(|m| m.handle != handle) {
                members.push(Member { handle, name, devices: Vec::new(), account, email });
            }
            if let (Some(device), Some(last)) = (device, members.last_mut()) {
                last.devices.push(device);
            }
        }
        Ok(members)
    }

    /// `handle`'s name alone, by one lookup; `None` for nobody. For a path
    /// that labels one thing and must not read every member.
    pub fn member_name(&self, handle: &str) -> Result<Option<String>> {
        self.conn()
            .query_row("SELECT name FROM members WHERE handle = ?1", [handle], |r| r.get(0))
            .optional()
            .map_err(map_err)
    }

    /// The member `handle` names. `NotFound` for nobody.
    pub fn member(&self, handle: &str) -> Result<Member> {
        self.members()?.into_iter().find(|m| m.handle == handle).ok_or(DomainError::NotFound)
    }

    /// Whose device `client_id` is: a member's handle, or `None` for the
    /// owner's.
    pub fn member_of_device(&self, client_id: &str) -> Result<Option<String>> {
        self.conn()
            .query_row("SELECT handle FROM member_devices WHERE client_id = ?1", [client_id], |r| r.get(0))
            .optional()
            .map_err(map_err)
    }

    /// The member who leads `workspace`, or `None` when the owner does.
    pub fn workspace_lead(&self, workspace: Uuid) -> Result<Option<String>> {
        self.conn()
            .query_row("SELECT handle FROM workspace_leads WHERE workspace_id = ?1", [uuid_blob(workspace)], |r| r.get(0))
            .optional()
            .map_err(map_err)
    }

    /// Hand `workspace`'s lead to `handle`, or back to the owner with `None`.
    /// `NotFound` for no such workspace or member.
    pub fn set_workspace_lead(&self, workspace: Uuid, handle: Option<&str>) -> Result<()> {
        self.get_workspace(workspace)?;
        let conn = self.conn();
        match handle {
            None => {
                conn.execute("DELETE FROM workspace_leads WHERE workspace_id = ?1", [uuid_blob(workspace)])
                    .map_err(map_err)?;
            }
            Some(handle) => {
                let known: bool = conn
                    .query_row("SELECT EXISTS(SELECT 1 FROM members WHERE handle = ?1)", [handle], |r| r.get(0))
                    .map_err(map_err)?;
                if !known {
                    return Err(DomainError::NotFound);
                }
                conn.execute(
                    "INSERT INTO workspace_leads (workspace_id, handle) VALUES (?1, ?2)
                     ON CONFLICT(workspace_id) DO UPDATE SET handle = excluded.handle",
                    params![uuid_blob(workspace), handle],
                )
                .map_err(map_err)?;
            }
        }
        Ok(())
    }

    /// `handle`'s desk card in `workspace`, if one was made.
    pub fn rep_desk(&self, workspace: Uuid, handle: &str) -> Result<Option<Uuid>> {
        self.conn()
            .query_row(
                "SELECT task_id FROM rep_desks WHERE workspace_id = ?1 AND handle = ?2",
                params![uuid_blob(workspace), handle],
                |r| get_uuid(r, 0),
            )
            .optional()
            .map_err(map_err)
    }

    /// Record `task` as `handle`'s desk card in `workspace`.
    pub fn set_rep_desk(&self, workspace: Uuid, handle: &str, task: Uuid) -> Result<()> {
        self.conn()
            .execute(
                "INSERT INTO rep_desks (workspace_id, handle, task_id) VALUES (?1, ?2, ?3)
                 ON CONFLICT(workspace_id, handle) DO UPDATE SET task_id = excluded.task_id",
                params![uuid_blob(workspace), handle, uuid_blob(task)],
            )
            .map_err(map_err)?;
        Ok(())
    }

    /// Bind `terminal` as `handle`'s rep in `workspace`. Refused as
    /// `rep_taken` when they have one there already: the caller removes a
    /// stopped one first.
    pub fn bind_rep(&self, terminal: Uuid, workspace: Uuid, handle: &str) -> Result<Rep> {
        let inserted = self.conn().execute(
            "INSERT INTO reps (terminal_id, workspace_id, handle) VALUES (?1, ?2, ?3)",
            params![uuid_blob(terminal), uuid_blob(workspace), handle],
        );
        match inserted {
            Ok(_) => Ok(Rep { terminal, workspace, handle: handle.to_string() }),
            Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ConstraintViolation => {
                Err(DomainError::Conflict { what: "rep_taken" })
            }
            Err(e) => Err(map_err(e)),
        }
    }

    /// The rep `terminal` is, if the runner bound it to someone.
    pub fn rep_of_terminal(&self, terminal: Uuid) -> Result<Option<Rep>> {
        self.reps_where("terminal_id = ?1", uuid_blob(terminal)).map(|mut r| r.pop())
    }

    /// `handle`'s rep in `workspace`, if they have one.
    pub fn rep_for(&self, workspace: Uuid, handle: &str) -> Result<Option<Rep>> {
        Ok(self.reps_in(workspace)?.into_iter().find(|r| r.handle == handle))
    }

    /// Every rep `handle` has, one per workspace.
    pub fn reps_of(&self, handle: &str) -> Result<Vec<Rep>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT terminal_id, workspace_id, handle FROM reps WHERE handle = ?1").map_err(map_err)?;
        let rows = stmt
            .query_map([handle], |r| Ok(Rep { terminal: get_uuid(r, 0)?, workspace: get_uuid(r, 1)?, handle: r.get(2)? }))
            .map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    /// Every rep on the runner: one read for a walk that asks which panes
    /// are reps (ov-744), not one per pane.
    pub fn reps(&self) -> Result<Vec<Rep>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT terminal_id, workspace_id, handle FROM reps ORDER BY handle").map_err(map_err)?;
        let rows = stmt
            .query_map([], |r| Ok(Rep { terminal: get_uuid(r, 0)?, workspace: get_uuid(r, 1)?, handle: r.get(2)? }))
            .map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    /// Every rep in `workspace`, by handle.
    pub fn reps_in(&self, workspace: Uuid) -> Result<Vec<Rep>> {
        self.reps_where("workspace_id = ?1", uuid_blob(workspace))
    }

    fn reps_where(&self, clause: &str, value: Vec<u8>) -> Result<Vec<Rep>> {
        let conn = self.conn();
        let sql = format!("SELECT terminal_id, workspace_id, handle FROM reps WHERE {clause} ORDER BY handle");
        let mut stmt = conn.prepare(&sql).map_err(map_err)?;
        let rows = stmt
            .query_map([value], |r| Ok(Rep { terminal: get_uuid(r, 0)?, workspace: get_uuid(r, 1)?, handle: r.get(2)? }))
            .map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }
}

#[cfg(test)]
#[path = "people_tests.rs"]
mod tests;
