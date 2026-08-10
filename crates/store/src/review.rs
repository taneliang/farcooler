//! What a worktree is compared against, and whether you have read it.
//!
//! This used to hold a review buffer: captured comments, their anchors, the
//! dispatch outbox, attachments. All of it is gone. The buffer existed because
//! there was nowhere to put a diff — you noticed something, held it in your
//! head, and typed it at an agent later — and with a diff tile beside an agent
//! tile there is nothing to hold.
//!
//! What is left was never about comments. `review_bases` is what a branch is
//! compared against when somebody pinned it, `review_reviewed` is how the fleet
//! knows a worktree moved since you last looked at it, and
//! `review_stack_parents` is a branch's parent when inference got it wrong.

use farcooler_core::Result;
use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use crate::error::map_err;
use crate::models::uuid_blob;
use crate::store::Store;

/// The last time you said you had read a worktree, and what it looked like then.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewedMark {
    pub head_commit: String,
    pub worktree_digest: String,
    /// mtimes of `HEAD` and the index when the mark was made.
    ///
    /// Kept alongside the digest so the fleet can answer "changed since you
    /// looked" with two stats rather than a `git status` per worktree.
    pub gate_head: i64,
    pub gate_index: i64,
    pub marked_at: i64,
}

impl Store {
    pub fn review_base(&self, workspace_id: Uuid) -> Result<Option<String>> {
        self.conn()
            .query_row(
                "SELECT base_ref FROM review_bases WHERE workspace_id = ?1",
                params![uuid_blob(workspace_id)],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_err)
    }

    pub fn set_review_base(&self, workspace_id: Uuid, base_ref: &str) -> Result<()> {
        self.conn()
            .execute(
                "INSERT INTO review_bases (workspace_id, base_ref) VALUES (?1, ?2)
                 ON CONFLICT(workspace_id) DO UPDATE SET base_ref = excluded.base_ref",
                params![uuid_blob(workspace_id), base_ref],
            )
            .map_err(map_err)?;
        Ok(())
    }

    pub fn mark_reviewed(
        &self,
        workspace_id: Uuid,
        branch: &str,
        head_commit: &str,
        worktree_digest: &str,
        now_millis: i64,
    ) -> Result<()> {
        self.mark_reviewed_with_gate(
            workspace_id,
            branch,
            head_commit,
            worktree_digest,
            0,
            0,
            now_millis,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn mark_reviewed_with_gate(
        &self,
        workspace_id: Uuid,
        branch: &str,
        head_commit: &str,
        worktree_digest: &str,
        gate_head: i64,
        gate_index: i64,
        now_millis: i64,
    ) -> Result<()> {
        self.conn()
            .execute(
                "INSERT INTO review_reviewed
                 (workspace_id, branch, head_commit, worktree_digest, gate_head, gate_index, marked_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(workspace_id, branch) DO UPDATE SET
                   head_commit = excluded.head_commit,
                   worktree_digest = excluded.worktree_digest,
                   gate_head = excluded.gate_head,
                   gate_index = excluded.gate_index,
                   marked_at = excluded.marked_at",
                params![
                    uuid_blob(workspace_id),
                    branch,
                    head_commit,
                    worktree_digest,
                    gate_head,
                    gate_index,
                    now_millis
                ],
            )
            .map_err(map_err)?;
        Ok(())
    }

    pub fn reviewed_mark(&self, workspace_id: Uuid, branch: &str) -> Result<Option<ReviewedMark>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT head_commit, worktree_digest, gate_head, gate_index, marked_at
                 FROM review_reviewed WHERE workspace_id = ?1 AND branch = ?2",
            )
            .map_err(map_err)?;
        stmt.query_row(params![uuid_blob(workspace_id), branch], |r| {
            Ok(ReviewedMark {
                head_commit: r.get(0)?,
                worktree_digest: r.get(1)?,
                gate_head: r.get(2)?,
                gate_index: r.get(3)?,
                marked_at: r.get(4)?,
            })
        })
        .optional()
        .map_err(map_err)
    }

    // ---- stack parents ----

    pub fn stack_parent(&self, repository_id: Uuid, branch: &str) -> Result<Option<String>> {
        self.conn()
            .query_row(
                "SELECT parent_branch FROM review_stack_parents
                 WHERE repository_id = ?1 AND branch = ?2",
                params![uuid_blob(repository_id), branch],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_err)
    }

    pub fn set_stack_parent(
        &self,
        repository_id: Uuid,
        branch: &str,
        parent_branch: &str,
    ) -> Result<()> {
        self.conn()
            .execute(
                "INSERT INTO review_stack_parents (repository_id, branch, parent_branch)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(repository_id, branch)
                 DO UPDATE SET parent_branch = excluded.parent_branch",
                params![uuid_blob(repository_id), branch, parent_branch],
            )
            .map_err(map_err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The buffer is deleted, and the schema is where that has to stay true.
    ///
    /// A table reappearing here means somebody rebuilt the thing the tile layout
    /// was supposed to make unnecessary — worth failing a build over, because it
    /// would come back one convenience method at a time.
    #[test]
    fn the_review_buffer_has_no_tables_left() {
        let s = Store::open_in_memory().unwrap();
        for gone in ["review_entries", "review_dispatches", "review_attachments", "review_viewed"]
        {
            assert!(
                s.column_names(gone).is_empty(),
                "`{gone}` is back: the review buffer was deleted deliberately"
            );
        }
    }

    #[test]
    fn what_survives_is_the_base_and_whether_you_have_read_it() {
        let s = Store::open_in_memory().unwrap();
        for kept in ["review_bases", "review_reviewed", "review_stack_parents"] {
            assert!(!s.column_names(kept).is_empty(), "`{kept}` should still exist");
        }
    }

    #[test]
    fn a_read_mark_round_trips_with_its_gate() {
        let s = Store::open_in_memory().unwrap();
        let host = Uuid::now_v7();
        let root = s.create_repository_root(host, "/tmp/root", 1).unwrap();
        let repo = s.create_repository(host, root.id, "r", "/tmp/root/r/.git", "").unwrap();
        let ws = s.create_workspace(repo.id, "feat/x", "/tmp/root/r-wt", false).unwrap();

        assert!(s.reviewed_mark(ws.id, "feat/x").unwrap().is_none());
        s.mark_reviewed_with_gate(ws.id, "feat/x", "head", "digest", 11, 22, 99).unwrap();

        let m = s.reviewed_mark(ws.id, "feat/x").unwrap().expect("a mark");
        assert_eq!((m.gate_head, m.gate_index), (11, 22));
        assert_eq!(m.worktree_digest, "digest");
        assert_eq!(m.marked_at, 99);
    }

    #[test]
    fn a_pinned_base_overrides_whatever_would_have_been_guessed() {
        let s = Store::open_in_memory().unwrap();
        let host = Uuid::now_v7();
        let root = s.create_repository_root(host, "/tmp/root", 1).unwrap();
        let repo = s.create_repository(host, root.id, "r", "/tmp/root/r/.git", "").unwrap();
        let ws = s.create_workspace(repo.id, "feat/x", "/tmp/root/r-wt", false).unwrap();

        assert!(s.review_base(ws.id).unwrap().is_none());
        s.set_review_base(ws.id, "release/2").unwrap();
        assert_eq!(s.review_base(ws.id).unwrap().as_deref(), Some("release/2"));
    }
}
