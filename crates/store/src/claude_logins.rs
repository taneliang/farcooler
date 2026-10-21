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

/// Three nullable columns on 0053's `member_logins` (ov-587): when the member's
/// agents last failed to authenticate (`stopped_at`), when a rep they asked
/// for was held back for want of a login (`asked_at`), and when they last
/// signed in (`signed_in_at`, so a failure from before it is not news), all
/// Unix milliseconds. Written only by the 587 daemon and read by nothing older, so
/// a build from before them reads the table as it did; they go with the row.
pub(crate) fn migration_0055_login_flags(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        "ALTER TABLE member_logins ADD COLUMN stopped_at INTEGER;
         ALTER TABLE member_logins ADD COLUMN asked_at INTEGER;
         ALTER TABLE member_logins ADD COLUMN signed_in_at INTEGER;",
    )
}

/// Two nullable columns on `member_logins` (ov-652): when a Codex pane
/// (`codex_asked_at`) or a Cursor pane (`cursor_asked_at`) was first refused
/// for this member, Unix milliseconds. Neither harness has a sign-in of its
/// own yet, so nothing clears them but the row going with the member.
pub(crate) fn migration_0056_harness_asks(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        "ALTER TABLE member_logins ADD COLUMN codex_asked_at INTEGER;
         ALTER TABLE member_logins ADD COLUMN cursor_asked_at INTEGER;",
    )
}

/// How long after a sign-in an authentication failure is taken to be from
/// before it. A turn that began on the old token can end after the sign-in,
/// and the daemon is not told when a turn began, so a failure this soon is
/// the old login's last word, not the new one's.
pub const SIGN_IN_GRACE_MS: i64 = 30_000;

/// How long a Codex or Cursor ask stays on Needs You, and how long before a
/// refusal raises it again (ov-652): neither has a sign-in to answer it with.
pub const HARNESS_ASK_TTL_MS: i64 = 24 * 60 * 60 * 1000;

/// What a member's login row says is wrong with it (ov-587).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginFlags {
    pub handle: String,
    /// When their agents' turns first failed to authenticate since they last
    /// signed in, in Unix milliseconds.
    pub stopped_at: Option<i64>,
    /// When a rep for them was first refused for want of a login since they
    /// last signed in.
    pub asked_at: Option<i64>,
    /// When a Codex pane for them was first refused (ov-652).
    pub codex_asked_at: Option<i64>,
    /// When a Cursor pane for them was first refused (ov-652).
    pub cursor_asked_at: Option<i64>,
}

/// The login a pane records: whose, which generation of them, and the exact
/// directory string its first launch passed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalLogin {
    pub handle: String,
    pub login_id: String,
    pub config_dir: String,
    /// The pane runs on the runner's own login for its harness, which its
    /// admins allow (ov-652, `member_login_policy`): `config_dir` is never
    /// passed. Still the member's pane in every other way.
    pub shared: bool,
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

    /// Whether any member has a login now: the cheap form of
    /// `member_login_ids` for a path that only asks whether there is a choice.
    pub fn any_member_login(&self) -> Result<bool> {
        self.conn().query_row("SELECT EXISTS(SELECT 1 FROM member_logins)", [], |r| r.get(0)).map_err(map_err)
    }

    /// Every member's login id now (ov-588): logins no removal may wipe.
    pub fn member_login_ids(&self) -> Result<Vec<String>> {
        crate::read_counts::note_whole_table_read();
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT login_id FROM member_logins").map_err(map_err)?;
        let ids = stmt.query_map([], |r| r.get(0)).map_err(map_err)?;
        ids.collect::<rusqlite::Result<Vec<String>>>().map_err(map_err)
    }

    /// Note that `handle`'s agents stopped authenticating on login `login_id`
    /// at `at`. `true` when that is news: it names the member's current
    /// generation (a removed member's panes say nothing about whoever holds
    /// the handle now) and wasn't already noted.
    pub fn mark_login_stopped(&self, handle: &str, login_id: &str, at: i64) -> Result<bool> {
        let changed = self
            .conn()
            .execute(
                "UPDATE member_logins SET stopped_at = ?3 WHERE handle = ?1 AND login_id = ?2 AND stopped_at IS NULL
                 AND (signed_in_at IS NULL OR ?3 - signed_in_at > ?4)",
                params![handle, login_id, at, SIGN_IN_GRACE_MS],
            )
            .map_err(map_err)?;
        Ok(changed > 0)
    }

    /// Note that a rep for `handle` was refused for want of a login at `at`.
    /// `true` when that is news.
    pub fn mark_sign_in_asked(&self, handle: &str, at: i64) -> Result<bool> {
        let changed = self
            .conn()
            .execute("UPDATE member_logins SET asked_at = ?2 WHERE handle = ?1 AND asked_at IS NULL", params![handle, at])
            .map_err(map_err)?;
        Ok(changed > 0)
    }

    /// Note that a `harness` (`codex` or `cursor`) pane for `handle` was
    /// refused at `at`. `true` when that is news: none was noted, or the last
    /// has expired (`HARNESS_ASK_TTL_MS`). `false` for any other harness,
    /// which has no column.
    pub fn mark_harness_asked(&self, handle: &str, harness: &str, at: i64) -> Result<bool> {
        let sql = match harness {
            "codex" => "UPDATE member_logins SET codex_asked_at = ?2 WHERE handle = ?1 AND (codex_asked_at IS NULL OR ?2 - codex_asked_at > ?3)",
            "cursor" => "UPDATE member_logins SET cursor_asked_at = ?2 WHERE handle = ?1 AND (cursor_asked_at IS NULL OR ?2 - cursor_asked_at > ?3)",
            _ => return Ok(false),
        };
        Ok(self.conn().execute(sql, params![handle, at, HARNESS_ASK_TTL_MS]).map_err(map_err)? > 0)
    }

    /// Forget both notes for `handle`: they signed out. `true` when there
    /// was one to forget.
    pub fn clear_login_flags(&self, handle: &str) -> Result<bool> {
        let changed = self
            .conn()
            .execute(
                "UPDATE member_logins SET stopped_at = NULL, asked_at = NULL, signed_in_at = NULL
                 WHERE handle = ?1 AND (stopped_at IS NOT NULL OR asked_at IS NOT NULL OR signed_in_at IS NOT NULL)",
                [handle],
            )
            .map_err(map_err)?;
        Ok(changed > 0)
    }

    /// `handle` signed in at `at`: both notes go, and a failure within
    /// `SIGN_IN_GRACE_MS` of it is not noted (`mark_login_stopped`).
    pub fn record_sign_in(&self, handle: &str, at: i64) -> Result<()> {
        self.conn()
            .execute("UPDATE member_logins SET stopped_at = NULL, asked_at = NULL, signed_in_at = ?2 WHERE handle = ?1", params![handle, at])
            .map_err(map_err)?;
        Ok(())
    }

    /// Every member with a note on their login, by handle.
    pub fn login_flags(&self) -> Result<Vec<LoginFlags>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT handle, stopped_at, asked_at, codex_asked_at, cursor_asked_at FROM member_logins
                 WHERE stopped_at IS NOT NULL OR asked_at IS NOT NULL OR codex_asked_at IS NOT NULL OR cursor_asked_at IS NOT NULL ORDER BY handle")
            .map_err(map_err)?;
        let rows = stmt
            .query_map([], |r| Ok(LoginFlags { handle: r.get(0)?, stopped_at: r.get(1)?, asked_at: r.get(2)?, codex_asked_at: r.get(3)?, cursor_asked_at: r.get(4)? }))
            .map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    /// `handle`'s notes, if they have a login row.
    pub fn login_flags_of(&self, handle: &str) -> Result<Option<LoginFlags>> {
        self.conn()
            .query_row(
                "SELECT handle, stopped_at, asked_at, codex_asked_at, cursor_asked_at FROM member_logins WHERE handle = ?1",
                [handle],
                |r| Ok(LoginFlags { handle: r.get(0)?, stopped_at: r.get(1)?, asked_at: r.get(2)?, codex_asked_at: r.get(3)?, cursor_asked_at: r.get(4)? }),
            )
            .optional()
            .map_err(map_err)
    }

    /// Record that `terminal` runs on `login`. Written once, before the
    /// pane's first launch (or an old rep's first relaunch); a second write
    /// for the same pane is refused, so nothing can move a pane to another
    /// person's login.
    pub fn set_terminal_login(&self, terminal: Uuid, login: &TerminalLogin) -> Result<()> {
        let inserted = self.conn().execute(
            "INSERT INTO terminal_logins (terminal_id, handle, login_id, config_dir, shared) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![uuid_blob(terminal), login.handle, login.login_id, login.config_dir, login.shared],
        );
        match inserted {
            Ok(_) => Ok(()),
            Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ConstraintViolation => {
                Err(DomainError::Conflict { what: "login_taken" })
            }
            Err(e) => Err(map_err(e)),
        }
    }

    /// Whether `terminal`, a member's pane, runs on the runner's own login
    /// for its harness (`shared`) or on its member's, decided afresh at each
    /// launch (ov-652). Only for generation `login_id`, so nothing moves a
    /// pane to another person's login. `true` when it changed.
    pub fn set_terminal_login_shared(&self, terminal: Uuid, login_id: &str, shared: bool) -> Result<bool> {
        let changed = self
            .conn()
            .execute(
                "UPDATE terminal_logins SET shared = ?3 WHERE terminal_id = ?1 AND login_id = ?2 AND shared != ?3",
                params![uuid_blob(terminal), login_id, shared],
            )
            .map_err(map_err)?;
        Ok(changed > 0)
    }

    /// The login `terminal` records, or `None` for the runner's.
    pub fn terminal_login(&self, terminal: Uuid) -> Result<Option<TerminalLogin>> {
        self.conn()
            .query_row(
                "SELECT handle, login_id, config_dir, shared FROM terminal_logins WHERE terminal_id = ?1",
                [uuid_blob(terminal)],
                |r| Ok(TerminalLogin { handle: r.get(0)?, login_id: r.get(1)?, config_dir: r.get(2)?, shared: r.get(3)? }),
            )
            .optional()
            .map_err(map_err)
    }

    /// Every pane's recorded login, by pane (ov-588): which panes are on a
    /// removed member's login, and every directory string any login was
    /// launched with, so a removal can tell its member's Keychain entry from
    /// everyone else's.
    pub fn terminal_logins(&self) -> Result<Vec<(Uuid, TerminalLogin)>> {
        crate::read_counts::note_whole_table_read();
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT terminal_id, handle, login_id, config_dir, shared FROM terminal_logins").map_err(map_err)?;
        let rows = stmt
            .query_map([], |r| {
                let id: Vec<u8> = r.get(0)?;
                Ok((id, TerminalLogin { handle: r.get(1)?, login_id: r.get(2)?, config_dir: r.get(3)?, shared: r.get(4)? }))
            })
            .map_err(map_err)?;
        let mut out = Vec::new();
        for row in rows {
            let (id, login) = row.map_err(map_err)?;
            if let Ok(id) = Uuid::from_slice(&id) {
                out.push((id, login));
            }
        }
        Ok(out)
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
        TerminalLogin { handle: "alice".into(), login_id: "id-1".into(), config_dir: dir.into(), shared: false }
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
        assert_eq!(s.terminal_logins().unwrap(), vec![(id, alices("/r/claude/id-1"))], "listed by pane");
        let t = s.get_terminal(id).unwrap();
        s.delete_terminal(id, t.resource_version).unwrap();
        let left: i64 = s.conn().query_row("SELECT COUNT(*) FROM terminal_logins", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0, "the row went with its pane");
        assert_eq!(s.terminal_logins().unwrap(), vec![]);
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
        assert_eq!(s.member_login_ids().unwrap(), std::slice::from_ref(&first));
        s.remove_member("alice").unwrap();
        assert_eq!(s.member_login("alice").unwrap(), None, "went with them");
        s.set_member("alice", "Another Alice", &[]).unwrap();
        assert_ne!(s.ensure_member_login("alice").unwrap(), first, "a new person, a new login");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The migration is the 46th (0055) and `Welcome`: a build from before it
    /// never names either column, and reads the table as it did.
    #[test]
    fn the_flags_migration_is_welcome() {
        let it = &crate::migrate::MIGRATIONS[45];
        assert!(std::ptr::fn_addr_eq(it.0, super::migration_0055_login_flags as fn(&rusqlite::Transaction) -> rusqlite::Result<()>));
        assert_eq!(it.1, crate::compat::Older::Welcome);
    }

    /// A note is made once, only for the member's current generation, and is
    /// cleared whole; a removed member's goes with their row.
    #[test]
    fn login_flags_are_noted_once_for_a_generation_and_cleared() {
        let dir = farcooler_core::scratch::dir("login-flags");
        std::fs::create_dir_all(&dir).unwrap();
        let s = Store::open(dir.join("state.db")).unwrap();
        s.set_member("alice", "Alice", &[]).unwrap();
        let id = s.ensure_member_login("alice").unwrap();
        assert!(s.login_flags().unwrap().is_empty(), "a new login has no note");
        assert!(!s.mark_login_stopped("alice", "an-older-generation", 5).unwrap(), "not their login now");
        assert!(s.mark_login_stopped("alice", &id, 10).unwrap());
        assert!(!s.mark_login_stopped("alice", &id, 20).unwrap(), "already noted, and the first time stays");
        assert!(s.mark_sign_in_asked("alice", 30).unwrap());
        let flags = s.login_flags().unwrap();
        assert_eq!(flags.len(), 1);
        assert_eq!((flags[0].stopped_at, flags[0].asked_at), (Some(10), Some(30)));
        // A failure just after a sign-in is the old token's last word.
        s.record_sign_in("alice", 100).unwrap();
        assert!(s.login_flags().unwrap().is_empty(), "signing in clears both notes");
        assert!(!s.mark_login_stopped("alice", &id, 100 + 29_999).unwrap(), "inside the grace");
        assert!(s.mark_login_stopped("alice", &id, 100 + 30_001).unwrap(), "after it, news again");
        s.mark_sign_in_asked("alice", 30).unwrap();
        assert!(s.clear_login_flags("alice").unwrap());
        assert!(!s.clear_login_flags("alice").unwrap(), "nothing left to clear");
        assert!(s.login_flags().unwrap().is_empty());
        assert!(s.mark_sign_in_asked("nobody", 1).is_ok_and(|news| !news), "no row, no note");
        s.mark_sign_in_asked("alice", 40).unwrap();
        s.remove_member("alice").unwrap();
        assert!(s.login_flags().unwrap().is_empty(), "went with the member");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
