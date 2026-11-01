//! Themes (ov-268): why a group of cards exists, beside the board.
//!
//! A **theme** has a name, an outcome, a story rewritten at checkpoints, what
//! happens next and what it asks of the owner. A card is in at most one theme.
//! What a theme's cards are doing is the line's (`ticket_lines.rs`); who works
//! them is `ticket_work.rs`; the plan read (`plan_read.rs`) draws them all.
//!
//! A theme's history is kept in `line_events`, never in `task_notes`: that
//! table is append-only by trigger, so a note kind written there could not be
//! taken out again with the layer.

use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use farcooler_core::{DomainError, Result};

use crate::error::map_err;
use crate::models::{Actor, get_uuid, uuid_blob};
use crate::store::Store;
use crate::tasks::now_millis;
use crate::ticket_lines::{LineSubject, line_event};

/// How long a name or a line may be. Generous for a person, and a ceiling for
/// a script that has lost its mind.
const NAME_MAX: usize = 60;
const LINE_MAX: usize = 300;
const STORY_MAX: usize = 4000;

/// Where a theme stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeState {
    /// Being worked.
    Active,
    /// Set aside on purpose.
    Paused,
    /// Its outcome holds.
    Done,
    /// Given up, and its name is free again.
    Dropped,
}

impl ThemeState {
    /// The stored word.
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeState::Active => "active",
            ThemeState::Paused => "paused",
            ThemeState::Done => "done",
            ThemeState::Dropped => "dropped",
        }
    }

    /// The state a stored word names.
    pub fn parse(raw: &str) -> Option<ThemeState> {
        Some(match raw {
            "active" => ThemeState::Active,
            "paused" => ThemeState::Paused,
            "done" => ThemeState::Done,
            "dropped" => ThemeState::Dropped,
            _ => return None,
        })
    }
}

/// One theme on one workspace's board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardTheme {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    /// One sentence: the world when it's done.
    pub outcome: String,
    /// Where it stands, rewritten at checkpoints; the old one is kept in
    /// `plan_events`.
    pub story: String,
    /// What happens next, in one line.
    pub next: String,
    /// What needs the owner, in one line; empty when nothing does.
    pub owner_ask: String,
    pub state: ThemeState,
    pub ordinal: i64,
    /// When `story` was last written, in ms; 0 if never.
    pub story_at: i64,
    pub created_at: i64,
    pub resource_version: u64,
}

/// A theme to create.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewTheme {
    pub name: String,
    pub outcome: String,
}

/// What `update_theme` changes; each `None` leaves a field alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeUpdate {
    pub name: Option<String>,
    pub outcome: Option<String>,
    pub story: Option<String>,
    pub next: Option<String>,
    /// An empty string clears it.
    pub owner_ask: Option<String>,
    pub state: Option<ThemeState>,
    pub ordinal: Option<i64>,
}

fn invalid(what: &'static str) -> DomainError {
    DomainError::InvalidArgument { what }
}

/// `value` trimmed, refused if it is longer than `max` characters, or empty
/// where `required`.
fn clean(value: &str, max: usize, required: bool, what: &'static str) -> Result<String> {
    let value = value.trim();
    if value.chars().count() > max || (required && value.is_empty()) {
        return Err(invalid(what));
    }
    Ok(value.to_string())
}

pub(crate) fn board_exists(conn: &Connection, workspace: Uuid) -> Result<()> {
    conn.query_row("SELECT 1 FROM workspaces WHERE id = ?1", params![uuid_blob(workspace)], |_| Ok(()))
        .optional()
        .map_err(map_err)?
        .ok_or(DomainError::NotFound)
}

/// Each card must exist and be on `workspace`'s own board: a theme belongs to
/// one board, as a task does.
pub(crate) fn check_cards(conn: &Connection, workspace: Uuid, tasks: &[Uuid]) -> Result<()> {
    for task in tasks {
        let board: Option<Vec<u8>> = conn
            .query_row("SELECT workspace_id FROM tasks WHERE id = ?1", params![uuid_blob(*task)], |r| r.get(0))
            .optional()
            .map_err(map_err)?;
        match board {
            None => return Err(DomainError::NotFound),
            Some(b) if b != uuid_blob(workspace) => return Err(invalid("other_board")),
            Some(_) => {}
        }
    }
    Ok(())
}

fn task_key(conn: &Connection, task: Uuid) -> String {
    conn.query_row("SELECT key FROM tasks WHERE id = ?1", params![uuid_blob(task)], |r| r.get(0))
        .unwrap_or_else(|_| "a card".to_string())
}

/// Drop rows that point at a card now on another board. A moved card leaves
/// its theme; a read already ignores those rows, and the next write on the
/// board removes them, so `task move` needs no hook.
pub(crate) fn prune_moved(tx: &Connection, workspace: Uuid) -> Result<()> {
    tx.execute(
        "DELETE FROM board_theme_tasks
          WHERE theme_id IN (SELECT id FROM board_themes WHERE workspace_id = ?1)
            AND task_id NOT IN (SELECT id FROM tasks WHERE workspace_id = ?1)",
        params![uuid_blob(workspace)],
    )
    .map_err(map_err)?;
    Ok(())
}

/// One entry in a theme's timeline.
fn event(conn: &Connection, theme: Uuid, kind: &str, actor: Actor, body: &str, extra: serde_json::Value) -> Result<()> {
    line_event(conn, LineSubject::Theme(theme), kind, actor, body, extra)
}

pub(crate) const THEME_COLS: &str =
    "id, workspace_id, name, outcome, story, next, owner_ask, state, ordinal, story_at, created_at, resource_version";

pub(crate) fn row_to_theme(r: &rusqlite::Row) -> rusqlite::Result<BoardTheme> {
    let state: String = r.get(7)?;
    Ok(BoardTheme {
        id: get_uuid(r, 0)?,
        workspace_id: get_uuid(r, 1)?,
        name: r.get(2)?,
        outcome: r.get(3)?,
        story: r.get(4)?,
        next: r.get(5)?,
        owner_ask: r.get(6)?,
        state: ThemeState::parse(&state).unwrap_or(ThemeState::Active),
        ordinal: r.get(8)?,
        story_at: r.get(9)?,
        created_at: r.get(10)?,
        resource_version: r.get::<_, i64>(11)?.max(0) as u64,
    })
}

fn theme_in(conn: &Connection, id: Uuid) -> Result<BoardTheme> {
    conn.query_row(&format!("SELECT {THEME_COLS} FROM board_themes WHERE id = ?1"), params![uuid_blob(id)], row_to_theme)
        .optional()
        .map_err(map_err)?
        .ok_or(DomainError::NotFound)
}

fn theme_name_free(conn: &Connection, workspace: Uuid, name: &str, except: Option<Uuid>) -> Result<()> {
    let taken: bool = conn
        .query_row(
            "SELECT 1 FROM board_themes WHERE workspace_id = ?1 AND name = ?2 COLLATE NOCASE
                AND state <> 'dropped' AND id <> ?3",
            params![uuid_blob(workspace), name, uuid_blob(except.unwrap_or_else(Uuid::nil))],
            |_| Ok(()),
        )
        .optional()
        .map_err(map_err)?
        .is_some();
    if taken { Err(invalid("name_taken")) } else { Ok(()) }
}

fn ordinal_word(n: u32) -> String {
    match n {
        1 => "next up".to_string(),
        2 => "2nd".to_string(),
        3 => "3rd".to_string(),
        n => format!("{n}th"),
    }
}

impl Store {
    /// Make a theme on `workspace`'s board, with `tasks` in it.
    ///
    /// A card already in another theme moves here: a card is in one theme.
    pub fn create_theme(&self, workspace: Uuid, new: &NewTheme, tasks: &[Uuid], actor: Actor) -> Result<BoardTheme> {
        let name = clean(&new.name, NAME_MAX, true, "name")?;
        let outcome = clean(&new.outcome, LINE_MAX, false, "outcome")?;
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        board_exists(&tx, workspace)?;
        check_cards(&tx, workspace, tasks)?;
        theme_name_free(&tx, workspace, &name, None)?;
        prune_moved(&tx, workspace)?;
        let id = Uuid::now_v7();
        let now = now_millis();
        let ordinal: i64 = tx
            .query_row(
                "SELECT coalesce(max(ordinal), 0) + 1 FROM board_themes WHERE workspace_id = ?1",
                params![uuid_blob(workspace)],
                |r| r.get(0),
            )
            .map_err(map_err)?;
        tx.execute(
            "INSERT INTO board_themes (id, workspace_id, name, outcome, state, ordinal, created_at, resource_version)
             VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?6, 1)",
            params![uuid_blob(id), uuid_blob(workspace), name, outcome, ordinal, now],
        )
        .map_err(map_err)?;
        event(&tx, id, "state", actor, "Created.", serde_json::json!({}))?;
        add_theme_cards(&tx, id, tasks, actor)?;
        let theme = theme_in(&tx, id)?;
        tx.commit().map_err(map_err)?;
        Ok(theme)
    }

    /// Change a theme. Writing `story` keeps the previous one as a `story`
    /// event, so "what changed since I last looked" has both halves.
    pub fn update_theme(&self, theme: Uuid, update: &ThemeUpdate, actor: Actor) -> Result<BoardTheme> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let before = theme_in(&tx, theme)?;
        let name = match &update.name {
            Some(n) => clean(n, NAME_MAX, true, "name")?,
            None => before.name.clone(),
        };
        let pick = |new: &Option<String>, old: &str, max: usize, what: &'static str| match new {
            Some(v) => clean(v, max, false, what),
            None => Ok(old.to_string()),
        };
        let outcome = pick(&update.outcome, &before.outcome, LINE_MAX, "outcome")?;
        let story = pick(&update.story, &before.story, STORY_MAX, "story")?;
        let next = pick(&update.next, &before.next, LINE_MAX, "next")?;
        let ask = pick(&update.owner_ask, &before.owner_ask, LINE_MAX, "owner_ask")?;
        let state = update.state.unwrap_or(before.state);
        if (name != before.name || state != before.state) && state != ThemeState::Dropped {
            theme_name_free(&tx, before.workspace_id, &name, Some(theme))?;
        }
        let story_changed = story != before.story;
        let now = now_millis();
        tx.execute(
            "UPDATE board_themes SET name = ?2, outcome = ?3, story = ?4, next = ?5, owner_ask = ?6, state = ?7,
                    ordinal = ?8, story_at = ?9, resource_version = resource_version + 1
              WHERE id = ?1",
            params![
                uuid_blob(theme),
                name,
                outcome,
                story,
                next,
                ask,
                state.as_str(),
                update.ordinal.unwrap_or(before.ordinal),
                if story_changed { now } else { before.story_at },
            ],
        )
        .map_err(map_err)?;
        if story_changed {
            event(&tx, theme, "story", actor, &before.story, serde_json::json!({ "to": story }))?;
        }
        if state != before.state {
            let said = format!("Moved to {}.", state.as_str());
            event(&tx, theme, "state", actor, &said, serde_json::json!({ "from": before.state.as_str() }))?;
        }
        let after = theme_in(&tx, theme)?;
        tx.commit().map_err(map_err)?;
        Ok(after)
    }

    /// Put `theme` at place `to` of its workspace's ranking, 1 being first:
    /// the themes still in the plan are renumbered 1, 2, 3 in the new order,
    /// all in one transaction, and one `rank` event names `actor` when the
    /// theme's place changed. A place off the ranking is `place`.
    pub fn move_theme(&self, theme: Uuid, to: u32, actor: Actor) -> Result<BoardTheme> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let moving = theme_in(&tx, theme)?;
        let mut order: Vec<(Uuid, i64)> = {
            let mut stmt = tx
                .prepare(
                    "SELECT id, ordinal FROM board_themes WHERE workspace_id = ?1 AND state <> 'dropped'
                      ORDER BY ordinal, created_at, id",
                )
                .map_err(map_err)?;
            let rows = stmt
                .query_map(params![uuid_blob(moving.workspace_id)], |r| Ok((get_uuid(r, 0)?, r.get::<_, i64>(1)?)))
                .map_err(map_err)?;
            rows.collect::<rusqlite::Result<_>>().map_err(map_err)?
        };
        let was = order.iter().position(|(id, _)| *id == theme).ok_or_else(|| invalid("place"))?;
        if to == 0 || to as usize > order.len() {
            return Err(invalid("place"));
        }
        let held = order.remove(was);
        order.insert(to as usize - 1, held);
        for (i, (id, ordinal)) in order.iter().enumerate() {
            let rank = i as i64 + 1;
            if *ordinal != rank {
                tx.execute(
                    "UPDATE board_themes SET ordinal = ?2, resource_version = resource_version + 1 WHERE id = ?1",
                    params![uuid_blob(*id), rank],
                )
                .map_err(map_err)?;
            }
        }
        if was + 1 != to as usize {
            let said = format!("Moved to {}.", ordinal_word(to));
            event(&tx, theme, "rank", actor, &said, serde_json::json!({ "from": was + 1, "to": to }))?;
        }
        let after = theme_in(&tx, theme)?;
        tx.commit().map_err(map_err)?;
        Ok(after)
    }

    /// Add cards to a theme and take others out. An added card in another
    /// theme moves; a removed card that isn't in this one is skipped.
    pub fn theme_cards(&self, theme: Uuid, add: &[Uuid], remove: &[Uuid], actor: Actor) -> Result<BoardTheme> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let found = theme_in(&tx, theme)?;
        check_cards(&tx, found.workspace_id, add)?;
        prune_moved(&tx, found.workspace_id)?;
        add_theme_cards(&tx, theme, add, actor)?;
        for task in remove {
            let gone = tx
                .execute(
                    "DELETE FROM board_theme_tasks WHERE task_id = ?1 AND theme_id = ?2",
                    params![uuid_blob(*task), uuid_blob(theme)],
                )
                .map_err(map_err)?;
            if gone > 0 {
                let said = format!("Removed {}.", task_key(&tx, *task));
                event(&tx, theme, "cards", actor, &said, serde_json::json!({}))?;
            }
        }
        tx.execute("UPDATE board_themes SET resource_version = resource_version + 1 WHERE id = ?1", params![uuid_blob(theme)])
            .map_err(map_err)?;
        let after = theme_in(&tx, theme)?;
        tx.commit().map_err(map_err)?;
        Ok(after)
    }

    /// A theme by id.
    pub fn board_theme(&self, theme: Uuid) -> Result<BoardTheme> {
        theme_in(&self.conn(), theme)
    }
}

fn add_theme_cards(tx: &Connection, theme: Uuid, tasks: &[Uuid], actor: Actor) -> Result<()> {
    for task in tasks {
        let from: Option<Vec<u8>> = tx
            .query_row("SELECT theme_id FROM board_theme_tasks WHERE task_id = ?1", params![uuid_blob(*task)], |r| r.get(0))
            .optional()
            .map_err(map_err)?;
        if from.as_ref().is_some_and(|id| *id == uuid_blob(theme)) {
            continue;
        }
        tx.execute(
            "INSERT INTO board_theme_tasks (task_id, theme_id, added_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(task_id) DO UPDATE SET theme_id = excluded.theme_id, added_at = excluded.added_at",
            params![uuid_blob(*task), uuid_blob(theme), now_millis()],
        )
        .map_err(map_err)?;
        let key = task_key(tx, *task);
        event(tx, theme, "cards", actor, &format!("Added {key}."), serde_json::json!({}))?;
        if let Some(old) = from.and_then(|b| Uuid::from_slice(&b).ok()) {
            let to: String = tx
                .query_row("SELECT name FROM board_themes WHERE id = ?1", params![uuid_blob(theme)], |r| r.get(0))
                .unwrap_or_default();
            event(tx, old, "cards", actor, &format!("Removed {key}: it moved to {to}."), serde_json::json!({}))?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
