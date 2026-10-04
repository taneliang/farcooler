//! `workspace.mark_read` and the read state `task.list` carries (ov-113).
//!
//! What has been read on a board lives on the runner, so every device
//! enrolled on it sees the same Unread. The store merges by max
//! (`farcooler_store::board_reads`); this is the wire half: ids out of bytes,
//! the runner's clock, and the announce.
//!
//! The announce is `board_reads_changed`, the whole board's state, and only
//! when a write changed something. It is not `task_changed`: no task moved,
//! and that event wakes managers and makes every board re-read.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1 as pb;
use farcooler_store::board_reads::{BoardReads, ReadsDelta};
use uuid::Uuid;

use crate::service::Service;
use crate::watch::Watcher;
use crate::wire::{id_bytes, parse_id};

pub(crate) fn to_pb(reads: &BoardReads) -> pb::BoardReads {
    pb::BoardReads {
        workspace_id: id_bytes(reads.workspace_id),
        floor_ms: reads.floor_ms,
        opened: reads
            .opened
            .iter()
            .map(|(task, ms)| pb::TaskRead { task_id: id_bytes(*task), opened_ms: *ms })
            .collect(),
    }
}

/// A board's read state, making its first look if it has none yet.
pub(crate) fn reads_of(svc: &Service, workspace: Uuid) -> Result<pb::BoardReads> {
    Ok(to_pb(&svc.store.board_reads(workspace, crate::review::now_millis())?))
}

/// `workspace.mark_read`: raise what is read on one board.
///
/// Answers with the board's state after the merge. `seeds_floor` is read by
/// no one: the floor merges by max like every other value, so a seed is a
/// plain raise and the outcome never depends on arrival order. Announces to every
/// connected client when anything changed, so a retried write, or one a
/// newer device already beat, is silent.
pub(crate) fn mark_read(svc: &Service, watcher: &Watcher, req: &pb::WorkspaceMarkRead) -> Result<pb::BoardReads> {
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
    let (reads, changed) = svc.store.merge_board_reads(workspace, &delta, crate::review::now_millis())?;
    let reads = to_pb(&reads);
    if changed {
        watcher.announce_board_reads(reads.clone());
    }
    Ok(reads)
}
