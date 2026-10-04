//! The announce for orchestrator pages (ov-269).

use farcooler_protocol::v1::{Event, PagesChanged, event};
use farcooler_store::models::Actor;
use uuid::Uuid;

use super::Watcher;
use crate::wire::id_bytes;

impl Watcher {
    /// A page on a board was written or removed.
    ///
    /// Carries the board, the slot and the revision and nothing else: a client
    /// with that page open re-reads it, which is one call. Not `task_changed`,
    /// since no task moved and that event wakes managers. Not debounced; see
    /// `rpc_pages` for what bounds it.
    pub fn announce_pages_changed(&self, workspace: Uuid, slot: &str, revision: u64, actor: Actor, removed: bool) {
        let _ = self.events.send(Event {
            event_id: bytes::Bytes::copy_from_slice(Uuid::now_v7().as_bytes()),
            sequence: 0,
            payload: Some(event::Payload::PagesChanged(PagesChanged {
                workspace_id: id_bytes(workspace),
                slot: slot.to_string(),
                revision,
                actor: actor.to_string(),
                removed,
            })),
        });
    }
}
