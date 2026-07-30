//! Durable tiling: which terminals belong on screen together, and how.
//!
//! This file is deliberately dumb. It reads and writes rows; every rule about
//! what a layout *means* — that zoom follows focus, that dropping the focused
//! pane has to move focus somewhere, that a group with no members is worth
//! deleting — lives in the daemon's service layer, because those rules are
//! policy and policy that lives in SQL cannot be tested against a fleet.
//!
//! The one invariant enforced here rather than above is membership exclusivity,
//! and only because the schema can state it: `pane_members.terminal_id` is the
//! primary key, so adding a pane to a second group moves it out of the first.
//! That is tmux's behaviour too, and it is the kind of rule that gets forgotten
//! at exactly one of the nine call sites if it is written in Rust.

use overnight_core::{DomainError, Result};
use overnight_protocol::v1::LayoutPreset;
use rusqlite::{Connection, OptionalExtension, Row, params};
use uuid::Uuid;

use crate::error::map_err;
use crate::models::{get_uuid, uuid_blob};
use crate::store::Store;

/// A set of terminals shown together. tmux's window, near enough.
#[derive(Debug, Clone, PartialEq)]
pub struct PaneGroup {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub preset: LayoutPreset,
    /// Fraction of the long axis for the main pane. Meaningless for the even
    /// and tiled layouts, which have no main pane, and stored anyway so that
    /// cycling away from `main-vertical` and back does not forget it.
    pub ratio: f64,
    pub zoomed: Option<Uuid>,
    pub focused: Option<Uuid>,
    pub active: bool,
    pub position: i64,
    /// Terminals in display order.
    pub members: Vec<Uuid>,
    pub resource_version: u64,
}

/// The default main-pane share.
///
/// Golden-ish. A 50/50 split of a main-vertical layout is just even-horizontal
/// with extra steps, and the point of having a main pane is that it is the one
/// you are reading.
pub const DEFAULT_RATIO: f64 = 0.62;

const COLUMNS: &str = "id, workspace_id, name, preset, ratio, zoomed, focused, \
                       active, position, resource_version";

fn row_to_group(row: &Row) -> rusqlite::Result<PaneGroup> {
    Ok(PaneGroup {
        id: get_uuid(row, 0)?,
        workspace_id: get_uuid(row, 1)?,
        name: row.get(2)?,
        preset: LayoutPreset::try_from(row.get::<_, i32>(3)?).unwrap_or(LayoutPreset::Tiled),
        ratio: row.get(4)?,
        zoomed: optional_uuid(row, 5)?,
        focused: optional_uuid(row, 6)?,
        active: row.get(7)?,
        position: row.get(8)?,
        members: Vec::new(),
        resource_version: row.get::<_, i64>(9)? as u64,
    })
}

fn optional_uuid(row: &Row, idx: usize) -> rusqlite::Result<Option<Uuid>> {
    let bytes: Option<Vec<u8>> = row.get(idx)?;
    Ok(bytes.and_then(|b| Uuid::from_slice(&b).ok()))
}

fn members_of(conn: &Connection, group: Uuid) -> Result<Vec<Uuid>> {
    let mut stmt = conn
        .prepare("SELECT terminal_id FROM pane_members WHERE group_id = ?1 ORDER BY position")
        .map_err(map_err)?;
    let rows = stmt
        .query_map(params![uuid_blob(group)], |r| get_uuid(r, 0))
        .map_err(map_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_err)?;
    Ok(rows)
}

impl Store {
    /// Every group in a workspace, in display order, members included.
    pub fn pane_groups(&self, workspace: Uuid) -> Result<Vec<PaneGroup>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {COLUMNS} FROM pane_groups WHERE workspace_id = ?1 ORDER BY position"
            ))
            .map_err(map_err)?;
        let mut groups = stmt
            .query_map(params![uuid_blob(workspace)], row_to_group)
            .map_err(map_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_err)?;
        drop(stmt);
        for group in &mut groups {
            group.members = members_of(&conn, group.id)?;
        }
        Ok(groups)
    }

    pub fn pane_group(&self, id: Uuid) -> Result<PaneGroup> {
        let conn = self.conn();
        let mut group: PaneGroup = conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM pane_groups WHERE id = ?1"),
                params![uuid_blob(id)],
                row_to_group,
            )
            .optional()
            .map_err(map_err)?
            .ok_or(DomainError::NotFound)?;
        group.members = members_of(&conn, id)?;
        Ok(group)
    }

    /// The group a workspace is currently showing, if it has one.
    pub fn active_pane_group(&self, workspace: Uuid) -> Result<Option<PaneGroup>> {
        Ok(self.pane_groups(workspace)?.into_iter().find(|g| g.active))
    }

    /// Which group a terminal is in, if any.
    ///
    /// `None` is the ordinary answer: most terminals are backgrounded, which is
    /// what makes tiling opt-in rather than something that happens to you.
    pub fn pane_group_of(&self, terminal: Uuid) -> Result<Option<Uuid>> {
        self.conn()
            .query_row(
                "SELECT group_id FROM pane_members WHERE terminal_id = ?1",
                params![uuid_blob(terminal)],
                |r| get_uuid(r, 0),
            )
            .optional()
            .map_err(map_err)
    }

    /// Add a group to a workspace and make it the one on screen.
    ///
    /// Creating a group is always an act of wanting to look at it — nobody makes
    /// a layout to leave it behind — so activation is not a separate call.
    pub fn create_pane_group(
        &self,
        id: Uuid,
        workspace: Uuid,
        name: &str,
        preset: LayoutPreset,
    ) -> Result<PaneGroup> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let next: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(position), -1) + 1 FROM pane_groups WHERE workspace_id = ?1",
                params![uuid_blob(workspace)],
                |r| r.get(0),
            )
            .map_err(map_err)?;
        tx.execute(
            "UPDATE pane_groups SET active = 0, resource_version = resource_version + 1
             WHERE workspace_id = ?1 AND active = 1",
            params![uuid_blob(workspace)],
        )
        .map_err(map_err)?;
        tx.execute(
            "INSERT INTO pane_groups
               (id, workspace_id, name, preset, ratio, zoomed, focused, active, position,
                resource_version)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, 1, ?6, 1)",
            params![
                uuid_blob(id),
                uuid_blob(workspace),
                name,
                preset as i32,
                DEFAULT_RATIO,
                next
            ],
        )
        .map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        drop(conn);
        self.pane_group(id)
    }

    /// Write back the settable fields. Members are set separately.
    pub fn save_pane_group(&self, group: &PaneGroup) -> Result<PaneGroup> {
        let changed = self
            .conn()
            .execute(
                "UPDATE pane_groups
                   SET name = ?1, preset = ?2, ratio = ?3, zoomed = ?4, focused = ?5,
                       resource_version = resource_version + 1
                 WHERE id = ?6",
                params![
                    group.name,
                    group.preset as i32,
                    group.ratio,
                    group.zoomed.map(uuid_blob),
                    group.focused.map(uuid_blob),
                    uuid_blob(group.id),
                ],
            )
            .map_err(map_err)?;
        if changed == 0 {
            return Err(DomainError::NotFound);
        }
        self.pane_group(group.id)
    }

    /// Replace a group's membership wholesale, in the order given.
    ///
    /// Wholesale rather than incrementally because every caller — tile, add,
    /// drop, swap, reorder — already knows the list it wants, and an
    /// incremental API would make each of them re-derive it. Duplicates in
    /// `terminals` collapse to the first occurrence.
    pub fn set_pane_members(&self, group: Uuid, terminals: &[Uuid]) -> Result<PaneGroup> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        tx.execute("DELETE FROM pane_members WHERE group_id = ?1", params![uuid_blob(group)])
            .map_err(map_err)?;

        let mut seen = std::collections::HashSet::new();
        let mut position = 0i64;
        for terminal in terminals {
            if !seen.insert(*terminal) {
                continue;
            }
            // Upsert on the terminal, not the pair: a pane joining this group
            // leaves whichever group it was in, which is why the primary key is
            // the terminal.
            tx.execute(
                "INSERT INTO pane_members (terminal_id, group_id, position) VALUES (?1, ?2, ?3)
                 ON CONFLICT(terminal_id) DO UPDATE
                   SET group_id = excluded.group_id, position = excluded.position",
                params![uuid_blob(*terminal), uuid_blob(group), position],
            )
            .map_err(map_err)?;
            position += 1;
        }
        tx.execute(
            "UPDATE pane_groups SET resource_version = resource_version + 1 WHERE id = ?1",
            params![uuid_blob(group)],
        )
        .map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        drop(conn);
        self.pane_group(group)
    }

    /// Take terminals out of whatever group holds them.
    pub fn drop_pane_members(&self, terminals: &[Uuid]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        for terminal in terminals {
            tx.execute(
                "UPDATE pane_groups SET resource_version = resource_version + 1
                 WHERE id = (SELECT group_id FROM pane_members WHERE terminal_id = ?1)",
                params![uuid_blob(*terminal)],
            )
            .map_err(map_err)?;
            tx.execute(
                "DELETE FROM pane_members WHERE terminal_id = ?1",
                params![uuid_blob(*terminal)],
            )
            .map_err(map_err)?;
            // A zoom or focus pointing at a pane that is no longer in the group
            // would render as an empty screen, so clear both here rather than
            // hoping the service noticed.
            tx.execute(
                "UPDATE pane_groups SET zoomed = NULL WHERE zoomed = ?1",
                params![uuid_blob(*terminal)],
            )
            .map_err(map_err)?;
            tx.execute(
                "UPDATE pane_groups SET focused = NULL WHERE focused = ?1",
                params![uuid_blob(*terminal)],
            )
            .map_err(map_err)?;
        }
        tx.commit().map_err(map_err)?;
        Ok(())
    }

    /// Make one group the one on screen, and the others not.
    pub fn activate_pane_group(&self, id: Uuid) -> Result<PaneGroup> {
        let workspace = self.pane_group(id)?.workspace_id;
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        tx.execute(
            "UPDATE pane_groups SET active = 0, resource_version = resource_version + 1
             WHERE workspace_id = ?1 AND active = 1 AND id != ?2",
            params![uuid_blob(workspace), uuid_blob(id)],
        )
        .map_err(map_err)?;
        tx.execute(
            "UPDATE pane_groups SET active = 1, resource_version = resource_version + 1
             WHERE id = ?1 AND active = 0",
            params![uuid_blob(id)],
        )
        .map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        drop(conn);
        self.pane_group(id)
    }

    /// Delete a group. Its members go back to being backgrounded, not killed.
    ///
    /// That distinction is the whole reason closing a layout is safe: un-tiling
    /// three agents must not be a way to accidentally stop three agents.
    pub fn delete_pane_group(&self, id: Uuid) -> Result<()> {
        let group = self.pane_group(id)?;
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        tx.execute("DELETE FROM pane_members WHERE group_id = ?1", params![uuid_blob(id)])
            .map_err(map_err)?;
        tx.execute("DELETE FROM pane_groups WHERE id = ?1", params![uuid_blob(id)])
            .map_err(map_err)?;
        // Something has to be on screen if anything is, so the neighbour takes
        // over rather than the workspace showing nothing.
        if group.active {
            tx.execute(
                "UPDATE pane_groups SET active = 1, resource_version = resource_version + 1
                 WHERE id = (SELECT id FROM pane_groups WHERE workspace_id = ?1
                             ORDER BY position LIMIT 1)",
                params![uuid_blob(group.workspace_id)],
            )
            .map_err(map_err)?;
        }
        tx.commit().map_err(map_err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use overnight_protocol::v1::TerminalIntent;

    use super::*;

    /// A workspace with `count` terminals, since nothing here is testable
    /// without panes to put in a group.
    fn fixture(count: usize) -> (Store, Uuid, Vec<Uuid>) {
        let store = Store::open_in_memory().unwrap();
        let host = Uuid::now_v7();
        let root = store.create_repository_root(host, "/repos", 0).unwrap();
        let repo = store.create_repository(host, root.id, "r", "/g", "origin").unwrap();
        let ws = store.create_workspace(repo.id, "task", "feature/x", "/wt").unwrap();
        let terminals = (0..count)
            .map(|i| {
                store
                    .create_terminal(
                        ws.id,
                        &format!("t{i}"),
                        "claude",
                        TerminalIntent::Running,
                        80,
                        24,
                    )
                    .unwrap()
                    .id
            })
            .collect();
        (store, ws.id, terminals)
    }

    #[test]
    fn a_new_group_is_the_active_one_and_the_old_one_is_not() {
        let (store, ws, _) = fixture(0);
        let first =
            store.create_pane_group(Uuid::now_v7(), ws, "1", LayoutPreset::Tiled).unwrap();
        assert!(first.active);

        let second =
            store.create_pane_group(Uuid::now_v7(), ws, "2", LayoutPreset::Tiled).unwrap();
        assert!(second.active);
        // Exactly one, or the client has to pick and two clients would pick
        // differently.
        let groups = store.pane_groups(ws).unwrap();
        assert_eq!(groups.iter().filter(|g| g.active).count(), 1);
        assert_eq!(store.active_pane_group(ws).unwrap().unwrap().id, second.id);
    }

    #[test]
    fn members_keep_the_order_they_were_given() {
        let (store, ws, terminals) = fixture(3);
        let group =
            store.create_pane_group(Uuid::now_v7(), ws, "1", LayoutPreset::Tiled).unwrap();
        let reversed: Vec<Uuid> = terminals.iter().rev().copied().collect();
        let saved = store.set_pane_members(group.id, &reversed).unwrap();
        assert_eq!(saved.members, reversed, "display order is the stored order");
    }

    #[test]
    fn a_pane_added_to_a_second_group_leaves_the_first() {
        // Enforced by the schema rather than by every caller remembering, which
        // is the whole reason the membership table is keyed by terminal.
        let (store, ws, terminals) = fixture(2);
        let a = store.create_pane_group(Uuid::now_v7(), ws, "a", LayoutPreset::Tiled).unwrap();
        let b = store.create_pane_group(Uuid::now_v7(), ws, "b", LayoutPreset::Tiled).unwrap();

        store.set_pane_members(a.id, &terminals).unwrap();
        store.set_pane_members(b.id, &terminals[..1]).unwrap();

        assert_eq!(store.pane_group(a.id).unwrap().members, terminals[1..].to_vec());
        assert_eq!(store.pane_group(b.id).unwrap().members, terminals[..1].to_vec());
        assert_eq!(store.pane_group_of(terminals[0]).unwrap(), Some(b.id));
    }

    #[test]
    fn duplicates_collapse_rather_than_producing_two_panes() {
        let (store, ws, terminals) = fixture(1);
        let group =
            store.create_pane_group(Uuid::now_v7(), ws, "1", LayoutPreset::Tiled).unwrap();
        let saved = store
            .set_pane_members(group.id, &[terminals[0], terminals[0], terminals[0]])
            .unwrap();
        assert_eq!(saved.members.len(), 1);
    }

    #[test]
    fn dropping_a_pane_clears_a_zoom_or_focus_pointing_at_it() {
        // A group still zoomed on a pane it no longer holds renders as a blank
        // screen, which reads as a crash rather than as a mistake.
        let (store, ws, terminals) = fixture(2);
        let group =
            store.create_pane_group(Uuid::now_v7(), ws, "1", LayoutPreset::Tiled).unwrap();
        let mut saved = store.set_pane_members(group.id, &terminals).unwrap();
        saved.zoomed = Some(terminals[0]);
        saved.focused = Some(terminals[0]);
        store.save_pane_group(&saved).unwrap();

        store.drop_pane_members(&[terminals[0]]).unwrap();
        let after = store.pane_group(group.id).unwrap();
        assert_eq!(after.members, vec![terminals[1]]);
        assert_eq!(after.zoomed, None);
        assert_eq!(after.focused, None);
    }

    #[test]
    fn deleting_a_group_leaves_its_terminals_alone() {
        // Un-tiling three agents must never be a way to stop three agents.
        let (store, ws, terminals) = fixture(2);
        let group =
            store.create_pane_group(Uuid::now_v7(), ws, "1", LayoutPreset::Tiled).unwrap();
        store.set_pane_members(group.id, &terminals).unwrap();

        store.delete_pane_group(group.id).unwrap();
        assert!(store.pane_groups(ws).unwrap().is_empty());
        for terminal in &terminals {
            assert!(store.get_terminal(*terminal).is_ok(), "the terminal must survive");
            assert_eq!(store.pane_group_of(*terminal).unwrap(), None);
        }
    }

    #[test]
    fn closing_the_active_group_promotes_a_neighbour() {
        let (store, ws, _) = fixture(0);
        let first =
            store.create_pane_group(Uuid::now_v7(), ws, "1", LayoutPreset::Tiled).unwrap();
        let second =
            store.create_pane_group(Uuid::now_v7(), ws, "2", LayoutPreset::Tiled).unwrap();
        assert!(second.active);

        store.delete_pane_group(second.id).unwrap();
        let remaining = store.active_pane_group(ws).unwrap().unwrap();
        assert_eq!(remaining.id, first.id, "something must be on screen if anything is");
    }

    #[test]
    fn removing_a_terminal_takes_its_membership_with_it() {
        // Through the foreign key, not through the caller remembering: a stale
        // membership row would put a dead pane in a layout forever.
        let (store, ws, terminals) = fixture(2);
        let group =
            store.create_pane_group(Uuid::now_v7(), ws, "1", LayoutPreset::Tiled).unwrap();
        store.set_pane_members(group.id, &terminals).unwrap();

        let record = store.get_terminal(terminals[0]).unwrap();
        store.delete_terminal(terminals[0], record.resource_version).unwrap();
        assert_eq!(store.pane_group(group.id).unwrap().members, terminals[1..].to_vec());
    }

    #[test]
    fn saving_bumps_the_version_so_a_stale_client_can_tell() {
        let (store, ws, _) = fixture(0);
        let group =
            store.create_pane_group(Uuid::now_v7(), ws, "1", LayoutPreset::Tiled).unwrap();
        let mut edited = group.clone();
        edited.preset = LayoutPreset::MainVertical;
        let saved = store.save_pane_group(&edited).unwrap();
        assert_eq!(saved.preset, LayoutPreset::MainVertical);
        assert!(saved.resource_version > group.resource_version);
    }

    #[test]
    fn a_group_from_another_workspace_is_not_found_by_accident() {
        let (store, _, _) = fixture(0);
        assert!(matches!(store.pane_group(Uuid::now_v7()), Err(DomainError::NotFound)));
    }
}
