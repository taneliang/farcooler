//! Whether a runner's members must sign in to each agent with their own
//! account (ov-652, the owner's decision of Oct 21).
//!
//! **Off by default, per harness.** With no row, a member's Claude, Codex or
//! Cursor pane may run on the runner's own login for that agent: its owner's
//! account, quota and history. A row that says `members_own` turns that off
//! for one harness, and a member's pane of it then needs their own login, as
//! every member's did before (ov-583, ov-652's Lane A).
//!
//! **Per runner, not per team.** One team can span runners owned by different
//! people, and the shared login is the owner's own account on this runner, so
//! only this runner's admins decide (the daemon's `member.set_own_login`,
//! `host_admin`). Each change keeps who made it and when, beside the log line
//! the daemon writes.
//!
//! In the store rather than config.toml: the launch checks read it on every
//! member launch, beside the logins they already read here, and config.toml
//! is per Unix user (every runner under one `HOME` reads the one file), is
//! edited by hand with no admin check, and keeps no record of who set it.

use rusqlite::{OptionalExtension, params};

use crate::error::map_err;
use crate::store::Store;
use farcooler_core::{DomainError, Result};

/// One new table, and one defaulted column on 0053's `terminal_logins`.
///
/// `member_login_policy`: a row per harness an admin has set, absent for
/// one never set (off).
///
/// `terminal_logins.shared`: 1 for a member's pane launched on the runner's
/// own login for its harness. The row still names the member and their
/// generation, so the pane is still theirs (who it speaks as, what it may
/// open, and that a removal stops it); only the credential is the runner's.
/// Its `config_dir` is still their directory's string, never passed.
///
/// A build from before reads neither: its member panes need their own login
/// again, and a shared pane it relaunches comes back on the member's own
/// directory, never on anyone else's.
pub(crate) fn migration_0057_member_login_policy(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        "CREATE TABLE member_login_policy (
             harness TEXT PRIMARY KEY NOT NULL,
             members_own INTEGER NOT NULL,
             set_at INTEGER NOT NULL,
             set_by TEXT NOT NULL
         );
         ALTER TABLE terminal_logins ADD COLUMN shared INTEGER NOT NULL DEFAULT 0;",
    )
}

/// The harnesses a policy row may name, as stored.
pub const HARNESSES: [&str; 3] = ["claude", "codex", "cursor"];

/// For each harness, whether members must sign in with their own account.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemberLoginPolicy {
    pub claude: bool,
    pub codex: bool,
    pub cursor: bool,
}

impl MemberLoginPolicy {
    /// What it says for `harness`; `None` for a harness it doesn't name.
    pub fn members_own(&self, harness: &str) -> Option<bool> {
        match harness {
            "claude" => Some(self.claude),
            "codex" => Some(self.codex),
            "cursor" => Some(self.cursor),
            _ => None,
        }
    }
}

impl Store {
    /// This runner's policy now: off for every harness never set.
    pub fn member_login_policy(&self) -> Result<MemberLoginPolicy> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT harness, members_own FROM member_login_policy").map_err(map_err)?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?))).map_err(map_err)?;
        let mut policy = MemberLoginPolicy::default();
        for row in rows {
            match row.map_err(map_err)? {
                (h, on) if h == "claude" => policy.claude = on,
                (h, on) if h == "codex" => policy.codex = on,
                (h, on) if h == "cursor" => policy.cursor = on,
                _ => {}
            }
        }
        Ok(policy)
    }

    /// Set whether members sign in to `harness` with their own account, as
    /// `by` (a device id, `-` for the runner's own socket) at `at`. `true`
    /// when that changed it. `invalid-argument: harness` for any other name.
    pub fn set_member_login_policy(&self, harness: &str, members_own: bool, by: &str, at: i64) -> Result<bool> {
        if !HARNESSES.contains(&harness) {
            return Err(DomainError::InvalidArgument { what: "harness" });
        }
        let before: Option<bool> = self
            .conn()
            .query_row("SELECT members_own FROM member_login_policy WHERE harness = ?1", [harness], |r| r.get(0))
            .optional()
            .map_err(map_err)?;
        self.conn()
            .execute(
                "INSERT INTO member_login_policy (harness, members_own, set_at, set_by) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(harness) DO UPDATE SET members_own = ?2, set_at = ?3, set_by = ?4",
                params![harness, members_own, at, by],
            )
            .map_err(map_err)?;
        Ok(before.unwrap_or(false) != members_own)
    }

    /// Who last set `harness`'s policy, and when: `None` for one never set.
    pub fn member_login_policy_set(&self, harness: &str) -> Result<Option<(String, i64)>> {
        self.conn()
            .query_row("SELECT set_by, set_at FROM member_login_policy WHERE harness = ?1", [harness], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .map_err(map_err)
    }
}

#[cfg(test)]
#[path = "member_login_policy_tests.rs"]
mod tests;
