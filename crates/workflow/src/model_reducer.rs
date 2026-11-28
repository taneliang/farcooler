//! Running a failed step again, and changing the model a step runs on (ov-1515): the orchestrator's move, and the
//! workflow's fallback when a provider is at its limit. Either keeps the run's worktree.
use super::*;

/// A model's harness: `codex` in `codex:gpt-5.5`.
fn harness_of(model: &str) -> &str { model.split(':').next().unwrap_or(model) }

/// What running a failed step again costs: one of the run's retries, unless the runner or the provider refused it.
pub(super) fn spend_retry(item: &mut Item) -> Result<(), Refusal> {
    if item.refused_launch != Some(item.fence) {
        if item.retries >= item.definition.execution_limits().retries { return Err(Refusal::Limit); }
        item.retries += 1;
    }
    Ok(())
}

/// A run whose review ran out of send-backs goes back to its build, with that gate's allowance and its step
/// retries fresh.
pub(super) fn back_to_build(item: &mut Item) -> Result<(), Refusal> {
    let build = item.review_restart_stage().ok_or(Refusal::Phase)?;
    let prior = item.last_run();
    item.history.push(prior);
    let spent = if item.definition.schema == 2 {
        item.back_edges.remove(&item.definition.stages[item.stage].id).unwrap_or(0)
    } else { item.iteration };
    item.iteration -= spent;
    item.rework_history.clear();
    item.stage = build;
    item.candidate = None;
    item.last_evidence = None;
    item.run.clear();
    item.retries = 0;
    Ok(())
}

/// A run's step waits for its resource again, like new work.
pub(super) fn requeue(item: &mut Item) -> Result<(), Refusal> {
    item.refused_launch = None;
    item.attempt = 1;
    item.failure = None;
    item.reason = None;
    item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
    item.phase = Phase::Blocked;
    Ok(())
}

/// The step's agent is at its provider's limit: it moves to its fallback on another harness, or the run fails now.
pub(super) fn usage_limit(item: &mut Item, id: &str, input: &Input, harness: &str, reason: &str,
    effects: &mut Vec<Effect>) -> Result<(), Refusal> {
    if item.run.starts_with("cancel:") || !starts_agent(&item.definition.stages[item.stage].kind)
        || !valid_id(harness) || !valid_rework_reason(reason) { return Err(Refusal::Invalid); }
    item.pending = None;
    item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
    let step = item.definition.stages[item.stage].id.clone();
    let fallback = item.definition.fallbacks.get(&step).filter(|model| harness_of(model) != harness).cloned();
    if let Some(model) = fallback {
        item.reason = Some(format_reason(&step, &format!("{reason}. The step moved to its fallback, {model}."), None));
        item.models.insert(step.clone(), model);
        item.moved.insert(step);
        launch(item, id, input, effects)?;
    } else {
        // The provider would refuse the next launch too, so the run waits, spending nothing.
        item.failure = Some(reason.to_owned());
        item.refused_launch = Some(item.fence);
        item.phase = Phase::Failed;
    }
    Ok(())
}

/// A ticket waiting to start has no run to move: the pick is kept on it for the run. Its own columns list no
/// models, so the runner names the step and checks the model against the published workflow.
fn keep_for_start(item: &mut Item, model: &str, step: Option<&str>) -> Result<(), Refusal> {
    if item.phase != Phase::WaitingApproval || item.follows.is_some() || item.closed.is_some() {
        return Err(Refusal::Phase);
    }
    let step = step.ok_or_else(|| Refusal::ModelRefused("Name the step this model is for.".into()))?;
    if !valid_id(step) || !crate::definition::valid_model(model) { return Err(Refusal::Invalid); }
    if !item.models.contains_key(step) && item.models.len() >= usize::from(MAX_TICKET_PICKS) { return Err(Refusal::Limit); }
    item.models.insert(step.to_owned(), model.to_owned());
    item.moved.insert(step.to_owned());
    Ok(())
}
const MAX_TICKET_PICKS: u8 = 16;

/// A run starting from a waiting ticket: the model chosen for the ticket holds for each step that lists it, unless
/// Start named another.
pub(super) fn carry_picks(run: &mut Item, ticket: &Item, named: &BTreeMap<String, String>) {
    for step in &ticket.moved {
        let Some(model) = ticket.models.get(step) else { continue };
        if !run.definition.models.get(step).is_some_and(|list| list.contains(model))
            || named.get(step).is_some_and(|other| other != model) { continue; }
        run.models.insert(step.clone(), model.clone());
        run.moved.insert(step.clone());
    }
}

/// Move a queued, failed or running ticket to another of a step's listed models (`Command::SetModel`).
pub(super) fn set_model(item: &mut Item, id: &str, input: &Input, model: &str, step: Option<&str>,
    effects: &mut Vec<Effect>) -> Result<(), Refusal> {
    if item.definition.is_ticket() { return keep_for_start(item, model, step); }
    if item.group_of.is_some() || item.follows.is_some()
        || item.stopping.is_some() || item.closed.is_some()
        || matches!(item.phase, Phase::Done | Phase::Cancelled | Phase::Cancelling | Phase::Uncertain)
    { return Err(Refusal::Phase); }
    let step = match step {
        Some(step) => step.to_owned(),
        None => item.definition.stages.get(..=item.stage).and_then(|done| done.iter().rev()
            .find(|s| item.definition.models.contains_key(&s.id))).map(|s| s.id.clone())
            .ok_or_else(|| Refusal::ModelRefused("This run has no step that lists models.".into()))?,
    };
    let listed = item.definition.models.get(&step).ok_or_else(||
        Refusal::ModelRefused("That step lists no models.".into()))?;
    if !listed.iter().any(|known| known == model) {
        return Err(Refusal::ModelRefused(format!("That model is not listed for step {step}. Listed: {}.", listed.join(", "))));
    }
    item.models.insert(step.clone(), model.to_owned());
    item.moved.insert(step.clone());
    let here = item.definition.stages[item.stage].id == step;
    if item.phase == Phase::Failed {
        if item.pending.is_some() { return Err(Refusal::Phase); }
        // As Retry would: out of review rounds, it builds again with a fresh allowance.
        if item.review_restart_stage().is_some() { back_to_build(item)?; }
        else { spend_retry(item)?; }
        requeue(item)?;
    } else if here && let Some(operation) = item.pending.take() {
        // Never two agents in one tree: the old one is stopped, and its stop's receipt queues the step again.
        item.deciding = None;
        item.resume_after_stop = true;
        item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
        item.phase = Phase::Cancelling;
        item.stopping = Some(operation.clone());
        let stop_run = format!("cancel:{}", run_id(item, id, input));
        item.stopped_from = Some(std::mem::replace(&mut item.run, stop_run));
        emit(item, id, input, EffectKind::Stop { operation }, effects);
    }
    Ok(())
}
