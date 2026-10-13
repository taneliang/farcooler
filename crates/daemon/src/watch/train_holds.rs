//! A train held by a lane that isn't ready is told to its orchestrator, once
//! (ov-503).
//!
//! A train waiting for lanes is *held* when some of its lanes are ready, one
//! isn't, and it has waited 45 minutes (`farcooler_store::train_phase`). The
//! plan read flags it (`BoardTrain.held_by`); this is the other half, the
//! runner telling the orchestrator, because a flag nobody looks at held
//! trains 86 and 89 for two hours. It never moves the lane: the lane has to
//! rebase, so the orchestrator decides, with `plan train move`.
//!
//! **Once per hold.** The store records a `held` event before the message is
//! queued, keyed by when the wait began, so a restart, a second tick and a
//! lane that stays blocked tell nobody twice; a lane that comes back and goes
//! wrong again is a new wait, and told again. A crash between the record and
//! the message loses the notice and repeats none.
//!
//! **Where it lives.** A notice is a note on a card, so it goes on the
//! train's card, else the first open card of its lanes. With no running
//! orchestrator, no open card or a board that doesn't wake on answers it is
//! not sent and not recorded, so it is told once there is someone to tell.

use farcooler_core::DomainError;
use farcooler_store::models::{Actor, TaskStatus};
use farcooler_store::train_phase::HeldTrain;

use super::Watcher;

/// "Train 86 is held by ios-overwrite and fix-x."
pub(super) fn notice(held: &HeldTrain, now_ms: i64) -> String {
    let train = match farcooler_store::trains::train_number(&held.train.name) {
        Some(n) => format!("Train {n}"),
        None => held.train.name.clone(),
    };
    let lanes = match held.names.as_slice() {
        [one] => one.clone(),
        [first @ .., last] => format!("{} and {last}", first.join(", ")),
        [] => "a lane".to_string(),
    };
    let minutes = (now_ms - held.since).max(0) / 60_000;
    let waited = if minutes < 120 { format!("{minutes} minutes") } else { format!("{} hours", minutes / 60) };
    let first = held.names.first().map_or("<lane>", String::as_str);
    format!(
        "{train} is held by {lanes}. The rest of its lanes are ready, and it has waited {waited}. \
         Take the lane off with `farcooler plan train move {first}` once you've decided, or wait for it."
    )
}

impl Watcher {
    /// Tell the orchestrator of every hold it hasn't been told of.
    pub(crate) fn tell_held_trains(&self) {
        let store = &self.service.store;
        for workspace in store.workspaces_with_integrating_trains().unwrap_or_default() {
            let Ok(held) = store.holds_untold(workspace) else { continue };
            for held in held {
                self.tell_hold(workspace, &held);
            }
        }
    }

    fn tell_hold(&self, workspace: uuid::Uuid, held: &HeldTrain) {
        let store = &self.service.store;
        let wakes = store.get_workspace(workspace).is_ok_and(|w| w.wake_on_answer);
        let running = wakes && self.service.live_orchestrator(workspace).ok().flatten().is_some_and(|o| self.service.is_running(&o));
        let open = |id: &uuid::Uuid| store.get_task(*id).ok().filter(|t| !matches!(t.status, TaskStatus::Done | TaskStatus::Cancelled));
        let Some(task) = held.cards.iter().find_map(open).filter(|_| running) else { return };
        if !matches!(store.record_hold_told(held), Ok(true)) {
            return;
        }
        let body = notice(held, crate::review::now_millis());
        let extra = serde_json::json!({ "message": { "to": "the orchestrator", "about": held.train.id.to_string() } });
        match store.send_runner_notice(task.id, &body, extra) {
            Ok(_) => {
                self.announce_task_changed(&task, None, Actor::Runner);
                self.announce_plan_changed(workspace, Actor::Runner);
                self.message_queued();
            }
            // The board stopped waking on answers between the check and now.
            Err(DomainError::Conflict { what: "typing_off" }) => {}
            Err(e) => tracing::warn!(train = %held.train.name, error = %e, "couldn't tell the orchestrator a train is held"),
        }
    }
}
