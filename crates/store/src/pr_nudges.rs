//! One polite comment per pull request, at most (ov-580): the record that it
//! was sent, and the two settings that allow it.
//!
//! **Additive and removable.** Migration 0052 adds two columns to
//! `workspace_landing` (`pr_nudge`, defaulted off, and `pr_nudge_hours`,
//! nullable) and one table, `pr_nudges`, one row per pull request nudged. A
//! build from before reads `workspace_landing` by the columns it names and never
//! opens `pr_nudges`, so a rollback keeps a working database (`Older::Welcome`)
//! and nudges nothing.
//!
//! **Keyed by GitHub's repository, not the store's.** A row is the pull
//! request's repository as GitHub names it (`host/owner/repo`, from its URL,
//! lower case) and its number. A repository registered twice, or again after it
//! was removed, is one repository to GitHub, and a nudge must not go out twice
//! because the runner's own ids changed. So the rows do not go with a store
//! repository; they are a few bytes each.
//!
//! **The row is written before the comment is.** `claim` is the only way to
//! send: it inserts the row and says whether this call made it, so two ticks,
//! two daemons on one store, or a restart between the claim and the post can
//! each send at most once. The caller gives the claim back (`release`) only when
//! `gh` could not be started, so certainly nothing was posted; once it has run,
//! whatever it said, the claim stays, because a nudge sent twice is worse than
//! one never sent.

use rusqlite::{Transaction, params};

use farcooler_core::Result;

use crate::error::map_err;
use crate::store::Store;
use crate::tasks::now_millis;

pub(crate) fn migration_0052_pr_nudge(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        ALTER TABLE workspace_landing ADD COLUMN pr_nudge INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE workspace_landing ADD COLUMN pr_nudge_hours INTEGER;
        CREATE TABLE pr_nudges (
            -- `host/owner/repo` on GitHub, lower case.
            repo TEXT NOT NULL,
            pr_number INTEGER NOT NULL,
            -- Unix milliseconds, when the comment was claimed (and so, but for a crash, sent).
            claimed_at INTEGER NOT NULL,
            PRIMARY KEY (repo, pr_number)
        );
        "#,
    )
}

impl Store {
    /// Take the one nudge of pull request `pr` of `repo` (GitHub's `host/owner/repo`).
    /// `true` when this call took it and the comment may be posted; `false`
    /// when it was taken before.
    pub fn claim_pr_nudge(&self, repo: &str, pr: u32) -> Result<bool> {
        let conn = self.conn();
        let added = conn
            .execute(
                "INSERT OR IGNORE INTO pr_nudges (repo, pr_number, claimed_at) VALUES (?1, ?2, ?3)",
                params![repo.to_lowercase(), pr, now_millis()],
            )
            .map_err(map_err)?;
        Ok(added == 1)
    }

    /// Give a claim back, after `gh` could not be started.
    pub fn release_pr_nudge(&self, repo: &str, pr: u32) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM pr_nudges WHERE repo = ?1 AND pr_number = ?2", params![repo.to_lowercase(), pr]).map_err(map_err)?;
        Ok(())
    }

    /// Whether `pr` has been nudged (or claimed).
    pub fn pr_nudged(&self, repo: &str, pr: u32) -> Result<bool> {
        let conn = self.conn();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM pr_nudges WHERE repo = ?1 AND pr_number = ?2", params![repo.to_lowercase(), pr], |r| r.get(0))
            .map_err(map_err)?;
        Ok(n > 0)
    }
}

#[cfg(test)]
#[path = "pr_nudges_tests.rs"]
mod tests;
