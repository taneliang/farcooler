//! The invariants a stored state must hold before the reducer will step it.
use super::*;

/// Groups and the tickets queued into them agree with each other, in both directions.
fn valid_membership(state: &State, id: &str, item: &Item) -> bool {
    let queued_ok = match (&item.grouped_in, &item.phase) {
        (Some(gid), Phase::Joined) => state.items.get(gid).is_some_and(|g| g.group_of.is_some()
            && g.members.iter().any(|m| m.item == id)),
        (None, Phase::Joined) | (Some(_), _) => false,
        (None, _) => true,
    };
    // Landing batches use the published GroupDef.limit, independently of the build follower budget.
    let group_ok = item.members.is_empty() || (item.group_of.is_some()
        && item.members.len() <= MAX_ITEMS
        && item.members.iter().all(|m| valid_id(&m.candidate)
            && state.items.get(&m.item).is_some_and(|t| t.phase == Phase::Joined && t.grouped_in.as_deref() == Some(id))));
    let waiting_ok = item.phase != Phase::WaitingJoin
        || matches!(item.definition.stages.get(item.stage).map(|s| &s.kind), Some(StageKind::Join { .. }));
    queued_ok && group_ok && waiting_ok
}

fn usage_within_limits(item: &Item) -> bool {
    let limits = item.definition.execution_limits();
    (if item.definition.schema == 2 {
        item.back_edges.iter().all(|(stage, taken)| item.definition.stages.iter()
            .any(|s| &s.id == stage && s.limit >= *taken))
    } else { item.iteration <= item.definition.max_reworks })
        && item.retries <= limits.retries
        && item.sent_back.len() <= usize::from(limits.send_backs)
        && item.followers.len() <= usize::from(limits.group_members)
}

pub(super) fn valid_state(state: &State) -> bool {
    state.grouping.as_ref().is_none_or(|p| p.definition.validate().is_ok() && valid_id(&p.policy_pin))
        && valid_id(&state.workspace) && state.epoch > 0 && state.items.len() <= MAX_ITEMS
        && state.archive.iter().all(|(id, a)| valid_id(id) && !state.items.contains_key(id)
            && matches!(a.phase, Phase::Done | Phase::Failed | Phase::Cancelled)
            && a.candidate.as_deref().is_none_or(valid_id))
        && state.items.iter().all(|(id, item)| {
            item.execution.is_none() && valid_id(id) && item.pin == item.definition.pin()
                && if item.group_of.is_some() { item.definition.validate_group() } else { item.definition.validate() }.is_ok()
                && valid_membership(state, id, item)
                && (item.pushed.is_none() || (item.group_of.is_some() && item.pushed.as_deref().is_some_and(valid_id)))
                && valid_id(&item.policy_pin) && valid_id(&item.resource)
                // A compatible publication may retain usage above its lowered limits.
                && (item.carried_progress || usage_within_limits(item))
                && if item.definition.schema == 2 {
                    usize::from(item.iteration) == item.back_edges.values().map(|n| usize::from(*n)).sum::<usize>()
                        && item.back_edges.keys().all(|stage| item.definition.stages.iter().any(|s| &s.id == stage))
                    } else { item.back_edges.is_empty() }
                && item.ci_reruns.iter().all(|(step, count)| *count == 1
                    && (item.definition.policies.values().any(|p| &p.step == step)
                        || item.definition.stages.iter().any(|s| &s.id == step && s.kind == StageKind::AwaitCi)))
                && item.ci_retry.as_ref().is_none_or(|r| r.run.valid()
                    && (item.definition.policies.contains_key(&r.policy) || r.policy == crate::ci::SETUP_RERUN)
                    && item.definition.stages.get(item.stage).is_some_and(|s|
                        s.kind == StageKind::AwaitCi && item.ci_reruns.get(&s.id) == Some(&1)))
                && item.rework_history.len() <= usize::from(item.iteration)
                && item.rework_history.iter().enumerate().all(|(index, r)|
                    valid_id(&r.id) && valid_id(&r.principal) && valid_id(&r.run)
                    // History retains the pins the decision actually used, even after an engine re-pin.
                    && valid_id(&r.definition_pin) && valid_id(&r.evidence.policy_pin)
                    && valid_rework_reason(&r.reason) && r.iteration < item.iteration
                    && (index == 0 || item.rework_history[index-1].iteration < r.iteration))
                && item.sent_back.len() <= usize::from(u8::MAX)
                && item.sent_back.iter().all(|b| valid_id(&b.id) && valid_rework_reason(&b.reason))
                && item.brief.as_deref().is_none_or(valid_rework_reason)
                && item.failure.as_deref().is_none_or(valid_rework_reason)
                && item.queue_hold.as_ref().is_none_or(|hold| item.phase == Phase::WaitingJoin && valid_rework_reason(&hold.reason))
                && item.followers.len() <= usize::from(u8::MAX)
                // A live lead's tickets follow it and no other; a closed one's list is history, as they've let go.
                && (matches!(item.phase, Phase::Done | Phase::Cancelled)
                    || item.followers.iter().all(|f| state.items.get(f).is_some_and(|t| t.follows.as_deref() == Some(id.as_str()))))
                && item.follows.as_ref().is_none_or(|lead| item.definition.is_ticket() && item.phase == Phase::WaitingApproval
                    && item.followers.is_empty() && state.items.get(lead).is_some_and(|l| l.followers.iter().any(|f| f == id)))
                && if item.phase == Phase::Done { item.stage == item.definition.stages.len() }
                    else { item.stage < item.definition.stages.len() }
        })
}
