//! Tiling, delegated to tmux.
//!
//! A tmux **window** is a layout and a tmux **pane** is a terminal. That is the
//! whole model, and everything else follows from it:
//!
//! - Splitting a pane arbitrarily is `split-window -h|-v [-b]`, so a drop on any
//!   edge of any pane is one command and there is no tree here to maintain.
//! - The five named arrangements are `select-layout`, under tmux's own names.
//! - Dividers move with `resize-pane`. Zoom is `resize-pane -Z`.
//! - Moving a pane between layouts is `join-pane`; pulling one out is
//!   `break-pane`.
//! - Where every pane sits comes from `list-panes`, in cells, computed by tmux.
//!
//! Overnight stores none of it, and that is the same rule the rest of the daemon
//! already follows: tmux is the authority for live runtime, and an arrangement of
//! live processes is runtime. If the server dies the panes die with it, and there
//! is no arrangement left to restore them into.
//!
//! What the daemon still owns is identity — which pane is which terminal, tagged
//! on the pane itself, so a window holding four panes reports four terminals.
//!
//! This replaces a durable split model in SQLite with five preset arrangements, a
//! geometry function in the daemon, and a second geometry function in every client
//! that drew it: three implementations of one question, and the preset model could
//! not express an arbitrary split at all.

use overnight_core::inventory::TaggedPane;
use overnight_core::{DomainError, Result};
use overnight_protocol::v1::{LayoutPreset, SplitSide};
use overnight_tmux::windows::{Axis, ManagedLayout, Preset};
use uuid::Uuid;

use crate::service::Service;

/// A layout and the panes tmux placed in it.
#[derive(Debug, Clone)]
pub struct LayoutView {
    pub window: ManagedLayout,
    /// Left to right, top to bottom — the order a person counts panes on screen,
    /// and the order `prefix 1..9` selects them in.
    pub panes: Vec<TaggedPane>,
}

impl LayoutView {
    /// The window's size in cells.
    ///
    /// Derived from the panes rather than queried separately: a window's size is
    /// exactly the extent of its panes, and a second round trip could disagree
    /// with the first.
    pub fn size(&self) -> (u32, u32) {
        let columns = self.panes.iter().map(|p| p.left + p.columns).max().unwrap_or(0);
        let rows = self.panes.iter().map(|p| p.top + p.rows).max().unwrap_or(0);
        (columns, rows)
    }

    pub fn focused(&self) -> Option<&TaggedPane> {
        self.panes.iter().find(|p| p.pane_active)
    }
}

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
    tmux_preset(preset).as_str()
}

fn tmux_preset(preset: LayoutPreset) -> Preset {
    match preset {
        LayoutPreset::EvenHorizontal => Preset::EvenHorizontal,
        LayoutPreset::EvenVertical => Preset::EvenVertical,
        LayoutPreset::MainVertical => Preset::MainVertical,
        LayoutPreset::MainHorizontal => Preset::MainHorizontal,
        LayoutPreset::Tiled | LayoutPreset::Unspecified => Preset::Tiled,
    }
}

/// A drop edge, as tmux's split arguments.
///
/// Four sides need only two axes and a flag: left is right with `-b`, and top is
/// bottom with `-b`, because `-b` means "put the new pane first". That is the
/// entire vocabulary of an arbitrary directional split, which is why delegating
/// leaves nothing here to maintain.
pub fn split_args(side: SplitSide) -> (Axis, bool) {
    match side {
        SplitSide::Left => (Axis::Horizontal, true),
        SplitSide::Top => (Axis::Vertical, true),
        SplitSide::Bottom => (Axis::Vertical, false),
        // A caller that did not say meant `prefix %`.
        SplitSide::Right | SplitSide::Unspecified => (Axis::Horizontal, false),
    }
}

impl Service {
    /// Every layout in a workspace, in window order, with tmux's geometry.
    pub async fn layout(&self, workspace: Uuid) -> Result<Vec<LayoutView>> {
        let panes = self.tmux.list_tagged_panes().await?;
        let mut windows: Vec<ManagedLayout> = self
            .tmux
            .list_layouts()
            .await?
            .into_iter()
            .filter(|w| w.workspace_id == workspace)
            .collect();
        windows.sort_by_key(|w| w.index);

        Ok(windows
            .into_iter()
            .map(|window| {
                let mut mine: Vec<TaggedPane> =
                    panes.iter().filter(|p| p.window_id == window.window_id).cloned().collect();
                mine.sort_by_key(|p| (p.top, p.left));
                LayoutView { window, panes: mine }
            })
            // A window with no tagged panes is not ours to show.
            .filter(|view| !view.panes.is_empty())
            .collect())
    }

    /// The layout on screen for a workspace.
    ///
    /// tmux marks one window active per SESSION, and the session spans every
    /// workspace — so "active" is read within the workspace, and a workspace whose
    /// windows are all inactive still has to show something.
    pub async fn active_layout(&self, workspace: Uuid) -> Result<Option<LayoutView>> {
        let layouts = self.layout(workspace).await?;
        Ok(layouts.iter().find(|l| l.window.active).or_else(|| layouts.first()).cloned())
    }

    /// The layout being acted on: the one named, or the active one.
    async fn resolve(&self, workspace: Uuid, group: Option<&str>) -> Result<LayoutView> {
        match group {
            Some(id) if !id.is_empty() => self
                .layout(workspace)
                .await?
                .into_iter()
                .find(|l| l.window.window_id == id)
                .ok_or(DomainError::NotFound),
            _ => self.active_layout(workspace).await?.ok_or(DomainError::NotFound),
        }
    }

    /// The pane a terminal lives in.
    pub async fn pane_of(&self, terminal: Uuid) -> Result<TaggedPane> {
        self.tmux
            .list_tagged_panes()
            .await?
            .into_iter()
            .find(|p| p.terminal_id == terminal)
            .ok_or(DomainError::NotFound)
    }

    /// Move an existing pane against another, on a given edge.
    ///
    /// A directional drag and drop, in one command — and it works across layouts,
    /// because `join-pane` does: dropping a pane from one arrangement onto another
    /// moves it there.
    pub async fn layout_move(
        &self,
        workspace: Uuid,
        dragged: Uuid,
        target: Uuid,
        side: SplitSide,
    ) -> Result<Vec<LayoutView>> {
        if dragged == target {
            return self.layout(workspace).await;
        }
        let source = self.pane_of(dragged).await?;
        let destination = self.pane_of(target).await?;
        let (axis, before) = split_args(side);

        self.tmux.join_pane(&source.pane_id, &destination.pane_id, axis, before).await?;
        self.tmux.select_pane(&source.pane_id).await?;
        self.layout(workspace).await
    }

    /// Rearrange a layout into one of tmux's five.
    pub async fn layout_preset(
        &self,
        workspace: Uuid,
        group: Option<&str>,
        preset: LayoutPreset,
    ) -> Result<Vec<LayoutView>> {
        let view = self.resolve(workspace, group).await?;
        self.tmux.select_preset(&view.window.window_id, tmux_preset(preset)).await?;
        self.layout(workspace).await
    }

    /// tmux's `prefix Space`.
    pub async fn layout_cycle(
        &self,
        workspace: Uuid,
        group: Option<&str>,
    ) -> Result<Vec<LayoutView>> {
        let view = self.resolve(workspace, group).await?;
        self.tmux.next_preset(&view.window.window_id).await?;
        self.layout(workspace).await
    }

    /// Focus a pane, which is also what puts its layout on screen.
    ///
    /// One call, because a pane cannot hold the keyboard while a different
    /// arrangement is showing. Two calls could disagree about which pane you
    /// asked for, and did.
    pub async fn layout_focus(&self, workspace: Uuid, terminal: Uuid) -> Result<Vec<LayoutView>> {
        let pane = self.pane_of(terminal).await?;
        // Was anything zoomed before we moved? tmux drops zoom when the active
        // pane changes, which is right for one pane at a time and wrong here.
        //
        // Zooming a single pane is a way to see more of it; zooming across four
        // agents is a reading posture, and being dropped back into the grid
        // between each one is not what anyone meant. So the zoom is re-applied to
        // wherever focus landed. This is the one place Overnight overrides tmux's
        // own behaviour rather than borrowing it, and it is deliberate.
        let was_zoomed = self
            .tmux
            .list_tagged_panes()
            .await?
            .iter()
            .any(|p| p.window_id == pane.window_id && p.zoomed);

        self.tmux.select_window(&pane.window_id).await?;
        self.tmux.select_pane(&pane.pane_id).await?;
        if was_zoomed {
            self.tmux.unzoom(&pane.window_id).await?;
            self.tmux.toggle_zoom(&pane.pane_id).await?;
        }
        self.layout(workspace).await
    }

    /// Move focus by position, wrapping. `+1` is `prefix o`.
    pub async fn layout_focus_step(
        &self,
        workspace: Uuid,
        group: Option<&str>,
        delta: i64,
    ) -> Result<Vec<LayoutView>> {
        let view = self.resolve(workspace, group).await?;
        if view.panes.is_empty() {
            return self.layout(workspace).await;
        }
        let count = view.panes.len() as i64;
        let at = view.panes.iter().position(|p| p.pane_active).unwrap_or(0) as i64;
        let next = ((at + delta) % count + count) % count;
        self.layout_focus(workspace, view.panes[next as usize].terminal_id).await
    }

    /// Focus the Nth pane, one-based.
    pub async fn layout_focus_index(
        &self,
        workspace: Uuid,
        group: Option<&str>,
        index: usize,
    ) -> Result<Vec<LayoutView>> {
        let view = self.resolve(workspace, group).await?;
        let Some(pane) = index.checked_sub(1).and_then(|i| view.panes.get(i)) else {
            return Err(DomainError::InvalidArgument { what: "pane index" });
        };
        self.layout_focus(workspace, pane.terminal_id).await
    }

    /// Zoom a pane, or clear the zoom. `None` toggles the focused one.
    pub async fn layout_zoom(
        &self,
        workspace: Uuid,
        group: Option<&str>,
        terminal: Option<Uuid>,
        off: bool,
    ) -> Result<Vec<LayoutView>> {
        let view = self.resolve(workspace, group).await?;
        if off {
            self.tmux.unzoom(&view.window.window_id).await?;
            return self.layout(workspace).await;
        }
        match terminal {
            Some(id) => {
                let pane = self.pane_of(id).await?;
                // Focus first: a zoomed pane you cannot type in is a screenshot.
                self.tmux.select_pane(&pane.pane_id).await?;
                // Naming a pane that is already zoomed means "zoom THIS one", not
                // "turn zoom off" — otherwise asking twice undoes itself.
                if !pane.zoomed {
                    self.tmux.toggle_zoom(&pane.pane_id).await?;
                }
            }
            None => {
                let pane = view.focused().ok_or(DomainError::NotFound)?;
                self.tmux.toggle_zoom(&pane.pane_id).await?;
            }
        }
        self.layout(workspace).await
    }

    /// Exchange two panes' positions.
    pub async fn layout_swap(&self, workspace: Uuid, a: Uuid, b: Uuid) -> Result<Vec<LayoutView>> {
        let first = self.pane_of(a).await?;
        let second = self.pane_of(b).await?;
        self.tmux.swap_panes(&first.pane_id, &second.pane_id).await?;
        self.layout(workspace).await
    }

    /// Move a divider, in cells.
    pub async fn layout_resize(
        &self,
        workspace: Uuid,
        terminal: Uuid,
        side: SplitSide,
        cells: i32,
    ) -> Result<Vec<LayoutView>> {
        let pane = self.pane_of(terminal).await?;
        let (axis, _) = split_args(side);
        self.tmux.resize_pane(&pane.pane_id, axis, cells).await?;
        self.layout(workspace).await
    }

    /// Pull a pane out into a layout of its own.
    pub async fn layout_break(&self, workspace: Uuid, terminal: Uuid) -> Result<Vec<LayoutView>> {
        let pane = self.pane_of(terminal).await?;
        self.tmux.break_pane(&pane.pane_id, workspace).await?;
        self.layout(workspace).await
    }

    /// Show the next or previous layout, wrapping.
    pub async fn layout_group_step(&self, workspace: Uuid, delta: i64) -> Result<Vec<LayoutView>> {
        let layouts = self.layout(workspace).await?;
        if layouts.len() < 2 {
            return Ok(layouts);
        }
        let count = layouts.len() as i64;
        let at = layouts.iter().position(|l| l.window.active).unwrap_or(0) as i64;
        let next = ((at + delta) % count + count) % count;
        self.show(&layouts[next as usize]).await?;
        self.layout(workspace).await
    }

    /// Show a specific layout.
    pub async fn layout_group_select(
        &self,
        workspace: Uuid,
        group: &str,
    ) -> Result<Vec<LayoutView>> {
        let view = self.resolve(workspace, Some(group)).await?;
        self.show(&view).await?;
        self.layout(workspace).await
    }

    /// Put a layout on screen, keyboard included.
    ///
    /// Focus follows the layout, or the keyboard stays behind in the one you just
    /// left — which is the same fault as switching layouts and landing on the
    /// wrong pane, one level down.
    async fn show(&self, view: &LayoutView) -> Result<()> {
        self.tmux.select_window(&view.window.window_id).await?;
        if let Some(pane) = view.focused().or(view.panes.first()) {
            self.tmux.select_pane(&pane.pane_id).await?;
        }
        Ok(())
    }

    /// Name a layout, which is what a client shows in its tab.
    pub async fn layout_rename(
        &self,
        workspace: Uuid,
        group: Option<&str>,
        name: &str,
    ) -> Result<Vec<LayoutView>> {
        let view = self.resolve(workspace, group).await?;
        let trimmed: String = name.trim().chars().take(48).collect();
        if !trimmed.is_empty() {
            self.tmux.rename_layout(&view.window.window_id, &trimmed).await?;
        }
        self.layout(workspace).await
    }

    /// Size a layout to the viewport actually showing it.
    ///
    /// tmux lays out for the window's size, so a client with a 200-column view has
    /// to say so before asking where the panes are — otherwise it gets an
    /// arrangement computed for whatever size the window last had.
    pub async fn layout_resize_window(
        &self,
        workspace: Uuid,
        group: Option<&str>,
        columns: u32,
        rows: u32,
    ) -> Result<Vec<LayoutView>> {
        let view = self.resolve(workspace, group).await?;
        // A window smaller than this cannot hold a usable pane, and tmux refuses
        // sizes it cannot satisfy anyway.
        if columns >= 20 && rows >= 5 {
            self.tmux.resize_window(&view.window.window_id, columns, rows).await?;
        }
        self.layout(workspace).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmux_names_parse_and_round_trip() {
        for preset in [
            LayoutPreset::EvenHorizontal,
            LayoutPreset::EvenVertical,
            LayoutPreset::MainVertical,
            LayoutPreset::MainHorizontal,
            LayoutPreset::Tiled,
        ] {
            assert_eq!(parse_preset(preset_name(preset)), Some(preset), "{preset:?}");
        }
    }

    #[test]
    fn the_names_people_actually_type_also_work() {
        assert_eq!(parse_preset("grid"), Some(LayoutPreset::Tiled));
        assert_eq!(parse_preset("main"), Some(LayoutPreset::MainVertical));
        assert_eq!(parse_preset("COLUMNS"), Some(LayoutPreset::EvenHorizontal));
        assert_eq!(parse_preset("diagonal"), None);
    }

    #[test]
    fn four_drop_edges_are_two_axes_and_a_flag() {
        // Which is the whole reason an arbitrary directional drop needs no tree
        // of our own: tmux already expresses all four.
        assert_eq!(split_args(SplitSide::Left), (Axis::Horizontal, true));
        assert_eq!(split_args(SplitSide::Right), (Axis::Horizontal, false));
        assert_eq!(split_args(SplitSide::Top), (Axis::Vertical, true));
        assert_eq!(split_args(SplitSide::Bottom), (Axis::Vertical, false));
    }

    #[test]
    fn an_unsaid_side_splits_the_way_prefix_percent_does() {
        assert_eq!(split_args(SplitSide::Unspecified), (Axis::Horizontal, false));
    }
}
