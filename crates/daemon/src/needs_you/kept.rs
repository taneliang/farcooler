//! A member's Keep as a quiet Needs You FYI for the workspace's lead (ov-756).
//! Its own file to keep `needs_you.rs` inside its size budget.
//!
//! Any member may Keep a ruling, or Keep All (R-84), so the lead can find a
//! ruling settled that they never saw. When a member other than the lead keeps
//! rulings, the lead's list says who kept what: `Bob kept R-84: Ship the
//! relay first`, or `Bob kept 3 rulings: R-12, R-13, R-14` for a Keep All.
//!
//! **Nothing is stored for it.** The FYI is read from the kept rulings
//! themselves (`settled_by`, `settled_at`), one item per keeper and moment, so
//! a Keep All is one item rather than one per ruling. It ages out after
//! `WINDOW_MS`, and a ruling reversed since isn't kept any more, so its line
//! goes at once.
//!
//! **Quiet.** It is not an ask: no buttons, the lowest rank, no push, and no
//! share of the count the relay is told, because the runner-wide read has no
//! viewer (`Inputs::viewer`). Only the lead sees it. The lead's own Keep says
//! nothing to anyone, and neither does a member's to themself.

use farcooler_protocol::v1::{self as pb, NeedsYouKind};
use farcooler_store::rulings::Ruling;
use uuid::Uuid;

use super::{Inputs, rank};
use crate::people::Person;
use crate::wire::{id_bytes, timestamp};

/// How long a Keep stays on the lead's list: three days, so a weekend away
/// doesn't lose it.
pub const WINDOW_MS: i64 = 3 * 24 * 60 * 60 * 1000;

/// The longest a ruling's decision runs on a row, in characters.
const DECISION_CHARS: usize = 80;

/// How many rulings a Keep All names before it says how many more.
const NAMED: usize = 4;

/// One member's Keep: the rulings they kept in one moment on one board.
#[derive(Debug, Clone)]
pub struct Kept {
    pub workspace: Uuid,
    /// Who leads the board: a member's handle, or `None` for the owner.
    pub lead: Option<String>,
    /// The keeper's handle and the name people read them by.
    pub handle: String,
    pub name: String,
    pub at_ms: i64,
    /// The rulings, in the order they were kept.
    pub rulings: Vec<Ruling>,
}

/// Group `rulings` (as `Store::member_kept_rulings_since` returns them) into
/// Keeps, leaving out the lead's own. `lead_of` and `name_of` say who leads a
/// board and what a handle is called.
pub fn group(
    rulings: Vec<Ruling>,
    lead_of: impl Fn(Uuid) -> Option<String>,
    name_of: impl Fn(&str) -> Option<String>,
) -> Vec<Kept> {
    let mut keeps: Vec<Kept> = Vec::new();
    for ruling in rulings {
        let (Some(by), Some(at_ms)) = (ruling.settled_by.as_deref(), ruling.settled_at) else { continue };
        let Some(handle) = by.strip_prefix("member:") else { continue };
        let lead = lead_of(ruling.workspace_id);
        if lead.as_deref() == Some(handle) {
            continue;
        }
        match keeps.iter_mut().find(|k| k.workspace == ruling.workspace_id && k.handle == handle && k.at_ms == at_ms) {
            Some(keep) => keep.rulings.push(ruling),
            None => keeps.push(Kept {
                workspace: ruling.workspace_id,
                lead,
                handle: handle.to_string(),
                name: name_of(handle).unwrap_or_else(|| handle.to_string()),
                at_ms,
                rulings: vec![ruling],
            }),
        }
    }
    keeps
}

fn short(decision: &str) -> String {
    let decision = decision.split_whitespace().collect::<Vec<_>>().join(" ");
    if decision.chars().count() <= DECISION_CHARS {
        return decision;
    }
    let cut: String = decision.chars().take(DECISION_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

/// What the item says: who kept what.
fn question(keep: &Kept) -> String {
    match keep.rulings.as_slice() {
        [one] => format!("{} kept {}: {}", keep.name, one.short(), short(&one.decision)),
        many => {
            let named: Vec<String> = many.iter().take(NAMED).map(Ruling::short).collect();
            let more = many.len().saturating_sub(NAMED);
            let tail = if more > 0 { format!(" and {more} more") } else { String::new() };
            format!("{} kept {} rulings: {}{tail}", keep.name, many.len(), named.join(", "))
        }
    }
}

fn item(keep: &Kept, workspace_name: &str, now_ms: i64) -> pb::NeedsYouItem {
    pb::NeedsYouItem {
        id: format!("fyi:kept:{}:{}:{}", keep.workspace.simple(), keep.handle, keep.at_ms),
        kind: NeedsYouKind::Fyi as i32,
        rank: rank(NeedsYouKind::Fyi, keep.at_ms, now_ms),
        since: Some(timestamp(keep.at_ms)),
        workspace_id: id_bytes(keep.workspace),
        workspace_name: workspace_name.to_string(),
        question: question(keep),
        concern: Some(pb::NeedsYouConcern { handle: keep.handle.clone(), name: keep.name.clone(), yours: false }),
        ..pb::NeedsYouItem::default()
    }
}

/// The FYIs `inputs.viewer` is to see: the lead's, and no one else's.
pub(super) fn items(inputs: &Inputs, now_ms: i64) -> Vec<pb::NeedsYouItem> {
    let leads = |keep: &Kept| match (&inputs.viewer, &keep.lead) {
        (Some(Person::Owner), None) => true,
        (Some(Person::Member(me)), Some(lead)) => me == lead,
        _ => false,
    };
    inputs
        .kept
        .iter()
        .filter(|k| now_ms - k.at_ms <= WINDOW_MS && leads(k))
        .map(|k| item(k, inputs.workspaces.get(&k.workspace).map(String::as_str).unwrap_or(""), now_ms))
        .collect()
}

#[cfg(test)]
#[path = "kept_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "kept_rpc_tests.rs"]
mod rpc_tests;
