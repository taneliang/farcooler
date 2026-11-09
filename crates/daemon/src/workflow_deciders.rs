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
//! told so, and that the owner can reassign it (`member_gone`).
use farcooler_store::Store;
use farcooler_workflow::{Definition, Phase, Progression, Reviewer, StageKind, State};

/// The role every Maintainer holds as a decider.
pub(crate) const MAINTAINER: &str = "role:maintainer";

/// Whether `name` is one a gate may name, by its shape alone.
pub(crate) fn well_formed(name: &str) -> bool {
    name == "owner" || name == MAINTAINER
        || name.strip_prefix("member:").is_some_and(|h| farcooler_store::people::valid_handle(h))
}

/// The person deciders `definition` names, in step order, each once.
pub(crate) fn named(definition: &Definition) -> Vec<&str> {
    let mut names: Vec<&str> = Vec::new();
    for stage in &definition.stages {
        let approval = match &stage.progression { Progression::AwaitApproval { principal } => Some(principal.as_str()), _ => None };
        let person = match &stage.kind { StageKind::Review(Reviewer::Person(p)) => Some(p.as_str()), _ => None };
        for name in approval.into_iter().chain(person) {
            if !names.contains(&name) { names.push(name); }
        }
    }
    names
}

/// Refuse a definition that names a decider nobody could be: a malformed name, or a member who isn't one.
/// The error is the name, for the person drafting it.
pub(crate) fn refuse_unknown(store: &Store, definition: &Definition) -> Result<(), String> {
    for name in named(definition) {
        if !well_formed(name) { return Err(name.into()); }
        if let Some(handle) = name.strip_prefix("member:")
            && store.member(handle).is_err()
        {
            return Err(name.into());
        }
    }
    Ok(())
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

/// The words a card waiting on a removed member is told (ov-1002).
pub(crate) fn gone_note(handle: &str) -> String {
    format!("This is waiting on {handle}, who was removed from this runner. Nobody else can decide it as it stands. \
        The owner can reassign it: cancel the run, and start it again under a workflow that names someone who is here.")
}

/// After `handle` is removed: a note on every engine card waiting on them, saying so. The runs keep waiting.
pub(crate) fn member_gone(svc: &crate::service::Service, watcher: &crate::watch::Watcher, handle: &str) {
    use farcooler_store::models::{Actor, NoteKind};
    let name = format!("member:{handle}");
    let Ok(workspaces) = svc.store.workflow_workspaces() else { return };
    for workspace in workspaces {
        let Ok(Some(state)) = svc.store.workflow_state(&workspace) else { continue };
        for item in state.items.keys() {
            if waiting_on(&state, item) != Some(name.as_str()) { continue; }
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
