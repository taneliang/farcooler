//! A run that moves while the plan doesn't (ov-1040).

use uuid::Uuid;

use super::Watcher;

impl Watcher {
    /// A workflow command was accepted on `workspace`: when a run's step, phase or gate moved, open plans re-read,
    /// since the plan carries the run's note and nothing else of the plan changed (ov-1040). Said once per change.
    pub(crate) fn announce_run_moves(&self, workspace: &str) {
        let Ok(board) = Uuid::parse_str(workspace) else { return };
        let Ok(Some(state)) = self.service.store.workflow_state(workspace) else { return };
        let now = crate::plan_run::signature(&state);
        let told = self.runs_told.lock().unwrap_or_else(|e| e.into_inner()).insert(workspace.to_string(), now);
        if told != Some(now) {
            self.announce_plan_changed(board, farcooler_store::models::Actor::Runner);
        }
    }
}

#[cfg(test)]
#[path = "run_moves_tests.rs"]
mod tests;
