//! The announce for the plan layer (ov-268), and the plan's glance the
//! count notice carries to the relay (ov-310).

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
        // And the glance on the watch, the widgets and the Live Activity: a
        // lane that moved, a plan reordered or a theme's ask (ov-310). Sent
        // only when it differs from what the relay holds.
        self.schedule_count_notice();
    }

    /// Every board with a plan, as the glance says it (`plan_glance`).
    /// `None` when the store can't be read, which sends nothing. The
    /// needs-you list is gathered only when some board has a plan.
    pub(crate) async fn plan_glance(&self) -> Option<Vec<crate::plan_glance::BoardGlance>> {
        let now = std::time::SystemTime::now();
        let now_ms = now.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
        let planned = crate::plan_glance::planned(&self.service.store, now_ms).ok()?;
        if planned.is_empty() {
            return Some(Vec::new());
        }
        let inputs = crate::needs_you::gather(&self.service, self).await.ok()?;
        Some(crate::plan_glance::boards(planned, &crate::needs_you::assemble(&inputs, now)))
    }

    /// Whether `glance` is news to the relay: not the last one it took.
    pub(crate) fn plan_moved(&self, glance: &Option<Vec<crate::plan_glance::BoardGlance>>) -> bool {
        glance.is_some() && *self.last_plan.lock().unwrap_or_else(|e| e.into_inner()) != *glance
    }

    /// Note that the relay now holds `glance`: once a notice carrying it
    /// landed, as `told` is for the count.
    pub(crate) fn told_plan(&self, glance: Option<Vec<crate::plan_glance::BoardGlance>>) {
        if glance.is_some() {
            *self.last_plan.lock().unwrap_or_else(|e| e.into_inner()) = glance;
        }
    }
}
