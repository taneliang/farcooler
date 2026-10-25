//! Whose decision an answer was, when it was not the answerer's (ov-737).
//!
//! Anyone may answer any item in Needs You, so a member can settle a card
//! that is another's: the apps ask them to mean it first, and the record
//! says so afterward. When a person's answer lands on a task waiting for a
//! decision that concerns someone else, the note carries `extra.for`, the
//! person the card was for: `{"handle": "bob", "name": "Bob Okafor"}`, both
//! empty for the runner's owner. The board's line for the note then reads
//! "Answer · Alice Chen for Bob Okafor · 4m ago": the kind label says it is an
//! answer, and the byline adds who it was for ("for You" when that is the
//! reader).
//!
//! Written here, from the same rule that files the item in Needs You
//! (`needs_you::concern`), never taken from a caller: a `for` in a request's
//! `extra` is dropped, so nobody can forge who a decision was for. An answer
//! on one's own card, on nobody's, or on a runner with no members carries
//! none, and notes written before this have none. An older client simply
//! doesn't draw it.
//!
//! **A held ask** (ov-743) says the same on its row: `ask_people` names the
//! person whose device answered it, and whose ask it was when that is someone
//! else, by the rule that files the ask in Needs You (`whose_terminal`). The
//! person comes from the authenticated device, or on the local socket from
//! the pane the caller's process runs in (`caller_person`, ov-747), never
//! from the request. On the local socket that is attribution only, and
//! grants nothing: every process there is the one Unix user, so keys sent
//! into a member's pane (`tmux send-keys`) answer as that member, and a
//! process that leaves its pane answers as the owner. The note itself only the runner writes
//! (`SessionProjectors::ask_ended`; a hook frame of that name is dropped),
//! and it lands on the row holding the ask's id, which only the runner puts
//! on a row: `hook_ingress` strips the id from every frame before folding
//! it, and adds it back only to the request it holds.

use farcooler_core::Result;
use farcooler_store::Store;
use farcooler_store::models::{Actor, NoteKind, TaskStatus};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::needs_you::concern::{self, Whose};

/// Drop any `for` a caller sent, and mark `extra` with the person `task`'s
/// decision was for when `actor` answered it and isn't them.
pub fn stamp(store: &Store, task: Uuid, kind: NoteKind, actor: Actor, extra: &mut Value) -> Result<()> {
    if let Some(map) = extra.as_object_mut() {
        map.remove("for");
    }
    let answerer = match actor {
        Actor::User => Whose::Owner,
        Actor::Member { handle } => Whose::Member(handle.to_string()),
        _ => return Ok(()),
    };
    if kind != NoteKind::Answer || !extra.is_object() {
        return Ok(());
    }
    let task = store.get_task(task)?;
    if task.status != TaskStatus::NeedsDecision {
        return Ok(());
    }
    let Some(owner) = concern::whose_task(store, &task) else { return Ok(()) };
    if owner == answerer {
        return Ok(());
    }
    let (handle, name) = match owner {
        Whose::Owner => (String::new(), String::new()),
        Whose::Member(handle) => {
            let name = store.members()?.into_iter().find(|m| m.handle == handle).map_or_else(|| handle.clone(), |m| m.name);
            (handle, name)
        }
    };
    extra["for"] = json!({ "handle": handle, "name": name });
    Ok(())
}

/// What a held ask's row says of `answerer`'s answer on `terminal`
/// (ov-743): `{"person": {...}, "for": {...}}`, each `{"handle", "name"}`
/// with both empty for the owner, `for` only when the ask was someone
/// else's. `None` on a runner with no members, where every answer is the
/// owner's and rows stay as they were, and for a released device.
pub fn ask_people(store: &Store, terminal: Uuid, answerer: &crate::people::Person) -> Option<Value> {
    use crate::people::Person;
    let me = match answerer {
        Person::Owner if !store.any_members().unwrap_or(false) => return None,
        Person::Owner => Whose::Owner,
        Person::Member(handle) => Whose::Member(handle.clone()),
        Person::Released => return None,
    };
    let named = |whose: &Whose| match whose {
        Whose::Owner => json!({ "handle": "", "name": "" }),
        Whose::Member(handle) => {
            let name = store.member_name(handle).ok().flatten().unwrap_or_else(|| handle.clone());
            json!({ "handle": handle, "name": name })
        }
    };
    let mut people = json!({ "person": named(&me) });
    if let Some(whose) = concern::whose_terminal(store, terminal).filter(|w| *w != me) {
        people["for"] = named(&whose);
    }
    Some(people)
}

#[cfg(test)]
#[path = "answered_for_tests.rs"]
mod tests;
