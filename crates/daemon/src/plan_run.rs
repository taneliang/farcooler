//! Each ticket's engine run, laid onto the plan as it is read (ov-1040). The
//! plan column used to ask `workflow.snapshot` beside `plan.get` for this;
//! now `plan.get` carries it, and the core words the note once for every app.
//!
//! Facts only: the run's phase, the stage it is on and who it waits on. A
//! tracker-only ticket is a board card, not a run, so it gets none (as the
//! workflow presentation leaves it out).

use farcooler_protocol::v1 as pb;
use farcooler_workflow::{Phase, Progression, Reviewer, StageKind, State};


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
        reason: item.failed_reason().or_else(|| item.execution.as_ref().map(|view| view.text.clone())),
        send_back_refused: None,
    })
}

/// Lay each ticket's run onto the wire plan, in step with `plan.lines`.
pub fn annotate(state: &State, wire: &mut pb::Plan, refusals: &std::collections::BTreeMap<String, String>) {
    let keys: Vec<(Vec<u8>, String)> = wire
        .cards
        .iter()
        .map(|c| (c.task_id.to_vec(), c.key.clone()))
        .collect();
    for line in &mut wire.lines {
        let Some((_, key)) = keys.iter().find(|(id, _)| id.as_slice() == line.task_id.as_ref()) else { continue };
        line.run = run_of(state, key);
        if let Some(run) = &mut line.run {
            run.send_back_refused = refusals.iter().find(|(item, _)| item.eq_ignore_ascii_case(key))
                .map(|(_, reason)| reason.clone());
        }
    }
}

/// What every run of `state` says on the plan, as one number.
pub(crate) fn signature(state: &State, refusals: &std::collections::BTreeMap<String, String>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for key in state.items.keys() {
        if let Some(run) = run_of(state, key) {
            (key, run.run, run.phase, run.stage_kind, run.gate, run.gate_principal, run.reason, refusals.get(key))
                .hash(&mut hasher);
        }
    }
    hasher.finish()
}

#[cfg(test)]
#[path = "plan_run_tests.rs"]
mod tests;
