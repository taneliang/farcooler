//! Registry 67: the minimal durable authority the engine needs for retained
//! communication. Old `message_wakes` rows stay inert history; only a row bound
//! here to the CURRENT released incarnation may be inserted, claimed or settled.
//! Notice eligibility is a floor: pending outbox rows at or below it are
//! retained history and are never rediscovered by the consumer.
use rusqlite::{Transaction, params};
use uuid::Uuid;
use farcooler_core::{DomainError, Result};
use crate::error::map_err;
use crate::models::{Actor, TaskNote, uuid_blob};
use crate::store::Store;

pub(crate) fn migration(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    let bound = |note: &str| format!("EXISTS(SELECT 1 FROM engine_manual_delivery d JOIN engine_cutover_authority a
        ON a.singleton=1 AND a.phase='released' AND a.incarnation=d.incarnation WHERE d.note_id={note})");
    tx.execute_batch(&format!("
        CREATE TABLE engine_manual_delivery(note_id BLOB PRIMARY KEY, task_id BLOB NOT NULL, incarnation INTEGER NOT NULL);
        CREATE TRIGGER engine_manual_no_update BEFORE UPDATE ON engine_manual_delivery BEGIN SELECT RAISE(ABORT,'immutable manual delivery binding'); END;
        CREATE TRIGGER engine_manual_no_delete BEFORE DELETE ON engine_manual_delivery BEGIN SELECT RAISE(ABORT,'immutable manual delivery binding'); END;
        CREATE TRIGGER engine_manual_insert BEFORE INSERT ON engine_manual_delivery
            WHEN NOT EXISTS(SELECT 1 FROM engine_cutover_authority WHERE singleton=1 AND phase='released' AND incarnation=NEW.incarnation)
            BEGIN SELECT RAISE(ABORT,'manual delivery needs the released engine incarnation'); END;
        CREATE TABLE engine_notice_eligibility(singleton INTEGER PRIMARY KEY CHECK(singleton=1), floor INTEGER NOT NULL, incarnation INTEGER NOT NULL);
        CREATE TRIGGER engine_notice_eligibility_no_update BEFORE UPDATE ON engine_notice_eligibility BEGIN SELECT RAISE(ABORT,'immutable notice eligibility'); END;
        CREATE TRIGGER engine_notice_eligibility_no_delete BEFORE DELETE ON engine_notice_eligibility BEGIN SELECT RAISE(ABORT,'immutable notice eligibility'); END;
        DROP TRIGGER engine_fence_message_wakes_i;
        DROP TRIGGER engine_fence_message_wakes_u;
        CREATE TRIGGER engine_fence_message_wakes_i BEFORE INSERT ON message_wakes
            WHEN (SELECT phase FROM engine_cutover_authority WHERE singleton=1)!='legacy' AND NOT {new}
            BEGIN SELECT RAISE(ABORT,'legacy execution authority retired'); END;
        CREATE TRIGGER engine_fence_message_wakes_u BEFORE UPDATE ON message_wakes
            WHEN (SELECT phase FROM engine_cutover_authority WHERE singleton=1)!='legacy' AND NOT {old}
            BEGIN SELECT RAISE(ABORT,'legacy execution authority retired'); END;",
        new = bound("NEW.note_id"), old = bound("OLD.note_id")))
}

/// SQL fragment: which message wakes the retained consumer may see right now.
pub(crate) const VISIBLE_MESSAGE: &str = "((SELECT phase FROM engine_cutover_authority WHERE singleton=1)='legacy'
    OR EXISTS(SELECT 1 FROM engine_manual_delivery d JOIN engine_cutover_authority a
        ON a.singleton=1 AND a.phase='released' AND a.incarnation=d.incarnation WHERE d.note_id=w.note_id))";
/// Answers and ended holds are legacy progression: visible only while legacy.
pub(crate) const VISIBLE_LEGACY: &str = "(SELECT phase FROM engine_cutover_authority WHERE singleton=1)='legacy'";

/// Recorded in the SAME transaction as the engine commit.
pub(crate) fn record_notice_floor(tx: &Transaction<'_>, incarnation: u64) -> rusqlite::Result<()> {
    tx.execute("INSERT INTO engine_notice_eligibility SELECT 1, COALESCE(MAX(sequence),0), ?1 FROM workflow_notice_outbox", [incarnation])?;
    Ok(())
}

/// A notice may be prepared or admitted only above the cutover floor.
pub(crate) fn notice_eligible(tx: &rusqlite::Connection, id: &str) -> rusqlite::Result<bool> {
    tx.query_row("SELECT COALESCE((SELECT sequence FROM workflow_notice_outbox WHERE id=?1),0) > COALESCE((SELECT floor FROM engine_notice_eligibility WHERE singleton=1),0)
        AND EXISTS(SELECT 1 FROM workflow_notice_outbox WHERE id=?1)", [id], |r| r.get(0))
}

impl Store {
    /// A person's explicit manual message. The caller (Service) has already
    /// authenticated the actor; an authenticated caller's actor (person, member, manager or
    /// agent) is accepted here, never the runner. Under a released engine it writes the note, the
    /// binding to the current incarnation and the wake in one transaction;
    /// before cutover it is the ordinary `send_message`. Other phases refuse.
    pub fn send_manual_message(&self, task: Uuid, actor: Actor, body: &str, to: Option<Uuid>, extra: serde_json::Value) -> Result<TaskNote> {
        if actor == Actor::Runner { return Err(DomainError::InvalidArgument { what: "manual_actor" }); }
        let phase = crate::engine_cutover::authority(&self.conn()).map_err(|_| DomainError::OperationFailed)?;
        if phase.phase == "legacy" { return self.send_message(task, actor, body, to, extra); }
        if phase.phase != "released" || phase.incarnation != self.engine_incarnation { return Err(DomainError::Conflict { what: "engine_not_released" }); }
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(map_err)?;
        let wakes: Option<bool> = {
            use rusqlite::OptionalExtension;
            tx.query_row("SELECT w.wake_on_answer FROM tasks t JOIN workspaces w ON w.id = t.workspace_id WHERE t.id = ?1",
                params![uuid_blob(task)], |r| r.get(0)).optional().map_err(map_err)?
        };
        match wakes { None => return Err(DomainError::NotFound), Some(false) => return Err(DomainError::Conflict { what: "typing_off" }), Some(true) => {} }
        let note = crate::tasks::insert_note(&tx, task, crate::models::NoteKind::Comment, actor, body, &extra, None)?;
        tx.execute("INSERT INTO engine_manual_delivery(note_id,task_id,incarnation) VALUES(?1,?2,?3)",
            params![uuid_blob(note.id), uuid_blob(task), self.engine_incarnation]).map_err(map_err)?;
        tx.execute("INSERT INTO message_wakes (note_id, task_id, to_terminal, enqueued_at) VALUES (?1, ?2, ?3, ?4)",
            params![uuid_blob(note.id), uuid_blob(task), to.map(uuid_blob), crate::tasks::now_millis()]).map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        Ok(note)
    }
}
