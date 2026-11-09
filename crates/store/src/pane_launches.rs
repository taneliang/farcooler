//! Which launch of a pane is the current one (ov-886).
//!
//! Every time the daemon launches an agent into a pane it mints a launch id,
//! exports it to the pane as `pane_env::LAUNCH`, and records it here. The hook
//! CLI copies the variable into its frame, so a hook says which launch fired
//! it, and ingress routes it only if that launch is still this pane's.
//!
//! That's what tells a late hook from the process a restart killed apart from
//! one fired by its replacement. Session ids can't: a restarted Claude or Codex
//! resumes the same session on purpose. Neither can the pane, the worktree or
//! the timing.
//!
//! One row per pane, replaced on each launch. A launch no row holds is retired,
//! by definition, so there's no list of retired launches to keep or collect.
//!
//! The rows live in `meta`, keyed `pane_launch/<terminal id>`, rather than in a
//! table or column of their own. A migration would move the schema version,
//! and `engine_cutover::preflight` accepts only the exact versions it pins, so
//! a runner that hasn't cut over yet would be refused. `meta` already holds
//! per-device rows (`device_added_by.rs`). An older build never reads these
//! keys: it launches without exporting a launch id, so its hooks carry none
//! and are routed as they always were.
//!
//! `delete_terminal` removes a pane's row. A pane deleted by cascade (its
//! worktree or workspace going) leaves its row behind, and every lookup here
//! checks the pane still exists, so that row routes nothing. The design is in
//! `reports/ov-886/design.md`.

use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use farcooler_core::{DomainError, Result};

use crate::error::map_err;
use crate::models::{Terminal, uuid_blob};
use crate::store::Store;

/// `meta`'s key for pane `terminal`'s current launch.
fn key(terminal: Uuid) -> String {
    format!("{KEY_PREFIX}{terminal}")
}
const KEY_PREFIX: &str = "pane_launch/";

/// A launch `begin_launch` recorded, and the one it replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub id: Uuid,
    pub prior: Option<Uuid>,
}

impl Store {
    /// Make a fresh launch current for pane `terminal`, if the pane is still
    /// at `expected_version`, and hand back the row and the launch.
    ///
    /// One transaction: the version check, the version bump and the launch
    /// write. So of two restarts of one pane racing each other, one gets
    /// `ResourceConflict` here, before it has spawned anything. From the commit
    /// on, the prior launch's hooks reach nothing.
    pub fn begin_launch(&self, terminal: Uuid, expected_version: u64) -> Result<(Terminal, Launch)> {
        let launch = Uuid::now_v7();
        let prior = {
            let mut conn = self.conn();
            let tx = conn.transaction().map_err(map_err)?;
            let bumped = tx
                .execute(
                    "UPDATE terminals SET resource_version = resource_version + 1
                     WHERE id = ?1 AND resource_version = ?2",
                    params![uuid_blob(terminal), expected_version as i64],
                )
                .map_err(map_err)?;
            if bumped != 1 {
                let exists: Option<i64> = tx
                    .query_row("SELECT 1 FROM terminals WHERE id = ?1", params![uuid_blob(terminal)], |r| r.get(0))
                    .optional()
                    .map_err(map_err)?;
                return Err(if exists.is_some() { DomainError::ResourceConflict } else { DomainError::NotFound });
            }
            let prior: Option<String> = tx
                .query_row("SELECT value FROM meta WHERE key = ?1", params![key(terminal)], |r| r.get(0))
                .optional()
                .map_err(map_err)?;
            tx.execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key(terminal), launch.to_string()],
            )
            .map_err(map_err)?;
            tx.commit().map_err(map_err)?;
            prior.and_then(|p| Uuid::parse_str(&p).ok())
        };
        Ok((self.get_terminal(terminal)?, Launch { id: launch, prior }))
    }

    /// Put `launch.prior` back as pane `terminal`'s launch, but only while
    /// `launch.id` is still current. Returns whether it did.
    ///
    /// For a launch whose spawn failed. If a newer launch has replaced this
    /// one, that launch is left alone: a failed restart never overwrites one
    /// that came after it. With no prior, the row goes, which is how the pane
    /// stood before.
    pub fn roll_back_launch(&self, terminal: Uuid, launch: &Launch) -> Result<bool> {
        let conn = self.conn();
        let changed = match launch.prior {
            Some(prior) => conn.execute(
                "UPDATE meta SET value = ?3 WHERE key = ?1 AND value = ?2",
                params![key(terminal), launch.id.to_string(), prior.to_string()],
            ),
            None => conn.execute(
                "DELETE FROM meta WHERE key = ?1 AND value = ?2",
                params![key(terminal), launch.id.to_string()],
            ),
        }
        .map_err(map_err)?;
        Ok(changed == 1)
    }

    /// The pane whose current launch is `launch`, or `None` when no pane's
    /// is: a retired launch, or one this runner never made.
    pub fn terminal_for_launch(&self, launch: Uuid) -> Result<Option<Uuid>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT key FROM meta WHERE value = ?1 AND key LIKE 'pane_launch/%'")
            .map_err(map_err)?;
        let keys = stmt
            .query_map(params![launch.to_string()], |r| r.get::<_, String>(0))
            .map_err(map_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_err)?;
        // Launch ids are minted fresh, so two panes never hold one; if they
        // somehow did, neither is the answer.
        let [only] = keys.as_slice() else { return Ok(None) };
        let Some(terminal) = only.strip_prefix(KEY_PREFIX).and_then(|t| Uuid::parse_str(t).ok()) else {
            return Ok(None);
        };
        let exists: Option<i64> = conn
            .query_row("SELECT 1 FROM terminals WHERE id = ?1", params![uuid_blob(terminal)], |r| r.get(0))
            .optional()
            .map_err(map_err)?;
        Ok(exists.map(|_| terminal))
    }

    /// Pane `terminal`'s current launch, if it has one.
    pub fn current_launch(&self, terminal: Uuid) -> Result<Option<Uuid>> {
        let launch: Option<String> = self
            .conn()
            .query_row("SELECT value FROM meta WHERE key = ?1", params![key(terminal)], |r| r.get(0))
            .optional()
            .map_err(map_err)?;
        Ok(launch.and_then(|l| Uuid::parse_str(&l).ok()))
    }

    /// Forget pane `terminal`'s launch, for `delete_terminal`.
    pub(crate) fn forget_launch(&self, terminal: Uuid) -> Result<()> {
        self.conn().execute("DELETE FROM meta WHERE key = ?1", params![key(terminal)]).map_err(map_err)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "pane_launches_tests.rs"]
mod tests;
