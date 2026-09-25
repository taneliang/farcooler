//! What a board looks like once it leaves the wire.
//!
//! One implementation, two callers, for `changes_json`'s reason. The Mac reads
//! a board by shelling out to `farcooler task list --json` and `task show
//! --json` (`crates/cli/src/tasks.rs`); the phones read the same board through
//! the FFI's `task.list` and `task.get` (`session.rs`). AgentKit decodes both
//! with ONE decoder, `TaskBoardModel.decode` and `TaskDetailModel.decode`, so
//! the two producers must not be two copies that are merely meant to agree.
//! `changes_json`'s own header is the record of what that costs: a key added
//! for one client that never reached the other, with every test green.
//!
//! Keys are snake_case, unlike the rest of the FFI's camelCase, because they
//! are the CLI's and have been since before a phone could read a board: an
//! agent parses `task list --json`, and renaming its keys to match the FFI
//! would break every script and skill that reads them.

use farcooler_protocol::v1 as pb;
use serde_json::json;

use crate::session::{short, uuid_of};

/// A status as the wire's enum, in the one vocabulary every client reads:
/// the proto's `TASK_STATUS_*` names, lowercased. `unknown` for a number this
/// build does not define, never a guess — a runner newer than this client
/// naming a status it has not heard of must not have a finished task printed
/// as unstarted. `crates/cli/src/tasks.rs` checks these words against the
/// store's own `TaskStatus::as_str`.
pub fn status_word(raw: i32) -> &'static str {
    match pb::TaskStatus::try_from(raw) {
        Ok(pb::TaskStatus::Backlog) => "backlog",
        Ok(pb::TaskStatus::Todo) => "todo",
        Ok(pb::TaskStatus::NeedsDecision) => "needs_decision",
        Ok(pb::TaskStatus::InProgress) => "in_progress",
        Ok(pb::TaskStatus::InReview) => "in_review",
        Ok(pb::TaskStatus::Done) => "done",
        Ok(pb::TaskStatus::Cancelled) => "cancelled",
        Ok(pb::TaskStatus::Unspecified) | Err(_) => "unknown",
    }
}

/// A note's kind, on `status_word`'s terms.
pub fn note_kind_word(raw: i32) -> &'static str {
    match pb::TaskNoteKind::try_from(raw) {
        Ok(pb::TaskNoteKind::Decision) => "decision",
        Ok(pb::TaskNoteKind::Finding) => "finding",
        Ok(pb::TaskNoteKind::Question) => "question",
        Ok(pb::TaskNoteKind::Answer) => "answer",
        Ok(pb::TaskNoteKind::Progress) => "progress",
        Ok(pb::TaskNoteKind::Comment) => "comment",
        Ok(pb::TaskNoteKind::StatusChange) => "status_change",
        Ok(pb::TaskNoteKind::Created) => "created",
        Ok(pb::TaskNoteKind::Unspecified) | Err(_) => "unknown",
    }
}

/// How long a task has sat where it is. Never negative: a runner whose clock is
/// a little ahead of this one has not moved a task in the future.
pub fn stale_for_seconds(status_since: i64, now: i64) -> i64 {
    now.saturating_sub(status_since).max(0) / 1000
}

/// The board: `{"tasks": [...]}`, the shape `task list --json` prints and the
/// FFI's `task.list` returns.
pub fn list_json(tasks: &[pb::Task], now: i64) -> serde_json::Value {
    json!({ "tasks": tasks.iter().map(|t| task_json(t, now)).collect::<Vec<_>>() })
}

/// One task with its record: the shape `task show --json` prints and the FFI's
/// `task.get` returns.
pub fn detail_json(detail: &pb::TaskDetail, now: i64) -> serde_json::Value {
    json!({
        "task": detail.task.as_ref().map(|t| task_json(t, now)),
        "notes": detail.notes.iter().map(note_json).collect::<Vec<_>>(),
        "blocks": detail.blocks.iter().map(block_json).collect::<Vec<_>>(),
    })
}

/// One row. `now` is Unix milliseconds, passed in so a test can pin it.
pub fn task_json(task: &pb::Task, now: i64) -> serde_json::Value {
    json!({
        "id": uuid_of(&task.id).to_string(),
        "short": short(&task.id),
        "repository_id": uuid_of(&task.repository_id).to_string(),
        "resource_version": task.resource_version,
        "key": task.key,
        "title": task.title,
        "status": status_word(task.status),
        "status_since": task.status_since,
        "stale_for_seconds": stale_for_seconds(task.status_since, now),
        "intent": task.intent,
        "acceptance": task.acceptance.iter().map(|a| json!({
            "id": uuid_of(&a.id).to_string(),
            "text": a.text,
            "met": a.met,
        })).collect::<Vec<_>>(),
        "constraints": task.constraints,
        "labels": task.labels,
        "workspace_id": task.workspace_id.as_ref().map(|b| uuid_of(b).to_string()),
    })
}

pub fn note_json(note: &pb::TaskNote) -> serde_json::Value {
    json!({
        "id": uuid_of(&note.id).to_string(),
        "short": short(&note.id),
        "task_id": uuid_of(&note.task_id).to_string(),
        "kind": note_kind_word(note.kind),
        "actor": note.actor,
        "at": note.at,
        "body": note.body,
        // Parsed rather than passed through as a string, so a reader does not
        // have to decode JSON a second time to reach a decision's rejected
        // alternatives.
        "extra": parsed_extra(&note.extra_json),
        "supersedes": note.supersedes.as_ref().map(|b| uuid_of(b).to_string()),
    })
}

pub fn block_json(block: &pb::TaskBlock) -> serde_json::Value {
    json!({
        "task_id": uuid_of(&block.task_id).to_string(),
        "blocked_by": uuid_of(&block.blocked_by).to_string(),
        "short": short(&block.blocked_by),
        "reason": block.reason,
    })
}

/// A note's structured half, or `{}` when it is not an object's worth of JSON.
pub fn parsed_extra(raw: &str) -> serde_json::Value {
    serde_json::from_str(raw).unwrap_or_else(|_| json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> bytes::Bytes {
        bytes::Bytes::copy_from_slice(&[n; 16])
    }

    /// Every key AgentKit's `WireTask` reads, by the name it reads it under.
    /// The decoder is the other half of this contract, and it is in Swift, so
    /// the names are pinned here where a rename on this side goes red.
    #[test]
    fn a_row_carries_every_key_the_board_decodes() {
        let task = pb::Task {
            id: id(1),
            repository_id: id(2),
            key: "-20".into(),
            title: "A task board on iOS".into(),
            status: pb::TaskStatus::NeedsDecision as i32,
            status_since: 1_000,
            intent: "why".into(),
            acceptance: vec![
                pb::TaskAcceptanceItem { id: id(3), text: "one".into(), met: true },
                pb::TaskAcceptanceItem { id: id(4), text: "two".into(), met: false },
            ],
            labels: vec!["ios".into()],
            workspace_id: Some(id(5)),
            ..Default::default()
        };
        let row = task_json(&task, 61_000);
        assert_eq!(row["key"], "-20");
        assert_eq!(row["status"], "needs_decision");
        assert_eq!(row["status_since"], 1_000);
        assert_eq!(row["stale_for_seconds"], 60);
        assert_eq!(row["acceptance"][0]["met"], true);
        assert_eq!(row["acceptance"][1]["text"], "two");
        assert_eq!(row["labels"][0], "ios");
        assert_eq!(row["workspace_id"], uuid_of(&id(5)).to_string());
        assert_eq!(row["id"], uuid_of(&id(1)).to_string());
        for key in ["intent", "constraints", "title", "short", "repository_id"] {
            assert!(row.get(key).is_some(), "{key} is missing");
        }
    }

    #[test]
    fn a_status_this_build_does_not_define_is_unknown_not_a_guess() {
        assert_eq!(status_word(9_999), "unknown");
        assert_eq!(status_word(pb::TaskStatus::Unspecified as i32), "unknown");
        assert_eq!(note_kind_word(9_999), "unknown");
    }

    #[test]
    fn a_clock_ahead_of_this_one_is_not_a_negative_age() {
        assert_eq!(stale_for_seconds(10_000, 5_000), 0);
    }
}
