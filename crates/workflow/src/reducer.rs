use crate::*;
use std::collections::BTreeMap;

pub fn step(state: &State, input: &Input) -> Result<Transition, Refusal> {
    if !valid_state(state) { return Err(Refusal::Invalid); }
    if input.schema != 1 || state.schema != 1 || !valid_id(&input.id)
        || !valid_id(&input.source) || !valid_id(&input.workspace)
        || input.source_sequence == 0 || state.items.len() > 1024
        || input.busy_resources.len() > 1024 || input.busy_resources.iter().any(|r| !valid_id(r))
        || request_size(input)? > 65536
    { return Err(Refusal::Invalid); }
    if state.seen.contains(&input.id) { return Err(Refusal::Duplicate); }
    if input.workspace != state.workspace || input.epoch != state.epoch
        || input.expected_revision != state.revision
        || input.source_sequence <= *state.source_sequences.get(&input.source).unwrap_or(&0)
    { return Err(Refusal::Stale); }
    let mut next = state.clone();
    let mut effects = Vec::new();
    let mut notices = Vec::new();
    match &input.command {
        Command::Advance => { controller(input)?; }
        Command::Enqueue { item, definition, policy_pin, resource, dependencies, initial_candidate } => {
            controller(input)?;
            definition.validate()?;
            if !valid_id(item) || !valid_id(resource) || !valid_id(policy_pin)
                || next.items.contains_key(item) || next.archive.contains_key(item) || next.items.len() >= 1024
                || dependencies.len() > 128
            { return Err(Refusal::Invalid); }
            let tracker = matches!(definition.stages[0].kind, StageKind::Tracker);
            if tracker != initial_candidate.is_some()
                || initial_candidate.as_ref().is_some_and(|c| !valid_id(c))
            { return Err(Refusal::Evidence); }
            // Dependencies can name existing items only: insertion cannot create a cycle.
            for dep in dependencies {
                if !next.items.contains_key(dep) && !next.archive.contains_key(dep) { return Err(Refusal::Missing); }
            }
            if dependencies.iter().all(|d| dependency_done(&next, d))
                && (input.busy_resources.contains(resource)
                    || next.items.values().any(|i| owns_resource(i) && i.resource == *resource)) {
                return Err(Refusal::Resource);
            }
            let mut record = Item { definition: definition.clone(), pin: definition.pin(),
                policy_pin: policy_pin.clone(), dependencies: dependencies.clone(),
                resource: resource.clone(), stage: 0, iteration: 0, attempt: 1, fence: 1,
                phase: Phase::Blocked, run: String::new(), pending: None,
                candidate: initial_candidate.clone(), last_evidence: None, stopping: None, rework_history: Vec::new(),
                back_edges: BTreeMap::new(), retries: 0,
                members: Vec::new(), group_of: None, grouped_in: None };
            if ready(&next, &record, input) { enter(&mut record, item, input, &mut effects)?; }
            next.items.insert(item.clone(), record);
        }
        command => {
            let id = match command {
                Command::Approve { item, .. } | Command::Review { item, .. }
                | Command::Observe { item, .. } | Command::Cancel { item } | Command::Retry { item }
                | Command::Queue { item } | Command::Leave { item }
                | Command::Resolve { item, .. } => item,
                Command::Enqueue { .. } | Command::Advance => unreachable!(),
            };
            let mut item = next.items.remove(id).ok_or(Refusal::Missing)?;
            match command {
                Command::Approve { run, pin, candidate, .. } => {
                    if item.phase != Phase::WaitingApproval { return Err(Refusal::Phase); }
                    if run != &item.run || pin != &item.pin || candidate != &item.candidate {
                        return Err(Refusal::Evidence);
                    }
                    let Progression::AwaitApproval { principal } = &item.definition.stages[item.stage].progression else {
                        return Err(Refusal::Phase);
                    };
                    decision(input, principal)?;
                    launch(&mut item, id, input, &mut effects)?;
                }
                Command::Review { run, evidence, passed, reason, .. } => {
                    if item.phase != Phase::WaitingReview || run != &item.run {
                        return Err(Refusal::Phase);
                    }
                    let StageKind::Review(Reviewer::Person(p) | Reviewer::Orchestrator(p)) =
                        &item.definition.stages[item.stage].kind else { return Err(Refusal::Phase); };
                    decision(input, p)?;
                    evidence_matches(&item, evidence)?;
                    if let Some(reason) = reason {
                        if *passed || !valid_rework_reason(reason) { return Err(Refusal::Invalid); }
                        if item.last_evidence.as_ref() != Some(evidence) { return Err(Refusal::Evidence); }
                        if item.definition.schema == 2 { rework_room(&item, "rejected")?; }
                        else if item.iteration >= item.definition.max_reworks { return Err(Refusal::Limit); }
                        item.rework_history.push(ReworkDecision {
                            id: input.id.clone(), principal: p.clone(), revision: input.expected_revision,
                            run: item.run.clone(), definition_pin: item.pin.clone(), evidence: evidence.clone(),
                            iteration: item.iteration, reason: reason.clone(),
                        });
                    }
                    finish(&mut item, id, input, *passed, evidence, &mut effects)?;
                }
                Command::Observe { effect, run, fence, outcome, .. }
                | Command::Resolve { effect, run, fence, outcome, .. } => {
                    if effect != item.pending.as_ref().ok_or(Refusal::Phase)?
                        || run != &item.run || fence != &item.fence
                    { return Err(Refusal::Stale); }
                    let resolving = matches!(command, Command::Resolve { .. });
                    if resolving {
                        controller(input)?;
                        if item.phase != Phase::Uncertain { return Err(Refusal::Phase); }
                    } else {
                        if input.grant != (Grant::Operation { effect: effect.clone(), run: run.clone(), fence: *fence }) {
                            return Err(Refusal::Unauthorized);
                        }
                        if !matches!(item.phase, Phase::Running | Phase::Cancelling) {
                            return Err(Refusal::Phase);
                        }
                    }
                    match outcome {
                        Outcome::Uncertain => { item.phase = Phase::Uncertain; }
                        Outcome::Stopped { operation, receipt } => {
                            // A stop receipt is only valid for the cancellation operation, never execution.
                            if !item.run.starts_with("cancel:") { return Err(Refusal::Phase); }
                            if item.stopping.as_ref() != Some(operation) || !valid_id(receipt) {
                                return Err(Refusal::Evidence);
                            }
                            item.stopping = None;
                            item.phase = Phase::Cancelled;
                            item.pending = None;
                        }
                        Outcome::Labeled(label, evidence) => {
                            if item.run.starts_with("cancel:") { return Err(Refusal::Phase); }
                            let stage = &item.definition.stages[item.stage];
                            if item.definition.schema != 2 { return Err(Refusal::Invalid); }
                            if !(matches!(label.as_str(), "passed" | "rejected") || stage.routes.contains_key(label)) {
                                return Err(Refusal::Evidence);
                            }
                            evidence_matches(&item, evidence)?;
                            route(&mut item, id, input, label, evidence, &mut effects)?;
                        }
                        Outcome::Passed(evidence) | Outcome::Rejected(evidence) => {
                            if item.run.starts_with("cancel:") { return Err(Refusal::Phase); }
                            evidence_matches(&item, evidence)?;
                            finish(&mut item, id, input, matches!(outcome, Outcome::Passed(_)), evidence, &mut effects)?;
                        }
                        Outcome::SafeFailure => {
                            if item.run.starts_with("cancel:") { return Err(Refusal::Phase); }
                            item.pending = None;
                            if item.attempt >= item.definition.stages[item.stage].max_attempts {
                                item.phase = Phase::Failed;
                            } else {
                                item.attempt += 1;
                                item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
                                enter(&mut item, id, input, &mut effects)?;
                            }
                        }
                    }
                }
                Command::Cancel { .. } => {
                    controller(input)?;
                    // A failed run is closed by cancelling it: that is the controller's way off the needs-you list.
                    if matches!(item.phase, Phase::Done | Phase::Cancelled) { return Err(Refusal::Phase); }
                    if item.phase == Phase::Cancelling || item.run.starts_with("cancel:") {
                        return Err(Refusal::Phase);
                    }
                    item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
                    if let Some(operation) = item.pending.take() {
                        item.phase = Phase::Cancelling;
                        item.stopping = Some(operation.clone());
                        item.run = format!("cancel:{}", run_id(&item, id, input));
                        emit(&mut item, id, input, EffectKind::Stop { operation }, &mut effects);
                    } else { item.phase = Phase::Cancelled; }
                }
                Command::Queue { .. } | Command::Leave { .. } => { return Err(Refusal::Phase); }
                Command::Retry { .. } => {
                    controller(input)?;
                    // A failed run does not own its resource, so a retry queues like new work.
                    if item.phase != Phase::Failed { return Err(Refusal::Phase); }
                    if item.retries >= MAX_RETRIES { return Err(Refusal::Limit); }
                    item.retries += 1;
                    item.attempt = 1;
                    item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
                    item.phase = Phase::Blocked;
                }
                Command::Enqueue { .. } | Command::Advance => unreachable!(),
            }
            next.items.insert(id.clone(), item);
        }
    }
    // A failed run stays: cancelling it is the controller's way off the needs-you list.
    // An item that was already finished before this step leaves live state now, so its final
    // phase has been seen by the board and the notices before it goes. The event log keeps all of it.
    let finished: Vec<_> = state.items.iter().filter(|(_, i)| matches!(i.phase, Phase::Done | Phase::Cancelled))
        .map(|(id, _)| id.clone()).collect();
    for id in finished {
        if let Some(item) = next.items.remove(&id) {
            next.archive.insert(id, Archived { phase: item.phase, candidate: item.candidate });
        }
    }
    // Completion of a prerequisite can release blocked work without another controller command.
    let ready_ids: Vec<_> = next.items.iter().filter(|(_, i)| i.phase == Phase::Blocked && ready(&next, i, input))
        .map(|(id, _)| id.clone()).collect();
    for id in ready_ids {
        let mut item = next.items.remove(&id).ok_or(Refusal::Missing)?;
        // Earlier eligible items may have acquired this resource during this sweep.
        if !ready(&next, &item, input) { next.items.insert(id, item); continue; }
        enter(&mut item, &id, input, &mut effects)?;
        next.items.insert(id, item);
    }
    for (id, item) in &next.items {
        if state.items.get(id) != Some(item) {
            notices.push(format!("{id}:{:?}", item.phase));
        }
    }
    next.revision = next.revision.checked_add(1).ok_or(Refusal::Limit)?;
    next.seen.insert(input.id.clone());
    next.source_sequences.insert(input.source.clone(), input.source_sequence);
    Ok(Transition { state: next, effects, notices })
}

pub fn owns_resource(item: &Item) -> bool {
    !matches!(item.phase, Phase::Blocked | Phase::Done | Phase::Failed | Phase::Cancelled)
}
fn ready(state: &State, item: &Item, input: &Input) -> bool {
    if input.busy_resources.contains(&item.resource) { return false; }
    item.dependencies.iter().all(|d| dependency_done(state, d))
        && !state.items.values().any(|i| owns_resource(i) && i.resource == item.resource)
}
fn dependency_done(state: &State, id: &str) -> bool {
    state.items.get(id).map(|i| &i.phase).or_else(|| state.archive.get(id).map(|a| &a.phase)) == Some(&Phase::Done)
}
fn controller(input: &Input) -> Result<(), Refusal> {
    if input.grant == Grant::Controller { Ok(()) } else { Err(Refusal::Unauthorized) }
}
fn decision(input: &Input, principal: &str) -> Result<(), Refusal> {
    if input.grant == (Grant::Decision { principal: principal.into() }) { Ok(()) }
    else { Err(Refusal::Unauthorized) }
}
fn run_id(item: &Item, id: &str, input: &Input) -> String {
    digest(&("run/v1", &input.workspace, id, &item.pin, &item.policy_pin,
        item.stage, item.iteration, item.attempt, input.epoch, item.fence))
}
fn enter(item: &mut Item, id: &str, input: &Input, effects: &mut Vec<Effect>) -> Result<(), Refusal> {
    item.run = run_id(item, id, input);
    item.pending = None;
    if matches!(item.definition.stages[item.stage].progression, Progression::AwaitApproval { .. }) {
        item.phase = Phase::WaitingApproval;
        Ok(())
    } else { launch(item, id, input, effects) }
}
fn launch(item: &mut Item, id: &str, input: &Input, effects: &mut Vec<Effect>) -> Result<(), Refusal> {
    let kind = item.definition.stages[item.stage].kind.clone();
    if matches!(kind, StageKind::Review(Reviewer::Person(_) | Reviewer::Orchestrator(_))) {
        item.phase = Phase::WaitingReview;
    } else {
        item.phase = Phase::Running;
        emit(item, id, input, EffectKind::Execute(kind), effects);
    }
    Ok(())
}
fn emit(item: &mut Item, id: &str, input: &Input, kind: EffectKind, effects: &mut Vec<Effect>) {
    let effect_id = digest(&("effect/v1", &input.workspace, &input.id, effects.len()));
    item.pending = Some(effect_id.clone());
    effects.push(Effect { id: effect_id, item: id.into(), run: item.run.clone(),
        epoch: input.epoch, fence: item.fence, resource: item.resource.clone(),
        definition_pin: item.pin.clone(), policy_pin: item.policy_pin.clone(),
        candidate: item.candidate.clone(), members: item.members.clone(), kind,
        operation: item.definition.stages[item.stage].operation.clone(), created_ms: input.now_ms });
}
fn evidence_matches(item: &Item, e: &Evidence) -> Result<(), Refusal> {
    if !valid_id(&e.candidate) || !valid_id(&e.manifest) || !valid_id(&e.receipt)
        || e.policy_pin != item.policy_pin
    { return Err(Refusal::Evidence); }
    if matches!(item.definition.stages[item.stage].kind,
        StageKind::Check | StageKind::Review(_) | StageKind::Land | StageKind::Ci | StageKind::Tracker | StageKind::Evaluate(_))
        && item.candidate.as_ref() != Some(&e.candidate)
    { return Err(Refusal::Evidence); }
    Ok(())
}
fn finish(item: &mut Item, id: &str, input: &Input, passed: bool, evidence: &Evidence,
    effects: &mut Vec<Effect>) -> Result<(), Refusal>
{
    if item.definition.schema == 2 {
        return route(item, id, input, if passed { "passed" } else { "rejected" }, evidence, effects);
    }
    item.pending = None;
    item.last_evidence = Some(evidence.clone());
    if !passed {
        if matches!(item.definition.stages[item.stage].kind, StageKind::Review(_) | StageKind::Evaluate(_))
            && item.iteration < item.definition.max_reworks
        {
            item.iteration += 1;
            item.stage = item.definition.stages.iter().position(|stage|
                Some(&stage.id) == item.definition.rework_target.as_ref()).ok_or(Refusal::Definition)?;
            item.attempt = 1;
            item.candidate = None;
            item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
            return enter(item, id, input, effects);
        }
        item.phase = Phase::Failed;
        return Ok(());
    }
    if matches!(item.definition.stages[item.stage].kind, StageKind::Build | StageKind::Tracker | StageKind::Action(_)) {
        item.candidate = Some(evidence.candidate.clone());
    }
    item.stage += 1;
    item.attempt = 1;
    item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
    if item.stage == item.definition.stages.len() { item.phase = Phase::Done; Ok(()) }
    else { enter(item, id, input, effects) }
}

/// Where a label goes from the current step: its route, else forward on `passed` and to `Fail`.
fn target_of(item: &Item, label: &str) -> Target {
    let stage = &item.definition.stages[item.stage];
    stage.routes.get(label).cloned().unwrap_or_else(|| match label {
        "passed" if item.stage + 1 < item.definition.stages.len() =>
            Target::Step(item.definition.stages[item.stage + 1].id.clone()),
        "passed" => Target::Done,
        _ => Target::Fail,
    })
}
fn is_back_edge(item: &Item, target: &Target) -> bool {
    matches!(target, Target::Step(to) if item.definition.stages.iter().position(|s| &s.id == to)
        .is_some_and(|t| t <= item.stage))
}
/// Whether the step's route for `label` is a back edge with room left to take it.
fn rework_room(item: &Item, label: &str) -> Result<(), Refusal> {
    if !is_back_edge(item, &target_of(item, label)) { return Err(Refusal::Invalid); }
    let stage = &item.definition.stages[item.stage];
    if item.back_edges.get(&stage.id).copied().unwrap_or(0) >= stage.limit { return Err(Refusal::Limit); }
    Ok(())
}
/// Schema 2: take the route a label names. A back edge past its limit fails the run instead.
fn route(item: &mut Item, id: &str, input: &Input, label: &str, evidence: &Evidence,
    effects: &mut Vec<Effect>) -> Result<(), Refusal>
{
    item.pending = None;
    item.last_evidence = Some(evidence.clone());
    let stage = &item.definition.stages[item.stage];
    if label != "rejected" && makes_candidate(&stage.kind) { item.candidate = Some(evidence.candidate.clone()); }
    let target = target_of(item, label);
    let to = match &target {
        Target::Fail | Target::Split { .. } => { item.phase = Phase::Failed; return Ok(()); }
        Target::Done => {
            item.stage = item.definition.stages.len();
            item.phase = Phase::Done;
            return Ok(());
        }
        Target::Step(to) => item.definition.stages.iter().position(|s| &s.id == to).ok_or(Refusal::Definition)?,
    };
    if is_back_edge(item, &target) {
        let (key, limit) = (stage.id.clone(), stage.limit);
        let taken = item.back_edges.get(&key).copied().unwrap_or(0);
        if taken >= limit { item.phase = Phase::Failed; return Ok(()); }
        item.back_edges.insert(key, taken + 1);
        item.iteration = item.iteration.checked_add(1).ok_or(Refusal::Limit)?;
    }
    if makes_candidate_anew(&item.definition.stages[to].kind) { item.candidate = None; }
    item.stage = to;
    item.attempt = 1;
    item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
    enter(item, id, input, effects)
}

fn valid_state(state: &State) -> bool {
    valid_id(&state.workspace) && state.epoch > 0 && state.items.len() <= 1024
        && state.archive.iter().all(|(id, a)| valid_id(id) && !state.items.contains_key(id)
            && matches!(a.phase, Phase::Done | Phase::Failed | Phase::Cancelled)
            && a.candidate.as_deref().is_none_or(valid_id))
        && state.items.iter().all(|(id, item)| {
            valid_id(id) && item.definition.validate().is_ok() && item.pin == item.definition.pin()
                && valid_id(&item.policy_pin) && valid_id(&item.resource)
                && if item.definition.schema == 2 {
                    usize::from(item.iteration) == item.back_edges.values().map(|n| usize::from(*n)).sum::<usize>()
                        && item.back_edges.iter().all(|(stage, taken)| item.definition.stages.iter()
                            .any(|s| &s.id == stage && s.limit >= *taken))
                    } else { item.iteration <= item.definition.max_reworks && item.back_edges.is_empty() }
                && item.retries <= MAX_RETRIES
                && item.rework_history.len() <= usize::from(item.iteration)
                && item.rework_history.iter().enumerate().all(|(index, r)|
                    valid_id(&r.id) && valid_id(&r.principal) && valid_id(&r.run)
                    && r.definition_pin == item.pin && r.evidence.policy_pin == item.policy_pin
                    && valid_rework_reason(&r.reason) && r.iteration < item.iteration
                    && (index == 0 || item.rework_history[index-1].iteration < r.iteration))
                && if item.phase == Phase::Done { item.stage == item.definition.stages.len() }
                    else { item.stage < item.definition.stages.len() }
        })
}

fn request_size(input: &Input) -> Result<usize, Refusal> {
    // Occupancy is separately bounded store metadata, not caller command payload.
    let mut request = input.clone();
    request.busy_resources.clear();
    serde_json::to_vec(&request).map(|v| v.len()).map_err(|_| Refusal::Invalid)
}
