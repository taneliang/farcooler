//! Tiling policy: what a layout change *means*.
//!
//! The store reads and writes rows. This file holds the rules, and there are
//! only a handful of them, but each one is the difference between tiling that
//! feels like tmux and tiling that feels like a bug:
//!
//! - **Zoom follows focus.** In tmux, moving to another pane while zoomed
//!   un-zooms. Here it does not: you stay zoomed and the new pane fills the
//!   screen instead. That is a deliberate divergence — a zoomed agent is a
//!   reading posture, and cycling between four agents each at full size is the
//!   thing people actually want when they zoom. `prefix z` is still the way out.
//!
//! - **Nothing tiles by default.** A terminal belongs to a group only because
//!   someone put it there. Four agents in a worktree with three on screen is the
//!   stated case, and it only works if membership is opt-in.
//!
//! - **Un-tiling never kills.** Dropping a pane backgrounds its terminal.
//!   Closing a group backgrounds all of them. If closing a layout could stop an
//!   agent, nobody would ever close one.
//!
//! - **A group that empties behind your back disappears.** Moving three panes
//!   into another group leaves the first one empty, and an empty layout you did
//!   not ask for is a ghost the next `tile` has to reason about. The group you
//!   are looking at is exempt: `group new` deliberately makes an empty one, and
//!   dropping the last pane out of the group in front of you leaves somewhere to
//!   tile into rather than silently un-tiling.
//!
//! All of it is durable and all of it is reachable from the CLI, which is the
//! same requirement stated twice: an agent that can open a terminal but not
//! place it on screen is only half automatable.

use overnight_core::{DomainError, Result};
use overnight_protocol::v1::{LayoutPreset, TerminalState};
use overnight_store::PaneGroup;
use uuid::Uuid;

use crate::service::Service;

/// The presets `layout.cycle` walks, in order.
///
/// tmux's `prefix Space` order, so the muscle memory transfers. Ends back at the
/// start, because a cycle that stops is a list.
const CYCLE: [LayoutPreset; 5] = [
    LayoutPreset::EvenHorizontal,
    LayoutPreset::EvenVertical,
    LayoutPreset::MainVertical,
    LayoutPreset::MainHorizontal,
    LayoutPreset::Tiled,
];

/// Parse a preset from the name tmux uses for it.
///
/// The tmux spellings are the canonical ones, plus the obvious short forms,
/// because `overnight layout preset grid` is what someone types before they
/// remember it is called `tiled`.
pub fn parse_preset(text: &str) -> Option<LayoutPreset> {
    Some(match text.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "even-horizontal" | "columns" | "cols" | "row" => LayoutPreset::EvenHorizontal,
        "even-vertical" | "rows" | "stack" | "column" => LayoutPreset::EvenVertical,
        "main-vertical" | "main" | "main-v" => LayoutPreset::MainVertical,
        "main-horizontal" | "main-h" => LayoutPreset::MainHorizontal,
        "tiled" | "grid" | "tile" => LayoutPreset::Tiled,
        _ => return None,
    })
}

pub fn preset_name(preset: LayoutPreset) -> &'static str {
    match preset {
        LayoutPreset::EvenHorizontal => "even-horizontal",
        LayoutPreset::EvenVertical => "even-vertical",
        LayoutPreset::MainVertical => "main-vertical",
        LayoutPreset::MainHorizontal => "main-horizontal",
        LayoutPreset::Tiled | LayoutPreset::Unspecified => "tiled",
    }
}

fn next_preset(current: LayoutPreset) -> LayoutPreset {
    let at = CYCLE.iter().position(|p| *p == current).unwrap_or(CYCLE.len() - 1);
    CYCLE[(at + 1) % CYCLE.len()]
}

impl Service {
    /// Every group in a workspace, in order.
    pub fn layout(&self, workspace: Uuid) -> Result<Vec<PaneGroup>> {
        self.store.pane_groups(workspace)
    }

    /// What every mutation returns: the workspace's groups, minus the ghosts.
    ///
    /// A pane joining another group leaves the one it was in, and if it was the
    /// last one that group is now an empty layout nobody asked for. The active
    /// group is spared, because there an empty layout IS the request — `group
    /// new` makes one on purpose, and emptying the group in front of you should
    /// leave somewhere to tile into rather than quietly turning tiling off.
    fn finish(&self, workspace: Uuid) -> Result<Vec<PaneGroup>> {
        for group in self.store.pane_groups(workspace)? {
            if group.members.is_empty() && !group.active {
                self.store.delete_pane_group(group.id)?;
            }
        }
        self.layout(workspace)
    }

    /// The group being acted on: the one named, or the workspace's active one.
    ///
    /// "The active one" is what a person means by "here" and what an agent means
    /// when it does not say, so omitting the id is the common case rather than
    /// an error.
    fn resolve_group(&self, workspace: Uuid, group: Option<Uuid>) -> Result<PaneGroup> {
        match group {
            Some(id) => {
                let found = self.store.pane_group(id)?;
                if found.workspace_id != workspace {
                    return Err(DomainError::NotFound);
                }
                Ok(found)
            }
            None => self.store.active_pane_group(workspace)?.ok_or(DomainError::NotFound),
        }
    }

    /// The group to act on, creating one if the workspace has none.
    ///
    /// Only for the calls that are unambiguously "put this on screen". A focus
    /// or a zoom against a workspace with no layout is a mistake, not a request
    /// to invent one.
    async fn group_or_create(&self, workspace: Uuid, group: Option<Uuid>) -> Result<PaneGroup> {
        match self.resolve_group(workspace, group) {
            Ok(found) => Ok(found),
            Err(DomainError::NotFound) if group.is_none() => {
                self.store.create_pane_group(
                    Uuid::now_v7(),
                    workspace,
                    "1",
                    LayoutPreset::Tiled,
                )
            }
            Err(e) => Err(e),
        }
    }

    /// Put a set of terminals on screen together, replacing what was there.
    ///
    /// An empty list means every live terminal in the workspace, which is the
    /// one-word version of the whole feature: `overnight layout tile <ws>` and
    /// the worktree you are in is now four panes.
    pub async fn layout_tile(
        &self,
        workspace: Uuid,
        group: Option<Uuid>,
        terminals: &[Uuid],
        preset: Option<LayoutPreset>,
    ) -> Result<Vec<PaneGroup>> {
        let target = self.group_or_create(workspace, group).await?;
        let members = if terminals.is_empty() {
            self.live_terminals(workspace).await?
        } else {
            self.verify_members(workspace, terminals).await?
        };

        if members.is_empty() {
            // Nothing to show. Tiling an empty workspace leaving an empty group
            // behind would mean the next tile has a ghost to reason about.
            self.store.delete_pane_group(target.id)?;
            return self.layout(workspace);
        }

        let mut saved = self.store.set_pane_members(target.id, &members)?;
        saved.focused = Some(match saved.focused {
            Some(existing) if members.contains(&existing) => existing,
            _ => members[0],
        });
        // Re-tiling drops zoom. You asked to see several things.
        saved.zoomed = None;
        if let Some(preset) = preset {
            if preset != LayoutPreset::Unspecified {
                saved.preset = preset;
            }
        }
        self.store.save_pane_group(&saved)?;
        self.finish(workspace)
    }

    /// Add terminals to a group without disturbing the ones already in it.
    pub async fn layout_add(
        &self,
        workspace: Uuid,
        group: Option<Uuid>,
        terminals: &[Uuid],
    ) -> Result<Vec<PaneGroup>> {
        let target = self.group_or_create(workspace, group).await?;
        let incoming = self.verify_members(workspace, terminals).await?;
        if incoming.is_empty() {
            return self.layout(workspace);
        }

        let mut members = target.members.clone();
        members.retain(|m| !incoming.contains(m));
        members.extend(incoming.iter().copied());

        let mut saved = self.store.set_pane_members(target.id, &members)?;
        // Focus the newest arrival: adding a pane is how you make a pane you
        // intend to type in.
        saved.focused = incoming.last().copied();
        // A pane arriving while another is zoomed would otherwise be invisible.
        saved.zoomed = saved.zoomed.and(saved.focused);
        self.store.save_pane_group(&saved)?;
        self.finish(workspace)
    }

    /// Take terminals off screen. They keep running.
    pub async fn layout_drop(&self, workspace: Uuid, terminals: &[Uuid]) -> Result<Vec<PaneGroup>> {
        // Which groups are affected has to be read before the membership rows
        // are gone, or there is nothing left to point at.
        let mut touched = Vec::new();
        for terminal in terminals {
            if let Some(group) = self.store.pane_group_of(*terminal)? {
                if !touched.contains(&group) {
                    touched.push(group);
                }
            }
        }
        self.store.drop_pane_members(terminals)?;

        for id in touched {
            let group = self.store.pane_group(id)?;
            if group.members.is_empty() {
                self.store.delete_pane_group(id)?;
                continue;
            }
            if group.focused.is_none() {
                // The focused pane left, so focus has to land somewhere or the
                // next keystroke has nothing to act on.
                let mut fixed = group.clone();
                fixed.focused = group.members.first().copied();
                self.store.save_pane_group(&fixed)?;
            }
        }
        self.finish(workspace)
    }

    /// Change how a group is arranged.
    pub async fn layout_configure(
        &self,
        workspace: Uuid,
        group: Option<Uuid>,
        preset: Option<LayoutPreset>,
        ratio: Option<f64>,
        name: Option<&str>,
    ) -> Result<Vec<PaneGroup>> {
        let mut target = self.resolve_group(workspace, group)?;
        if let Some(preset) = preset {
            if preset != LayoutPreset::Unspecified {
                target.preset = preset;
            }
        }
        if let Some(ratio) = ratio {
            // Clamped rather than rejected: a main pane at 2% is not a layout,
            // and an agent that computed 0.98 meant "mostly this one".
            if !ratio.is_finite() {
                return Err(DomainError::InvalidArgument { what: "ratio" });
            }
            target.ratio = ratio.clamp(0.15, 0.85);
        }
        if let Some(name) = name {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                target.name = trimmed.chars().take(48).collect();
            }
        }
        self.store.save_pane_group(&target)?;
        self.finish(workspace)
    }

    /// tmux's `prefix Space`: the next arrangement of the same panes.
    pub async fn layout_cycle(&self, workspace: Uuid, group: Option<Uuid>) -> Result<Vec<PaneGroup>> {
        let mut target = self.resolve_group(workspace, group)?;
        target.preset = next_preset(target.preset);
        self.store.save_pane_group(&target)?;
        self.finish(workspace)
    }

    /// Focus a specific pane.
    ///
    /// If the group is zoomed, the zoom moves with the focus. That is the
    /// divergence from tmux described at the top of this file, and it is the
    /// behaviour that makes zoom useful with more than one agent.
    pub async fn layout_focus(&self, workspace: Uuid, terminal: Uuid) -> Result<Vec<PaneGroup>> {
        let group_id = self.store.pane_group_of(terminal)?.ok_or(DomainError::NotFound)?;
        let mut target = self.resolve_group(workspace, Some(group_id))?;
        target.focused = Some(terminal);
        if target.zoomed.is_some() {
            target.zoomed = Some(terminal);
        }
        self.store.save_pane_group(&target)?;
        self.finish(workspace)
    }

    /// Move focus by position: `+1` is tmux's `prefix o`, `-1` its `prefix ;`.
    ///
    /// Wraps, because a pane cycle that stops at the end makes you look at the
    /// screen to find out whether pressing it again will do anything.
    pub async fn layout_focus_step(
        &self,
        workspace: Uuid,
        group: Option<Uuid>,
        delta: i64,
    ) -> Result<Vec<PaneGroup>> {
        let target = self.resolve_group(workspace, group)?;
        if target.members.is_empty() {
            return self.layout(workspace);
        }
        let count = target.members.len() as i64;
        let at = target
            .focused
            .and_then(|f| target.members.iter().position(|m| *m == f))
            .unwrap_or(0) as i64;
        let next = ((at + delta) % count + count) % count;
        self.layout_focus(workspace, target.members[next as usize]).await
    }

    /// Focus the Nth pane, one-based, the way `prefix 1` reads.
    pub async fn layout_focus_index(
        &self,
        workspace: Uuid,
        group: Option<Uuid>,
        index: usize,
    ) -> Result<Vec<PaneGroup>> {
        let target = self.resolve_group(workspace, group)?;
        let Some(terminal) = index.checked_sub(1).and_then(|i| target.members.get(i)).copied()
        else {
            return Err(DomainError::InvalidArgument { what: "pane index" });
        };
        self.layout_focus(workspace, terminal).await
    }

    /// Zoom a pane, or clear the zoom.
    ///
    /// `None` toggles on whatever is focused, which is what a keystroke means.
    /// An explicit terminal also focuses it, since a zoomed pane you cannot type
    /// in is a screenshot.
    pub async fn layout_zoom(
        &self,
        workspace: Uuid,
        group: Option<Uuid>,
        terminal: Option<Uuid>,
        off: bool,
    ) -> Result<Vec<PaneGroup>> {
        let mut target = self.resolve_group(workspace, group)?;
        if off {
            target.zoomed = None;
        } else if let Some(terminal) = terminal {
            if !target.members.contains(&terminal) {
                return Err(DomainError::NotFound);
            }
            target.zoomed = Some(terminal);
            target.focused = Some(terminal);
        } else {
            let focused = target.focused.or_else(|| target.members.first().copied());
            target.zoomed = if target.zoomed.is_some() { None } else { focused };
            target.focused = focused;
        }
        self.store.save_pane_group(&target)?;
        self.finish(workspace)
    }

    /// Exchange two panes' positions, tmux's `prefix {` and `prefix }`.
    pub async fn layout_swap(&self, workspace: Uuid, a: Uuid, b: Uuid) -> Result<Vec<PaneGroup>> {
        let target = self.resolve_group(workspace, self.store.pane_group_of(a)?)?;
        let (Some(i), Some(j)) = (
            target.members.iter().position(|m| *m == a),
            target.members.iter().position(|m| *m == b),
        ) else {
            return Err(DomainError::NotFound);
        };
        let mut members = target.members.clone();
        members.swap(i, j);
        self.store.set_pane_members(target.id, &members)?;
        self.finish(workspace)
    }

    /// Move the focused pane one place along, carrying focus with it.
    ///
    /// The rotation people actually use: `prefix {` to promote the agent you
    /// care about into the main slot without naming either pane.
    pub async fn layout_shift(
        &self,
        workspace: Uuid,
        group: Option<Uuid>,
        delta: i64,
    ) -> Result<Vec<PaneGroup>> {
        let target = self.resolve_group(workspace, group)?;
        if target.members.len() < 2 {
            return self.layout(workspace);
        }
        let count = target.members.len() as i64;
        let at = target
            .focused
            .and_then(|f| target.members.iter().position(|m| *m == f))
            .unwrap_or(0) as i64;
        let to = ((at + delta) % count + count) % count;
        let mut members = target.members.clone();
        members.swap(at as usize, to as usize);
        self.store.set_pane_members(target.id, &members)?;
        self.finish(workspace)
    }

    /// A new, empty group, and it becomes the one on screen.
    ///
    /// Empty is correct: this is tmux's `prefix c` for layouts, and what follows
    /// is putting things in it. Nothing is destroyed — the previous group's panes
    /// are still running, still in that group, one keystroke away.
    pub async fn layout_group_new(&self, workspace: Uuid, name: &str) -> Result<Vec<PaneGroup>> {
        let existing = self.store.pane_groups(workspace)?;
        let name = match name.trim() {
            "" => (existing.len() + 1).to_string(),
            given => given.chars().take(48).collect(),
        };
        self.store.create_pane_group(Uuid::now_v7(), workspace, &name, LayoutPreset::Tiled)?;
        self.finish(workspace)
    }

    /// Show a different group.
    pub async fn layout_group_select(&self, workspace: Uuid, group: Uuid) -> Result<Vec<PaneGroup>> {
        let target = self.resolve_group(workspace, Some(group))?;
        self.store.activate_pane_group(target.id)?;
        self.finish(workspace)
    }

    /// Show the next or previous group, wrapping. tmux's `prefix n` and `p`.
    pub async fn layout_group_step(&self, workspace: Uuid, delta: i64) -> Result<Vec<PaneGroup>> {
        let groups = self.store.pane_groups(workspace)?;
        if groups.len() < 2 {
            return Ok(groups);
        }
        let count = groups.len() as i64;
        let at = groups.iter().position(|g| g.active).unwrap_or(0) as i64;
        let next = ((at + delta) % count + count) % count;
        self.store.activate_pane_group(groups[next as usize].id)?;
        self.finish(workspace)
    }

    /// Stop showing a group. Its terminals keep running, backgrounded.
    pub async fn layout_group_close(
        &self,
        workspace: Uuid,
        group: Option<Uuid>,
    ) -> Result<Vec<PaneGroup>> {
        let target = self.resolve_group(workspace, group)?;
        self.store.delete_pane_group(target.id)?;
        self.finish(workspace)
    }

    /// The terminals in a workspace that are worth putting on screen.
    async fn live_terminals(&self, workspace: Uuid) -> Result<Vec<Uuid>> {
        let ws = self.store.get_workspace(workspace)?;
        let view = self.workspace_view(&ws).await?;
        Ok(view
            .terminals
            .iter()
            .filter(|t| matches!(t.state(), TerminalState::Running | TerminalState::Starting))
            .map(|t| t.terminal.id)
            .collect())
    }

    /// Keep only terminals that belong to this workspace.
    ///
    /// Silently, rather than by refusing the whole call: an agent asking to tile
    /// four terminals when one has just exited meant the other three, and a hard
    /// error there turns a race into a failure.
    async fn verify_members(&self, workspace: Uuid, terminals: &[Uuid]) -> Result<Vec<Uuid>> {
        let mut kept = Vec::new();
        for id in terminals {
            let Ok(record) = self.store.get_terminal(*id) else { continue };
            if record.workspace_id == workspace && !kept.contains(id) {
                kept.push(*id);
            }
        }
        Ok(kept)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmux_names_parse_and_round_trip() {
        for preset in CYCLE {
            assert_eq!(parse_preset(preset_name(preset)), Some(preset), "{preset:?}");
        }
    }

    #[test]
    fn the_names_people_actually_type_also_work() {
        assert_eq!(parse_preset("grid"), Some(LayoutPreset::Tiled));
        assert_eq!(parse_preset("main"), Some(LayoutPreset::MainVertical));
        assert_eq!(parse_preset("COLUMNS"), Some(LayoutPreset::EvenHorizontal));
        assert_eq!(parse_preset("main_horizontal"), Some(LayoutPreset::MainHorizontal));
        assert_eq!(parse_preset("diagonal"), None);
    }

    #[test]
    fn cycling_visits_every_layout_and_returns() {
        let mut seen = vec![CYCLE[0]];
        let mut at = CYCLE[0];
        for _ in 0..CYCLE.len() {
            at = next_preset(at);
            if !seen.contains(&at) {
                seen.push(at);
            }
        }
        assert_eq!(seen.len(), CYCLE.len(), "cycling must reach all of them");
        assert_eq!(at, CYCLE[0], "and come back");
    }

    #[test]
    fn an_unknown_preset_cycles_to_the_first_rather_than_sticking() {
        // A database written by a newer version could hold a preset this build
        // does not know. Cycling out of it has to work.
        assert_eq!(next_preset(LayoutPreset::Unspecified), CYCLE[0]);
    }
}
