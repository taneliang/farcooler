//! A run that moves while the plan doesn't (ov-1040).

use uuid::Uuid;

use super::Watcher;

impl Watcher {
    /// After an accepted command or a send-back refusal, open plans re-read if a run's display changed.
    pub(crate) fn announce_run_moves(&self, workspace: &str) {
        let Ok(board) = Uuid::parse_str(workspace) else { return };
        let Ok(Some(state)) = self.service.store.workflow_state(workspace) else { return };
        let refusals = self.service.store.workflow_send_back_refusals(&state).unwrap_or_else(|error| {
            tracing::warn!(%error, "workflow send-back receipts unreadable");
            Default::default()
        });
        let now = crate::plan_run::signature(&state, &refusals);
        let told = self.runs_told.lock().unwrap_or_else(|e| e.into_inner()).insert(workspace.to_string(), now);
        if told != Some(now) {
            self.announce_plan_changed(board, farcooler_store::models::Actor::Runner);
        }
    }
}

#[cfg(test)]
#[path = "run_moves_tests.rs"]
mod tests;
