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

#[cfg(test)]
#[path = "answered_for_tests.rs"]
mod tests;
