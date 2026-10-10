//! A pane opened for a plan lane (ov-457): `terminal.create` with a `lane`.
//!
//! A lane is the unit of execution: one agent, or a chain of them, in one
//! worktree on one branch, working one or more cards. `task dispatch` opens a
//! pane on one card; a lane dispatch opens it on the lane's first open card
//! and names the lane, and the runner does the rest:
//!
//! - **refuses** a lane that isn't live on the task's own board, or doesn't
//!   hold the task (`resolve`), before anything is made;
//! - **briefs** the agent on every card the lane holds, after the task's own
//!   opening prompt (`brief`);
//! - **records** the pane as the lane's build agent, puts the lane on the
//!   pane's worktree and branch, and moves a queued lane to building
//!   (`start`, `Store::start_lane_pane`), so nobody runs `plan lane start
//!   --agent` by hand. The pane's turns are then the lane's spend
//!   (`plan_read::TURNS_OF_AGENT`);
//! - **exports** `FARCOOLER_LANE` on every launch while the pane works the
//!   lane (`env`), read off the layer, never off the terminal's row.
//!
//! The link is the layer's, so the board never names it, and a pane whose
//! lane is gone simply exports no lane.

use farcooler_core::{DomainError, Result};
use farcooler_store::Store;
use farcooler_store::models::{Actor, TaskStatus, Terminal, Worktree};
use farcooler_store::plan::Lane;
use farcooler_store::plan_panes::PaneStart;
use uuid::Uuid;

/// The widest a card's title is quoted in a brief.
const TITLE_WIDTH: usize = 60;

/// The live lane `name` on `task`'s board that holds `task`, or `lane` when
/// there is none: no task to resolve it on, no such lane, or a lane without
/// the task. `None` when no lane was asked for.
pub(crate) fn resolve(store: &Store, task: Option<Uuid>, name: Option<&str>) -> Result<Option<Lane>> {
    let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) else { return Ok(None) };
    let refused = DomainError::InvalidArgument { what: "lane" };
    let task = store.get_task(task.ok_or(refused.clone())?)?;
    let lane = store.live_lane_named(task.workspace_id, name).map_err(|_| refused.clone())?;
    if !store.lane_cards_in_order(lane.id)?.iter().any(|c| c.task_id == task.id) {
        return Err(refused);
    }
    Ok(Some(lane))
}

/// What the agent is told about its lane, after its task's opening prompt:
/// the lane's name and every card on it still to do, in order, and how to
/// move on to the next. `cli` is quoted as `opening_prompt`'s is.
pub(crate) fn brief(store: &Store, lane: &Lane, cli: &str) -> Result<String> {
    let mut cards = Vec::new();
    for card in store.lane_cards_in_order(lane.id)? {
        let Ok(task) = store.get_task(card.task_id) else { continue };
        if matches!(task.status, TaskStatus::Done | TaskStatus::Cancelled) {
            continue;
        }
        let title = crate::watch::answer_wake::one_line(&task.title, TITLE_WIDTH);
        cards.push(match card.slice.as_str() {
            "" => format!("{} ({title})", task.key),
            slice => format!("{} ({title}; only {slice})", task.key),
        });
    }
    Ok(format!(
        "This pane works the lane {name}, whose cards are, in order: {list}. Work them one at a time, \
         in this worktree and on this branch. When one is done, move it the way it says, then start \
         the next: read it with {cli} task show <key>, and move it into progress with {cli} task set \
         <key> --status in_progress. {cli} plan lane show {name} shows the lane.",
        name = lane.name,
        list = cards.join(", "),
    ))
}

/// `prompt`, the task's opening prompt, with `lane`'s brief after it, the
/// CLI named as `opening_prompt` names it. `prompt` unchanged with no lane.
pub(crate) fn with_brief(store: &Store, lane: Option<&Lane>, prompt: Option<String>) -> Result<Option<String>> {
    let Some(lane) = lane else { return Ok(prompt) };
    let cli = crate::service::shell_quote(&crate::service::shim_binary(std::env::current_exe().ok().as_deref()));
    let brief = brief(store, lane, &cli)?;
    Ok(Some(match prompt {
        Some(opening) => format!("{opening}\n\n{brief}"),
        None => brief,
    }))
}

/// Record `term`'s pane, just made in `ws` running `preset`, as `lane`'s
/// build agent: see this module's docs. The runner's own write.
pub(crate) fn start(store: &Store, lane: &Lane, term: &Terminal, preset: &str, ws: &Worktree) -> Result<()> {
    let start = PaneStart {
        terminal: term.id,
        preset: preset.to_string(),
        worktree: ws.id,
        worktree_path: ws.worktree_path.clone(),
        branch: ws.branch.clone(),
    };
    store.start_lane_pane(lane.id, &start, Actor::Runner).map(|_| ())
}

/// `FARCOOLER_LANE` and the lane's name, for a pane that works a live lane
/// and whose name is a plain word. Nothing for any other pane.
pub(crate) fn env(store: &Store, term: &Terminal) -> Option<(String, String)> {
    let lane = store.lane_of_pane(term.id).ok().flatten()?;
    let plain = lane.name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    plain.then(|| (farcooler_core::pane_env::LANE.to_string(), lane.name))
}

#[cfg(test)]
#[path = "lane_panes_tests.rs"]
mod tests;
