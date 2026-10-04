//! Which paths of a worktree are still Git LFS pointer files (ov-199).
//!
//! A worktree of an LFS repository hydrates its large files best effort: an
//! object missing from the local LFS store, or a hydrate that ran out of time,
//! leaves the pointer. The daemon writes down which paths, so the apps can say
//! "Some large files weren't downloaded." and offer a retry.
//!
//! A table of its own, whose rows go with the worktree by cascade, so a build
//! from before it reads every table it knows exactly as it did.

use std::collections::HashMap;

use rusqlite::params;
use uuid::Uuid;

use farcooler_core::Result;
use rusqlite::Transaction;

use crate::error::map_err;
use crate::models::{get_uuid, uuid_blob};
use crate::store::Store;

/// One new table that only this file touches. An older build never reads or
/// writes it, and deleting a worktree from any build cascades its rows.
pub(crate) fn migration_0023_lfs_pointers(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        CREATE TABLE worktree_lfs_pointers (
            worktree_id BLOB NOT NULL REFERENCES worktrees(id) ON DELETE CASCADE,
            path TEXT NOT NULL,
            PRIMARY KEY (worktree_id, path)
        );
        "#,
    )
}

impl Store {
    /// Record `paths` as the worktree's pointer files, replacing what was
    /// there. Answers whether the set changed; when it did the worktree's
    /// `resource_version` moves, so a client that cached the count sees it.
    pub fn set_lfs_pointers(&self, worktree: Uuid, paths: &[String]) -> Result<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let mut before = pointer_paths(&tx, worktree)?;
        let mut after: Vec<String> = paths.to_vec();
        before.sort();
        after.sort();
        after.dedup();
        if before == after {
            return Ok(false);
        }
        tx.execute("DELETE FROM worktree_lfs_pointers WHERE worktree_id = ?1", params![uuid_blob(worktree)])
            .map_err(map_err)?;
        for path in &after {
            tx.execute(
                "INSERT INTO worktree_lfs_pointers (worktree_id, path) VALUES (?1, ?2)",
                params![uuid_blob(worktree), path],
            )
            .map_err(map_err)?;
        }
        tx.execute(
            "UPDATE worktrees SET resource_version = resource_version + 1 WHERE id = ?1",
            params![uuid_blob(worktree)],
        )
        .map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        Ok(true)
    }

    /// The worktree's recorded pointer paths, sorted.
    pub fn lfs_pointer_paths(&self, worktree: Uuid) -> Result<Vec<String>> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let mut paths = pointer_paths(&tx, worktree)?;
        paths.sort();
        Ok(paths)
    }

    /// How many pointer paths this worktree has recorded.
    pub fn lfs_pointer_count(&self, worktree: Uuid) -> Result<u32> {
        self.conn()
            .query_row(
                "SELECT COUNT(*) FROM worktree_lfs_pointers WHERE worktree_id = ?1",
                params![uuid_blob(worktree)],
                |r| r.get(0),
            )
            .map_err(map_err)
    }

    /// How many pointer paths each worktree has recorded, for the fleet.
    /// Worktrees with none are absent.
    pub fn lfs_pointer_counts(&self) -> Result<HashMap<Uuid, u32>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT worktree_id, COUNT(*) FROM worktree_lfs_pointers GROUP BY worktree_id")
            .map_err(map_err)?;
        let rows = stmt
            .query_map([], |r| Ok((get_uuid(r, 0)?, r.get::<_, u32>(1)?)))
            .map_err(map_err)?;
        let mut counts = HashMap::new();
        for row in rows {
            let (worktree, count) = row.map_err(map_err)?;
            counts.insert(worktree, count);
        }
        Ok(counts)
    }
}

fn pointer_paths(tx: &Transaction, worktree: Uuid) -> Result<Vec<String>> {
    let mut stmt = tx
        .prepare("SELECT path FROM worktree_lfs_pointers WHERE worktree_id = ?1")
        .map_err(map_err)?;
    let rows = stmt.query_map(params![uuid_blob(worktree)], |r| r.get::<_, String>(0)).map_err(map_err)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
}

#[cfg(test)]
#[path = "lfs_pointers_tests.rs"]
mod tests;
