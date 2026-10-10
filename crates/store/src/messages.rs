//! The queue of messages between a workspace's orchestrator and its lanes
//! (ov-455).
//!
//! A message is a note on a card and a row in `message_wakes`, written in one
//! transaction, so a crash can't keep one and lose the other. The note is the
//! record: a COMMENT from whoever sent it, or a PROGRESS note from the runner
//! for its own notice, so the card's thread shows the conversation with no
//! screen of its own. The row is the delivery, told by the answer wake's pump
//! under its gate and its rules (`wakes.rs`): claimed before anything is
//! typed, so at most once; marked done with a note saying what happened; and
//! kept across a restart. `to_terminal` names the pane it is for, or is NULL
//! for the workspace's orchestrator, found when it is told, so a restarted
//! orchestrator still gets it.
//!
//! Only a board that wakes on answers queues: the switch is what lets the
//! runner type into a pane at all.

use rusqlite::params;
use uuid::Uuid;

use farcooler_core::{DomainError, Result};

use crate::error::map_err;
use crate::models::{Actor, NoteKind, TaskNote, uuid_blob};
use crate::store::Store;
use crate::tasks::{insert_note, now_millis};
use crate::wakes::{PendingWake, WakeKind};

/// One new table, so a build from before it reads every table it knows
/// exactly as it did.
pub(crate) fn migration_0043_message_wakes(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        CREATE TABLE message_wakes (
            note_id BLOB PRIMARY KEY NOT NULL,
            task_id BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            to_terminal BLOB,
            enqueued_at INTEGER NOT NULL,
            claimed_at INTEGER,
            pasted_at INTEGER,
            done_at INTEGER
        );
        CREATE INDEX message_wakes_pending ON message_wakes (enqueued_at) WHERE done_at IS NULL;
        "#,
    )
}

impl Store {
    /// File `body` on `task` from `actor` and queue it for `to` (a terminal,
    /// or `None` for the workspace's orchestrator), in one transaction. A
    /// COMMENT, or a PROGRESS note when the runner sends it. Refused as
    /// `typing_off` when the task's board doesn't wake on answers, writing
    /// nothing.
    pub fn send_message(
        &self,
        task: Uuid,
        actor: Actor,
        body: &str,
        to: Option<Uuid>,
        extra: serde_json::Value,
    ) -> Result<TaskNote> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let wakes: Option<bool> = {
            use rusqlite::OptionalExtension;
            tx.query_row(
                "SELECT w.wake_on_answer FROM tasks t JOIN workspaces w ON w.id = t.workspace_id WHERE t.id = ?1",
                params![uuid_blob(task)],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_err)?
        };
        match wakes {
            None => return Err(DomainError::NotFound),
            Some(false) => return Err(DomainError::Conflict { what: "typing_off" }),
            Some(true) => {}
        }
        let kind = if actor == Actor::Runner { NoteKind::Progress } else { NoteKind::Comment };
        let note = insert_note(&tx, task, kind, actor, body, &extra, None)?;
        tx.execute(
            "INSERT INTO message_wakes (note_id, task_id, to_terminal, enqueued_at) VALUES (?1, ?2, ?3, ?4)",
            params![uuid_blob(note.id), uuid_blob(task), to.map(uuid_blob), now_millis()],
        )
        .map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        Ok(note)
    }

    /// Every message not yet told, oldest first.
    pub fn pending_message_wakes(&self) -> Result<Vec<PendingWake>> {
        self.pending_wakes(WakeKind::Message)
    }

    /// How many messages wait for `to` (`None`: an orchestrator) on
    /// `workspace`'s board, not yet told.
    pub fn messages_waiting(&self, workspace: Uuid, to: Option<Uuid>) -> Result<u32> {
        self.conn()
            .query_row(
                "SELECT count(*) FROM message_wakes m JOIN tasks t ON t.id = m.task_id
                  WHERE m.done_at IS NULL AND t.workspace_id = ?1 AND m.to_terminal IS ?2",
                params![uuid_blob(workspace), to.map(uuid_blob)],
                |r| r.get(0),
            )
            .map_err(map_err)
    }

    /// How many messages `actor` sent to `to` (`None`: an orchestrator) on
    /// `workspace`'s board since `since` (Unix milliseconds), told or not.
    pub fn messages_sent_since(&self, workspace: Uuid, actor: Actor, to: Option<Uuid>, since: i64) -> Result<u32> {
        self.conn()
            .query_row(
                "SELECT count(*) FROM message_wakes m JOIN tasks t ON t.id = m.task_id
                   JOIN task_notes n ON n.id = m.note_id
                  WHERE t.workspace_id = ?1 AND n.actor = ?2 AND m.to_terminal IS ?3 AND m.enqueued_at >= ?4",
                params![uuid_blob(workspace), actor.to_string(), to.map(uuid_blob), since],
                |r| r.get(0),
            )
            .map_err(map_err)
    }

    /// Whether the runner's own notice about `task` waits untold, or anything
    /// `actor` wrote on `task` since `since`: a note, a message, a move. What
    /// a turn-end notice is skipped for (`watch::answer_wake::notices`).
    pub fn reported_since(&self, task: Uuid, actor: Actor, since: i64) -> Result<bool> {
        self.conn()
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM task_notes WHERE task_id = ?1 AND actor = ?2 AND at >= ?3)
                     OR EXISTS (SELECT 1 FROM message_wakes m JOIN task_notes n ON n.id = m.note_id
                                 WHERE m.task_id = ?1 AND m.done_at IS NULL AND n.actor = 'runner')",
                params![uuid_blob(task), actor.to_string(), since],
                |r| r.get(0),
            )
            .map_err(map_err)
    }
}

#[cfg(test)]
#[path = "messages_tests.rs"]
mod tests;
