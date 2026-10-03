//! The queue of answers waiting to be told to the agent that asked.
//!
//! When somebody answers a task's decision, the daemon types the answer into
//! the agent working that task (or the workspace's orchestrator), but only
//! once that terminal is idle and nobody is typing in it. Until then the
//! answer waits here, so a daemon restart in between neither loses it nor
//! tells it twice.
//!
//! One row per ANSWER note, keyed by the note. A told row keeps its key with
//! `done_at` set, so enqueueing the same note again is a no-op: exactly once
//! is the primary key's job, not a caller's memory. Marking a row done and
//! writing the note that says what happened are one transaction, so the
//! record and the queue can't disagree.

use rusqlite::params;
use uuid::Uuid;

use farcooler_core::Result;

use crate::error::map_err;
use crate::models::{Actor, NoteKind, TaskNote, get_uuid, uuid_blob};
use crate::store::Store;
use crate::tasks::{insert_note, now_millis};

/// An answer not yet told, with what it said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingWake {
    /// The ANSWER note.
    pub note: Uuid,
    pub task: Uuid,
    /// The answer as written.
    pub body: String,
    /// Who wrote it.
    pub actor: Actor,
    /// Unix milliseconds.
    pub enqueued_at: i64,
}

impl Store {
    /// Queue `note`, an answer on `task`, to be told. False when it was
    /// already queued, told or not: an answer is told at most once.
    pub fn enqueue_answer_wake(&self, note: Uuid, task: Uuid) -> Result<bool> {
        let inserted = self
            .conn()
            .execute(
                "INSERT OR IGNORE INTO answer_wakes (note_id, task_id, enqueued_at) VALUES (?1, ?2, ?3)",
                params![uuid_blob(note), uuid_blob(task), now_millis()],
            )
            .map_err(map_err)?;
        Ok(inserted == 1)
    }

    /// Every answer not yet told, oldest first.
    pub fn pending_answer_wakes(&self) -> Result<Vec<PendingWake>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT w.note_id, w.task_id, n.body, n.actor, w.enqueued_at
                   FROM answer_wakes w JOIN task_notes n ON n.id = w.note_id
                  WHERE w.done_at IS NULL
                  ORDER BY w.enqueued_at, w.rowid",
            )
            .map_err(map_err)?;
        let rows = stmt
            .query_map([], |r| {
                let actor: String = r.get(3)?;
                Ok(PendingWake {
                    note: get_uuid(r, 0)?,
                    task: get_uuid(r, 1)?,
                    body: r.get(2)?,
                    // An unreadable word is nobody in particular: the wake
                    // still goes to the agent rather than being dropped.
                    actor: Actor::parse(&actor).unwrap_or(Actor::User),
                    enqueued_at: r.get(4)?,
                })
            })
            .map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    /// Whether any answer is waiting to be told. Cheap enough to ask every
    /// tick: an indexed probe of an almost always empty set.
    pub fn any_pending_answer_wake(&self) -> Result<bool> {
        self.conn()
            .query_row("SELECT EXISTS (SELECT 1 FROM answer_wakes WHERE done_at IS NULL)", [], |r| r.get(0))
            .map_err(map_err)
    }

    /// Mark `note`'s wake done and, when `record` is given, say so on its
    /// task as a PROGRESS note from the runner, in one transaction. `None`
    /// when it was already done, so a second finisher writes nothing.
    pub fn finish_answer_wake(&self, note: Uuid, record: Option<&str>) -> Result<Option<Option<TaskNote>>> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let task: Option<Vec<u8>> = {
            use rusqlite::OptionalExtension;
            tx.query_row(
                "UPDATE answer_wakes SET done_at = ?2 WHERE note_id = ?1 AND done_at IS NULL RETURNING task_id",
                params![uuid_blob(note), now_millis()],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_err)?
        };
        let Some(task) = task else { return Ok(None) };
        let task = Uuid::from_slice(&task).map_err(|_| farcooler_core::DomainError::NotFound)?;
        let written = match record {
            Some(body) => {
                Some(insert_note(&tx, task, NoteKind::Progress, Actor::Runner, body, &serde_json::json!({}), None)?)
            }
            None => None,
        };
        tx.commit().map_err(map_err)?;
        Ok(Some(written))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TaskStatus;

    fn board() -> (Store, Uuid) {
        let store = Store::open_in_memory().unwrap();
        let repo = store.register_repository_for_test("Far Cooler");
        let main = store.ensure_main_workspace(repo).unwrap().id;
        let task = store.create_task(main, "Drill-in layout", Actor::Manager).unwrap().id;
        (store, task)
    }

    /// An answer is queued once, listed with its words until it's told, and
    /// told once: a second enqueue of the same note, or a second finish,
    /// changes nothing.
    #[test]
    fn an_answer_is_queued_and_told_exactly_once() {
        let (store, task) = board();
        store.set_task_status(task, TaskStatus::NeedsDecision, Actor::Manager).unwrap();
        let answer = store.add_note(task, NoteKind::Answer, Actor::User, "Drill in", serde_json::json!({})).unwrap();

        assert!(!store.any_pending_answer_wake().unwrap());
        assert!(store.enqueue_answer_wake(answer.id, task).unwrap());
        assert!(!store.enqueue_answer_wake(answer.id, task).unwrap(), "queued twice");
        let pending = store.pending_answer_wakes().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!((pending[0].note, pending[0].task, pending[0].body.as_str()), (answer.id, task, "Drill in"));
        assert_eq!(pending[0].actor, Actor::User);
        assert!(store.any_pending_answer_wake().unwrap());

        let told = store.finish_answer_wake(answer.id, Some("Told Agent 2 about the decision")).unwrap();
        let note = told.flatten().expect("a note");
        assert_eq!((note.kind, note.actor, note.body.as_str()), (NoteKind::Progress, Actor::Runner, "Told Agent 2 about the decision"));
        assert!(store.pending_answer_wakes().unwrap().is_empty());
        assert!(!store.any_pending_answer_wake().unwrap());

        assert_eq!(store.finish_answer_wake(answer.id, Some("again")).unwrap(), None, "finished twice");
        assert!(!store.enqueue_answer_wake(answer.id, task).unwrap(), "a told answer queued again");
        let progress = store.notes_for(task, Some(NoteKind::Progress)).unwrap();
        assert_eq!(progress.len(), 1, "{progress:?}");
    }

    /// The runner's note reads back as the runner's: the word round-trips.
    #[test]
    fn the_runner_is_an_actor_that_round_trips() {
        assert_eq!(Actor::Runner.to_string(), "runner");
        assert_eq!(Actor::parse("runner"), Some(Actor::Runner));
    }

    /// Finishing without a record marks it done and writes nothing.
    #[test]
    fn a_silent_finish_writes_no_note() {
        let (store, task) = board();
        let answer = store.add_note(task, NoteKind::Answer, Actor::User, "Yes", serde_json::json!({})).unwrap();
        store.enqueue_answer_wake(answer.id, task).unwrap();
        assert_eq!(store.finish_answer_wake(answer.id, None).unwrap(), Some(None));
        assert!(store.notes_for(task, Some(NoteKind::Progress)).unwrap().is_empty());
        assert!(store.pending_answer_wakes().unwrap().is_empty());
    }

    /// Every workspace wakes its agent unless somebody turned it off, and the
    /// switch is a versioned write like a rename.
    #[test]
    fn a_workspace_wakes_on_answer_until_turned_off() {
        let store = Store::open_in_memory().unwrap();
        let repo = store.register_repository_for_test("Far Cooler");
        let main = store.ensure_main_workspace(repo).unwrap();
        assert!(main.wake_on_answer, "on by default");
        let off = store.set_workspace_wake_on_answer(main.id, main.resource_version, false).unwrap();
        assert!(!off.wake_on_answer);
        assert!(!store.get_workspace(main.id).unwrap().wake_on_answer);
        assert_eq!(off.resource_version, main.resource_version + 1);
        assert!(
            matches!(
                store.set_workspace_wake_on_answer(main.id, main.resource_version, true),
                Err(farcooler_core::DomainError::ResourceConflict)
            ),
            "a stale version"
        );
        assert!(matches!(
            store.set_workspace_wake_on_answer(Uuid::now_v7(), 1, true),
            Err(farcooler_core::DomainError::NotFound)
        ));
    }
}
