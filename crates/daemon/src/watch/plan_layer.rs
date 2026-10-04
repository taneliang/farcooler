//! The announce for the plan layer (ov-268).

use farcooler_protocol::v1::{Event, PlanChanged, event};
use farcooler_store::models::Actor;
use uuid::Uuid;

use super::Watcher;
use crate::wire::id_bytes;

impl Watcher {
    /// A theme, a lane or the plan on a board was written.
    ///
    /// Carries the board and the actor and nothing else: a client with the
    /// Plan view open re-reads `plan.get`, which is one call. Not
    /// `task_changed`, since no task moved and that event wakes managers.
    pub fn announce_plan_changed(&self, workspace: Uuid, actor: Actor) {
        let _ = self.events.send(Event {
            event_id: bytes::Bytes::copy_from_slice(Uuid::now_v7().as_bytes()),
            sequence: 0,
            payload: Some(event::Payload::PlanChanged(PlanChanged {
                workspace_id: id_bytes(workspace),
                actor: actor.to_string(),
            })),
        });
    }
}
