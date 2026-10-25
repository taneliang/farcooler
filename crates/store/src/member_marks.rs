//! Where a member's read marks live (ov-740, the owner's ruling on ov-716
//! rule 1): what they have read on a board and which worktrees they have
//! reviewed, apart from the owner's, so a member opening a card or a diff
//! clears nothing for the owner, and the owner nothing for them.
//!
//! The owner's marks stay in `board_read_floors`, `task_reads` and
//! `review_reviewed`, exactly as before; these are copies of those three
//! with a member's handle in the key (`board_reads::Reader`). On a runner
//! with no members nothing is ever written here.

/// Three new tables, each a copy of an older one with the member's handle
/// in its key. A row goes with its board, task or worktree by cascade, as
/// the owner's do, and with its member, so a handle used again later
/// inherits nothing.
///
/// `Older::Welcome`: a build from before it never names them. It reads and
/// writes the owner's marks where they always were, and a member's device
/// on it shares them again, as it did.
pub(crate) fn migration_0059_member_marks(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        CREATE TABLE member_board_read_floors (
            workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            handle TEXT NOT NULL REFERENCES members(handle) ON DELETE CASCADE,
            floor_ms INTEGER NOT NULL,
            PRIMARY KEY (workspace_id, handle)
        );
        CREATE TABLE member_task_reads (
            task_id BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            handle TEXT NOT NULL REFERENCES members(handle) ON DELETE CASCADE,
            opened_ms INTEGER NOT NULL,
            PRIMARY KEY (task_id, handle)
        );
        CREATE TABLE member_review_reviewed (
            worktree_id BLOB NOT NULL REFERENCES worktrees(id) ON DELETE CASCADE,
            branch TEXT NOT NULL,
            handle TEXT NOT NULL REFERENCES members(handle) ON DELETE CASCADE,
            head_commit TEXT NOT NULL,
            worktree_digest TEXT NOT NULL,
            gate_head INTEGER NOT NULL DEFAULT 0,
            gate_index INTEGER NOT NULL DEFAULT 0,
            marked_at INTEGER NOT NULL,
            PRIMARY KEY (worktree_id, branch, handle)
        );
        "#,
    )
}

#[cfg(test)]
#[path = "member_marks_tests.rs"]
mod tests;
