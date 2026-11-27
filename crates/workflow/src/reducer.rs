use crate::*;
use std::collections::{BTreeMap, BTreeSet};

pub fn step(state: &State, input: &Input) -> Result<Transition, Refusal> {
    step_inner(state, input, false)
}
fn new_item(definition: &Definition, policy_pin: &str, resource: &str,
    dependencies: &BTreeSet<String>, initial_candidate: &Option<String>) -> Item {
    Item {
        start_load: None, execution: None, definition: definition.clone(), pin: definition.pin(), policy_pin: policy_pin.into(),
        dependencies: dependencies.clone(), resource: resource.into(), stage: 0, iteration: 0, attempt: 1, fence: 1,
        phase: Phase::Blocked, run: String::new(), pending: None, candidate: initial_candidate.clone(),
        restart: None, reuse_checkout: false, carried_progress: false, history: Vec::new(), reason: None, landing_ci: None, last_evidence: None, stopping: None, stopped_from: None,
        cancel_reason: None, closed: None, failure: None, queue_hold: None, group_failure: None, deciding: None,
        hook_calls: BTreeMap::new(), hook_refused: None, ci_reruns: BTreeMap::new(), ci_retry: None, landed: None,
        rework_history: Vec::new(), back_edges: BTreeMap::new(), retries: 0, refused_launch: None, pushed: None, wait_since: None,
        members: Vec::new(), handed: Vec::new(), since_ms: 0, group_of: None, grouped_in: None,
        models: BTreeMap::new(), tree: None, sent_back: Vec::new(), brief: None, follows: None, followers: Vec::new(),
    }
}
#[path = "hook_reducer.rs"]
mod hook_reducer;
#[path = "state_validity.rs"]
mod state_validity;
use state_validity::valid_state;
fn step_inner(state: &State, input: &Input, settling_hook: bool) -> Result<Transition, Refusal> {
    if !valid_state(state) { return Err(Refusal::Invalid); }
    if input.schema != 1 || state.schema != 1 || !valid_id(&input.id)
        || !valid_id(&input.source) || !valid_id(&input.workspace)
        || input.source_sequence == 0 || state.items.len() > MAX_ITEMS
        || input.busy_resources.len() > 1024 || input.busy_resources.iter().any(|r| !valid_id(r))
        || request_size(input, false)? > 65536
    { return Err(Refusal::Invalid); }
    if state.seen.contains(&input.id) { return Err(Refusal::Duplicate); }
    if input.workspace != state.workspace || input.epoch != state.epoch
        || input.expected_revision != state.revision
        || input.source_sequence <= *state.source_sequences.get(&input.source).unwrap_or(&0)
    { return Err(Refusal::Stale); }
    if matches!(input.command, Command::Decide { .. }) { return hook_reducer::decide(state, input); }
    let mut next = state.clone();
    let mut effects = Vec::new();
    // Halves a group split off this step: (group it came from, members that form the new group).
    let mut spawn: Vec<(String, Vec<Member>)> = Vec::new();
    let mut notices = Vec::new();
    match &input.command {
        Command::Advance => { controller(input)?; }
        Command::RefreshGroups { definition, policy_pin } => {
            controller(input)?;
            definition.validate()?;
            if !valid_id(policy_pin) { return Err(Refusal::Invalid); }
            next.grouping = Some(Grouping { definition: definition.clone(), policy_pin: policy_pin.clone() });
        }
        Command::AdoptPublication { definition, policy_pin, items } => {
            controller(input)?;
            definition.validate()?;
            if !valid_id(policy_pin) || items.is_empty() || items.len() > MAX_ITEMS { return Err(Refusal::Invalid); }
            // An unpushed group belongs to the old landing plan. Release the members
            // without taking any of that plan's result routes.
            for (id, restart) in items {
                if !restart { continue; }
                let group = next.items.get_mut(id).ok_or(Refusal::Missing)?;
                if group.group_of.is_none() { continue; }
                if group.policy_pin == *policy_pin || group.pushed.is_some() || group.stopping.is_some() || group.deciding.is_some()
                    || matches!(group.phase, Phase::Done | Phase::Cancelled | Phase::Failed | Phase::Cancelling | Phase::Uncertain)
                    || group.members.iter().any(|member| items.get(&member.item) != Some(&true))
                { return Err(Refusal::Phase); }
                group.phase = Phase::Cancelled;
                group.pending = None;
                group.handed = group.members.iter().map(|m| m.item.clone()).collect();
                for member in std::mem::take(&mut group.members) {
                    let ticket = next.items.get_mut(&member.item).ok_or(Refusal::Missing)?;
                    ticket.phase = Phase::WaitingJoin;
                    ticket.grouped_in = None;
                }
            }
            for (id, restart) in items {
                let mut item = next.items.remove(id).ok_or(Refusal::Missing)?;
                if *restart && item.group_of.is_some() {
                    next.items.insert(id.clone(), item);
                    continue;
                }
                if (!restart && item.policy_pin == *policy_pin) || item.stopping.is_some() || item.deciding.is_some()
                    || matches!(item.phase, Phase::Done | Phase::Cancelled | Phase::Failed | Phase::Cancelling | Phase::Uncertain)
                { return Err(Refusal::Phase); }
                if *restart {
                    if item.grouped_in.is_some() { return Err(Refusal::Phase); }
                    let initial = matches!(definition.stages[0].kind, StageKind::Tracker).then(|| definition.pin());
                    let mut replacement = new_item(definition, policy_pin, &item.resource, &item.dependencies, &initial);
                    replacement.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
                    let mut prior = item.last_run();
                    prior.phase = Phase::Cancelled;
                    prior.reason = Some("A workflow publication restarted this run.".into());
                    replacement.history = std::mem::take(&mut item.history);
                    replacement.history.push(prior);
                    // The checkout identity is independent of the new run's fence and receipts.
                    replacement.tree = item.tree;
                    replacement.reuse_checkout = definition.stages.iter().any(|stage| stage.kind == StageKind::Build)
                        && (item.reuse_checkout || item.definition.stages[..item.stage].iter()
                            .any(|stage| stage.kind == StageKind::Provision));
                    replacement.restart = item.restart;
                    replacement.followers = item.followers;
                    replacement.start_load = item.start_load;
                    if replacement.reuse_checkout {
                        replacement.brief = Some("The workflow changed. Continue from the work already on this branch.".into());
                    }
                    replacement.models = definition.picked_models(&BTreeMap::new())?;
                    item = replacement;
                } else {
                    item.definition = if let Some(group) = &mut item.group_of {
                        group.pin = definition.pin();
                        definition.group_definition(&group.name).ok_or(Refusal::Definition)?
                    } else { definition.clone() };
                    item.pin = item.definition.pin();
                    item.policy_pin = policy_pin.clone();
                    let picks = item.models.into_iter().filter(|(stage, model)|
                        item.definition.models.get(stage).is_some_and(|list| list.contains(model))).collect();
                    item.models = item.definition.picked_models(&picks)?;
                    item.carried_progress = true;
                    item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
                    if item.pending.is_some() {
                        let kind = if item.ci_retry.as_ref().is_some_and(|retry| retry.pending) { StageKind::RerunCi }
                            else { item.definition.stages.get(item.stage).ok_or(Refusal::Definition)?.kind.clone() };
                        emit(&mut item, id, input, EffectKind::Execute(kind), &mut effects);
                    }
                }
                next.items.insert(id.clone(), item);
            }
        }
        Command::Enqueue { item, definition, policy_pin, resource, dependencies, initial_candidate } => {
            controller(input)?;
            definition.validate()?;
            if !valid_id(item) || !valid_id(resource) || !valid_id(policy_pin)
                || reserved_id(item) || next.items.contains_key(item) || next.archive.contains_key(item) || next.items.len() >= MAX_ITEMS
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
            let mut record = new_item(definition, policy_pin, resource, dependencies, initial_candidate);
            record.models = definition.picked_models(&BTreeMap::new())?;
            if ready(&next, &record, input) { enter(&mut record, item, input, &mut effects)?; }
            next.items.insert(item.clone(), record);
        }
        Command::Reopen { item, definition, policy_pin, resource } => {
            controller(input)?;
            // A closed ticket comes back at rest, under its own name: archived, or closed this step and not yet.
            let closed = |p: &Phase| matches!(p, Phase::Done | Phase::Cancelled);
            let live = next.items.get(item);
            if live.is_some_and(|i| !closed(&i.phase)) || live.is_some_and(|i| i.group_of.is_some()) { return Err(Refusal::Phase); }
            if live.is_none() && !next.archive.get(item).is_some_and(|a| closed(&a.phase)) { return Err(Refusal::Missing); }
            if !definition.is_ticket() || !valid_id(policy_pin) || !valid_id(resource) || reserved_id(item) { return Err(Refusal::Invalid); }
            definition.validate()?;
            if input.busy_resources.contains(resource) || next.items.values().any(|i| owns_resource(i) && i.resource == *resource) {
                return Err(Refusal::Resource);
            }
            let mut history = Vec::new();
            if let Some(mut prior) = next.items.remove(item) {
                history = std::mem::take(&mut prior.history);
                if !prior.definition.is_ticket() { history.push(prior.last_run()); }
            }
            if let Some(prior) = next.archive.remove(item) {
                history = prior.history;
                if let Some(run) = prior.last_run { history.push(run); }
            }
            if next.items.len() >= MAX_ITEMS { return Err(Refusal::Limit); }
            let mut record = new_item(definition, policy_pin, resource, &BTreeSet::new(), &Some(definition.pin()));
            record.history = history;
            if ready(&next, &record, input) { enter(&mut record, item, input, &mut effects)?; }
            next.items.insert(item.clone(), record);
        }
        Command::Start { item: id, definition, policy_pin, resource, dependencies, initial_candidate, models, tree, with, start_load } => {
            controller(input)?;
            let prior = next.items.remove(id);
            let archived = next.archive.remove(id);
            if let Some(item) = &prior {
                if !item.can_start() { return Err(Refusal::Phase); }
            } else {
                if !archived.as_ref().ok_or(Refusal::Missing)?.can_start() { return Err(Refusal::Phase); }
                if next.items.len() >= MAX_ITEMS { return Err(Refusal::Limit); }
            }
            if prior.as_ref().is_some_and(|item| item.phase == Phase::Failed
                && item.followers.iter().any(|f| !with.contains(f))) { return Err(Refusal::Phase); }
            // The run's other tickets: each a ticket at rest of its own, named once (ov-970).
            if with.len() > usize::from(definition.execution_limits().group_members) || with.iter().any(|f| f == id || !valid_id(f))
                || with.iter().collect::<BTreeSet<_>>().len() != with.len()
            { return Err(Refusal::Invalid); }
            for follower in with {
                let f = next.items.get(follower).ok_or(Refusal::Missing)?;
                if !f.definition.is_ticket() || f.phase != Phase::WaitingApproval || f.follows.as_ref().is_some_and(|lead| lead != id) || !f.followers.is_empty() {
                    return Err(Refusal::Phase);
                }
            }
            definition.validate()?;
            if definition.is_ticket() { return Err(Refusal::Invalid); }
            if !valid_id(resource) || !valid_id(policy_pin) || dependencies.len() > 128 { return Err(Refusal::Invalid); }
            let picked = definition.picked_models(models)?;
            if tree.as_ref().is_some_and(|t| !t.valid()) { return Err(Refusal::Invalid); }
            if matches!(definition.stages[0].kind, StageKind::Tracker) != initial_candidate.is_some()
                || initial_candidate.as_ref().is_some_and(|c| !valid_id(c))
            { return Err(Refusal::Evidence); }
            for dep in dependencies {
                if !next.items.contains_key(dep) && !next.archive.contains_key(dep) { return Err(Refusal::Missing); }
            }
            if dependencies.iter().all(|d| dependency_done(&next, d))
                && (input.busy_resources.contains(resource)
                    || next.items.values().any(|i| owns_resource(i) && i.resource == *resource)) {
                return Err(Refusal::Resource);
            }
            let mut item = new_item(definition, policy_pin, resource, dependencies, initial_candidate);
            if let Some(mut prior) = prior {
                item.fence = prior.fence.checked_add(1).ok_or(Refusal::Limit)?;
                item.history = std::mem::take(&mut prior.history);
                if matches!(prior.phase, Phase::Failed | Phase::Cancelled) {
                    item.history.push(prior.last_run());
                }
            } else if let Some(archived) = archived {
                let (run, pin) = archived.start_identity(id);
                let previous = archived.last_run.unwrap_or_else(|| RunRecord {
                    phase: Phase::Cancelled, run, stop_run: None, pin, fence: 0, candidate: archived.candidate,
                    tree: None, models: BTreeMap::new(), reason: None, rework_history: Vec::new(),
                    sent_back: Vec::new(), back_edges: BTreeMap::new(),
                });
                item.fence = previous.fence.checked_add(1).ok_or(Refusal::Limit)?;
                item.history = archived.history;
                item.history.push(previous);
            }
            if !item.history.is_empty() { item.restart = Some(input.id.clone()); }
            item.start_load = start_load.clone();
            item.models = picked;
            item.tree = tree.as_deref().cloned();
            item.followers = with.clone();
            for follower in with { next.items.get_mut(follower).ok_or(Refusal::Missing)?.follows = Some(id.clone()); }
            next.items.insert(id.clone(), item);
        }
        command => {
            let id = match command {
                Command::Approve { item, .. } | Command::Review { item, .. }
                | Command::Observe { item, .. } | Command::Cancel { item, .. } | Command::Retry { item } | Command::RestartReview { item }
                | Command::Queue { item } | Command::Leave { item } | Command::RefreshJoin { item, .. }
                | Command::HoldQueue { item, .. } | Command::Move { item, .. }
                | Command::Resolve { item, .. } | Command::SendBack { item, .. } | Command::Close { item, .. }
                | Command::Land { item, .. } | Command::RepinPending { item, .. } => item,
                Command::Decide { .. } | Command::Enqueue { .. } | Command::Advance | Command::RefreshGroups { .. } | Command::AdoptPublication { .. } | Command::Reopen { .. } | Command::Start { .. } => unreachable!(),
            };
            let mut item = next.items.remove(id).ok_or(Refusal::Missing)?;
            // Manual moves supersede a read-only decision without needing to stop an operation.
            if item.deciding.is_some() {
                if matches!(command, Command::Observe { .. } | Command::Resolve { .. }) { return Err(Refusal::Stale); }
                item.deciding = None; item.pending = None;
            }
            if !settling_hook && hook_reducer::ci(&mut item, id, input, &mut effects)? {
                next.items.insert(id.clone(), item);
            } else {
            match command {
                Command::RepinPending { effect, policy_pin, definition, .. } => {
                    controller(input)?;
                    if item.phase != Phase::Running || item.deciding.is_some() || item.stopping.is_some() {
                        return Err(Refusal::Phase);
                    }
                    if item.pending.as_ref() != Some(effect) || item.policy_pin == *policy_pin {
                        return Err(Refusal::Stale);
                    }
                    if !valid_id(policy_pin) { return Err(Refusal::Invalid); }
                    if let Some(definition) = definition {
                        definition.validate()?;
                        item.definition = if let Some(group) = &mut item.group_of {
                            group.pin = definition.pin();
                            definition.group_definition(&group.name).ok_or(Refusal::Definition)?
                        } else { *definition.clone() };
                        item.pin = item.definition.pin();
                        item.policy_pin = policy_pin.clone();
                        item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
                        let kind = if item.ci_retry.as_ref().is_some_and(|retry| retry.pending) {
                            StageKind::RerunCi
                        } else { item.definition.stages.get(item.stage).ok_or(Refusal::Definition)?.kind.clone() };
                        emit(&mut item, id, input, EffectKind::Execute(kind), &mut effects);
                    } else {
                        item.phase = Phase::Failed;
                        item.pending = None;
                        item.failure = Some("The published workflow re-pin changed more than the engine's own hooks. Carry this run's work to a new run under the published workflow.".into());
                    }
                }
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
                    // An in-place publication preserves the accepted evidence (including
                    // its original pin). Only that exact evidence can cross a pin change.
                    if !item.carried_progress || item.last_evidence.as_ref() != Some(evidence)
                        || item.candidate.as_ref() != Some(&evidence.candidate) {
                        evidence_matches(&item, evidence)?;
                    }
                    let rework = if let Some(reason) = reason {
                        if *passed || !valid_rework_reason(reason) { return Err(Refusal::Invalid); }
                        if item.last_evidence.as_ref() != Some(evidence) { return Err(Refusal::Evidence); }
                        if item.definition.schema == 2 {
                            if !is_back_edge(&item, &target_of(&item, "rejected")) { return Err(Refusal::Invalid); }
                        } else if item.definition.rework_target.is_none() { return Err(Refusal::Limit); }
                        Some(ReworkDecision {
                            id: input.id.clone(), principal: p.clone(), revision: input.expected_revision,
                            run: item.run.clone(), definition_pin: item.pin.clone(), evidence: evidence.clone(),
                            iteration: item.iteration, reason: reason.clone(),
                        })
                    } else { None };
                    finish(&mut item, id, input, *passed, evidence, reason.as_deref(),
                        Out { room: room(&next, &spawn), effects: &mut effects, spawn: &mut spawn })?;
                    // The verdict's brief survives a terminal rejection. History counts send-backs taken.
                    if let Some(rework) = rework.filter(|decision| item.iteration > decision.iteration) {
                        item.rework_history.push(rework);
                    }
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
                    // Whatever the answer, the wait's first deadline is spent unless a restart cut it short.
                    let wait_since = item.wait_since.take();
                    // A new answer replaces the last attempt's status.
                    item.failure = None;
                    item.reason = None;
                    // A rerun is an outside side effect. Its success resumes the CI look; anything
                    // ambiguous waits for reconciliation and can never dispatch the rerun again.
                    let rerunning = item.ci_retry.as_ref().is_some_and(|r| r.pending) && !item.run.starts_with("cancel:");
                    match outcome {
                        Outcome::Passed(e) if rerunning => {
                            evidence_matches(&item, e)?;
                            item.ci_retry.as_mut().unwrap().pending = false;
                            item.wait_since = None;
                            item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
                            launch(&mut item, id, input, &mut effects)?;
                        }
                        Outcome::Rejected(e) if rerunning && resolving => {
                            evidence_matches(&item, e)?;
                            route(&mut item, id, input, "rejected", Some(e), Out { room: room(&next, &spawn), effects: &mut effects, spawn: &mut spawn })?;
                        }
                        _ if rerunning => { item.phase = Phase::Uncertain; }
                        Outcome::CiResult { label, evidence, run, failure, landing_ci } => {
                            if item.run.starts_with("cancel:")
                                || item.definition.stages[item.stage].kind != StageKind::AwaitCi
                                || !matches!(label.as_str(), "passed" | "rejected" | "cancelled" | "timeout")
                            { return Err(Refusal::Phase); }
                            evidence_matches(&item, evidence)?;
                            if let Some(run) = run { if !run.valid() { return Err(Refusal::Evidence); } }
                            if let Some(retry) = &item.ci_retry {
                                if label != "timeout" && !run.as_ref().is_some_and(|r| r.id == retry.run.id && r.attempt > retry.run.attempt) {
                                    return Err(Refusal::Evidence);
                                }
                            }
                            if let Some(ci) = landing_ci {
                                if item.group_of.is_none() || item.pushed.as_ref() != Some(&evidence.candidate)
                                    || matches!(ci, LandingCi::Red { origin: Some(sha), .. }
                                        if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()))
                                    || ci.note().is_some_and(|note| !valid_rework_reason(&note) || note.chars().any(char::is_control))
                                { return Err(Refusal::Evidence); }
                            }
                            // A recovered receipt can settle uncertainty, but Uncertain is never a policy point.
                            if !resolving && label == "rejected" && crate::ci::rerun(&mut item, id, input, run.as_ref(), &mut effects)? {
                                item.last_evidence = Some(evidence.clone());
                            } else {
                                item.ci_retry = None;
                                item.landing_ci = landing_ci.clone();
                                if label != "passed" {
                                    if let Some(detail) = failure {
                                        if detail.job.is_empty() || detail.job.len() > 1024 || detail.first_line.len() > 4096
                                            || detail.job.chars().any(char::is_control) || detail.first_line.chars().any(char::is_control)
                                        { return Err(Refusal::Evidence); }
                                        if item.group_of.is_some() { item.group_failure = Some(detail.clone()); }
                                    }
                                }
                                route(&mut item, id, input, label, Some(evidence), Out { room: room(&next, &spawn), effects: &mut effects, spawn: &mut spawn })?;
                            }
                        }
                        Outcome::Hook(_) => return Err(Refusal::Invalid),
                        Outcome::Uncertain => { item.phase = Phase::Uncertain; }
                        Outcome::UncertainBecause(reason) => {
                            if !resolving || !valid_rework_reason(reason) { return Err(Refusal::Invalid); }
                            item.phase = Phase::Uncertain;
                            item.failure = Some(reason.clone());
                            if item.since_ms == 0 { item.since_ms = input.now_ms; }
                        }
                        Outcome::Stopped { operation, receipt } => {
                            // A stop receipt is only valid for the cancellation operation, never execution.
                            if !item.run.starts_with("cancel:") { return Err(Refusal::Phase); }
                            if item.stopping.as_ref() != Some(operation) || !valid_id(receipt) {
                                return Err(Refusal::Evidence);
                            }
                            item.stopping = None;
                            item.pending = None;
                            finish_close(&mut item);
                        }
                        Outcome::FoldConflict { evidence, files, members: None } => {
                            if item.run.starts_with("cancel:") || item.group_of.is_none()
                                || item.definition.stages[item.stage].kind != StageKind::Fold
                            { return Err(Refusal::Phase); }
                            evidence_matches(&item, evidence)?;
                            route(&mut item, id, input, "conflict", Some(evidence), Out { room: room(&next, &spawn), effects: &mut effects, spawn: &mut spawn })?;
                            if item.phase == Phase::Failed {
                                item.brief = Some(conflict_brief(evidence, files)?);
                            }
                        }
                        Outcome::FoldConflict { members: Some(members), evidence, files } => {
                            if item.run.starts_with("cancel:") { return Err(Refusal::Phase); }
                            if item.definition.schema != 2 || item.group_of.is_none()
                                || item.definition.stages[item.stage].kind != StageKind::Fold
                                || item.pushed.is_some() || members.is_empty()
                                || members.iter().collect::<BTreeSet<_>>().len() != members.len()
                                || members.iter().any(|id| !item.members.iter().any(|m| &m.item == id))
                            { return Err(Refusal::Evidence); }
                            evidence_matches(&item, evidence)?;
                            // A published non-splitting conflict policy still decides as written.
                            if let Target::Split { step, .. } = target_of(&item, "conflict") {
                                for member in members {
                                    let mut ticket = next.items.remove(member).ok_or(Refusal::Missing)?;
                                    ticket.grouped_in = None;
                                    if let Some(build) = ticket.definition.stages.iter().position(|s| s.kind == StageKind::Build) {
                                        ticket.brief = Some(conflict_brief(evidence, files)?);
                                        ticket.candidate = None; ticket.stage = build; ticket.attempt = 1;
                                        ticket.fence = ticket.fence.checked_add(1).ok_or(Refusal::Limit)?;
                                        ticket.phase = Phase::Blocked;
                                    } else {
                                        route(&mut ticket, member, input, "rejected", None,
                                            Out { room: room(&next, &spawn), effects: &mut effects, spawn: &mut spawn })?;
                                        ticket.brief = Some(conflict_brief(evidence, files)?);
                                    }
                                    next.items.insert(member.clone(), ticket);
                                }
                                item.members.retain(|m| !members.contains(&m.item));
                                if item.members.is_empty() {
                                    item.handed = members.clone();
                                    item.pending = None; item.last_evidence = Some(evidence.clone());
                                    item.phase = Phase::Failed;
                                } else {
                                    take_route(&mut item, id, input, ("conflict", Some(evidence)), Target::Step(step), None,
                                        Out { room: room(&next, &spawn), effects: &mut effects, spawn: &mut spawn })?;
                                }
                            } else {
                                route(&mut item, id, input, "conflict", Some(evidence),
                                    Out { room: room(&next, &spawn), effects: &mut effects, spawn: &mut spawn })?;
                                if item.phase == Phase::Failed { item.brief = Some(conflict_brief(evidence, files)?); }
                            }
                        }
                        Outcome::GateRejected { members, evidence, brief, failure } => {
                            if !valid_rework_reason(brief) || item.run.starts_with("cancel:") { return Err(Refusal::Phase); }
                            if item.definition.schema != 2 || item.group_of.is_none()
                                || !matches!(item.definition.stages[item.stage].kind, StageKind::Run | StageKind::Check)
                                || item.pushed.is_some()
                                || members.iter().collect::<BTreeSet<_>>().len() != members.len()
                                || members.iter().any(|id| !item.members.iter().any(|m| &m.item == id))
                            { return Err(Refusal::Evidence); }
                            evidence_matches(&item, evidence)?;
                            if failure.job.is_empty() || failure.job.len() > 1024 || failure.first_line.len() > 4096
                                || failure.job.chars().any(char::is_control) || failure.first_line.chars().any(char::is_control)
                            { return Err(Refusal::Evidence); }
                            item.group_failure = Some(failure.clone());
                            // A published non-splitting failure policy still decides as written.
                            if let Target::Split { step, .. } = target_of(&item, "rejected")
                                && !members.is_empty() {
                                for member in members {
                                    let mut ticket = next.items.remove(member).ok_or(Refusal::Missing)?;
                                    ticket.grouped_in = None;
                                    if let Some(build) = ticket.definition.stages.iter().position(|s| s.kind == StageKind::Build) {
                                        ticket.brief = Some(brief.clone());
                                        ticket.candidate = None; ticket.stage = build; ticket.attempt = 1;
                                        ticket.fence = ticket.fence.checked_add(1).ok_or(Refusal::Limit)?;
                                        ticket.phase = Phase::Blocked;
                                    } else {
                                        route(&mut ticket, member, input, "rejected", None,
                                            Out { room: room(&next, &spawn), effects: &mut effects, spawn: &mut spawn })?;
                                        ticket.brief = Some(brief.clone());
                                    }
                                    next.items.insert(member.clone(), ticket);
                                }
                                item.members.retain(|m| !members.contains(&m.item));
                                if item.members.is_empty() {
                                    item.handed = members.clone();
                                    item.pending = None; item.last_evidence = Some(evidence.clone());
                                    item.phase = Phase::Failed;
                                } else {
                                    // Removing at least one member bounds this retry by the original batch size.
                                    // Like a half split, it spends no review/back-edge budget.
                                    item.pending = None; item.ci_retry = None;
                                    item.last_evidence = Some(evidence.clone());
                                    item.stage = item.definition.stages.iter().position(|s| s.id == step).ok_or(Refusal::Definition)?;
                                    item.candidate = None; item.attempt = 1;
                                    item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
                                    enter(&mut item, id, input, &mut effects)?;
                                }
                            } else {
                                route(&mut item, id, input, "rejected", Some(evidence),
                                    Out { room: room(&next, &spawn), effects: &mut effects, spawn: &mut spawn })?;
                                if item.phase == Phase::Failed { item.brief = Some(brief.clone()); }
                            }
                        }
                        Outcome::Labeled(label, evidence) | Outcome::LabeledWithReason { label, evidence, .. } => {
                            if item.run.starts_with("cancel:") { return Err(Refusal::Phase); }
                            let stage = &item.definition.stages[item.stage];
                            if item.definition.schema != 2 { return Err(Refusal::Invalid); }
                            // Any label the step can emit is an answer, routed or not: an unrouted one fails the item
                            // (`target_of`) instead of being refused, which would leave the item uncertain for good.
                            if !(emits_unrouted(&stage.kind, label) || stage.routes.contains_key(label)) {
                                return Err(Refusal::Evidence);
                            }
                            evidence_matches(&item, evidence)?;
                            route(&mut item, id, input, label, Some(evidence), Out { room: room(&next, &spawn), effects: &mut effects, spawn: &mut spawn })?;
                        }
                        Outcome::ReviewRejected { evidence, summary, receipt } => {
                            if item.run.starts_with("cancel:") || !matches!(item.definition.stages[item.stage].kind, StageKind::Review(_))
                                || !valid_rework_reason(summary) || !valid_id(receipt) { return Err(Refusal::Evidence); }
                            evidence_matches(&item, evidence)?;
                            finish(&mut item, id, input, false, evidence, Some(summary),
                                Out { room: room(&next, &spawn), effects: &mut effects, spawn: &mut spawn })?;
                            if item.phase == Phase::Failed { describe_failure(&mut item, summary, Some(receipt)); }
                        }
                        Outcome::Passed(evidence) | Outcome::Rejected(evidence) | Outcome::RejectedWithDetails(evidence, _) | Outcome::RejectedWithBrief(evidence, _) => {
                            if item.run.starts_with("cancel:") { return Err(Refusal::Phase); }
                            evidence_matches(&item, evidence)?;
                            if let Outcome::RejectedWithDetails(_, detail) = outcome {
                                if detail.job.is_empty() || detail.job.len() > 1024 || detail.first_line.len() > 4096
                                    || detail.job.chars().any(char::is_control) || detail.first_line.chars().any(char::is_control)
                                { return Err(Refusal::Evidence); }
                                if item.group_of.is_some() { item.group_failure = Some(detail.clone()); }
                            }
                            if let Outcome::RejectedWithBrief(_, brief) = outcome {
                                if item.group_of.is_some() || item.definition.stages[item.stage].kind != StageKind::Check || !valid_rework_reason(brief) { return Err(Refusal::Evidence); }
                                item.brief = Some(brief.clone());
                            }
                            finish(&mut item, id, input, matches!(outcome, Outcome::Passed(_)), evidence, None,
                                Out { room: room(&next, &spawn), effects: &mut effects, spawn: &mut spawn })?;
                        }
                        Outcome::SafeFailure | Outcome::SafeFailureBecause(_) | Outcome::SafeFailureWithReceipt { .. } => {
                            if item.run.starts_with("cancel:") { return Err(Refusal::Phase); }
                            // An effect's reason that can't go on the ticket is dropped, not the failure: refusing the
                            // answer would leave a plain failure Uncertain, for a person (ov-1061). A person's own
                            // Resolve is still refused, so they can fix the sentence.
                            if let Outcome::SafeFailureWithReceipt { receipt, .. } = outcome {
                                if !valid_id(receipt) { return Err(Refusal::Evidence); }
                            }
                            if let Outcome::SafeFailureBecause(reason)
                                | Outcome::SafeFailureWithReceipt { reason: Some(reason), .. } = outcome {
                                if valid_rework_reason(reason) { item.failure = Some(reason.clone()); }
                                else if resolving { return Err(Refusal::Invalid); }
                            }
                            item.pending = None;
                            // Landing infrastructure gets two retries, independent of code-rejection routes.
                            let stage = &item.definition.stages[item.stage];
                            let attempts = if item.group_of.is_some() && matches!(stage.kind,
                                StageKind::AwaitBase | StageKind::Fold | StageKind::Run | StageKind::AwaitCi | StageKind::PushCandidate) {
                                3
                            } else { stage.max_attempts };
                            if item.attempt >= attempts {
                                item.phase = Phase::Failed;
                            } else {
                                item.attempt += 1;
                                item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
                                enter(&mut item, id, input, &mut effects)?;
                            }
                        }
                        Outcome::LaunchRefused(reason) => {
                            if item.run.starts_with("cancel:") { return Err(Refusal::Phase); }
                            if valid_rework_reason(reason) { item.failure = Some(reason.clone()); }
                            else if resolving { return Err(Refusal::Invalid); }
                            // Launching again would meet the same refusal, so the run waits for a person.
                            item.pending = None;
                            item.refused_launch = Some(item.fence);
                            item.phase = Phase::Failed;
                        }
                        Outcome::ResourceHold(reason) => {
                            // A group's gate waits for its resources; a build waits for disk space (ov-1516).
                            let holds = match item.definition.stages[item.stage].kind {
                                StageKind::Run => item.group_of.is_some(),
                                StageKind::Build => true,
                                _ => false,
                            };
                            if item.run.starts_with("cancel:") || !holds || !valid_rework_reason(reason) { return Err(Refusal::Invalid); }
                            item.failure = Some(reason.clone());
                            item.pending = None;
                            item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
                            enter(&mut item, id, input, &mut effects)?;
                        }
                        Outcome::Interrupted => {
                            if item.run.starts_with("cancel:") { return Err(Refusal::Phase); }
                            let stage = &item.definition.stages[item.stage];
                            if stage.kind.resumes_after_runner_restart() {
                                item.reason = Some(format!("The {} step was interrupted by an app update or runner restart and resumed.", stage.id));
                            } else if !is_wait(&stage.kind) && stage.kind != StageKind::Push {
                                return Err(Refusal::Phase);
                            }
                            item.pending = None;
                            item.wait_since = wait_since;
                            item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
                            // Keep the run identity and its attempt, including any recorded base and worktree.
                            launch(&mut item, id, input, &mut effects)?;
                        }
                    }
                    if item.phase == Phase::Failed {
                        match outcome {
                            Outcome::SafeFailure | Outcome::SafeFailureBecause(_) | Outcome::SafeFailureWithReceipt { .. } => {
                                let cause = item.failure.as_deref().unwrap_or("The operation failed before completing.").to_owned();
                                let step = &item.definition.stages[item.stage].id;
                                let receipt = match outcome {
                                    Outcome::SafeFailureWithReceipt { receipt, .. } => receipt.as_str(),
                                    // Pre-launch refusals and old observations have only the store receipt.
                                    _ => input.id.as_str(),
                                };
                                item.reason = Some(format_reason(step, &format!("{cause} Attempts exhausted."), Some(receipt)));
                            }
                            Outcome::GateRejected { evidence, failure, .. }
                            | Outcome::RejectedWithDetails(evidence, failure)
                            | Outcome::CiResult { label: _, evidence, failure: Some(failure), .. } => {
                                describe_failure(&mut item, &format!("{}: {}", failure.job, failure.first_line), Some(&evidence.receipt));
                            }
                            Outcome::RejectedWithBrief(evidence, brief)
                            | Outcome::LabeledWithReason { evidence, reason: brief, .. } =>
                                describe_failure(&mut item, brief, Some(&evidence.receipt)),
                            Outcome::FoldConflict { evidence, files, .. } => {
                                describe_failure(&mut item, &conflict_brief(evidence, files)?, Some(&evidence.receipt));
                            }
                            Outcome::LaunchRefused(_) => {
                                let cause = item.failure.as_deref().unwrap_or("The runner refused to launch the step.").to_owned();
                                describe_failure(&mut item, &cause, Some(&input.id));
                            }
                            _ => {}
                        }
                    }
                }
                Command::Close { resolution, .. } => {
                    controller(input)?;
                    // A group's push owns other work. Its settlement remains the landing protocol's.
                    if item.definition.is_ticket() || item.follows.is_some() || item.group_of.is_some()
                        || item.stopping.is_some()
                        || matches!(item.phase, Phase::Done | Phase::Cancelled | Phase::Cancelling) {
                        return Err(Refusal::Phase);
                    }
                    match resolution {
                        CloseResolution::Landed { sha } if sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()) => {}
                        CloseResolution::Continued { item: other } if other != id && valid_id(other) => {}
                        _ => return Err(Refusal::Invalid),
                    }
                    if item.phase == Phase::Joined { detach(&mut next, id, &mut item)?; }
                    item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
                    item.closed = Some(resolution.clone());
                    if let Some(operation) = item.pending.take() {
                        // The close survives a crash while Stop is outstanding. Its stop receipt, not the
                        // caller's claim, makes the final move, so no active worker is abandoned.
                        item.phase = Phase::Cancelling;
                        item.stopping = Some(operation.clone());
                        let stop_run = format!("cancel:{}", run_id(&item, id, input));
                        item.stopped_from = Some(std::mem::replace(&mut item.run, stop_run));
                        emit(&mut item, id, input, EffectKind::Stop { operation }, &mut effects);
                    } else { finish_close(&mut item); }
                }
                Command::Cancel { reason, .. } => {
                    controller(input)?;
                    // A follower goes where its lead's run goes (ov-970).
                    if item.follows.is_some() { return Err(Refusal::Phase); }
                    // A queued ticket can be cancelled only while its group still waits.
                    if item.phase == Phase::Joined { detach(&mut next, id, &mut item)?; }
                    // A failed run is closed by cancelling it: that is the controller's way off the needs-you list.
                    if matches!(item.phase, Phase::Done | Phase::Cancelled) { return Err(Refusal::Phase); }
                    if item.phase == Phase::Cancelling || item.run.starts_with("cancel:") {
                        return Err(Refusal::Phase);
                    }
                    // A group whose push may be in flight, or whose push landed, cannot be called back: its
                    // members' commits are on the target. A person settles a lost push; CI decides the rest.
                    if !item.members.is_empty() && (item.pushed.is_some()
                        || matches!(item.definition.stages[item.stage].kind, StageKind::Push)) {
                        return Err(Refusal::Phase);
                    }
                    item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
                    item.cancel_reason = reason.clone();
                    if let Some(operation) = item.pending.take() {
                        item.phase = Phase::Cancelling;
                        item.stopping = Some(operation.clone());
                        let stop_run = format!("cancel:{}", run_id(&item, id, input));
                        item.stopped_from = Some(std::mem::replace(&mut item.run, stop_run));
                        emit(&mut item, id, input, EffectKind::Stop { operation }, &mut effects);
                    } else { item.phase = Phase::Cancelled; }
                }
                Command::Queue { .. } => {
                    controller(input)?;
                    let StageKind::Join { group: name, .. } = &item.definition.stages[item.stage].kind.clone() else {
                        return Err(Refusal::Phase);
                    };
                    if item.phase != Phase::WaitingJoin { return Err(Refusal::Phase); }
                    let candidate = item.candidate.clone().ok_or(Refusal::Invalid)?;
                    let (definition, policy_pin) = next.grouping.as_ref()
                        .filter(|p| p.definition.groups.contains_key(name))
                        .map_or((&item.definition, &item.policy_pin), |p| (&p.definition, &p.policy_pin));
                    let pin = definition.pin();
                    let limit = definition.groups.get(name).ok_or(Refusal::Definition)?.limit;
                    // The first open group of this definition and policy with room, oldest first.
                    let open = next.items.iter().find(|(_, g)| g.phase == Phase::Blocked
                        && has_room(limit, g.members.len()) && g.stage == 0 && g.candidate.is_none() && g.policy_pin == *policy_pin
                        && g.group_of.as_ref().is_some_and(|o| &o.name == name && o.pin == pin))
                        .map(|(gid, _)| gid.clone());
                    let gid = match open {
                        Some(gid) => gid,
                        None => {
                            // The ticket is out of the map while it is worked on, so count it.
                            if next.items.len() + 1 >= MAX_ITEMS { return Err(Refusal::Limit); }
                            let gid = group_id(state, name);
                            let resource = definition.groups[name].exclusive.clone();
                            let definition = definition.group_definition(name).ok_or(Refusal::Definition)?;
                            next.items.insert(gid.clone(), Item { start_load: None, execution: None, restart: None, reuse_checkout: false, carried_progress: false, history: Vec::new(), reason: None, landing_ci: None, pin: definition.pin(), definition,
                                policy_pin: policy_pin.clone(), dependencies: BTreeSet::new(), resource,
                                stage: 0, iteration: 0, attempt: 1, fence: 1, phase: Phase::Blocked,
                                run: String::new(), pending: None, candidate: None, last_evidence: None,
                                stopping: None, stopped_from: None, cancel_reason: None, closed: None, failure: None, queue_hold: None, group_failure: None, deciding: None, hook_calls: BTreeMap::new(), hook_refused: None, ci_reruns: BTreeMap::new(), ci_retry: None, landed: None, rework_history: Vec::new(), back_edges: BTreeMap::new(), retries: 0, refused_launch: None, pushed: None, wait_since: None,
                                members: Vec::new(), handed: Vec::new(), since_ms: 0,
                                group_of: Some(GroupOf { name: name.clone(), pin }),
                                grouped_in: None, models: BTreeMap::new(), tree: None, sent_back: Vec::new(), brief: None, follows: None, followers: Vec::new() });
                            gid
                        }
                    };
                    next.items.get_mut(&gid).ok_or(Refusal::Missing)?.members.push(Member { item: id.clone(), candidate });
                    item.grouped_in = Some(gid);
                    item.phase = Phase::Joined;
                }
                Command::HoldQueue { reason, .. } => {
                    controller(input)?;
                    if item.phase != Phase::WaitingJoin { return Err(Refusal::Phase); }
                    if !valid_rework_reason(reason) { return Err(Refusal::Invalid); }
                    let hold = item.queue_hold.get_or_insert_with(|| QueueHold {
                        reason: reason.clone(), since_ms: input.now_ms, notified: false,
                    });
                    hold.reason.clone_from(reason);
                    hold.notified |= input.now_ms.saturating_sub(hold.since_ms) >= QUEUE_HOLD_NOTICE_MS;
                }
                Command::RefreshJoin { previous, base, result, .. } => {
                    controller(input)?;
                    if item.phase != Phase::WaitingJoin || item.pending.is_some() { return Err(Refusal::Phase); }
                    if item.candidate.as_ref() != Some(previous) { return Err(Refusal::Stale); }
                    let sha = |s: &str| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit());
                    if !sha(previous) || !sha(base) { return Err(Refusal::Invalid); }
                    match result {
                        JoinRefresh::Rebased { candidate } if sha(candidate) && candidate != previous => {
                            item.candidate = Some(candidate.clone());
                        }
                        JoinRefresh::Conflict { paths } if !paths.is_empty() => {
                            let build = item.definition.stages[..item.stage].iter().rposition(|s| s.kind == StageKind::Build)
                                .ok_or(Refusal::Phase)?;
                            if item.definition.stages[build..item.stage].iter().any(|s| lands(&s.kind)) { return Err(Refusal::Phase); }
                            item.brief = Some(rebase_conflict_brief(base, paths));
                            item.candidate = None;
                            item.stage = build;
                            item.attempt = 1;
                            item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
                            item.phase = Phase::Blocked;
                            item.run.clear();
                        }
                        _ => return Err(Refusal::Invalid),
                    }
                }
                Command::Leave { .. } => {
                    controller(input)?;
                    detach(&mut next, id, &mut item)?;
                }
                Command::Retry { .. } | Command::RestartReview { .. } => {
                    controller(input)?;
                    // A failed run does not own its resource, so a retry queues like new work.
                    if item.phase != Phase::Failed || item.group_of.is_some() { return Err(Refusal::Phase); }
                    if matches!(command, Command::RestartReview { .. }) {
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
                    } else if item.refused_launch != Some(item.fence) {
                        if item.retries >= item.definition.execution_limits().retries { return Err(Refusal::Limit); }
                        item.retries += 1;
                    }
                    item.refused_launch = None;
                    item.attempt = 1;
                    item.failure = None;
                    item.reason = None;
                    item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
                    item.phase = Phase::Blocked;
                }
                Command::Move { to, .. } => {
                    controller(input)?;
                    if !item.definition.is_ticket() || item.phase != Phase::WaitingApproval || item.follows.is_some() { return Err(Refusal::Phase); }
                    let (stage, phase) = match to.as_str() {
                        "done" => (item.definition.stages.len(), Phase::Done),
                        "cancelled" => (item.stage, Phase::Cancelled),
                        column => (item.definition.stages.iter().position(|s| s.id == column).ok_or(Refusal::Invalid)?,
                            Phase::WaitingApproval),
                    };
                    if phase == Phase::WaitingApproval && stage == item.stage { return Err(Refusal::Invalid); }
                    item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
                    item.stage = stage; item.phase = phase; item.attempt = 1;
                    // Each column is a run of its own, so a decision made in the last one cannot be replayed in this one.
                    if item.phase == Phase::WaitingApproval { item.run = run_id(&item, id, input); }
                }
                Command::SendBack { reason, .. } => {
                    controller(input)?;
                    // Only while nothing is in flight: an effect running now would answer for a step it left. A build
                    // left Uncertain is the one exception (ov-1090): its effect is fenced off by the send-back, and the
                    // runner takes it only once that build's process is known to have ended.
                    let uncertain_build = item.phase == Phase::Uncertain && item.stopping.is_none()
                        && !item.run.starts_with("cancel:") && item.definition.stages[item.stage].kind == StageKind::Build;
                    if !(matches!(item.phase, Phase::WaitingReview | Phase::WaitingApproval | Phase::WaitingJoin | Phase::Failed)
                            && item.pending.is_none() || uncertain_build)
                        || item.group_of.is_some() || item.definition.is_ticket()
                    { return Err(Refusal::Phase); }
                    if !valid_rework_reason(reason) { return Err(Refusal::Invalid); }
                    let to = item.definition.stages[..=item.stage].iter().rposition(|s| s.kind == StageKind::Build)
                        .ok_or(Refusal::Invalid)?;
                    // Never back across a landing: what it landed would be built and landed again.
                    if item.definition.stages[to..=item.stage].iter().any(|s| lands(&s.kind)) { return Err(Refusal::Phase); }
                    let stage = &item.definition.stages[item.stage];
                    let refused = |cause: &str| Refusal::SendBackRefused(format_reason(&stage.id, cause, None));
                    let limit = item.definition.send_back_limit();
                    if item.sent_back.len() >= usize::from(limit) {
                        return Err(refused(&format!("The send-back limit of {limit} has been reached.")));
                    }
                    if stage.limit > 0 && item.back_edges.get(&stage.id).copied().unwrap_or(0) >= stage.limit {
                        return Err(refused(&format!("The back-edge limit of {} has been reached.", stage.limit)));
                    }
                    if item.fence == u64::MAX {
                        return Err(refused(&format!("The fence limit of {} has been reached.", u64::MAX)));
                    }
                    if item.phase == Phase::WaitingJoin && stage.limit > 0 {
                        let taken = item.back_edges.entry(stage.id.clone()).or_default();
                        *taken += 1;
                        item.iteration = item.iteration.checked_add(1).ok_or(Refusal::Limit)?;
                    }
                    item.sent_back.push(SendBack { id: input.id.clone(), revision: input.expected_revision, run: item.run.clone(),
                        from: item.definition.stages[item.stage].id.clone(), to: item.definition.stages[to].id.clone(), reason: reason.clone() });
                    item.brief = Some(reason.clone());
                    item.failure = None;
                    item.reason = None;
                    // It builds anew in the same tree, and queues for its resource like new work.
                    item.stage = to; item.attempt = 1; item.candidate = None; item.pending = None;
                    item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
                    item.phase = Phase::Blocked; item.run = String::new();
                }
                Command::Land { commit, .. } => {
                    controller(input)?;
                    // Only a run that failed and holds nothing: not a group, not a follower, not a ticket at rest.
                    if item.phase != Phase::Failed || item.pending.is_some() || item.group_of.is_some()
                        || item.follows.is_some() || item.definition.is_ticket() { return Err(Refusal::Phase); }
                    if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) { return Err(Refusal::Invalid); }
                    item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
                    // Done is what a landed run is, and the commit is the candidate a landed run keeps.
                    item.candidate = Some(commit.to_ascii_lowercase());
                    item.landed = item.candidate.clone();
                    item.stage = item.definition.stages.len();
                    item.phase = Phase::Done;
                    item.failure = None; item.reason = None; item.brief = None; item.stopping = None;
                }
                Command::Decide { .. } | Command::Enqueue { .. } | Command::Advance | Command::RefreshGroups { .. } | Command::AdoptPublication { .. } | Command::Reopen { .. } | Command::Start { .. } => unreachable!(),
            }
            next.items.insert(id.clone(), item);
            for (from, members) in spawn.drain(..) {
                let mut rest = next.items.get(&from).ok_or(Refusal::Missing)?.clone();
                let name = rest.group_of.as_ref().ok_or(Refusal::Invalid)?.name.clone();
                let gid = group_id(state, &name);
                if next.items.len() >= MAX_ITEMS || next.items.contains_key(&gid) { return Err(Refusal::Limit); }
                for m in &members {
                    next.items.get_mut(&m.item).ok_or(Refusal::Missing)?.grouped_in = Some(gid.clone());
                }
                // The second half starts from nothing, behind whatever holds the resource.
                rest.members = members; rest.stage = 0; rest.iteration = 0; rest.attempt = 1; rest.fence = 1;
                rest.phase = Phase::Blocked; rest.run = String::new(); rest.pending = None; rest.candidate = None;
                rest.last_evidence = None; rest.stopping = None; rest.rework_history = Vec::new();
                rest.back_edges = BTreeMap::new(); rest.retries = 0; rest.wait_since = None;
                // This is a new run; its rules have not spent any calls or refused a decision.
                rest.hook_calls.clear(); rest.hook_refused = None;
                crate::hooks::fire(&mut rest, &gid, input, HookPoint::GroupSplits, None, &mut effects);
                next.items.insert(gid, rest);
            }
            }
        }
    }
    hook_reducer::failed(state, &mut next, input, &mut effects);
    // A failed run stays: cancelling it is the controller's way off the needs-you list.
    // An item that was already finished before this step leaves live state now, so its final
    // phase has been seen by the board and the notices before it goes. The event log keeps all of it.
    let finished: Vec<_> = state.items.iter().filter(|(_, i)| matches!(i.phase, Phase::Done | Phase::Cancelled))
        .map(|(id, _)| id.clone()).collect();
    for id in finished {
        // A ticket reopened this step stays (ov-968).
        if next.items.get(&id).is_some_and(|i| matches!(i.phase, Phase::Done | Phase::Cancelled)) {
            let item = next.items.remove(&id).ok_or(Refusal::Missing)?;
            let group = item.group_of.as_ref().map(|g| ArchivedGroup {
                name: g.name.clone(), tickets: item.handed.clone(), pushed: item.pushed.clone().or_else(|| item.candidate.clone()), since_ms: item.since_ms, landing_ci: item.landing_ci.clone() });
            let last_run = (group.is_none() && (!item.definition.is_ticket() || item.phase == Phase::Cancelled)).then(|| item.last_run());
            next.archive.insert(id, Archived { start_load: item.start_load, closed:item.closed, phase: item.phase, candidate: item.candidate,
                group, last_run, history: item.history });
        }
    }
    // A group that ended hands each member its result, through the member's own join routes.
    let ended: Vec<String> = next.items.iter().filter(|(_, g)| g.group_of.is_some() && !g.members.is_empty()
        && g.deciding.is_none() && matches!(g.phase, Phase::Done | Phase::Failed | Phase::Cancelled)).map(|(gid, _)| gid.clone()).collect();
    for gid in ended {
        let group = next.items.get_mut(&gid).ok_or(Refusal::Missing)?;
        // Once the push landed the members' commits are on the target: the result is `landed`, never
        // `rejected` or `dropped`, whatever ended the group after it.
        let label = match (&group.phase, &group.pushed) {
            (Phase::Done, _) => "passed",
            (_, Some(_)) => "landed",
            (Phase::Failed, None) => "rejected",
            _ => "dropped",
        };
        let result = if label == "landed" { group.pushed.clone() } else { group.candidate.clone() };
        let rebase = (label == "rejected" && group.definition.stages[group.stage].kind == StageKind::Fold)
            .then(|| group.brief.clone()).flatten();
        let gate_brief = (label == "rejected" && matches!(group.definition.stages[group.stage].kind,
            StageKind::Run | StageKind::Check | StageKind::Review(_)))
            .then(|| group.brief.clone()).flatten();
        let group_reason = group.failed_reason();
        let group_pushed = group.pushed.clone();
        let inherited = group.landing_ci.as_ref().filter(|ci| label == "passed" && ci.note().is_some()).cloned();
        let members = std::mem::take(&mut group.members);
        // Remember whom it ran: the board still draws a finished group with its tickets.
        group.handed = members.iter().map(|m| m.item.clone()).collect();
        for member in members {
            let mut ticket = next.items.remove(&member.item).ok_or(Refusal::Missing)?;
            ticket.grouped_in = None;
            if let StageKind::Join { adopt: true, .. } = ticket.definition.stages[ticket.stage].kind {
                if matches!(label, "passed" | "landed") && result.is_some() { ticket.candidate = result.clone(); }
            }
            if label == "dropped" && !ticket.definition.stages[ticket.stage].routes.contains_key("dropped") {
                ticket.pending = None;
                ticket.phase = Phase::WaitingJoin;
                ticket.fence = ticket.fence.checked_add(1).ok_or(Refusal::Limit)?;
            } else if let Some((reason, build)) = rebase.as_ref().and_then(|reason| ticket.definition.stages.iter()
                .position(|s| s.kind == StageKind::Build).map(|build| (reason, build))) {
                // A stale base is integration work, not a rejected review. It spends no back edge or iteration.
                // Queue like other join returns; enter() still enforces dependencies, resources and approval.
                ticket.brief = Some(reason.clone());
                ticket.candidate = None;
                ticket.stage = build;
                ticket.attempt = 1;
                ticket.fence = ticket.fence.checked_add(1).ok_or(Refusal::Limit)?;
                ticket.phase = Phase::Blocked;
            } else {
                if let Some(brief) = &gate_brief { ticket.brief = Some(brief.clone()); }
                route(&mut ticket, &member.item, input, label, None, Out { room: room(&next, &spawn), effects: &mut effects, spawn: &mut spawn })?;
                if ticket.phase == Phase::Done && inherited.is_some() {
                    ticket.landing_ci = inherited.clone();
                    ticket.landed = group_pushed.clone();
                }
                if ticket.phase == Phase::Failed && let Some(reason) = &group_reason {
                    describe_failure(&mut ticket, &format!("Group {gid} failed at {reason}"), None);
                }
            }
            next.items.insert(member.item, ticket);
        }
    }
    // A run that lands takes the tickets that follow it with it (ov-970). A cancelled one hands them back at
    // rest, as they were before it started: cancelling a run drops its own ticket, not the others it carried.
    // A failed one keeps them.
    let closing: Vec<_> = next.items.values().filter(|i| matches!(i.phase, Phase::Done | Phase::Cancelled))
        .flat_map(|lead| lead.followers.iter().map(|f| (f.clone(), lead.phase.clone(), lead.landed.clone(), lead.landing_ci.clone()))).collect();
    for (follower, phase, landed, landing_ci) in closing {
        let Some(ticket) = next.items.get_mut(&follower).filter(|t| t.follows.is_some()) else { continue };
        if phase == Phase::Done { ticket.stage = ticket.definition.stages.len(); ticket.phase = Phase::Done; ticket.landed = landed; ticket.landing_ci = landing_ci; }
        ticket.fence = ticket.fence.checked_add(1).ok_or(Refusal::Limit)?;
        ticket.follows = None;
    }
    // The runtime refreshes group rules after migration. The old rules must
    // not bind a just-migrated idle group back to its previous publication.
    if !matches!(input.command, Command::AdoptPublication { .. }) { consolidate_groups(&mut next)?; }
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
    if let Command::SendBack { item, .. } = &input.command {
        let restarted = next.items.get(item).ok_or(Refusal::Missing)?;
        if restarted.phase != Phase::Running || restarted.definition.stages[restarted.stage].kind != StageKind::Build {
            let reason = match restarted.phase {
                Phase::WaitingApproval => "The build could not start because it requires approval.",
                Phase::Blocked if !restarted.dependencies.iter().all(|d| dependency_done(&next, d)) =>
                    "The build could not start because a dependency is not done.",
                Phase::Blocked => "The build could not start because its resource is unavailable.",
                _ => "The build could not start.",
            };
            return Err(Refusal::SendBackRefused(format_reason(&restarted.definition.stages[restarted.stage].id, reason, None)));
        }
    }
    // A group result can fail a member at its join, after the first failure pass.
    hook_reducer::failed(state, &mut next, input, &mut effects);
    // A hold belongs to one waiting candidate and ends when it leaves that join.
    for (id, item) in &mut next.items {
        if item.phase != Phase::WaitingJoin || state.items.get(id).is_some_and(|old|
            old.run != item.run || old.stage != item.stage || old.candidate != item.candidate)
        {
            item.queue_hold = None;
        }
    }
    // A group says when it reached the step and phase it is in now.
    for (id, item) in next.items.iter_mut().filter(|(_, i)| i.group_of.is_some()) {
        if state.items.get(id).is_none_or(|old| old.stage != item.stage || old.phase != item.phase) {
            item.since_ms = input.now_ms;
        }
    }
    for (id, item) in &next.items {
        if state.items.get(id) != Some(item) {
            notices.push(format!("{id}:{:?}", item.phase));
        }
    }
    // Waiting groups are facts of this exact firing revision, never a runtime lookup.
    for effect in &mut effects {
        if let EffectKind::Decide { input: document, .. } = &mut effect.kind {
            let subject = next.items.get(&effect.item).ok_or(Refusal::Missing)?;
            document.groups = next.items.iter().filter(|(id, g)| *id != &effect.item && g.group_of.is_some()
                && g.group_of == subject.group_of && g.pin == subject.pin && g.resource == subject.resource)
                .map(|(id, g)| HookGroup { item: id.clone(), phase: g.phase.clone(), stage: g.stage, members: g.members.clone() }).collect();
            if let Some(call) = next.items.get_mut(&effect.item).and_then(|i| i.deciding.as_mut()) {
                call.input = *document.clone();
            }
        }
    }
    next.revision = next.revision.checked_add(1).ok_or(Refusal::Limit)?;
    next.seen.insert(input.id.clone());
    next.source_sequences.insert(input.source.clone(), input.source_sequence);
    Ok(Transition { state: next, effects, notices })
}

/// Quote paths and bound the build brief independently of the full fold receipt.
fn conflict_brief(evidence: &Evidence, files: &[String]) -> Result<String, Refusal> {
    let mut reason = format!("Rebase your existing work onto landing base {} and resolve the cherry-pick conflicts in: {}. Run the affected tests and commit the result. Checks and review will run again for the new candidate.",
        evidence.candidate, serde_json::to_string(files).map_err(|_| Refusal::Invalid)?);
    if reason.len() > MAX_REWORK_REASON_BYTES {
        let mut end = MAX_REWORK_REASON_BYTES - 80;
        while !reason.is_char_boundary(end) { end -= 1; }
        reason.truncate(end);
        reason.push_str("… See the fold receipt for the full conflicting file list.");
    }
    Ok(reason)
}

/// An absent limit leaves the landing batch uncapped. The state envelope still bounds memory.
fn has_room(limit: Option<u32>, count: usize) -> bool { limit.is_none_or(|n| count < n as usize) }
fn waiting_group(g: &Item) -> bool {
    g.group_of.is_some() && g.phase == Phase::Blocked && g.stage == 0
        && g.pending.is_none() && g.candidate.is_none() && g.pushed.is_none()
}
/// Upgrade idle groups and fold their members into the oldest compatible waiting group. The half
/// already retrying after a split owns the resource and is never folded back into its sibling.
fn consolidate_groups(state: &mut State) -> Result<(), Refusal> {
    let ids: Vec<_> = state.items.iter().filter(|(_, g)| waiting_group(g)).map(|(id, _)| id.clone()).collect();
    let mut open: Vec<(String, Option<u32>)> = Vec::new();
    for id in ids {
        let mut group = state.items.remove(&id).ok_or(Refusal::Missing)?;
        let origin = group.group_of.as_mut().ok_or(Refusal::Invalid)?;
        let published = state.grouping.as_ref().filter(|p| p.definition.groups.get(&origin.name)
            .is_some_and(|g| g.exclusive == group.resource));
        let limit = if let Some(p) = published {
            origin.pin = p.definition.pin();
            group.definition = p.definition.group_definition(&origin.name).ok_or(Refusal::Definition)?;
            group.pin = group.definition.pin(); group.policy_pin = p.policy_pin.clone();
            p.definition.groups[&origin.name].limit
        } else {
            group.members.first().and_then(|m| state.items.get(&m.item))
                .and_then(|t| t.definition.groups.get(&origin.name)).map_or(Some(1), |g| g.limit)
        };
        // A settled split script may deliberately keep its own group. Its pending call
        // already excludes it above; do not undo its choice on this or a later sweep.
        if group.definition.script_policies.iter().any(|(name, policy)| policy.point == HookPoint::GroupSplits
            && group.hook_calls.get(name).is_some_and(|calls| *calls > 0)) {
            state.items.insert(id, group);
            continue;
        }
        for (target, cap) in &open {
            let dest = state.items.get_mut(target).ok_or(Refusal::Missing)?;
            if dest.group_of != group.group_of || dest.policy_pin != group.policy_pin || dest.resource != group.resource { continue; }
            let count = cap.map_or(group.members.len(), |n| (n as usize).saturating_sub(dest.members.len()).min(group.members.len()));
            let moved: Vec<_> = group.members.drain(..count).collect();
            for member in &moved {
                state.items.get_mut(&member.item).ok_or(Refusal::Missing)?.grouped_in = Some(target.clone());
            }
            state.items.get_mut(target).ok_or(Refusal::Missing)?.members.extend(moved);
            if group.members.is_empty() { break; }
        }
        if group.members.is_empty() { group.phase = Phase::Cancelled; }
        else { open.push((id.clone(), limit)); }
        state.items.insert(id, group);
    }
    Ok(())
}

pub fn owns_resource(item: &Item) -> bool {
    !matches!(item.phase, Phase::Blocked | Phase::Done | Phase::Failed | Phase::Cancelled
        | Phase::WaitingJoin | Phase::Joined)
}
/// A new group's id: the revision it opens at, zero-padded, so ids sort in the order groups opened.
fn group_id(state: &State, name: &str) -> String { format!("group-{:020}-{name}", state.revision) }
/// The shape `group_id` makes: `group-` and twenty digits and `-`. Nothing else is reserved.
fn reserved_id(id: &str) -> bool {
    id.strip_prefix("group-").is_some_and(|r| r.len() > 20 && r.as_bytes()[..20].iter().all(u8::is_ascii_digit) && r.as_bytes()[20] == b'-')
}
/// Take a queued ticket out of a group that has not left yet; an empty group is cancelled.
fn detach(next: &mut State, id: &str, item: &mut Item) -> Result<(), Refusal> {
    if item.phase != Phase::Joined { return Err(Refusal::Phase); }
    let gid = item.grouped_in.clone().ok_or(Refusal::Invalid)?;
    let group = next.items.get_mut(&gid).ok_or(Refusal::Missing)?;
    if group.phase != Phase::Blocked { return Err(Refusal::Phase); }
    let before = group.members.len();
    group.members.retain(|m| m.item != id);
    if group.members.len() == before { return Err(Refusal::Invalid); }
    if group.members.is_empty() { group.phase = Phase::Cancelled; }
    item.grouped_in = None;
    item.phase = Phase::WaitingJoin;
    Ok(())
}
fn ready(state: &State, item: &Item, input: &Input) -> bool {
    if item.deciding.is_some() { return false; }
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
    let binding = ("run/v1", &input.workspace, id, &item.pin, &item.policy_pin,
        item.stage, item.iteration, item.attempt, input.epoch, item.fence);
    match &item.restart {
        Some(start) => digest(&(binding, start)),
        None => digest(&binding),
    }
}
fn enter(item: &mut Item, id: &str, input: &Input, effects: &mut Vec<Effect>) -> Result<(), Refusal> {
    // Once pushed, only this group's CI and follow-up steps remain. Keep their admission fence
    // on a private resource so the next group can fold and gate on the published head (ov-1209).
    if item.group_of.is_some() && item.pushed.is_some() {
        item.resource = digest(&("workflow-item/v1", &input.workspace, id));
    }
    item.run = run_id(item, id, input);
    item.pending = None;
    if matches!(item.definition.stages[item.stage].progression, Progression::AwaitApproval { .. }) {
        item.phase = Phase::WaitingApproval;
        Ok(())
    } else { launch(item, id, input, effects) }
}
fn launch(item: &mut Item, id: &str, input: &Input, effects: &mut Vec<Effect>) -> Result<(), Refusal> {
    let kind = item.definition.stages[item.stage].kind.clone();
    if matches!(kind, StageKind::Join { .. }) {
        // Waits, holding nothing, for the orchestrator to queue it.
        item.phase = Phase::WaitingJoin;
    } else if matches!(kind, StageKind::Review(Reviewer::Person(_) | Reviewer::Orchestrator(_))) {
        item.phase = Phase::WaitingReview;
    } else {
        item.phase = Phase::Running;
        emit(item, id, input, EffectKind::Execute(kind), effects);
    }
    Ok(())
}
pub(crate) fn emit(item: &mut Item, id: &str, input: &Input, kind: EffectKind, effects: &mut Vec<Effect>) {
    let effect_id = digest(&("effect/v1", &input.workspace, &input.id, effects.len()));
    item.pending = Some(effect_id.clone());
    // A resumed wait keeps the deadline stamped at its real stage entry.
    let created_ms = if !matches!(kind, EffectKind::Decide { .. })
        && (is_wait(&item.definition.stages[item.stage].kind)
            || item.definition.stages[item.stage].kind == StageKind::Push) {
        *item.wait_since.get_or_insert(input.now_ms)
    } else { input.now_ms };
    let reuse_checkout = item.reuse_checkout && matches!(kind, EffectKind::Execute(StageKind::Provision | StageKind::Build));
    effects.push(Effect { restart: item.restart.clone(), reuse_checkout, id: effect_id, item: id.into(), run: item.run.clone(),
        epoch: input.epoch, fence: item.fence, resource: item.resource.clone(),
        definition_pin: item.pin.clone(), policy_pin: item.policy_pin.clone(),
        candidate: item.candidate.clone(), members: item.members.clone(), kind,
        operation: item.definition.stages[item.stage].operation.clone(), created_ms,
        model: item.models.get(&item.definition.stages[item.stage].id).cloned(), tree: item.tree.clone(),
        brief: item.brief.clone().filter(|_| item.definition.stages[item.stage].kind == StageKind::Build),
        with: item.followers.clone(), ci_retry: item.ci_retry.clone() });
}
fn evidence_matches(item: &Item, e: &Evidence) -> Result<(), Refusal> {
    if !valid_id(&e.candidate) || !valid_id(&e.manifest) || !valid_id(&e.receipt)
        || e.policy_pin != item.policy_pin
    { return Err(Refusal::Evidence); }
    if matches!(item.definition.stages[item.stage].kind,
        StageKind::Check | StageKind::Review(_) | StageKind::Land | StageKind::Ci | StageKind::Tracker | StageKind::Evaluate(_)
        | StageKind::Run | StageKind::Push | StageKind::AwaitCi | StageKind::Route | StageKind::PushCandidate)
        && item.candidate.as_ref() != Some(&e.candidate)
    { return Err(Refusal::Evidence); }
    Ok(())
}
/// What a step may leave behind besides the item itself: new effects, and groups to open on a split.
struct Out<'a> { room: bool, effects: &'a mut Vec<Effect>, spawn: &'a mut Vec<(String, Vec<Member>)> }
fn finish(item: &mut Item, id: &str, input: &Input, passed: bool, evidence: &Evidence, brief: Option<&str>, out: Out) -> Result<(), Refusal>
{
    if item.definition.schema == 2 {
        let label = if passed { "passed" } else { "rejected" };
        let target = target_of(item, label);
        return take_route(item, id, input, (label, Some(evidence)), target, brief, out);
    }
    if let Some(brief) = brief { item.brief = Some(brief.to_owned()); }
    item.pending = None;
    item.last_evidence = Some(evidence.clone());
    // A build that passed has answered why it was sent back.
    if passed && item.definition.stages[item.stage].kind == StageKind::Build {
        item.brief = None;
        item.reuse_checkout = false;
    }
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
            return enter(item, id, input, out.effects);
        }
        let cause = if matches!(item.definition.stages[item.stage].kind, StageKind::Review(_) | StageKind::Evaluate(_)) {
            "Review rejected the candidate."
        } else { "The step rejected the candidate." };
        if matches!(item.definition.stages[item.stage].kind, StageKind::Review(_))
            && item.definition.rework_target.is_some()
        {
            fail_review_limit(item, item.definition.max_reworks, Some(evidence));
        } else {
            fail(item, cause, Some(evidence));
        }
        return Ok(());
    }
    if matches!(item.definition.stages[item.stage].kind, StageKind::Build | StageKind::Tracker | StageKind::Action(_)) {
        item.candidate = Some(evidence.candidate.clone());
    }
    item.stage += 1;
    item.attempt = 1;
    item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
    if item.stage == item.definition.stages.len() { item.phase = Phase::Done; Ok(()) }
    else { enter(item, id, input, out.effects) }
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
/// Whether one more item fits. The item being worked on is out of the map, so it counts too.
/// A split with no room takes its fallback: a refusal here would stop the service, not the item.
fn room(next: &State, spawn: &[(String, Vec<Member>)]) -> bool { next.items.len() + 1 + spawn.len() < MAX_ITEMS }
/// Schema 2: take the route a label names. A back edge past its limit fails the run instead.
/// A split keeps the first half of a group here and hands the rest to `spawn`.
fn route(item: &mut Item, id: &str, input: &Input, label: &str, evidence: Option<&Evidence>, out: Out) -> Result<(), Refusal>
{
    let target = target_of(item, label);
    take_route(item, id, input, (label, evidence), target, None, out)
}
/// Take an already selected route. A diagnosed fold conflict retries the whole healthy batch
/// through the split's back edge, spending the same published retry budget.
fn take_route(item: &mut Item, id: &str, input: &Input, answer: (&str, Option<&Evidence>),
    mut target: Target, brief: Option<&str>, out: Out) -> Result<(), Refusal>
{
    let (label, evidence) = answer;
    item.pending = None;
    item.ci_retry = None;
    let stage = &item.definition.stages[item.stage];
    let (key, limit) = (stage.id.clone(), stage.limit);
    let from_join = matches!(stage.kind, StageKind::Join { .. });
    let fold_conflict = stage.kind == StageKind::Fold && label == "conflict";
    if item.group_of.is_some() { item.brief = None; }
    // Install this verdict's brief before routing can launch a rework build.
    if let Some(brief) = brief { item.brief = Some(brief.to_owned()); }
    if let Some(evidence) = evidence {
        item.last_evidence = Some(evidence.clone());
        if label != "rejected" && makes_candidate(&stage.kind) { item.candidate = Some(evidence.candidate.clone()); }
    }
    if label == "passed" && stage.kind == StageKind::Build {
        item.brief = None;
        item.reuse_checkout = false;
    }
    if label == "passed" && matches!(stage.kind, StageKind::Push) {
        item.pushed = evidence.map(|e| e.candidate.clone());
    }
    let taken = item.back_edges.get(&key).copied().unwrap_or(0);
    let mut split = false;
    if let Target::Split { step, otherwise } = target {
        // Two or more members and room for another item: split. Otherwise the fallback decides. A split takes none of
        // the step's back edges (ov-1071): each one at least halves this group, and the half it hands on starts with
        // fewer members than this one had, so splitting ends within the group's size however big it is.
        split = item.members.len() >= 2 && out.room;
        target = if split {
            if let Some(failure) = &mut item.group_failure { failure.split = true; }
            let rest = item.members.split_off(item.members.len().div_ceil(2));
            out.spawn.push((id.to_string(), rest));
            Target::Step(step)
        } else { *otherwise };
    }
    let to = match &target {
        Target::Fail | Target::Split { .. } => {
            if fold_conflict {
                item.brief = Some(format!("Rebase your existing work onto landing base {} and resolve the cherry-pick conflicts. Run the affected tests and commit the result. Checks and review will run again for the new candidate.",
                    item.candidate.as_deref().unwrap_or("the current target")));
            }
            fail(item, &format!("The step returned {label}."), evidence);
            return Ok(());
        }
        Target::Done => {
            item.stage = item.definition.stages.len();
            item.phase = Phase::Done;
            return Ok(());
        }
        Target::Step(to) => item.definition.stages.iter().position(|s| &s.id == to).ok_or(Refusal::Definition)?,
    };
    if !split && is_back_edge(item, &target) {
        if taken >= limit {
            if label == "rejected" && matches!(stage.kind, StageKind::Review(_)) {
                fail_review_limit(item, limit, evidence);
            } else {
                fail(item, &back_edge_failure(label, limit), evidence);
            }
            return Ok(());
        }
        item.back_edges.insert(key, taken + 1);
        item.iteration = item.iteration.checked_add(1).ok_or(Refusal::Limit)?;
    }
    if makes_candidate_anew(&item.definition.stages[to].kind) { item.candidate = None; }
    item.stage = to;
    item.attempt = 1;
    item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
    if from_join {
        // A ticket that waited in a group holds no resource: it queues for it like new work.
        item.phase = Phase::Blocked;
        return Ok(());
    }
    enter(item, id, input, out.effects)
}

fn fail_review_limit(item: &mut Item, limit: u8, evidence: Option<&Evidence>) {
    let cause = review_limit_cause(limit);
    fail(item, &cause, evidence);
    item.failure = Some(cause);
}

fn fail(item: &mut Item, cause: &str, evidence: Option<&Evidence>) {
    item.phase = Phase::Failed;
    let step = &item.definition.stages[item.stage].id;
    item.reason = Some(format_reason(step, cause, evidence.map(|e| e.receipt.as_str())));
}

/// Add a diagnostic to the route verdict while keeping a single receipt reference.
fn describe_failure(item: &mut Item, summary: &str, receipt: Option<&str>) {
    let step = &item.definition.stages[item.stage].id;
    let verdict = item.reason.as_deref().and_then(|reason| reason.strip_prefix(step))
        .and_then(|reason| reason.strip_prefix(": "))
        .map(|reason| reason.rsplit_once(" (receipt ").map_or(reason, |(verdict, _)| verdict));
    let cause = verdict.map_or_else(|| summary.to_owned(), |verdict| format!("{verdict} {summary}"));
    item.reason = Some(format_reason(step, &cause, receipt));
}

/// A plain cancellation drops its card; an external close completes only after any stop was acknowledged.
fn finish_close(item: &mut Item) {
    match &item.closed {
        Some(CloseResolution::Landed { sha }) => {
            item.candidate = Some(sha.clone());
            item.stage = item.definition.stages.len();
            item.phase = Phase::Done;
        }
        _ => item.phase = Phase::Cancelled,
    }
}

pub fn request_size(input: &Input, count_occupancy: bool) -> Result<usize, Refusal> {
    // Occupancy is separately bounded store metadata, not caller command payload.
    let mut request = input.clone();
    if !count_occupancy { request.busy_resources.clear(); }
    match &mut request.command {
        Command::Start { start_load, .. } => *start_load = None,
        // This internal list is separately bounded by the live item map.
        Command::AdoptPublication { items, .. } => items.clear(),
        _ => {}
    }
    serde_json::to_vec(&request).map(|v| v.len()).map_err(|_| Refusal::Invalid)
}
