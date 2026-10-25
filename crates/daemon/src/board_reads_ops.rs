//! `workspace.mark_read` and the read state `task.list` carries (ov-113).
//!
//! What has been read on a board lives on the runner, so every device of one
//! person sees the same Unread, and per person (ov-740): a member's marks are
//! theirs, and leave the owner's alone (`people::reader_of`). The store merges
//! by max (`farcooler_store::board_reads`); this is the wire half: ids out of
//! bytes, the runner's clock, and the announce.
//!
//! The announce is `board_reads_changed`, the whole board's state for the
//! person who wrote, and only when a write changed something. It goes to that
//! person's connections alone (`notice_audience::admits_reads`). It is not
//! `task_changed`: no task moved, and that event wakes managers and makes
//! every board re-read.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1 as pb;
use farcooler_store::board_reads::{BoardReads, FIRST_LOOK_MS, ReadsDelta, Reader};
use uuid::Uuid;

use crate::people::{Person, reader_of};
use crate::service::Service;
use crate::watch::Watcher;
use crate::wire::{id_bytes, parse_id};

pub(crate) fn to_pb(reads: &BoardReads, reader: Reader) -> pb::BoardReads {
    pb::BoardReads {
        member: match reader {
            Reader::Owner => String::new(),
            Reader::Member(handle) => handle.to_string(),
        },
        workspace_id: id_bytes(reads.workspace_id),
        floor_ms: reads.floor_ms,
        opened: reads
            .opened
            .iter()
            .map(|(task, ms)| pb::TaskRead { task_id: id_bytes(*task), opened_ms: *ms })
            .collect(),
    }
}

/// `person`'s read state on a board, making their first look if they have
/// none yet. A released device has no marks and makes none: it reads a first
/// look, written nowhere.
pub(crate) fn reads_of(svc: &Service, workspace: Uuid, person: &Person) -> Result<pb::BoardReads> {
    let now = crate::review::now_millis();
    let Some(reader) = reader_of(person) else {
        let first = BoardReads { workspace_id: workspace, floor_ms: now.saturating_sub(FIRST_LOOK_MS), opened: Vec::new() };
        return Ok(to_pb(&first, Reader::Owner));
    };
    Ok(to_pb(&svc.store.board_reads_of(reader, workspace, now)?, reader))
}

/// `workspace.mark_read`: raise what `person` has read on one board.
///
/// Answers with their state after the merge. Every value merges by max, so
/// the outcome never depends on arrival order. Announces to their connected
/// clients when anything changed, so a retried write, or one a newer device
/// already beat, is silent. A released device's write is refused by the gate
/// before it gets here, and refused here too.
pub(crate) fn mark_read(svc: &Service, watcher: &Watcher, req: &pb::WorkspaceMarkRead, person: &Person) -> Result<pb::BoardReads> {
    let reader = reader_of(person).ok_or(DomainError::NotYours { what: "not_yours" })?;
    let workspace = parse_id(&req.workspace_id).ok_or(DomainError::NotFound)?;
    // The board must exist: a write names one, and there is no first look to
    // make for a board that isn't there.
    svc.store.get_workspace(workspace)?;
    let mut opened = Vec::with_capacity(req.opened.len());
    for mark in &req.opened {
        let task = parse_id(&mark.task_id).ok_or(DomainError::InvalidArgument { what: "task_ids" })?;
        opened.push((task, mark.opened_ms));
    }
    let delta = ReadsDelta { floor_ms: req.floor_ms, opened };
    let (reads, changed) = svc.store.merge_board_reads_of(reader, workspace, &delta, crate::review::now_millis())?;
    let reads = to_pb(&reads, reader);
    if changed {
        watcher.announce_board_reads(reads.clone());
    }
    Ok(reads)
}
