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
//!   `needs_you.list` on that workspace, plus its themes asking the owner;
//! - up to two lanes in Now, by name and state word;
//! - the lane next up, by name.
//!
//! **Lane names only.** No card text, no theme's story or ask, no reason, no
//! path or branch: a theme's ask counts, and is never said.

use farcooler_protocol::v1 as pb;
use farcooler_store::Store;
use farcooler_store::plan::{LaneState, ThemeState};
use farcooler_store::plan_read::Plan;
use uuid::Uuid;

/// How many boards one runner sends. The surfaces draw one.
pub const BOARDS_SENT: usize = 3;
/// How many Now lanes a board carries.
pub const NOW_SENT: usize = 2;
/// The longest name sent, in characters: the store's own bound on a lane's
/// name, so nothing it keeps is cut.
pub const NAME_MAX: usize = 60;
/// Finished lanes a plan read keeps, as `plan.get` does: a board whose plan
/// only landed things this week still has a plan.
const CLOSED_FOR_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// One board's glance, as the relay stores and forwards it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BoardGlance {
    pub workspace: String,
    #[serde(rename = "needsYou")]
    pub needs_you: u32,
    pub now: Vec<LaneGlance>,
    /// Absent when nothing is queued in the plan.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

/// A lane in Now: its name, and its state as the store spells it
/// (`building`, `review`, `fixing`, `landing`). The apps say the word.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LaneGlance {
    pub name: String,
    pub state: &'static str,
}

/// A board with a plan, read: its id, its name and the plan.
pub type Planned = (Uuid, String, Plan);

/// Each of `planned`'s glances. `items` is `needs_you::assemble`'s list,
/// which the watcher reads only when some board has a plan, so a runner
/// without one pays a workspace list and nothing more
/// (`Watcher::plan_glance`).
pub fn boards(planned: Vec<Planned>, items: &[pb::NeedsYouItem]) -> Vec<BoardGlance> {
    planned.into_iter().map(|(id, name, plan)| glance(id, name, &plan, items)).collect()
}

/// Every board on this runner with a plan, the busiest first, at most
/// `BOARDS_SENT`: one with lanes in Now first, then the one whose plan moved
/// last, then the board's own order.
pub fn planned(store: &Store, now_ms: i64) -> farcooler_core::Result<Vec<Planned>> {
    let mut found = Vec::new();
    for ws in store.list_workspaces(None)? {
        let plan = store.plan(ws.id, now_ms - CLOSED_FOR_MS)?;
        // `PlanModel.isEmpty`: no theme and no lane.
        if plan.themes.is_empty() && plan.lanes.is_empty() {
            continue;
        }
        found.push((ws.id, ws.name, plan));
    }
    found.sort_by_key(|(_, _, plan)| (now_lanes(plan).next().is_none(), std::cmp::Reverse(moved_at(plan))));
    found.truncate(BOARDS_SENT);
    Ok(found)
}

fn glance(workspace: Uuid, name: String, plan: &Plan, items: &[pb::NeedsYouItem]) -> BoardGlance {
    let on_board = crate::wire::id_bytes(workspace);
    let items = items.iter().filter(|item| item.workspace_id == on_board).count();
    // `shownThemes` with an `ownerAsk`, as the Mac counts them.
    let asks =
        plan.themes.iter().filter(|t| t.theme.state != ThemeState::Dropped && !t.theme.owner_ask.is_empty()).count();
    let next = plan
        .order
        .iter()
        .find_map(|id| plan.lanes.iter().find(|l| l.lane.id == *id && l.lane.state == LaneState::Queued))
        .map(|l| cut(&l.lane.name));
    BoardGlance {
        workspace: cut(&name),
        needs_you: (items + asks) as u32,
        now: now_lanes(plan)
            .take(NOW_SENT)
            .map(|l| LaneGlance { name: cut(&l.lane.name), state: l.lane.state.as_str() })
            .collect(),
        next,
    }
}

/// Now: `PlanModel.working`, the live lanes past queued, in the plan's order.
fn now_lanes(plan: &Plan) -> impl Iterator<Item = &farcooler_store::plan_read::LaneView> {
    plan.lanes
        .iter()
        .filter(|l| !matches!(l.lane.state, LaneState::Queued | LaneState::Landed | LaneState::Dropped))
}

/// When anything on the plan last moved: a lane's state, or a theme's story.
fn moved_at(plan: &Plan) -> i64 {
    let lanes = plan.lanes.iter().map(|l| l.lane.state_since);
    let themes = plan.themes.iter().map(|t| t.theme.story_at.max(t.theme.created_at));
    lanes.chain(themes).max().unwrap_or(0)
}

fn cut(name: &str) -> String {
    name.chars().take(NAME_MAX).collect()
}

#[cfg(test)]
#[path = "plan_glance_tests.rs"]
mod tests;
