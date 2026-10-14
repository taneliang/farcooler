//! Lanes that are ready to land and on no train join the collecting one, the
//! collecting train departs when it can, and the orchestrator hears of it
//! (ov-520).
//!
//! A lane that passes review joins in the same write
//! (`farcooler_store::open_train`), so the scan finds nothing to join in the
//! ordinary case. It catches up what that write could not see (a lane that
//! reached `landing` through a build from before the collecting train, or a
//! train dropped under a lane that was ready), and it retries a departure that
//! couldn't happen when it was asked for. It runs on the backstop tick and
//! announces a plan change for each board it changed.

use farcooler_core::DomainError;
use farcooler_store::models::{Actor, TaskStatus};
use farcooler_store::open_train::Departed;

use super::Watcher;

/// "Train 117 is ready for its checks: a and b."
pub(super) fn notice(departed: &Departed) -> String {
    let train = match farcooler_store::trains::train_number(&departed.train.name) {
        Some(n) => format!("Train {n}"),
        None => departed.train.name.clone(),
    };
    let lanes = match departed.lanes.as_slice() {
        [] => "no lanes".to_string(),
        [one] => one.clone(),
        [first @ .., last] => format!("{} and {last}", first.join(", ")),
    };
    format!("{train} is ready for its checks: {lanes}. It left on its own. Gate it, or start your own train with `train start`.")
}

impl Watcher {
    /// Put every ready lane that is on no live train on its board's collecting
    /// train, let that train depart if it can, and tell the orchestrator of any
    /// that left. The boards that changed.
    pub(crate) fn collect_ready_lanes(&self) -> Vec<uuid::Uuid> {
        let store = &self.service.store;
        let mut changed = Vec::new();
        for workspace in store.workspaces_to_collect().unwrap_or_default() {
            match store.collect_ready_lanes(workspace, Actor::Runner) {
                Ok(done) if !done.joined.is_empty() || done.departed.is_some() => {
                    self.announce_plan_changed(workspace, Actor::Runner);
                    changed.push(workspace);
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "couldn't put the ready lanes on the collecting train"),
            }
        }
        self.tell_departures();
        changed
    }

    /// Tell the orchestrator of every departure it hasn't been told of, once
    /// each: the store records it before the message is queued, so a restart, a
    /// second tick and a second watcher tell nobody twice. With no running
    /// orchestrator, no open card or a board that doesn't wake on answers it is
    /// not sent and not recorded, so it is told once there is someone to tell.
    pub(crate) fn tell_departures(&self) {
        let store = &self.service.store;
        for workspace in store.workspaces_with_gating_trains().unwrap_or_default() {
            let Ok(untold) = store.departures_untold(workspace) else { continue };
            for departed in untold {
                self.tell_departure(workspace, &departed);
            }
        }
    }

    fn tell_departure(&self, workspace: uuid::Uuid, departed: &Departed) {
        let store = &self.service.store;
        let wakes = store.get_workspace(workspace).is_ok_and(|w| w.wake_on_answer);
        let running = wakes && self.service.live_orchestrator(workspace).ok().flatten().is_some_and(|o| self.service.is_running(&o));
        let open = |id: &uuid::Uuid| store.get_task(*id).ok().filter(|t| !matches!(t.status, TaskStatus::Done | TaskStatus::Cancelled));
        let Some(task) = departed.cards.iter().find_map(open).filter(|_| running) else { return };
        if !matches!(store.record_departure_told(departed), Ok(true)) {
            return;
        }
        let extra = serde_json::json!({ "message": { "to": "the orchestrator", "about": departed.train.id.to_string() } });
        match store.send_runner_notice(task.id, &notice(departed), extra) {
            Ok(_) => {
                self.announce_task_changed(&task, None, Actor::Runner);
                self.announce_plan_changed(workspace, Actor::Runner);
                self.message_queued();
            }
            // The board stopped waking on answers between the check and now.
            Err(DomainError::Conflict { what: "typing_off" }) => {}
            Err(e) => tracing::warn!(train = %departed.train.name, error = %e, "couldn't tell the orchestrator a train left"),
        }
    }
}

#[cfg(test)]
mod tests {
    use farcooler_store::plan::{LaneState, NewLane};

    use super::*;

    /// A lane that reached landing with no train, as an older build left it, is
    /// on the open train after one scan, and a second scan finds nothing.
    #[tokio::test]
    async fn the_scan_puts_a_stranded_ready_lane_on_the_open_train() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let main = svc.store.ensure_main_workspace(repo).unwrap().id;
        let lane = svc.store.create_lane(main, &NewLane { name: "stranded".into(), ..Default::default() }, &[], None, Actor::Manager).unwrap();
        farcooler_store::testing::strand_in_landing(&svc.store, lane.id);
        let watcher = Watcher::new(svc.clone());
        assert_eq!(watcher.collect_ready_lanes(), vec![main]);
        let now = svc.store.lane(lane.id).unwrap();
        assert_eq!((now.state, now.train.as_deref()), (LaneState::Landing, Some("train-1")));
        // Nothing was ahead, so the train it joined left for its checks at once.
        assert_eq!(svc.store.train_named(main, "train-1").unwrap().state, farcooler_store::trains::TrainState::Gating);
        assert!(watcher.collect_ready_lanes().is_empty());
    }
}
