//! The queue of answers waiting to be told to the agent that asked.
//!
//! When somebody answers a task's decision, the daemon types the answer into
//! the agent working that task (or the workspace's orchestrator), but only
//! once that terminal is idle and nobody is typing in it. Until then the
//! answer waits here, so a daemon restart in between neither loses it nor
//! tells it twice.
//!
//! One row per ANSWER note, keyed by the note. A told row keeps its key with
//! `done_at` set, so enqueueing the same note again is a no-op. A row is
//! CLAIMED before anything is typed, so a crash mid-typing leaves a claimed
//! row that is never typed again (at most once, then a note saying it
//! couldn't be confirmed). A row the text was pasted for, whose Enter a dialog
//! stopped, is marked PASTED (ov-385): a later pass may press that Enter if
//! the box still holds exactly the text, and never types the text again. The
//! mark is taken off before that Enter goes, so a crash then reads as one
//! mid-typing. Marking a row done and writing the note that says
//! what happened are one transaction, so the record and the queue can't
//! disagree. The row is written in the same transaction as the answer itself
//! (`add_note_waking`), so a crash can't keep an answer and lose its wake.

use rusqlite::params;
use uuid::Uuid;

use farcooler_core::Result;

use crate::error::map_err;
use crate::models::{Actor, NoteKind, TaskNote, get_uuid, uuid_blob};
use crate::store::Store;
use crate::tasks::{insert_note, now_millis};

/// What a queued wake is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeKind {
    /// A person's answer to a decision (`answer_wakes`).
    Answer,
    /// A task held until a time, whose time came (`hold_wakes`, ov-212).
    HoldEnded,
    /// A message between an orchestrator and its lanes (`message_wakes`,
    /// ov-455, `messages.rs`).
    Message,
}

impl WakeKind {
    fn table(self) -> &'static str {
        match self {
            WakeKind::Answer => "answer_wakes",
            WakeKind::HoldEnded => "hold_wakes",
            WakeKind::Message => "message_wakes",
        }
    }

    /// The column naming whom a wake is for: only a message names one.
    fn to_column(self) -> &'static str {
        match self {
            WakeKind::Message => "w.to_terminal",
            WakeKind::Answer | WakeKind::HoldEnded => "NULL",
        }
    }
}

/// An answer, or a hold that ended, not yet told, with what it said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingWake {
    pub kind: WakeKind,
    /// The ANSWER note, or the runner's `wait` note that ended the hold.
    pub note: Uuid,
    pub task: Uuid,
    /// The answer as written.
    pub body: String,
    /// Who wrote it.
    pub actor: Actor,
    /// Unix milliseconds.
    pub enqueued_at: i64,
    /// When typing it began, if it did. Set and not done means it may or may
    /// not have reached the agent.
    pub claimed_at: Option<i64>,
    /// When its text was pasted and left in the box with the Enter not yet
    /// pressed, because a dialog came up in between (`mark_wake_pasted`).
    pub pasted_at: Option<i64>,
    /// A message's terminal, when it was sent to one; `None` for its
    /// workspace's orchestrator, and for every answer and hold.
    pub to: Option<Uuid>,
}

/// The pasted mark (ov-385): one nullable column on each queue. An older
/// build never names it, and reads a row carrying it as claimed, which it
/// settles as "Couldn't confirm" and never types again.
pub(crate) fn migration_0038_wake_pasted(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        "ALTER TABLE answer_wakes ADD COLUMN pasted_at INTEGER;
         ALTER TABLE hold_wakes ADD COLUMN pasted_at INTEGER;",
    )
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
        self.pending_wakes(WakeKind::Answer)
    }

    /// Every hold that ended and wasn't yet told, oldest first.
    pub fn pending_hold_wakes(&self) -> Result<Vec<PendingWake>> {
        self.pending_wakes(WakeKind::HoldEnded)
    }

    pub(crate) fn pending_wakes(&self, kind: WakeKind) -> Result<Vec<PendingWake>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT w.note_id, w.task_id, n.body, n.actor, w.enqueued_at, w.claimed_at, w.pasted_at, {}
                   FROM {} w JOIN task_notes n ON n.id = w.note_id
                  WHERE w.done_at IS NULL
                  ORDER BY w.enqueued_at, w.rowid",
                kind.to_column(),
                kind.table()
            ))
            .map_err(map_err)?;
        let rows = stmt
            .query_map([], |r| {
                let actor: String = r.get(3)?;
                Ok(PendingWake {
                    kind,
                    note: get_uuid(r, 0)?,
                    task: get_uuid(r, 1)?,
                    body: r.get(2)?,
                    // An unreadable word is nobody in particular: the wake
                    // still goes to the agent rather than being dropped.
                    actor: Actor::parse(&actor).unwrap_or(Actor::User),
                    enqueued_at: r.get(4)?,
                    claimed_at: r.get(5)?,
                    pasted_at: r.get(6)?,
                    to: r.get::<_, Option<Vec<u8>>>(7)?.and_then(|b| Uuid::from_slice(&b).ok()),
                })
            })
            .map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    /// Append a note and, when it's a person's answer on a board that wakes
    /// on answers, queue it to be told, in one transaction. The bool says
    /// whether it was queued. Otherwise as `add_note` and
    /// `add_note_superseding`.
    pub fn add_note_waking(
        &self,
        task: Uuid,
        kind: NoteKind,
        actor: Actor,
        body: &str,
        extra: serde_json::Value,
        supersedes: Option<Uuid>,
    ) -> Result<(TaskNote, bool)> {
        if kind != NoteKind::Answer || actor != Actor::User {
            let note = match supersedes {
                Some(s) => self.add_note_superseding(task, kind, actor, body, extra, s)?,
                None => self.add_note(task, kind, actor, body, extra)?,
            };
            return Ok((note, false));
        }
        if let Some(s) = supersedes {
            // The same checks `add_note_superseding` makes, before writing.
            let owner: Option<(Vec<u8>, String)> = {
                use rusqlite::OptionalExtension;
                self.conn()
                    .query_row("SELECT task_id, kind FROM task_notes WHERE id = ?1", params![uuid_blob(s)], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })
                    .optional()
                    .map_err(map_err)?
            };
            if !owner.is_some_and(|(t, k)| t == task.as_bytes().as_slice() && k == kind.as_str()) {
                return Err(farcooler_core::DomainError::InvalidArgument { what: "supersedes" });
            }
        }
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
        let Some(wakes) = wakes else { return Err(farcooler_core::DomainError::NotFound) };
        let note = insert_note(&tx, task, kind, actor, body, &extra, supersedes)?;
        if wakes {
            tx.execute(
                "INSERT OR IGNORE INTO answer_wakes (note_id, task_id, enqueued_at) VALUES (?1, ?2, ?3)",
                params![uuid_blob(note.id), uuid_blob(task), now_millis()],
            )
            .map_err(map_err)?;
        }
        tx.commit().map_err(map_err)?;
        Ok((note, wakes))
    }

    /// Claim `note`'s wake: say typing is about to begin. False when it was
    /// already claimed or done, and then nothing may be typed for it.
    pub fn claim_answer_wake(&self, note: Uuid) -> Result<bool> {
        self.claim(WakeKind::Answer, note)
    }

    /// `claim_answer_wake` for whichever queue `wake` came from.
    pub fn claim_wake(&self, wake: &PendingWake) -> Result<bool> {
        self.claim(wake.kind, wake.note)
    }

    fn claim(&self, kind: WakeKind, note: Uuid) -> Result<bool> {
        let claimed = self
            .conn()
            .execute(
                &format!(
                    "UPDATE {} SET claimed_at = ?2
                      WHERE note_id = ?1 AND claimed_at IS NULL AND done_at IS NULL",
                    kind.table()
                ),
                params![uuid_blob(note), now_millis()],
            )
            .map_err(map_err)?;
        Ok(claimed == 1)
    }

    /// Say `wake`'s text is in the box, its Enter held back by a dialog, at
    /// `at` (Unix milliseconds, when the paste began). False unless it's
    /// claimed and not done.
    pub fn mark_wake_pasted(&self, wake: &PendingWake, at: i64) -> Result<bool> {
        let marked = self
            .conn()
            .execute(
                &format!(
                    "UPDATE {} SET pasted_at = ?2
                      WHERE note_id = ?1 AND claimed_at IS NOT NULL AND done_at IS NULL",
                    wake.kind.table()
                ),
                params![uuid_blob(wake.note), at],
            )
            .map_err(map_err)?;
        Ok(marked == 1)
    }

    /// Take the pasted mark off `wake`, just before its Enter is pressed, so
    /// a crash from here on reads as one mid-typing and nothing is pressed
    /// again. False when it carried no mark, or is done, and then no Enter
    /// may be pressed for it.
    pub fn take_wake_paste(&self, wake: &PendingWake) -> Result<bool> {
        let taken = self
            .conn()
            .execute(
                &format!(
                    "UPDATE {} SET pasted_at = NULL
                      WHERE note_id = ?1 AND pasted_at IS NOT NULL AND done_at IS NULL",
                    wake.kind.table()
                ),
                params![uuid_blob(wake.note)],
            )
            .map_err(map_err)?;
        Ok(taken == 1)
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
        self.finish(WakeKind::Answer, note, record)
    }

    /// `finish_answer_wake` for whichever queue `wake` came from.
    pub fn finish_wake(&self, wake: &PendingWake, record: Option<&str>) -> Result<Option<Option<TaskNote>>> {
        self.finish(wake.kind, wake.note, record)
    }

    fn finish(&self, kind: WakeKind, note: Uuid, record: Option<&str>) -> Result<Option<Option<TaskNote>>> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let task: Option<Vec<u8>> = {
            use rusqlite::OptionalExtension;
            tx.query_row(
                &format!(
                    "UPDATE {} SET done_at = ?2 WHERE note_id = ?1 AND done_at IS NULL RETURNING task_id",
                    kind.table()
                ),
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

    /// A person's answer on a board that wakes is queued with the answer, in
    /// its transaction; anyone else's, or with the switch off, isn't. A claim
    /// is taken once.
    #[test]
    fn a_persons_answer_is_queued_with_it_and_claimed_once() {
        let (store, task) = board();
        let (note, queued) =
            store.add_note_waking(task, NoteKind::Answer, Actor::User, "Yes", serde_json::json!({}), None).unwrap();
        assert!(queued);
        assert_eq!(store.pending_answer_wakes().unwrap()[0].note, note.id);
        assert_eq!(store.pending_answer_wakes().unwrap()[0].claimed_at, None);
        assert!(store.claim_answer_wake(note.id).unwrap());
        assert!(!store.claim_answer_wake(note.id).unwrap(), "claimed twice");
        assert!(store.pending_answer_wakes().unwrap()[0].claimed_at.is_some());

        for actor in [Actor::Manager, Actor::Agent { terminal: Uuid::now_v7() }] {
            let (_, queued) =
                store.add_note_waking(task, NoteKind::Answer, actor, "No", serde_json::json!({}), None).unwrap();
            assert!(!queued, "{actor}");
        }
        let (_, queued) =
            store.add_note_waking(task, NoteKind::Comment, Actor::User, "Hm", serde_json::json!({}), None).unwrap();
        assert!(!queued, "a comment");
        let ws = store.get_task(task).unwrap().workspace_id;
        let version = store.get_workspace(ws).unwrap().resource_version;
        store.set_workspace_wake_on_answer(ws, version, false).unwrap();
        let (_, queued) =
            store.add_note_waking(task, NoteKind::Answer, Actor::User, "Off", serde_json::json!({}), Some(note.id)).unwrap();
        assert!(!queued, "switched off");
        assert_eq!(store.pending_answer_wakes().unwrap().len(), 1);
        assert!(store
            .add_note_waking(task, NoteKind::Answer, Actor::User, "x", serde_json::json!({}), Some(Uuid::now_v7()))
            .is_err());
    }

    /// The pasted mark (ov-385) goes only on a claimed row, reads back, and
    /// is taken off once: a second taker presses no Enter.
    #[test]
    fn a_paste_is_marked_on_a_claimed_row_and_taken_once() {
        let (store, task) = board();
        let (note, _) =
            store.add_note_waking(task, NoteKind::Answer, Actor::User, "Yes", serde_json::json!({}), None).unwrap();
        let wake = store.pending_answer_wakes().unwrap().remove(0);
        assert!(!store.mark_wake_pasted(&wake, 42).unwrap(), "marked before it was claimed");
        assert!(store.claim_answer_wake(note.id).unwrap());
        assert!(store.mark_wake_pasted(&wake, 42).unwrap());
        assert_eq!(store.pending_answer_wakes().unwrap()[0].pasted_at, Some(42));
        assert!(store.take_wake_paste(&wake).unwrap());
        assert!(!store.take_wake_paste(&wake).unwrap(), "taken twice");
        let after = &store.pending_answer_wakes().unwrap()[0];
        assert_eq!((after.pasted_at, after.claimed_at.is_some()), (None, true), "still claimed: never typed again");
        store.mark_wake_pasted(&wake, 43).unwrap();
        store.finish_wake(&wake, None).unwrap();
        assert!(!store.take_wake_paste(&wake).unwrap(), "taken once done");
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
