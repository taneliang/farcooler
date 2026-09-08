//! Board operations, one function per RPC method.
//!
//! Separate from `rpc.rs` so the dispatch table stays a table, and separate
//! from `farcooler_store::tasks` so the wire shapes stay out of the store.
//!
//! **The split this file exists to preserve.** Current understanding is mutable
//! and lives on the task row; the record of how you got there is append-only
//! and lives in typed notes that can be superseded but never edited. `update`
//! below revises the row. `note` appends. There is deliberately no function
//! here that changes a note, and there must never be one — `task_notes` carries
//! a `BEFORE UPDATE` trigger (`task_notes_forbid_update`) that refuses
//! unconditionally, so such a route would fail at runtime rather than at
//! review, on a caller's data.
//!
//! **Every write here announces.** A change no watcher observes leaves every
//! other client rendering the old state until something unrelated happens, and
//! for a status move there is not even a `resource_version` bump to notice —
//! `Store::set_task_status` deliberately does not bump one, so the announce is
//! the ONLY signal a client gets. The announce is at the foot of each write in
//! this file rather than in the dispatch arm so that a route cannot be added
//! without one being right there to copy.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1 as pb;
use farcooler_store::models::{
    AcceptanceItem, Actor, NoteHit, NoteKind, Task, TaskBlock, TaskNote, TaskStatus,
};
use uuid::Uuid;

use crate::service::Service;
use crate::watch::Watcher;
use crate::wire::id_bytes;

// ---------------------------------------------------------------------------
// wire conversions
// ---------------------------------------------------------------------------

fn pb_status(status: TaskStatus) -> i32 {
    (match status {
        TaskStatus::Backlog => pb::TaskStatus::Backlog,
        TaskStatus::Todo => pb::TaskStatus::Todo,
        TaskStatus::NeedsDecision => pb::TaskStatus::NeedsDecision,
        TaskStatus::InProgress => pb::TaskStatus::InProgress,
        TaskStatus::InReview => pb::TaskStatus::InReview,
        TaskStatus::Done => pb::TaskStatus::Done,
        TaskStatus::Cancelled => pb::TaskStatus::Cancelled,
    }) as i32
}

/// The status a request names, or `None` for UNSPECIFIED.
///
/// `None` is a legitimate answer only where the field is a FILTER — `task.list`
/// reads it as "every status", which is the store's own `Option<TaskStatus>`.
/// Where it is a value, `set_status` refuses it rather than defaulting; see
/// that function.
fn status_from_wire(raw: i32) -> Option<TaskStatus> {
    match pb::TaskStatus::try_from(raw).ok()? {
        pb::TaskStatus::Unspecified => None,
        pb::TaskStatus::Backlog => Some(TaskStatus::Backlog),
        pb::TaskStatus::Todo => Some(TaskStatus::Todo),
        pb::TaskStatus::NeedsDecision => Some(TaskStatus::NeedsDecision),
        pb::TaskStatus::InProgress => Some(TaskStatus::InProgress),
        pb::TaskStatus::InReview => Some(TaskStatus::InReview),
        pb::TaskStatus::Done => Some(TaskStatus::Done),
        pb::TaskStatus::Cancelled => Some(TaskStatus::Cancelled),
    }
}

fn pb_note_kind(kind: NoteKind) -> i32 {
    (match kind {
        NoteKind::Decision => pb::TaskNoteKind::Decision,
        NoteKind::Finding => pb::TaskNoteKind::Finding,
        NoteKind::Question => pb::TaskNoteKind::Question,
        NoteKind::Answer => pb::TaskNoteKind::Answer,
        NoteKind::Progress => pb::TaskNoteKind::Progress,
        NoteKind::Comment => pb::TaskNoteKind::Comment,
        NoteKind::StatusChange => pb::TaskNoteKind::StatusChange,
        NoteKind::Created => pb::TaskNoteKind::Created,
    }) as i32
}

/// The kind a request names, or `None` for UNSPECIFIED — a filter meaning
/// "every kind". `note` refuses `None` rather than picking one.
fn note_kind_from_wire(raw: i32) -> Option<NoteKind> {
    match pb::TaskNoteKind::try_from(raw).ok()? {
        pb::TaskNoteKind::Unspecified => None,
        pb::TaskNoteKind::Decision => Some(NoteKind::Decision),
        pb::TaskNoteKind::Finding => Some(NoteKind::Finding),
        pb::TaskNoteKind::Question => Some(NoteKind::Question),
        pb::TaskNoteKind::Answer => Some(NoteKind::Answer),
        pb::TaskNoteKind::Progress => Some(NoteKind::Progress),
        pb::TaskNoteKind::Comment => Some(NoteKind::Comment),
        pb::TaskNoteKind::StatusChange => Some(NoteKind::StatusChange),
        pb::TaskNoteKind::Created => Some(NoteKind::Created),
    }
}

/// Who a write says it is.
///
/// Empty is `user`: a call arriving with nobody named is a person at a client,
/// which is what `user` means, while `manager` and `agent:<uuid>` are claims
/// made out loud. A non-empty word that does not parse is REFUSED rather than
/// read as `user` — a typo in an agent's uuid must not file its work under a
/// person.
///
/// `Actor::parse` is the store's own reader, paired with the `Display` that
/// wrote the column and the event. There is no second stringifier here.
fn actor_from_wire(raw: &str) -> Result<Actor> {
    if raw.is_empty() {
        return Ok(Actor::User);
    }
    Actor::parse(raw).ok_or(DomainError::InvalidArgument { what: "actor" })
}

/// A uuid a request must carry. `NotFound` rather than `InvalidArgument`, the
/// same answer `Rpc::target` gives: to a caller, an id that cannot be read and
/// an id that names nothing are the same fact.
fn required_id(bytes: &[u8]) -> Result<Uuid> {
    crate::wire::parse_id(bytes).ok_or(DomainError::NotFound)
}

/// An optional uuid: absent stays absent, present must be readable.
///
/// Not silently dropped when unreadable. `workspace_id` is the lane a task is
/// using, and quietly clearing it because sixteen bytes were malformed would
/// unpick a link the caller believes it just made.
fn optional_id(bytes: Option<&bytes::Bytes>, what: &'static str) -> Result<Option<Uuid>> {
    match bytes {
        None => Ok(None),
        Some(raw) => {
            crate::wire::parse_id(raw).map(Some).ok_or(DomainError::InvalidArgument { what })
        }
    }
}

/// A note's kind-specific structure, as the store wants it.
///
/// Empty is `{}`. Anything that is not a JSON OBJECT is refused rather than
/// stored: `extra` is read by field name everywhere it is read, so a bare
/// string or an array is a value nothing downstream can ask a question of.
fn extra_from_wire(raw: &str) -> Result<serde_json::Value> {
    if raw.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(value @ serde_json::Value::Object(_)) => Ok(value),
        // Never the parser's message. A client shows its own sentence about
        // its own field; a serde error naming a byte offset is not one.
        _ => Err(DomainError::InvalidArgument { what: "extra_json" }),
    }
}

/// One line for the board.
///
/// Bounded because this is a column a person reads at a glance and an agent
/// writes unattended; two hundred scalars is longer than any title worth
/// having and short enough that a runaway prompt cannot make a row nobody can
/// render. `title` rather than `display_name` as the machine word, because the
/// field a client has to point its user at is called title.
fn checked_title(title: &str) -> Result<&str> {
    let trimmed = title.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 200 {
        return Err(DomainError::InvalidArgument { what: "title" });
    }
    Ok(trimmed)
}

fn pb_acceptance(item: &AcceptanceItem) -> pb::TaskAcceptanceItem {
    pb::TaskAcceptanceItem { id: id_bytes(item.id), text: item.text.clone(), met: item.met }
}

/// An acceptance item from the wire.
///
/// An empty `id` is a NEW item and gets one, so a client adding a line does not
/// have to mint a uuid to do it. A present id is kept, which is what lets a
/// client tick a box without the item becoming a different item.
fn acceptance_from_wire(item: &pb::TaskAcceptanceItem) -> Result<AcceptanceItem> {
    let id = if item.id.is_empty() {
        Uuid::now_v7()
    } else {
        crate::wire::parse_id(&item.id)
            .ok_or(DomainError::InvalidArgument { what: "acceptance" })?
    };
    Ok(AcceptanceItem { id, text: item.text.clone(), met: item.met })
}

fn pb_task(task: &Task) -> pb::Task {
    pb::Task {
        id: id_bytes(task.id),
        resource_version: task.resource_version,
        repository_id: id_bytes(task.repository_id),
        key: task.key.clone(),
        title: task.title.clone(),
        status: pb_status(task.status),
        status_since: task.status_since,
        intent: task.intent.clone(),
        acceptance: task.acceptance.iter().map(pb_acceptance).collect(),
        constraints: task.constraints.clone(),
        workspace_id: task.workspace_id.map(id_bytes),
        labels: task.labels.clone(),
    }
}

fn pb_note(note: &TaskNote) -> pb::TaskNote {
    pb::TaskNote {
        id: id_bytes(note.id),
        task_id: id_bytes(note.task_id),
        kind: pb_note_kind(note.kind),
        // The store's own `Display`, which is also what `TaskChanged.actor`
        // carries. One stringifier for one fact.
        actor: note.actor.to_string(),
        at: note.at,
        body: note.body.clone(),
        extra_json: note.extra.to_string(),
        supersedes: note.supersedes.map(id_bytes),
    }
}

fn pb_block(block: &TaskBlock) -> pb::TaskBlock {
    pb::TaskBlock {
        task_id: id_bytes(block.task_id),
        blocked_by: id_bytes(block.blocked_by),
        reason: block.reason.clone(),
    }
}

fn pb_hit(hit: &NoteHit) -> pb::TaskNoteHit {
    pb::TaskNoteHit { note: Some(pb_note(&hit.note)), superseded: hit.superseded }
}

/// Say that a task moved, naming who moved it.
///
/// Every write in this file ends here. See the module doc for why it is not
/// left to the dispatch arm.
fn announce(watcher: &Watcher, task: &Task, actor: Actor) {
    watcher.announce_task_changed(task.id, task.repository_id, actor);
}

// ---------------------------------------------------------------------------
// reads
// ---------------------------------------------------------------------------

/// `task.list`: a repository's board, or the part of it that has gone quiet.
///
/// `stale_after_millis` swaps the question rather than filtering the answer,
/// because staleness is a different query with a different order — oldest
/// sitting first, `done` and `cancelled` excluded outright. A staleness view
/// that listed every finished task beside the ones needing attention is a view
/// nobody reads.
pub fn list(svc: &Service, req: &pb::TaskListRequest) -> Result<pb::TaskList> {
    let repository = required_id(&req.repository_id)?;
    let tasks = match req.stale_after_millis.filter(|ms| *ms > 0) {
        Some(millis) => svc
            .store
            .list_tasks_stale_for(repository, std::time::Duration::from_millis(millis))?,
        None => svc.store.list_tasks(repository, status_from_wire(req.status))?,
    };
    Ok(pb::TaskList { items: tasks.iter().map(pb_task).collect() })
}

/// `task.get`: one task, its record and what it waits on, in a single read.
pub fn get(svc: &Service, req: &pb::TaskGetRequest) -> Result<pb::TaskDetail> {
    let id = required_id(&req.task_id)?;
    // Before the notes, so a task that does not exist answers `NotFound`
    // rather than a detail with an empty record in it.
    let task = svc.store.get_task(id)?;
    let notes = svc.store.notes_for(id, note_kind_from_wire(req.note_kind))?;
    let blocks = svc.store.blocks_for(id)?;
    Ok(pb::TaskDetail {
        task: Some(pb_task(&task)),
        notes: notes.iter().map(pb_note).collect(),
        blocks: blocks.iter().map(pb_block).collect(),
    })
}

/// `task.search`: every note in a repository whose body carries a phrase.
pub fn search(svc: &Service, req: &pb::TaskSearchRequest) -> Result<pb::TaskNoteHitList> {
    let repository = required_id(&req.repository_id)?;
    // An empty query matches every note in the repository, which is a whole
    // history dumped down a socket by a client that almost certainly sent an
    // empty search box by accident.
    if req.query.trim().is_empty() {
        return Err(DomainError::InvalidArgument { what: "query" });
    }
    let hits = svc.store.search_notes(repository, &req.query, note_kind_from_wire(req.kind))?;
    Ok(pb::TaskNoteHitList { items: hits.iter().map(pb_hit).collect() })
}

// ---------------------------------------------------------------------------
// writes
// ---------------------------------------------------------------------------

/// `task.create`: a new card in the backlog, with whatever is understood so
/// far already on it.
///
/// Two store calls, because `Store::create_task` takes only a title — it is
/// the call that mints the key and writes the `Created` note, and the rest of
/// the row is a revision like any other. One RPC rather than two so that a
/// client never has to show a card that exists with no intent on it; one
/// announce, at the end, because those two writes are one act.
///
/// **Every conversion that can fail runs BEFORE the first write, and that
/// ordering is the point.** Validating a field after `create_task` has
/// committed is not a rare interleaving — a client reaches it deterministically
/// by sending a malformed acceptance-item id or `workspace_id`, and what it
/// leaves behind is worse than the error it gets back: a titled task with a
/// `Created` note and no intent, sitting on the board, with NO announce, so
/// nothing tells any client it appeared. The caller reads `InvalidArgument`,
/// fixes its request, retries, and now there are two — the first one having
/// burned a key number. So a malformed request is refused having written
/// nothing at all.
///
/// The window this does NOT close is `update_task` failing after `create_task`
/// succeeded. That is left open deliberately: closing it needs a store call
/// that writes the whole row at once, which is a change to `farcooler-store`
/// rather than to this seam.
pub fn create(svc: &Service, watcher: &Watcher, req: &pb::TaskCreate) -> Result<pb::Task> {
    let repository = required_id(&req.repository_id)?;
    let actor = actor_from_wire(&req.actor)?;
    let update = farcooler_store::models::TaskUpdate {
        title: checked_title(&req.title)?.to_string(),
        intent: req.intent.clone(),
        acceptance: req
            .acceptance
            .iter()
            .map(acceptance_from_wire)
            .collect::<Result<Vec<_>>>()?,
        constraints: req.constraints.clone(),
        labels: req.labels.clone(),
        workspace_id: optional_id(req.workspace_id.as_ref(), "workspace_id")?,
    };

    // Nothing above this line has written anything.
    let created = svc.store.create_task(repository, &update.title, actor)?;
    // Skipped when there is nothing to revise, so the common case is one write
    // and the task comes back at version 1 rather than at 2 for no reason.
    let task = if update == blank_update(&created.title) {
        created
    } else {
        svc.store.update_task(created.id, created.resource_version, &update)?
    };

    announce(watcher, &task, actor);
    Ok(pb_task(&task))
}

/// What a freshly created task's revisable half already is, so `create` can
/// tell "nothing else was sent" from "everything else was sent empty on
/// purpose" — which are the same thing here, and both mean do not write again.
fn blank_update(title: &str) -> farcooler_store::models::TaskUpdate {
    farcooler_store::models::TaskUpdate {
        title: title.to_string(),
        intent: String::new(),
        acceptance: Vec::new(),
        constraints: Vec::new(),
        labels: Vec::new(),
        workspace_id: None,
    }
}

/// `task.update`: revise what is currently understood.
///
/// The mutable half of the design, and the whole of it: this writes no note,
/// deliberately. Intent and acceptance are meant to be rewritten as
/// understanding improves, and a log entry per wording change would bury the
/// reasoning the record exists to keep.
///
/// A whole revision, not a patch. Every field replaces what was there, which
/// is why `expected_version` is not optional: two clients revising the same
/// fields must not both believe they won.
pub fn update(svc: &Service, watcher: &Watcher, req: &pb::TaskUpdate) -> Result<pb::Task> {
    let id = required_id(&req.task_id)?;
    let actor = actor_from_wire(&req.actor)?;
    let update = farcooler_store::models::TaskUpdate {
        title: checked_title(&req.title)?.to_string(),
        intent: req.intent.clone(),
        acceptance: req
            .acceptance
            .iter()
            .map(acceptance_from_wire)
            .collect::<Result<Vec<_>>>()?,
        constraints: req.constraints.clone(),
        labels: req.labels.clone(),
        workspace_id: optional_id(req.workspace_id.as_ref(), "workspace_id")?,
    };
    let task = svc.store.update_task(id, req.expected_version, &update)?;
    announce(watcher, &task, actor);
    Ok(pb_task(&task))
}

/// `task.set_status`: move a task, and record the move in the same
/// transaction.
///
/// UNSPECIFIED is refused rather than defaulted. A client that failed to set
/// this field means "I do not know", and the honest answer to that is not
/// `BACKLOG` — which would quietly move a finished task back onto the board.
///
/// This is the write that most needs its announce. `Store::set_task_status`
/// deliberately does not bump `resource_version`, so a client that refetches
/// on a version change learns nothing at all from a move; the event is the
/// only signal there is.
pub fn set_status(svc: &Service, watcher: &Watcher, req: &pb::TaskSetStatus) -> Result<pb::Task> {
    let id = required_id(&req.task_id)?;
    let actor = actor_from_wire(&req.actor)?;
    let status =
        status_from_wire(req.status).ok_or(DomainError::InvalidArgument { what: "status" })?;
    let task = svc.store.set_task_status(id, status, actor)?;
    announce(watcher, &task, actor);
    Ok(pb_task(&task))
}

/// `task.note`: append one entry to a task's record.
///
/// There is no counterpart that edits one, and there must never be. Correcting
/// the record is this call again with `supersedes` set: both entries stay
/// readable forever, because "we decided X, then learned better and decided Y"
/// is what a reader in three weeks needs to see.
///
/// `STATUS_CHANGE` and `CREATED` are refused by the store, not here — they are
/// written by the transactions that actually move or make a task, and a caller
/// appending one would be writing a claim no move ever produced.
pub fn note(svc: &Service, watcher: &Watcher, req: &pb::TaskNoteAppend) -> Result<pb::TaskNote> {
    let id = required_id(&req.task_id)?;
    let actor = actor_from_wire(&req.actor)?;
    let kind =
        note_kind_from_wire(req.kind).ok_or(DomainError::InvalidArgument { what: "kind" })?;
    let extra = extra_from_wire(&req.extra_json)?;
    if req.body.trim().is_empty() {
        return Err(DomainError::InvalidArgument { what: "body" });
    }

    let written = match optional_id(req.supersedes.as_ref(), "supersedes")? {
        Some(superseded) => {
            svc.store.add_note_superseding(id, kind, actor, &req.body, extra, superseded)?
        }
        None => svc.store.add_note(id, kind, actor, &req.body, extra)?,
    };

    // The task rather than the note, because that is what a client re-reads —
    // and read after the append, so a `QUESTION` that moved nothing and one
    // that will be answered later look the same to a watcher.
    let task = svc.store.get_task(id)?;
    announce(watcher, &task, actor);
    Ok(pb_note(&written))
}

/// `task.block`: record that a task is waiting on another, or clear that.
///
/// One route for both directions; see `TaskBlockSet` in the proto for why.
/// Answers with everything the task is waiting on rather than with the one
/// edge, so a client renders the whole answer from one reply.
///
/// A cycle is refused by the store. A deadlock the manager would never resolve
/// presents as a queue that quietly stopped moving rather than as an error,
/// which is exactly the failure this board exists to make visible.
pub fn block(
    svc: &Service,
    watcher: &Watcher,
    req: &pb::TaskBlockSet,
) -> Result<pb::TaskBlockList> {
    let id = required_id(&req.task_id)?;
    let blocked_by = required_id(&req.blocked_by)?;
    let actor = actor_from_wire(&req.actor)?;

    if req.clear {
        svc.store.unblock(id, blocked_by)?;
    } else {
        svc.store.set_block(id, blocked_by, &req.reason)?;
    }

    let blocks = svc.store.blocks_for(id)?;
    // Read rather than assumed: `unblock` succeeds on an edge that was never
    // there, and the task itself may have gone in the meantime, which is a
    // `NotFound` a caller should see rather than a silent success.
    let task = svc.store.get_task(id)?;
    announce(watcher, &task, actor);
    Ok(pb::TaskBlockList { items: blocks.iter().map(pb_block).collect() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unnamed_actor_is_the_person_at_the_client() {
        assert_eq!(actor_from_wire("").unwrap(), Actor::User);
        assert_eq!(actor_from_wire("manager").unwrap(), Actor::Manager);
        let terminal = Uuid::now_v7();
        assert_eq!(
            actor_from_wire(&format!("agent:{terminal}")).unwrap(),
            Actor::Agent { terminal }
        );
    }

    #[test]
    fn a_word_that_does_not_parse_is_refused_rather_than_read_as_a_person() {
        // The failure this guards: an agent whose uuid was mistyped filing its
        // decisions under `user`, where nothing in the record would ever say
        // otherwise.
        for raw in ["agent:not-a-uuid", "agent:", "AGENT", "robot", " user"] {
            assert!(
                matches!(
                    actor_from_wire(raw),
                    Err(DomainError::InvalidArgument { what: "actor" })
                ),
                "{raw} was accepted"
            );
        }
    }

    #[test]
    fn an_unspecified_enum_is_a_filter_and_never_a_value() {
        // Both halves of the rule the proto states. `None` here is what
        // `list` and `get` pass to the store as "no narrowing"; `set_status`
        // and `note` turn the same `None` into a refusal.
        assert_eq!(status_from_wire(pb::TaskStatus::Unspecified as i32), None);
        assert_eq!(note_kind_from_wire(pb::TaskNoteKind::Unspecified as i32), None);
        assert_eq!(status_from_wire(pb::TaskStatus::Done as i32), Some(TaskStatus::Done));
        assert_eq!(
            note_kind_from_wire(pb::TaskNoteKind::Decision as i32),
            Some(NoteKind::Decision)
        );
        // A number no version of this enum has ever defined. Refused, not
        // defaulted: a newer client naming a status this build does not know
        // must not have it silently read as the backlog.
        assert_eq!(status_from_wire(9999), None);
        assert_eq!(note_kind_from_wire(9999), None);
    }

    #[test]
    fn every_status_and_kind_survives_the_round_trip() {
        // A pair of tables mapped by hand in two directions, which is exactly
        // where an arm gets copied onto the wrong neighbor.
        for status in [
            TaskStatus::Backlog,
            TaskStatus::Todo,
            TaskStatus::NeedsDecision,
            TaskStatus::InProgress,
            TaskStatus::InReview,
            TaskStatus::Done,
            TaskStatus::Cancelled,
        ] {
            assert_eq!(status_from_wire(pb_status(status)), Some(status), "{status:?}");
        }
        for kind in [
            NoteKind::Decision,
            NoteKind::Finding,
            NoteKind::Question,
            NoteKind::Answer,
            NoteKind::Progress,
            NoteKind::Comment,
            NoteKind::StatusChange,
            NoteKind::Created,
        ] {
            assert_eq!(note_kind_from_wire(pb_note_kind(kind)), Some(kind), "{kind:?}");
        }
    }

    #[test]
    fn a_notes_structure_has_to_be_an_object() {
        assert_eq!(extra_from_wire("").unwrap(), serde_json::json!({}));
        assert_eq!(extra_from_wire("   ").unwrap(), serde_json::json!({}));
        assert_eq!(
            extra_from_wire(r#"{"rejected":["files"]}"#).unwrap(),
            serde_json::json!({ "rejected": ["files"] })
        );
        // Read by field name everywhere it is read, so anything without fields
        // is a value nothing downstream can ask a question of.
        for raw in ["[1,2]", "\"a string\"", "7", "{not json"] {
            assert!(
                matches!(
                    extra_from_wire(raw),
                    Err(DomainError::InvalidArgument { what: "extra_json" })
                ),
                "{raw} was accepted"
            );
        }
    }

    #[test]
    fn a_title_is_one_line_and_has_to_be_one() {
        assert_eq!(checked_title("  fix the thing  ").unwrap(), "fix the thing");
        for raw in ["", "   ", "\n\t"] {
            assert!(matches!(
                checked_title(raw),
                Err(DomainError::InvalidArgument { what: "title" })
            ));
        }
        // Bounded, because an agent writes these unattended.
        assert!(checked_title(&"a".repeat(200)).is_ok());
        assert!(checked_title(&"a".repeat(201)).is_err());
    }

    #[test]
    fn a_new_acceptance_item_gets_an_id_and_an_existing_one_keeps_its_own() {
        let fresh = acceptance_from_wire(&pb::TaskAcceptanceItem {
            id: bytes::Bytes::new(),
            text: "the suite is green".into(),
            met: false,
        })
        .unwrap();
        assert!(!fresh.id.is_nil(), "a new line is mintable without the client minting it");

        let held = Uuid::now_v7();
        let kept = acceptance_from_wire(&pb::TaskAcceptanceItem {
            id: id_bytes(held),
            text: "the suite is green".into(),
            met: true,
        })
        .unwrap();
        // Ticking a box must not make it a different box.
        assert_eq!(kept.id, held);
        assert!(kept.met);

        // Sixteen bytes that are not a uuid are a mistake, not a new item.
        assert!(acceptance_from_wire(&pb::TaskAcceptanceItem {
            id: bytes::Bytes::from_static(b"nope"),
            text: "x".into(),
            met: false,
        })
        .is_err());
    }

    #[test]
    fn an_unreadable_optional_id_is_refused_rather_than_dropped() {
        assert_eq!(optional_id(None, "workspace_id").unwrap(), None);
        let held = Uuid::now_v7();
        assert_eq!(optional_id(Some(&id_bytes(held)), "workspace_id").unwrap(), Some(held));
        // Quietly clearing the lane because sixteen bytes were malformed would
        // unpick a link the caller believes it just made.
        assert!(matches!(
            optional_id(Some(&bytes::Bytes::from_static(b"nope")), "workspace_id"),
            Err(DomainError::InvalidArgument { what: "workspace_id" })
        ));
    }
}
