//! Read state on the runner, as an app sees it (ov-113).
//!
//! A board's read state (`BoardReads`) comes back with every `task.list` that
//! names a workspace, arrives as a `reads` event when another device changes
//! it, and is raised with `workspace.mark_read`. Every time in it is the
//! runner's clock, and every part only rises, so an app merges what it is told
//! with what it holds and takes the larger.
//!
//! Both halves are additive for an older app: it never decodes `reads`, and an
//! older runner has no `board_reads` capability, which an app reads as "keep
//! the per-device store".

use farcooler_protocol::v1 as pb;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Session, SessionError, request, require, result, uuid_of, wrong};

/// What an app raises on one board: `workspace.mark_read`'s request, before
/// it is bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarkRead {
    /// Mark All as Read. `None` for opening a ticket.
    pub floor_ms: Option<i64>,
    /// Tickets opened: the task and the runner-clock time seen through.
    pub opened: Vec<(Uuid, i64)>,
    /// The floor is this device's pre-sync one. The runner merges it by max
    /// like any floor; see `WorkspaceMarkRead`.
    pub seeds_floor: bool,
}

/// A board's read state as JSON, in the shape the apps and the CLI share:
/// `{"workspace_id", "floor_ms", "opened": [{"task_id", "opened_ms"}]}`.
pub fn reads_json(reads: &pb::BoardReads) -> Value {
    json!({
        "workspace_id": uuid_of(&reads.workspace_id).to_string(),
        "floor_ms": reads.floor_ms,
        "opened": reads.opened.iter().map(|m| json!({
            "task_id": uuid_of(&m.task_id).to_string(),
            "opened_ms": m.opened_ms,
        })).collect::<Vec<_>>(),
    })
}

impl MarkRead {
    pub(crate) fn into_wire(self, workspace: Uuid) -> pb::WorkspaceMarkRead {
        pb::WorkspaceMarkRead {
            workspace_id: bytes::Bytes::copy_from_slice(workspace.as_bytes()),
            floor_ms: self.floor_ms,
            opened: self
                .opened
                .into_iter()
                .map(|(task, ms)| pb::TaskRead {
                    task_id: bytes::Bytes::copy_from_slice(task.as_bytes()),
                    opened_ms: ms,
                })
                .collect(),
            seeds_floor: self.seeds_floor,
        }
    }
}

impl Session {
    /// Raise what is read on one board, and answer with the board's state
    /// after the merge, as `reads_json` shapes it.
    ///
    /// Refused here, without a round trip, on a runner that does not
    /// advertise `board_reads`: an app keeps its own store there.
    pub async fn mark_read(&self, workspace: Uuid, raise: MarkRead) -> Result<Value, SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::BOARD_READS, "workspace.mark_read")?;
        let payload = request::Payload::WorkspaceMarkRead(raise.into_wire(workspace));
        match self.value("workspace.mark_read", Some(workspace), Some(payload)).await? {
            result::Value::BoardReads(r) => Ok(reads_json(&r)),
            other => Err(wrong("board_reads", &other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_boards_reads_are_json_with_uuid_strings() {
        let (ws, task) = (Uuid::now_v7(), Uuid::now_v7());
        let wire = MarkRead { floor_ms: Some(5), opened: vec![(task, 9)], seeds_floor: true }.into_wire(ws);
        assert_eq!(wire.floor_ms, Some(5));
        assert!(wire.seeds_floor);
        let reads = pb::BoardReads { workspace_id: wire.workspace_id.clone(), floor_ms: 5, opened: wire.opened };
        assert_eq!(
            reads_json(&reads),
            json!({
                "workspace_id": ws.to_string(),
                "floor_ms": 5,
                "opened": [{ "task_id": task.to_string(), "opened_ms": 9 }],
            })
        );
    }

    /// The event carries the whole state, with each id and time as the runner
    /// sent them.
    #[test]
    fn the_event_carries_the_boards_state() {
        use farcooler_protocol::v1::event::Payload;
        let (ws, task) = (Uuid::now_v7(), Uuid::now_v7());
        let news = super::super::FleetEvent::of(Payload::BoardReadsChanged(pb::BoardReads {
            workspace_id: bytes::Bytes::copy_from_slice(ws.as_bytes()),
            floor_ms: 4,
            opened: vec![pb::TaskRead { task_id: bytes::Bytes::copy_from_slice(task.as_bytes()), opened_ms: 8 }],
        }));
        assert_eq!(
            news,
            Some(super::super::FleetEvent::Reads { workspace: ws, floor_ms: 4, opened: vec![(task, 8)] })
        );
    }

    /// A runner without `board_reads` is refused without a round trip, as a
    /// capability the app words as "keep your own store".
    #[test]
    fn an_older_runner_is_refused_before_the_wire() {
        let older = vec!["workspaces".to_string(), "tasks".to_string(), "workstreams".to_string()];
        let refused = require(&older, farcooler_protocol::capability::BOARD_READS, "workspace.mark_read");
        match refused {
            Err(SessionError::Refused { code, retryable, .. }) => {
                assert_eq!(code, pb::ErrorCode::CapabilityUnsupported as i32);
                assert!(!retryable);
            }
            other => panic!("{other:?}"),
        }
        let newer = vec!["board_reads".to_string()];
        assert!(require(&newer, farcooler_protocol::capability::BOARD_READS, "workspace.mark_read").is_ok());
    }

    /// `tasks` is untouched, and `reads` is there only when the runner sent it.
    #[test]
    fn a_board_names_its_reads_beside_its_tasks_only_when_it_has_them() {
        let with = pb::TaskList {
            items: vec![],
            reads: Some(pb::BoardReads { workspace_id: bytes::Bytes::from_static(&[1; 16]), floor_ms: 3, opened: vec![] }),
        };
        let board = crate::tasks_json::board_json(&with, 0);
        assert_eq!(board["tasks"], json!([]));
        assert_eq!(board["reads"]["floor_ms"], 3);
        let without = crate::tasks_json::board_json(&pb::TaskList { items: vec![], reads: None }, 0);
        assert!(without.get("reads").is_none(), "{without}");
    }
}
