use crate::*;
use std::collections::{BTreeMap, BTreeSet};

pub fn step(state: &State, input: &Input) -> Result<Transition, Refusal> {
    if !valid_state(state) { return Err(Refusal::Invalid); }
    if input.schema != 1 || state.schema != 1 || !valid_id(&input.id)
        || !valid_id(&input.source) || !valid_id(&input.workspace)
        || input.source_sequence == 0 || state.items.len() > MAX_ITEMS
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
    // Halves a group split off this step: (group it came from, members that form the new group).
    let mut spawn: Vec<(String, Vec<Member>)> = Vec::new();
    let mut notices = Vec::new();
    match &input.command {
        Command::Advance => { controller(input)?; }
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
            let mut record = Item { definition: definition.clone(), pin: definition.pin(),
                policy_pin: policy_pin.clone(), dependencies: dependencies.clone(),
                resource: resource.clone(), stage: 0, iteration: 0, attempt: 1, fence: 1,
                phase: Phase::Blocked, run: String::new(), pending: None,
                candidate: initial_candidate.clone(), last_evidence: None, stopping: None, rework_history: Vec::new(),
                back_edges: BTreeMap::new(), retries: 0, pushed: None, wait_since: None,
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
                    finish(&mut item, id, input, *passed, evidence, Out { room: room(&next, &spawn), effects: &mut effects, spawn: &mut spawn })?;
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
                            // Any label the step can emit is an answer, routed or not: an unrouted one fails the item
                            // (`target_of`) instead of being refused, which would leave the item uncertain for good.
                            if !(emits_unrouted(&stage.kind, label) || stage.routes.contains_key(label)) {
                                return Err(Refusal::Evidence);
                            }
                            evidence_matches(&item, evidence)?;
                            route(&mut item, id, input, label, Some(evidence), Out { room: room(&next, &spawn), effects: &mut effects, spawn: &mut spawn })?;
                        }
                        Outcome::Passed(evidence) | Outcome::Rejected(evidence) => {
                            if item.run.starts_with("cancel:") { return Err(Refusal::Phase); }
                            evidence_matches(&item, evidence)?;
                            finish(&mut item, id, input, matches!(outcome, Outcome::Passed(_)), evidence, Out { room: room(&next, &spawn), effects: &mut effects, spawn: &mut spawn })?;
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
                        Outcome::Interrupted => {
                            // Only a wait is read again for free, and only because a restart cut it short, not
                            // because it failed. Its patience is kept, so a loop still ends in `timeout`.
                            if item.run.starts_with("cancel:") { return Err(Refusal::Phase); }
                            if !matches!(item.definition.stages[item.stage].kind, StageKind::AwaitBase | StageKind::AwaitCi) {
                                return Err(Refusal::Phase);
                            }
                            item.pending = None;
                            item.wait_since = wait_since;
                            item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
                            enter(&mut item, id, input, &mut effects)?;
                        }
                    }
                }
                Command::Cancel { .. } => {
                    controller(input)?;
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
                    if let Some(operation) = item.pending.take() {
                        item.phase = Phase::Cancelling;
                        item.stopping = Some(operation.clone());
                        item.run = format!("cancel:{}", run_id(&item, id, input));
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
                    let limit = usize::from(item.definition.groups.get(name).ok_or(Refusal::Definition)?.limit);
                    // The first open group of this definition and policy with room, oldest first.
                    let open = next.items.iter().find(|(_, g)| g.phase == Phase::Blocked
                        && g.members.len() < limit && g.stage == 0 && g.candidate.is_none() && g.policy_pin == item.policy_pin
                        && g.group_of.as_ref().is_some_and(|o| &o.name == name && o.pin == item.pin))
                        .map(|(gid, _)| gid.clone());
                    let gid = match open {
                        Some(gid) => gid,
                        None => {
                            // The ticket is out of the map while it is worked on, so count it.
                            if next.items.len() + 1 >= MAX_ITEMS { return Err(Refusal::Limit); }
                            let gid = group_id(state, name);
                            let definition = item.definition.group_definition(name).ok_or(Refusal::Definition)?;
                            let resource = item.definition.groups[name].exclusive.clone();
                            next.items.insert(gid.clone(), Item { pin: definition.pin(), definition,
                                policy_pin: item.policy_pin.clone(), dependencies: BTreeSet::new(), resource,
                                stage: 0, iteration: 0, attempt: 1, fence: 1, phase: Phase::Blocked,
                                run: String::new(), pending: None, candidate: None, last_evidence: None,
                                stopping: None, rework_history: Vec::new(), back_edges: BTreeMap::new(), retries: 0, pushed: None, wait_since: None,
                                members: Vec::new(),
                                group_of: Some(GroupOf { name: name.clone(), pin: item.pin.clone() }),
                                grouped_in: None });
                            gid
                        }
                    };
                    next.items.get_mut(&gid).ok_or(Refusal::Missing)?.members.push(Member { item: id.clone(), candidate });
                    item.grouped_in = Some(gid);
                    item.phase = Phase::Joined;
                }
                Command::Leave { .. } => {
                    controller(input)?;
                    detach(&mut next, id, &mut item)?;
                }
                Command::Retry { .. } => {
                    controller(input)?;
                    // A failed run does not own its resource, so a retry queues like new work.
                    if item.phase != Phase::Failed || item.group_of.is_some() { return Err(Refusal::Phase); }
                    if item.retries >= MAX_RETRIES { return Err(Refusal::Limit); }
                    item.retries += 1;
                    item.attempt = 1;
                    item.fence = item.fence.checked_add(1).ok_or(Refusal::Limit)?;
                    item.phase = Phase::Blocked;
                }
                Command::Enqueue { .. } | Command::Advance => unreachable!(),
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
                rest.back_edges = BTreeMap::new(); rest.retries = 0;
                next.items.insert(gid, rest);
            }
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
    // A group that ended hands each member its result, through the member's own join routes.
    let ended: Vec<String> = next.items.iter().filter(|(_, g)| g.group_of.is_some() && !g.members.is_empty()
        && matches!(g.phase, Phase::Done | Phase::Failed | Phase::Cancelled)).map(|(gid, _)| gid.clone()).collect();
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
        let members = std::mem::take(&mut group.members);
        for member in members {
            let mut ticket = next.items.remove(&member.item).ok_or(Refusal::Missing)?;
            ticket.grouped_in = None;
            if let StageKind::Join { adopt: true, .. } = ticket.definition.stages[ticket.stage].kind {
                if matches!(label, "passed" | "landed") && result.is_some() { ticket.candidate = result.clone(); }
            }
            route(&mut ticket, &member.item, input, label, None, Out { room: room(&next, &spawn), effects: &mut effects, spawn: &mut spawn })?;
            next.items.insert(member.item, ticket);
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
fn emit(item: &mut Item, id: &str, input: &Input, kind: EffectKind, effects: &mut Vec<Effect>) {
    let effect_id = digest(&("effect/v1", &input.workspace, &input.id, effects.len()));
    item.pending = Some(effect_id.clone());
    // A wait looked at again after a restart keeps its first deadline.
    let created_ms = if matches!(item.definition.stages[item.stage].kind, StageKind::AwaitBase | StageKind::AwaitCi) {
        *item.wait_since.get_or_insert(input.now_ms)
    } else { input.now_ms };
    effects.push(Effect { id: effect_id, item: id.into(), run: item.run.clone(),
        epoch: input.epoch, fence: item.fence, resource: item.resource.clone(),
        definition_pin: item.pin.clone(), policy_pin: item.policy_pin.clone(),
        candidate: item.candidate.clone(), members: item.members.clone(), kind,
        operation: item.definition.stages[item.stage].operation.clone(), created_ms });
}
fn evidence_matches(item: &Item, e: &Evidence) -> Result<(), Refusal> {
    if !valid_id(&e.candidate) || !valid_id(&e.manifest) || !valid_id(&e.receipt)
        || e.policy_pin != item.policy_pin
    { return Err(Refusal::Evidence); }
    if matches!(item.definition.stages[item.stage].kind,
        StageKind::Check | StageKind::Review(_) | StageKind::Land | StageKind::Ci | StageKind::Tracker | StageKind::Evaluate(_)
        | StageKind::Run | StageKind::Push | StageKind::AwaitCi)
        && item.candidate.as_ref() != Some(&e.candidate)
    { return Err(Refusal::Evidence); }
    Ok(())
}
/// What a step may leave behind besides the item itself: new effects, and groups to open on a split.
struct Out<'a> { room: bool, effects: &'a mut Vec<Effect>, spawn: &'a mut Vec<(String, Vec<Member>)> }
fn finish(item: &mut Item, id: &str, input: &Input, passed: bool, evidence: &Evidence, out: Out) -> Result<(), Refusal>
{
    if item.definition.schema == 2 {
        return route(item, id, input, if passed { "passed" } else { "rejected" }, Some(evidence), out);
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
            return enter(item, id, input, out.effects);
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
/// Whether the step's route for `label` is a back edge with room left to take it.
fn rework_room(item: &Item, label: &str) -> Result<(), Refusal> {
    if !is_back_edge(item, &target_of(item, label)) { return Err(Refusal::Invalid); }
    let stage = &item.definition.stages[item.stage];
    if item.back_edges.get(&stage.id).copied().unwrap_or(0) >= stage.limit { return Err(Refusal::Limit); }
    Ok(())
}
/// Whether one more item fits. The item being worked on is out of the map, so it counts too.
/// A split with no room takes its fallback: a refusal here would stop the service, not the item.
fn room(next: &State, spawn: &[(String, Vec<Member>)]) -> bool { next.items.len() + 1 + spawn.len() < MAX_ITEMS }
/// Schema 2: take the route a label names. A back edge past its limit fails the run instead.
/// A split keeps the first half of a group here and hands the rest to `spawn`.
fn route(item: &mut Item, id: &str, input: &Input, label: &str, evidence: Option<&Evidence>, out: Out) -> Result<(), Refusal>
{
    item.pending = None;
    let stage = &item.definition.stages[item.stage];
    let (key, limit) = (stage.id.clone(), stage.limit);
    let from_join = matches!(stage.kind, StageKind::Join { .. });
    if let Some(evidence) = evidence {
        item.last_evidence = Some(evidence.clone());
        if label != "rejected" && makes_candidate(&stage.kind) { item.candidate = Some(evidence.candidate.clone()); }
    }
    if label == "passed" && matches!(stage.kind, StageKind::Push) {
        item.pushed = evidence.map(|e| e.candidate.clone());
    }
    let mut target = target_of(item, label);
    let taken = item.back_edges.get(&key).copied().unwrap_or(0);
    if let Target::Split { step, otherwise } = target {
        // Two or more members and room to go back: split. Otherwise the fallback decides.
        target = if item.members.len() >= 2 && taken < limit && out.room {
            let rest = item.members.split_off(item.members.len().div_ceil(2));
            out.spawn.push((id.to_string(), rest));
            Target::Step(step)
        } else { *otherwise };
    }
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
        if taken >= limit { item.phase = Phase::Failed; return Ok(()); }
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

/// Groups and the tickets queued into them agree with each other, in both directions.
fn valid_membership(state: &State, id: &str, item: &Item) -> bool {
    let queued_ok = match (&item.grouped_in, &item.phase) {
        (Some(gid), Phase::Joined) => state.items.get(gid).is_some_and(|g| g.group_of.is_some()
            && g.members.iter().any(|m| m.item == id)),
        (None, Phase::Joined) | (Some(_), _) => false,
        (None, _) => true,
    };
    let group_ok = item.members.is_empty() || (item.group_of.is_some()
        && item.members.len() <= usize::from(MAX_GROUP_MEMBERS)
        && item.members.iter().all(|m| valid_id(&m.candidate)
            && state.items.get(&m.item).is_some_and(|t| t.phase == Phase::Joined && t.grouped_in.as_deref() == Some(id))));
    let waiting_ok = item.phase != Phase::WaitingJoin
        || matches!(item.definition.stages.get(item.stage).map(|s| &s.kind), Some(StageKind::Join { .. }));
    queued_ok && group_ok && waiting_ok
}

fn valid_state(state: &State) -> bool {
    valid_id(&state.workspace) && state.epoch > 0 && state.items.len() <= MAX_ITEMS
        && state.archive.iter().all(|(id, a)| valid_id(id) && !state.items.contains_key(id)
            && matches!(a.phase, Phase::Done | Phase::Failed | Phase::Cancelled)
            && a.candidate.as_deref().is_none_or(valid_id))
        && state.items.iter().all(|(id, item)| {
            valid_id(id) && item.pin == item.definition.pin()
                && if item.group_of.is_some() { item.definition.validate_group() } else { item.definition.validate() }.is_ok()
                && valid_membership(state, id, item)
                && (item.pushed.is_none() || (item.group_of.is_some() && item.pushed.as_deref().is_some_and(valid_id)))
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
