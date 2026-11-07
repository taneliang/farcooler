//! Tickets that are ready to land and on no landing join the collecting one,
//! the collecting landing departs when it can, and the orchestrator hears of it
//! (ov-520, ov-872).
//!
//! A ticket that passes review joins in the same write
//! (`farcooler_store::collecting`), so the scan finds nothing to join in the
//! ordinary case. It catches up what that write could not take (a ticket whose
//! stack parent or same-branch group wasn't ready yet, or a landing dropped
//! under a ticket that was ready), and it retries a departure that couldn't
//! happen when it was asked for. It runs on the backstop tick and announces a
//! plan change for each board it changed.

use farcooler_core::DomainError;
use farcooler_store::collecting::Departed;
use farcooler_store::landings::landing_title;
use farcooler_store::models::{Actor, TaskStatus};

use super::Watcher;

/// "Landing 117 is ready for its checks: ov-1 and ov-2."
pub(super) fn notice(departed: &Departed) -> String {
    let landing = landing_title(&departed.landing);
    let tickets = match departed.keys.as_slice() {
        [] => "no tickets".to_string(),
        [one] => one.clone(),
        [first @ .., last] => format!("{} and {last}", first.join(", ")),
    };
    format!("{landing} is ready for its checks: {tickets}. It left on its own. Gate it, or start your own with `landing start`.")
}

impl Watcher {
    /// Put every ready ticket that is on no live landing on its board's
    /// collecting landing, let that landing depart if it can, and tell the
    /// orchestrator of any that left. The boards that changed.
    pub(crate) fn collect_ready_tickets(&self) -> Vec<uuid::Uuid> {
        let store = &self.service.store;
        // Collecting is legacy landing authority: after release its tables are
        // fenced and the engine lands (ov-947).
        if !store.legacy_authority() { return Vec::new(); }
        let mut changed = Vec::new();
        for workspace in store.workspaces_to_collect().unwrap_or_default() {
            match store.collect_ready_tickets(workspace, Actor::Runner) {
                Ok(done) if !done.joined.is_empty() || done.departed.is_some() => {
                    self.announce_plan_changed(workspace, Actor::Runner);
                    changed.push(workspace);
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "couldn't put the ready tickets on the collecting landing"),
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
        if !store.legacy_authority() { return; }
        for workspace in store.workspaces_with_gating_landings().unwrap_or_default() {
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
        let extra = serde_json::json!({ "message": { "to": "the orchestrator", "about": departed.landing.id.to_string() } });
        match store.send_runner_notice(task.id, &notice(departed), extra) {
            Ok(_) => {
                self.announce_task_changed(&task, None, Actor::Runner);
                self.announce_plan_changed(workspace, Actor::Runner);
                self.message_queued();
            }
            // The board stopped waking on answers between the check and now.
            Err(DomainError::Conflict { what: "typing_off" }) => {}
            Err(e) => {
                tracing::warn!(landing = %landing_title(&departed.landing), error = %e, "couldn't tell the orchestrator a landing left")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use farcooler_store::ticket_lines::{NewTicketLine, Stage};

    use super::*;

    /// A ticket that reached landing on no landing, as an older build left
    /// it, is on the collecting landing after one scan, and a second scan
    /// finds nothing.
    #[tokio::test]
    async fn the_scan_puts_a_stranded_ready_ticket_on_the_collecting_landing() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let main = svc.store.ensure_main_workspace(repo).unwrap().id;
        let task = svc.store.create_task(main, "stranded", Actor::Manager).unwrap();
        svc.store.put_on_line(task.id, &NewTicketLine::default(), Actor::Manager).unwrap();
        farcooler_store::testing::strand_in_landing(&svc.store, task.id);
        let watcher = Watcher::new(svc.clone());
        assert_eq!(watcher.collect_ready_tickets(), vec![main]);
        let landing = svc.store.landing_of(task.id).unwrap().expect("it joined");
        assert_eq!(svc.store.ticket_line(task.id).unwrap().stage, Stage::Landing);
        // Nothing was ahead, so the landing it joined left for its checks at once.
        assert_eq!(landing.state, farcooler_store::landings::LandingState::Gating);
        assert!(watcher.collect_ready_tickets().is_empty());
    }

    /// After release the scan joins nothing, so the fenced landing tables are never touched.
    #[tokio::test]
    async fn the_scan_does_nothing_once_the_engine_is_released() {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let main = svc.store.ensure_main_workspace(repo).unwrap().id;
        let task = svc.store.create_task(main, "stranded", Actor::Manager).unwrap();
        svc.store.put_on_line(task.id, &NewTicketLine::default(), Actor::Manager).unwrap();
        farcooler_store::testing::strand_in_landing(&svc.store, task.id);
        farcooler_store::testing::exec_for_tests(&svc.store, "UPDATE engine_cutover_authority SET phase='released'");
        // Lift the landing fences so only the gate, not the fence, can keep the scan out.
        for table in ["landings", "landing_tickets"] {
            for suffix in ["i", "u", "d"] {
                farcooler_store::testing::exec_for_tests(&svc.store, &format!("DROP TRIGGER engine_fence_{table}_{suffix}"));
            }
        }
        let watcher = Watcher::new(svc.clone());
        assert!(watcher.collect_ready_tickets().is_empty());
        assert!(svc.store.landing_of(task.id).unwrap().is_none(), "nothing joined a landing");
        assert!(crate::green_landing::land_green_landings(&svc, &watcher).await.is_empty());
    }
}
