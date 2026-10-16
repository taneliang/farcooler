//! Whose Claude login a pane runs on (ov-583, phase 1 of ov-581).
//!
//! A pane a member started, their rep, and an orchestrator whose lead is a
//! member run claude with `CLAUDE_CONFIG_DIR` set to that member's own config
//! directory. The daemon decides whose at the pane's first launch and writes
//! the member's handle and the exact directory string here, and every
//! relaunch reads them back, so a restart, a resume and a wake all come back
//! on the login the pane started on.
//!
//! **The string, not a path recomputed later** (ov-582): claude keys its
//! Keychain entry on the `CLAUDE_CONFIG_DIR` string as passed, so `/tmp/x`,
//! `/private/tmp/x` and `/private/tmp/x/` are three different logins. A
//! relaunch passes back exactly what the first launch passed.
//!
//! **A row is a member; no row is the runner's own login**, which is what
//! every pane from before this table used and every owner's pane still uses.
//!
//! In a table of its own rather than a column on `terminals`, so the wire's
//! `Terminal` never carries it (the design keeps the dir out of every answer),
//! and a build from before it reads every table it knows as it did.
//!
//! **A login is a member's generation, not their handle** (review 1, M2).
//! `member_logins` gives each member a login id the first time they launch,
//! and it goes with them when they're removed. Their directory is named by
//! it, and a pane records it, so a handle removed and given to someone else
//! gets a new id, a new directory and a new Keychain login, and the old
//! person's panes find their id gone (`login_gone`) instead of reviving on
//! the new person's.
//!
//! **No key from `terminal_logins` into `members`.** Removing a member must
//! not quietly turn their panes into the runner's: a cascade would delete the
//! row, and the pane's next restart would read "no row" as the owner's login.
//! The row outlives the member, and the daemon refuses to launch on it.

use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use crate::error::map_err;
use crate::models::uuid_blob;
use crate::store::Store;
use farcooler_core::{DomainError, Result};

/// Two new tables. `member_logins` rows go with their member, and
/// `terminal_logins` rows with their terminal, by cascade, whichever build
/// deletes them. A build from before them never names either: a pane it
/// relaunches comes back on the runner's login, as every pane did then.
pub(crate) fn migration_0053_terminal_logins(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        "CREATE TABLE member_logins (
             handle TEXT PRIMARY KEY NOT NULL REFERENCES members(handle) ON DELETE CASCADE,
             login_id TEXT NOT NULL UNIQUE
         );
         CREATE TABLE terminal_logins (
             terminal_id BLOB PRIMARY KEY NOT NULL REFERENCES terminals(id) ON DELETE CASCADE,
             handle TEXT NOT NULL,
             login_id TEXT NOT NULL,
             config_dir TEXT NOT NULL
         );",
    )
}

/// The login a pane records: whose, which generation of them, and the exact
/// directory string its first launch passed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalLogin {
    pub handle: String,
    pub login_id: String,
    pub config_dir: String,
}

impl Store {
    /// `handle`'s login id, made the first time it's asked for. `NotFound`
    /// for no such member.
    pub fn ensure_member_login(&self, handle: &str) -> Result<String> {
        let inserted = self.conn().execute(
            "INSERT OR IGNORE INTO member_logins (handle, login_id) VALUES (?1, ?2)",
            params![handle, Uuid::now_v7().simple().to_string()],
        );
        match inserted {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ConstraintViolation => {
                return Err(DomainError::NotFound);
            }
            Err(e) => return Err(map_err(e)),
        }
        self.member_login(handle)?.ok_or(DomainError::NotFound)
    }

    /// `handle`'s login id, if they have one now.
    pub fn member_login(&self, handle: &str) -> Result<Option<String>> {
        self.conn()
            .query_row("SELECT login_id FROM member_logins WHERE handle = ?1", [handle], |r| r.get(0))
            .optional()
            .map_err(map_err)
    }

    /// Record that `terminal` runs on `login`. Written once, before the
    /// pane's first launch (or an old rep's first relaunch); a second write
    /// for the same pane is refused, so nothing can move a pane to another
    /// person's login.
    pub fn set_terminal_login(&self, terminal: Uuid, login: &TerminalLogin) -> Result<()> {
        let inserted = self.conn().execute(
            "INSERT INTO terminal_logins (terminal_id, handle, login_id, config_dir) VALUES (?1, ?2, ?3, ?4)",
            params![uuid_blob(terminal), login.handle, login.login_id, login.config_dir],
        );
        match inserted {
            Ok(_) => Ok(()),
            Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ConstraintViolation => {
                Err(DomainError::Conflict { what: "login_taken" })
            }
            Err(e) => Err(map_err(e)),
        }
    }

    /// The login `terminal` records, or `None` for the runner's.
    pub fn terminal_login(&self, terminal: Uuid) -> Result<Option<TerminalLogin>> {
        self.conn()
            .query_row(
                "SELECT handle, login_id, config_dir FROM terminal_logins WHERE terminal_id = ?1",
                [uuid_blob(terminal)],
                |r| Ok(TerminalLogin { handle: r.get(0)?, login_id: r.get(1)?, config_dir: r.get(2)? }),
            )
            .optional()
            .map_err(map_err)
    }
}

#[cfg(test)]
mod tests {
    use super::TerminalLogin;
    use crate::store::Store;
    use farcooler_protocol::v1::TerminalIntent;

    fn pane(s: &Store) -> uuid::Uuid {
        let host = uuid::Uuid::now_v7();
        let root = s.create_repository_root(host, "/wt/logins", 1_000).unwrap();
        let repo = s.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();
        let ws = s.create_worktree(repo.id, "feature/x", "/wt/logins", false).unwrap();
        s.create_terminal(ws.id, "Agent", "claude", TerminalIntent::Running, 120, 40).unwrap().id
    }

    fn alices(dir: &str) -> TerminalLogin {
        TerminalLogin { handle: "alice".into(), login_id: "id-1".into(), config_dir: dir.into() }
    }

    /// A pane's login survives a reopen, can't be moved once written, and
    /// goes with its pane.
    #[test]
    fn a_panes_login_is_written_once_kept_and_goes_with_the_pane() {
        let dir = farcooler_core::scratch::dir("terminal-logins");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.db");
        let id = {
            let s = Store::open(&path).unwrap();
            let id = pane(&s);
            assert_eq!(s.terminal_login(id).unwrap(), None, "a new pane is the runner's");
            s.set_terminal_login(id, &alices("/r/claude/id-1")).unwrap();
            assert!(s.set_terminal_login(id, &alices("/r/claude/id-2")).is_err(), "a second login for one pane");
            id
        };
        let s = Store::open(&path).unwrap();
        assert_eq!(s.terminal_login(id).unwrap(), Some(alices("/r/claude/id-1")));
        let t = s.get_terminal(id).unwrap();
        s.delete_terminal(id, t.resource_version).unwrap();
        let left: i64 = s.conn().query_row("SELECT COUNT(*) FROM terminal_logins", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0, "the row went with its pane");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A member keeps one login id; removing them takes it, and the handle
    /// given to someone new gets a new one (review 1, M2). Nobody else has
    /// one.
    #[test]
    fn a_reused_handle_gets_a_new_login() {
        let dir = farcooler_core::scratch::dir("member-logins");
        std::fs::create_dir_all(&dir).unwrap();
        let s = Store::open(dir.join("state.db")).unwrap();
        assert!(s.ensure_member_login("alice").is_err(), "no such member");
        s.set_member("alice", "Alice", &[]).unwrap();
        let first = s.ensure_member_login("alice").unwrap();
        assert_eq!(s.ensure_member_login("alice").unwrap(), first, "one per member");
        s.remove_member("alice").unwrap();
        assert_eq!(s.member_login("alice").unwrap(), None, "went with them");
        s.set_member("alice", "Another Alice", &[]).unwrap();
        assert_ne!(s.ensure_member_login("alice").unwrap(), first, "a new person, a new login");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
