//! Who a workflow gate may name as its decider, and who holds that name (ov-1002, ruling R-107).
//!
//! A person gate (`Reviewer::Person`) or an approval step (`Progression::AwaitApproval`) names a principal:
//! - `owner`, the runner's owner;
//! - `member:<handle>`, one member, who must exist when the workflow is drafted or published;
//! - `role:maintainer`, any Maintainer, the owner included (`people::maintains`).
//!
//! The reducer stays a plain equality on the principal of a `Decision` grant. The daemon picks that principal
//! from the names the caller holds (`Admission::deciders`), read from the store at admission, never from the
//! wire: Alice holds `member:alice`, and `role:maintainer` while she is a Maintainer. A command whose caller
//! holds the gate's name decides as it; anyone else decides as their own principal, which the reducer refuses.
//!
//! A run waiting on a member who is removed keeps waiting: nothing about it changes in the engine. Its card is
//! told so, and that the owner can reassign it (`member_gone`). No new run starts under a workflow naming them
//! (`names_a_removed_member`), and their handle isn't issued again while a workflow names them, so a newcomer
//! given it can't decide their gates (`Store::set_member_releasing`).
use farcooler_store::Store;
use farcooler_workflow::{Definition, Phase, Progression, Reviewer, StageKind, State};

/// The role every Maintainer holds as a decider.
pub(crate) const MAINTAINER: &str = "role:maintainer";

/// Whether `name` is one a gate may name, by its shape alone.
pub(crate) fn well_formed(name: &str) -> bool {
    name == "owner" || name == MAINTAINER
        || name.strip_prefix("member:").is_some_and(|h| farcooler_store::people::valid_handle(h))
}

/// The deciders `definition` names, a group's steps included, in step order, each once.
pub(crate) fn named(definition: &Definition) -> Vec<&str> { definition.deciders() }

/// Refuse a definition that names a decider nobody could be: a malformed name, or a member who isn't one.
/// An approval step may also wait on `orchestrator`, as a ticket's does; an orchestrator's review gate only on
/// `orchestrator`, so no member can be named through one (review F4, F5). The error is the name, for the person drafting it.
pub(crate) fn refuse_unknown(store: &Store, definition: &Definition) -> Result<(), String> {
    for stage in steps(definition) {
        let ok = |name: &str| well_formed(name) && name.strip_prefix("member:").is_none_or(|h| store.member(h).is_ok());
        if let Progression::AwaitApproval { principal } = &stage.progression
            && principal != "orchestrator" && !ok(principal)
        {
            return Err(principal.clone());
        }
        match &stage.kind {
            StageKind::Review(Reviewer::Person(p)) if !ok(p) => return Err(p.clone()),
            StageKind::Review(Reviewer::Orchestrator(p)) if p != "orchestrator" => return Err(p.clone()),
            _ => {}
        }
    }
    Ok(())
}

/// The first member `definition` names who is no longer on this runner, if one is: a run under it would wait on
/// them for good, so none is started (review F2).
pub(crate) fn names_a_removed_member<'a>(store: &Store, definition: &'a Definition) -> Option<&'a str> {
    named(definition).into_iter().find(|name| name.strip_prefix("member:").is_some_and(|h| store.member(h).is_err()))
}

fn steps(definition: &Definition) -> impl Iterator<Item = &farcooler_workflow::Stage> {
    definition.stages.iter().chain(definition.groups.values().flat_map(|g| g.steps.iter()))
}

/// The decider `item` is waiting on in `state`, if it is waiting on a person.
pub(crate) fn waiting_on<'a>(state: &'a State, item: &str) -> Option<&'a str> {
    let item = state.items.get(item)?;
    let stage = item.definition.stages.get(item.stage)?;
    match (&item.phase, &stage.progression, &stage.kind) {
        (Phase::WaitingApproval, Progression::AwaitApproval { principal }, _) => Some(principal),
        (Phase::WaitingReview, _, StageKind::Review(Reviewer::Person(p) | Reviewer::Orchestrator(p))) => Some(p),
        _ => None,
    }
}

/// `named` principal, if `held` holds it: the principal a decision is made as. Else the caller's own.
pub(crate) fn decide_as<'a>(gate: Option<&'a str>, held: &'a [String], own: &'a str) -> &'a str {
    gate.filter(|g| held.iter().any(|h| h == g)).unwrap_or(own)
}

/// The words a card is told when the member its run's workflow waits on is removed (ov-1002).
pub(crate) fn gone_note(handle: &str) -> String {
    format!("This run's workflow waits on {handle} to decide a step, and {handle} was removed from this runner. \
        Nobody else can decide that step, so the run will wait there. \
        The owner can reassign it: cancel the run, and start it again under a workflow that names someone who is here.")
}

/// After `handle` is removed: a note on the card of every live run whose workflow names them, whether it waits on
/// them now or will at a later step (review F2). The runs keep waiting. A handle stays unissued while one does
/// (`Store::set_member_releasing`), so nobody new can answer as them (review F1).
pub(crate) fn member_gone(svc: &crate::service::Service, watcher: &crate::watch::Watcher, handle: &str) {
    use farcooler_store::models::{Actor, NoteKind};
    let name = format!("member:{handle}");
    let Ok(workspaces) = svc.store.workflow_workspaces() else { return };
    for workspace in workspaces {
        let Ok(Some(state)) = svc.store.workflow_state(&workspace) else { continue };
        for (item, record) in &state.items {
            if matches!(record.phase, Phase::Done | Phase::Cancelled) || !named(&record.definition).contains(&name.as_str()) { continue; }
            let Ok(Some(task)) = svc.store.workflow_item_task(&workspace, item) else { continue };
            if svc.store.add_note(task, NoteKind::Finding, Actor::Runner, &gone_note(handle), serde_json::json!({})).is_ok()
                && let Ok(card) = svc.store.get_task(task)
            {
                watcher.announce_task_changed(&card, None, Actor::Runner);
            }
        }
    }
}

#[cfg(test)]
#[path = "workflow_deciders_tests.rs"]
mod tests;
