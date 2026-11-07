//! Registry 67: the minimal durable authority the engine needs for retained
//! communication. Old `message_wakes`, `answer_wakes` and `hold_wakes` rows stay
//! inert history; only a row whose note is bound here to the CURRENT released
//! incarnation may be inserted, claimed or settled (ov-940, ov-949). The three
//! queues keep their own claim and finish steps, which is what tells each wake
//! at most once; the binding says which rows the engine admitted.
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
    tx.execute_batch("
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
        DROP TRIGGER engine_fence_answer_wakes_i;
        DROP TRIGGER engine_fence_answer_wakes_u;
        DROP TRIGGER engine_fence_hold_wakes_i;
        DROP TRIGGER engine_fence_hold_wakes_u;")?;
    for table in ["message_wakes","answer_wakes","hold_wakes"] {
        tx.execute_batch(&format!("
        CREATE TRIGGER engine_fence_{table}_i BEFORE INSERT ON {table}
            WHEN (SELECT phase FROM engine_cutover_authority WHERE singleton=1)!='legacy' AND NOT {new}
            BEGIN SELECT RAISE(ABORT,'legacy execution authority retired'); END;
        CREATE TRIGGER engine_fence_{table}_u BEFORE UPDATE ON {table}
            WHEN (SELECT phase FROM engine_cutover_authority WHERE singleton=1)!='legacy' AND NOT {old}
            BEGIN SELECT RAISE(ABORT,'legacy execution authority retired'); END;",
            new = bound("NEW.note_id"), old = bound("OLD.note_id")))?;
    }
    Ok(())
}

/// SQL fragment over a queue aliased `alias`: which wakes the retained consumer
/// may see right now. While legacy, all of them; after release, only the ones
/// the engine bound to this incarnation. Everything queued before is history.
pub(crate) fn visible(alias: &str) -> String {
    format!("((SELECT phase FROM engine_cutover_authority WHERE singleton=1)='legacy'
    OR EXISTS(SELECT 1 FROM engine_manual_delivery d JOIN engine_cutover_authority a
        ON a.singleton=1 AND a.phase='released' AND a.incarnation=d.incarnation WHERE d.note_id={alias}.note_id))")
}

/// What the engine says about queueing a wake for `note` right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Admission {
    /// Before cutover: the old queue works as it always did.
    Legacy,
    /// Released: the note is now bound to this incarnation, and may be queued.
    Bound,
    /// Quiesced, committed, or another incarnation's: nothing may be queued.
    Closed,
}

/// Decide, and when released bind `note` to this incarnation, inside the
/// caller's transaction, so the binding and the wake commit or fail together.
pub(crate) fn admit(tx: &rusqlite::Connection, incarnation: u64, note: Uuid, task: Uuid) -> Result<Admission> {
    let (phase, current) = tx
        .query_row("SELECT phase, incarnation FROM engine_cutover_authority WHERE singleton=1", [], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))
        .map_err(map_err)?;
    if phase == "legacy" { return Ok(Admission::Legacy); }
    if phase != "released" || current != incarnation { return Ok(Admission::Closed); }
    tx.execute("INSERT INTO engine_manual_delivery(note_id,task_id,incarnation) VALUES(?1,?2,?3)", params![uuid_blob(note), uuid_blob(task), incarnation])
        .map_err(map_err)?;
    Ok(Admission::Bound)
}

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
    /// agent) is accepted here, never the runner. It is `send_message`, which binds
    /// and queues through the engine once released and refuses in the cutover window.
    pub fn send_manual_message(&self, task: Uuid, actor: Actor, body: &str, to: Option<Uuid>, extra: serde_json::Value) -> Result<TaskNote> {
        if actor == Actor::Runner { return Err(DomainError::InvalidArgument { what: "manual_actor" }); }
        self.send_message(task, actor, body, to, extra)
    }
}
