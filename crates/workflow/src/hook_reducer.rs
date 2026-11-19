use super::*;

pub(super) fn failed(before: &State, next: &mut State, input: &Input, effects: &mut Vec<Effect>) {
    for (id, item) in &mut next.items {
        // A run stopped for carry must start no further process under its old pin.
        if matches!(&input.command, Command::RepinPending { item, definition: None, .. } if item == id) {
            continue;
        }
        if item.phase == Phase::Failed && before.items.get(id).is_none_or(|old| old.phase != Phase::Failed) {
            crate::hooks::fire(item, id, input, HookPoint::RunFails, None, effects);
        }
    }
}

/// Suspend only a validated CI observation. The original receipt remains the authority for routing.
pub(super) fn ci(item: &mut Item, id: &str, input: &Input, effects: &mut Vec<Effect>) -> Result<bool, Refusal> {
    let Command::Observe { effect, run, fence, outcome, .. } = &input.command else { return Ok(false) };
    // A rule may suspend only an observation the ordinary reducer can consume. In particular,
    // an uncertain operation still needs a controller's Resolve, and a closed run has no stage.
    if !matches!(item.phase, Phase::Running | Phase::Cancelling) || item.run.starts_with("cancel:") {
        return Ok(false);
    }
    if !matches!(item.definition.stages[item.stage].kind, StageKind::Ci | StageKind::AwaitCi) { return Ok(false); }
    let evidence = match outcome {
        Outcome::Passed(e) | Outcome::Rejected(e) | Outcome::Labeled(_, e)
        | Outcome::LabeledWithReason { evidence: e, .. } | Outcome::CiResult { evidence: e, .. } => e,
        _ => return Ok(false),
    };
    if item.pending.as_ref() != Some(effect) || &item.run != run || item.fence != *fence { return Err(Refusal::Stale); }
    if input.grant != (Grant::Operation { effect: effect.clone(), run: run.clone(), fence: *fence }) { return Err(Refusal::Unauthorized); }
    if let Outcome::Labeled(label, _) | Outcome::LabeledWithReason { label, .. } = outcome {
        let stage = &item.definition.stages[item.stage];
        if item.definition.schema != 2 { return Err(Refusal::Invalid); }
        if !(emits_unrouted(&stage.kind, label) || stage.routes.contains_key(label)) {
            return Err(Refusal::Evidence);
        }
    }
    if let Outcome::CiResult { label, run, .. } = outcome {
        if item.definition.stages[item.stage].kind != StageKind::AwaitCi
            || !matches!(label.as_str(), "passed" | "rejected" | "cancelled" | "timeout") { return Err(Refusal::Phase); }
        if run.as_ref().is_some_and(|r| !r.valid()) { return Err(Refusal::Evidence); }
        if let Some(retry) = &item.ci_retry {
            if retry.pending || (label != "timeout" && !run.as_ref().is_some_and(|r| r.id == retry.run.id && r.attempt > retry.run.attempt)) { return Err(Refusal::Evidence); }
        }
    }
    evidence_matches(item, evidence)?;
    item.last_evidence = Some(evidence.clone());
    Ok(crate::hooks::fire(item, id, input, HookPoint::CiResult, Some(Box::new(input.clone())), effects))
}

pub(super) fn decide(state: &State, input: &Input) -> Result<Transition, Refusal> {
    let Command::Decide { item: id, call, fence, policy_pin, result } = &input.command else { unreachable!() };
    let item = state.items.get(id).ok_or(Refusal::Stale)?;
    let open = item.deciding.as_ref().ok_or(Refusal::Stale)?;
    if &open.input.call != call || item.fence != *fence || open.input.fence != *fence { return Err(Refusal::Stale); }
    let effect = item.pending.as_ref().ok_or(Refusal::Stale)?;
    if input.grant != (Grant::Operation { effect: effect.clone(), run: item.run.clone(), fence: *fence }) { return Err(Refusal::Unauthorized); }
    // A fresh, authenticated answer under the wrong pin is a bad decision, not a stale
    // call. Settle it with the published fallback instead of leaving the call open.
    let pinned = policy_pin == &open.input.policy_pin && policy_pin == &item.policy_pin;
    let policy = item.definition.script_policies.get(&open.policy).ok_or(Refusal::Definition)?;
    if pinned && matches!(result, HookResult::Timeout | HookResult::Missing) && open.attempt == 1 {
        let mut base = state.clone();
        let record = base.items.get_mut(id).ok_or(Refusal::Missing)?;
        let retry = record.deciding.as_mut().ok_or(Refusal::Missing)?;
        retry.attempt = 2;
        retry.deadline_ms = input.now_ms.saturating_add(policy.timeout_ms as i64);
        let document = retry.input.clone();

        let mut advance = input.clone(); advance.command = Command::Advance; advance.grant = Grant::Controller;
        let mut t = step_inner(&base, &advance, true)?;
        let record = t.state.items.get_mut(id).ok_or(Refusal::Missing)?;
        emit(record, id, input, EffectKind::Decide { policy: policy.clone(), input: Box::new(document) }, &mut t.effects);
        return Ok(t);
    }
    let parsed = match result {
        HookResult::Receipt { raw, manifest } if pinned && raw.len() <= 16_384 && valid_id(manifest) => {
            serde_json::from_str::<HookDecision>(raw).ok().filter(|d| &d.call == call).map(|d| d.choice)
        }
        _ => None,
    };
    let mut base = state.clone();
    let record = base.items.get_mut(id).ok_or(Refusal::Missing)?;
    record.deciding = None; record.pending = None;
    let apply = |choice: &HookMove| -> Result<Transition, Refusal> {
        let mut next_input = input.clone(); next_input.grant = Grant::Controller;
        next_input.command = match (open.input.point, choice) {
            (HookPoint::RunFails, HookMove::Retry) => Command::Retry { item: id.clone() },
            (HookPoint::RunFails, HookMove::SendBack { reason }) => Command::SendBack { item: id.clone(), reason: reason.clone() },
            (HookPoint::RunFails, HookMove::Cancel { reason }) if valid_rework_reason(reason) => Command::Cancel { item: id.clone(), reason: Some(reason.clone()) },
            (HookPoint::RunFails, HookMove::Leave) | (HookPoint::GroupSplits, HookMove::OwnGroup) => Command::Advance,
            (HookPoint::CiResult, HookMove::Accept | HookMove::Relabel { .. }) => {
                let observation = open.observation.as_ref().ok_or(Refusal::Invalid)?;
                let mut command = observation.command.clone();
                if let HookMove::Relabel { label } = choice {
                    let Command::Observe { outcome, .. } = &mut command else { return Err(Refusal::Invalid) };
                    let green = matches!(outcome, Outcome::Passed(_)) || matches!(outcome,
                        Outcome::Labeled(label, _) | Outcome::LabeledWithReason { label, .. } | Outcome::CiResult { label, .. }
                        if label == "passed");
                    if label == "passed" && !green { return Err(Refusal::Evidence); }
                    let stage = &item.definition.stages[item.stage];
                    if !stage.routes.contains_key(label) { return Err(Refusal::Invalid); };
                    if let Outcome::CiResult { label: current, .. } | Outcome::LabeledWithReason { label: current, .. } = outcome { *current = label.clone(); }
                    else { *outcome = Outcome::Labeled(label.clone(), item.last_evidence.clone().ok_or(Refusal::Evidence)?); }
                }
                let mut ci_base = base.clone();
                if let Command::Observe { effect, .. } = &command { ci_base.items.get_mut(id).ok_or(Refusal::Missing)?.pending = Some(effect.clone()); }
                next_input.command = command; next_input.grant = observation.grant.clone();
                return step_inner(&ci_base, &next_input, true);
            }
            (HookPoint::GroupSplits, HookMove::Rejoin { group }) => {
                if group == id { return Err(Refusal::Invalid); }
                let dest = base.items.get(group).ok_or(Refusal::Missing)?;
                if dest.phase != Phase::Blocked || dest.deciding.is_some() || dest.stage != 0
                    || dest.group_of != item.group_of || dest.pin != item.pin || dest.resource != item.resource
                    || dest.policy_pin != item.policy_pin
                { return Err(Refusal::Phase); }
                let origin = dest.group_of.as_ref().ok_or(Refusal::Definition)?;
                // Idle groups may have adopted a publication newer than their members' build pins.
                let definition = base.grouping.as_ref().filter(|p| p.definition.pin() == origin.pin)
                    .map(|p| &p.definition)
                    .or_else(|| dest.members.first().and_then(|m| base.items.get(&m.item)).map(|t| &t.definition));
                let group_def = definition.and_then(|d| d.groups.get(&origin.name))
                    .ok_or(Refusal::Definition)?;
                if group_def.limit.is_some_and(|limit| dest.members.len() + item.members.len() > limit as usize) { return Err(Refusal::Limit); }
                let mut merged = base.clone();
                for m in &item.members { merged.items.get_mut(&m.item).ok_or(Refusal::Missing)?.grouped_in = Some(group.clone()); }
                merged.items.get_mut(group).ok_or(Refusal::Missing)?.members.extend(item.members.clone());
                let source = merged.items.get_mut(id).ok_or(Refusal::Missing)?;
                source.members.clear(); source.phase = Phase::Cancelled;
                next_input.command = Command::Advance;
                return step_inner(&merged, &next_input, true);
            }
            _ => return Err(Refusal::Invalid),
        };
        step_inner(&base, &next_input, true)
    };
    if let Some(choice) = parsed && let Ok(t) = apply(&choice) { return Ok(t); }
    let mut fallback = apply(&policy.fallback)?;
    if let Some(item) = fallback.state.items.get_mut(id) {
        let reason = "The rule did not return a legal decision; its fallback was applied.".to_string();
        item.hook_refused = Some(reason.clone()); item.failure = Some(reason);
    }
    Ok(fallback)
}
