//! Each ticket's engine run, laid onto the plan as it is read (ov-1040). The
//! plan column used to ask `workflow.snapshot` beside `plan.get` for this;
//! now `plan.get` carries it, and the core words the note once for every app.
//!
//! Facts only: the run's phase, the stage it is on and who it waits on. A
//! tracker-only ticket is a board card, not a run, so it gets none (as the
//! workflow presentation leaves it out).

use farcooler_protocol::v1 as pb;
use farcooler_workflow::{Phase, Progression, Reviewer, StageKind, State};
use uuid::Uuid;

use crate::service::Service;
use crate::watch::Watcher;

/// The run `state` holds for the ticket `key`, if it has one worth showing.
pub fn run_of(state: &State, key: &str) -> Option<pb::TicketRun> {
    let (_, item) = state.items.iter().find(|(id, _)| id.eq_ignore_ascii_case(key))?;
    if item.definition.is_ticket() {
        return None;
    }
    let stage = item.definition.stages.get(item.stage);
    let (gate, principal) = match (&item.phase, stage) {
        (Phase::WaitingApproval, Some(s)) => match &s.progression {
            Progression::AwaitApproval { principal } => ("approval", principal.clone()),
            _ => ("", String::new()),
        },
        (Phase::WaitingReview, Some(s)) => match &s.kind {
            StageKind::Review(Reviewer::Person(p) | Reviewer::Orchestrator(p)) => ("review", p.clone()),
            _ => ("", String::new()),
        },
        _ => ("", String::new()),
    };
    Some(pb::TicketRun {
        run: item.run.clone(),
        phase: serde_json::to_value(&item.phase).ok()?.as_str()?.to_string(),
        stage_kind: stage.map(|s| serde_json::to_string(&s.kind)).transpose().ok()?.unwrap_or_default(),
        gate: gate.into(),
        gate_principal: principal,
    })
}

/// Lay each ticket's run onto the wire plan, in step with `plan.lines`.
pub fn annotate(svc: &Service, workspace: Uuid, wire: &mut pb::Plan) {
    let Ok(Some(state)) = svc.store.workflow_state(&workspace.to_string()) else { return };
    let keys: Vec<(Vec<u8>, String)> = wire
        .cards
        .iter()
        .map(|c| (c.task_id.to_vec(), c.key.clone()))
        .collect();
    for line in &mut wire.lines {
        let Some((_, key)) = keys.iter().find(|(id, _)| id.as_slice() == line.task_id.as_ref()) else { continue };
        line.run = run_of(&state, key);
    }
}

/// What every run of `state` says on the plan, as one number: it differs when a run's step, phase or gate does.
fn signature(state: &State) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for key in state.items.keys() {
        if let Some(run) = run_of(state, key) {
            (key, run.run, run.phase, run.stage_kind, run.gate, run.gate_principal).hash(&mut hasher);
        }
    }
    hasher.finish()
}

impl Watcher {
    /// A workflow command was accepted on `workspace`: when a run's step, phase or gate moved, open plans re-read,
    /// since the plan carries the run's note and nothing else of the plan changed (ov-1040). Said once per change.
    pub(crate) fn announce_run_moves(&self, workspace: &str) {
        let Ok(board) = Uuid::parse_str(workspace) else { return };
        let Ok(Some(state)) = self.service.store.workflow_state(workspace) else { return };
        let now = signature(&state);
        let told = self.runs_told.lock().unwrap_or_else(|e| e.into_inner()).insert(workspace.to_string(), now);
        if told != Some(now) {
            self.announce_plan_changed(board, farcooler_store::models::Actor::Runner);
        }
    }
}

#[cfg(test)]
#[path = "plan_run_tests.rs"]
mod tests;
