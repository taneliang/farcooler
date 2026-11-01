//! The plan on the glance (ov-310, ov-268 P8): what the watch, the widgets
//! and the Live Activity say about a board that has a plan.
//!
//! The watch has no sockets: it hears only what the relay carries, over HTTPS
//! and APNs. So the runner decides the glance here, once, and sends it on its
//! count notice (`push::Outgoing::plan`); the relay keeps the newest and hands
//! it to the card and to `/v1/pulse`. No surface re-derives it.
//!
//! Per board with a plan, and nothing else:
//!
//! - its name, the workspace's;
//! - its Needs You count, the one number the Mac's title bar and sidebar say
//!   for it (`WorkspaceNeedsYou.count` in AgentKit): the items of
//!   `needs_you.list` on that workspace, plus its themes asking the owner
//!   (`needs_you`, the runner's one rule);
//! - up to two tickets in Now, by key and stage word;
//! - the ticket next up, by key.
//!
//! **Ticket keys only.** No card text, no theme's story or ask, no reason, no
//! path or branch: a theme's ask counts, and is never said.

use farcooler_protocol::v1 as pb;
use farcooler_store::Store;
use farcooler_store::plan_read::{LineView, Plan};
use farcooler_store::ticket_lines::Stage;
use std::collections::HashMap;
use uuid::Uuid;

/// How many boards one runner sends. The surfaces draw one.
pub const BOARDS_SENT: usize = 3;
/// How many Now tickets a board carries.
pub const NOW_SENT: usize = 2;
/// The longest name sent, in characters: a workspace's name, or a ticket's key.
pub const NAME_MAX: usize = 60;
/// Finished tickets a plan read keeps, as `plan.get` does: a board whose plan
/// only landed things this week still has a plan.
const CLOSED_FOR_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// One board's glance, as the relay stores and forwards it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BoardGlance {
    pub workspace: String,
    #[serde(rename = "needsYou")]
    pub needs_you: u32,
    pub now: Vec<TicketGlance>,
    /// Absent when nothing is queued in the plan.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

/// A ticket in Now: its key, and its stage as the store spells it
/// (`building`, `reviewing`, `needs_you`, `landing`). The apps say the word.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TicketGlance {
    pub ticket: String,
    pub stage: &'static str,
}

/// A board with a plan, read: its id, its name and the plan.
pub type Planned = (Uuid, String, Plan);

/// The runner's one Needs You rule (review M1): needs-you items plus themes
/// asking the owner, as the Mac's `WorkspaceNeedsYou.count` says it once its
/// list is read. The runner's count on every notice and each board's count
/// on the glance both come from here, so the card's header and the plan line
/// under it can't disagree about a theme's ask.
pub fn needs_you(items: usize, asks: u32) -> u32 {
    items as u32 + asks
}

/// Each of `planned`'s glances, the board that needs the owner first (review
/// M4), at most `BOARDS_SENT`. `items` is `needs_you::assemble`'s list and
/// `asks` is `Store::theme_asks`; the watcher reads the list only when some
/// board has a plan, so a runner without one pays a workspace list and
/// nothing more (`Watcher::plan_glance`).
pub fn boards(planned: Vec<Planned>, items: &[pb::NeedsYouItem], asks: &HashMap<Uuid, u32>) -> Vec<BoardGlance> {
    let mut sent: Vec<BoardGlance> = planned
        .into_iter()
        .map(|(id, name, plan)| glance(id, name, &plan, items, asks.get(&id).copied().unwrap_or(0)))
        .collect();
    // Stable: within each half, `planned`'s order holds.
    sent.sort_by_key(|board| board.needs_you == 0);
    sent.truncate(BOARDS_SENT);
    sent
}

/// Every board on this runner with a plan, the busiest first: one with
/// tickets in Now first, then the one whose plan moved last, then the board's
/// own order.
pub fn planned(store: &Store, now_ms: i64) -> farcooler_core::Result<Vec<Planned>> {
    let mut found = Vec::new();
    for ws in store.list_workspaces(None)? {
        let plan = store.plan(ws.id, now_ms - CLOSED_FOR_MS)?;
        // `PlanModel.isEmpty`: no theme and no ticket on the line.
        if plan.themes.is_empty() && plan.lines.is_empty() {
            continue;
        }
        found.push((ws.id, ws.name, plan));
    }
    found.sort_by_key(|(_, _, plan)| (now_tickets(plan).next().is_none(), std::cmp::Reverse(moved_at(plan))));
    Ok(found)
}

fn glance(workspace: Uuid, name: String, plan: &Plan, items: &[pb::NeedsYouItem], asks: u32) -> BoardGlance {
    let on_board = crate::wire::id_bytes(workspace);
    let items = items.iter().filter(|item| item.workspace_id == on_board).count();
    let key = |l: &LineView| plan.cards.iter().find(|c| c.task_id == l.line.task_id).map(|c| cut(&c.key));
    // The lines come the plan's order first, so the first ranked waiting one is next up.
    let next = plan.lines.iter().find(|l| l.line.stage == Stage::Waiting && l.line.plan_rank.is_some()).and_then(key);
    BoardGlance {
        workspace: cut(&name),
        needs_you: needs_you(items, asks),
        now: now_tickets(plan)
            .filter_map(|l| Some(TicketGlance { ticket: key(l)?, stage: l.line.stage.as_str() }))
            .take(NOW_SENT)
            .collect(),
        next,
    }
}

/// Now: the live tickets past waiting, in the line's order.
fn now_tickets(plan: &Plan) -> impl Iterator<Item = &LineView> {
    plan.lines.iter().filter(|l| !matches!(l.line.stage, Stage::Waiting | Stage::Landed | Stage::Dropped))
}

/// When anything on the plan last moved: a ticket's stage, or a theme's story.
fn moved_at(plan: &Plan) -> i64 {
    let lines = plan.lines.iter().map(|l| l.line.stage_since);
    let themes = plan.themes.iter().map(|t| t.theme.story_at.max(t.theme.created_at));
    lines.chain(themes).max().unwrap_or(0)
}

fn cut(name: &str) -> String {
    name.chars().take(NAME_MAX).collect()
}

#[cfg(test)]
#[path = "plan_glance_tests.rs"]
mod tests;
