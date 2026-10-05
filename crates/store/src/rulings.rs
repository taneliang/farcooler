//! Decided for you (ov-304): the reversible calls the orchestrator makes on
//! the owner's behalf, kept on the plan layer.
//!
//! A **ruling** is one such call: what was decided in a line, why in one or
//! two, what reversing it costs, the cards and the theme it touches, and
//! where it stands. It starts `standing`; the owner confirms it or has it
//! reversed by telling the orchestrator, which is the only writer. The apps
//! show rulings and offer to copy a reference ("ruling R-12: ..."), never an
//! edit.
//!
//! # Additive and removable, as the plan layer is
//!
//! Two tables of their own (migration 0026, `Older::Welcome`), pointing at
//! `workspaces` and `tasks` by cascade and at `board_themes` by `SET NULL`.
//! Nothing old gains a column, a trigger, a status or a note kind, and nothing
//! the board runs on names these tables (`scripts/plan-layer-lint.py` checks).
//! The plan read carries them (`plan_read.rs`), so the removal drill in
//! `plan_tests.rs` drops them with the rest of the layer.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

use farcooler_core::{DomainError, Result};

use crate::error::map_err;
use crate::models::{Actor, get_uuid, uuid_blob};
use crate::plan::{board_exists, check_cards};
use crate::plan_read::CardRef;
use crate::store::Store;
use crate::tasks::now_millis;

/// Two new tables that only this file touches.
///
/// `Older::Welcome`: a build from before them reads every table it knows as
/// it did. Their rows go with their workspace or task by cascade, and lose
/// their theme (`SET NULL`) if one is ever deleted, whichever build deletes
/// it. No trigger on an existing table, no new column on one.
pub(crate) fn migration_0026_rulings(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        CREATE TABLE board_rulings (
            id BLOB PRIMARY KEY NOT NULL,
            workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            -- The short id's number: R-12. Counts up per board, never reused.
            number INTEGER NOT NULL,
            decision TEXT NOT NULL,
            why TEXT NOT NULL,
            reversal TEXT NOT NULL,
            theme_id BLOB REFERENCES board_themes(id) ON DELETE SET NULL,
            state TEXT NOT NULL,
            -- The owner's words, or the orchestrator's, on the last move.
            note TEXT NOT NULL DEFAULT '',
            actor TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            settled_by TEXT,
            settled_at INTEGER,
            resource_version INTEGER NOT NULL,
            UNIQUE (workspace_id, number)
        );

        CREATE TABLE board_ruling_tasks (
            ruling_id BLOB NOT NULL REFERENCES board_rulings(id) ON DELETE CASCADE,
            task_id BLOB NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            PRIMARY KEY (ruling_id, task_id)
        );
        CREATE INDEX board_ruling_tasks_by_task ON board_ruling_tasks (task_id);
        "#,
    )
}

/// A decision is one line, the reason one or two, and the reversal cost one.
const DECISION_MAX: usize = 300;
const WHY_MAX: usize = 600;
const REVERSAL_MAX: usize = 300;
const NOTE_MAX: usize = 300;

/// Where a ruling stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RulingState {
    /// Decided, and holding until the owner says otherwise.
    Standing,
    /// The owner agreed.
    Confirmed,
    /// The owner had it undone.
    Reversed,
}

impl RulingState {
    /// The stored word.
    pub fn as_str(self) -> &'static str {
        match self {
            RulingState::Standing => "standing",
            RulingState::Confirmed => "confirmed",
            RulingState::Reversed => "reversed",
        }
    }

    /// The state a stored word names.
    pub fn parse(raw: &str) -> Option<RulingState> {
        Some(match raw {
            "standing" => RulingState::Standing,
            "confirmed" => RulingState::Confirmed,
            "reversed" => RulingState::Reversed,
            _ => return None,
        })
    }

    /// Standing goes to confirmed or reversed, and a confirmed ruling can
    /// still be reversed. A reversed one is finished, and nothing goes back to
    /// standing: a new call is a new ruling.
    pub fn can_move_to(self, to: RulingState) -> bool {
        use RulingState::*;
        matches!((self, to), (Standing, Confirmed) | (Standing, Reversed) | (Confirmed, Reversed))
    }
}

/// One ruling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ruling {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// R-`number`.
    pub number: u32,
    pub decision: String,
    pub why: String,
    pub reversal: String,
    /// The cards it touches, in the order given, on this board only.
    pub tasks: Vec<Uuid>,
    /// Their keys, in the same order. Carried on the ruling rather than in the
    /// plan's cards, which the reconciliation checks (review 1005a F2).
    pub task_keys: Vec<String>,
    pub theme_id: Option<Uuid>,
    pub state: RulingState,
    pub note: String,
    /// Who recorded it, in `task_notes.actor`'s words.
    pub actor: String,
    pub created_at: i64,
    /// Who confirmed or reversed it, and when; `None` while standing.
    pub settled_by: Option<String>,
    pub settled_at: Option<i64>,
    pub resource_version: u64,
}

impl Ruling {
    /// "R-12": how the owner names it to the orchestrator.
    pub fn short(&self) -> String {
        format!("R-{}", self.number)
    }
}

/// A ruling to record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewRuling {
    pub decision: String,
    pub why: String,
    pub reversal: String,
    pub theme_id: Option<Uuid>,
}

fn invalid(what: &'static str) -> DomainError {
    DomainError::InvalidArgument { what }
}

/// `value` trimmed, refused empty or longer than `max` characters.
fn required(value: &str, max: usize, what: &'static str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > max {
        return Err(invalid(what));
    }
    Ok(value.to_string())
}

const COLS: &str = "id, workspace_id, number, decision, why, reversal, theme_id, state, note, actor, created_at,
     settled_by, settled_at, resource_version";

fn row_to_ruling(r: &rusqlite::Row) -> rusqlite::Result<Ruling> {
    let theme: Option<Vec<u8>> = r.get(6)?;
    let state: String = r.get(7)?;
    Ok(Ruling {
        id: get_uuid(r, 0)?,
        workspace_id: get_uuid(r, 1)?,
        number: r.get::<_, i64>(2)?.max(0) as u32,
        decision: r.get(3)?,
        why: r.get(4)?,
        reversal: r.get(5)?,
        tasks: Vec::new(),
        task_keys: Vec::new(),
        theme_id: theme.and_then(|b| Uuid::from_slice(&b).ok()),
        state: RulingState::parse(&state).unwrap_or(RulingState::Standing),
        note: r.get(8)?,
        actor: r.get(9)?,
        created_at: r.get(10)?,
        settled_by: r.get(11)?,
        settled_at: r.get(12)?,
        resource_version: r.get::<_, i64>(13)?.max(0) as u64,
    })
}

/// A ruling's cards and their keys, in the order given.
fn tasks_of(conn: &Connection, ruling: Uuid) -> Result<Vec<(Uuid, String)>> {
    let mut stmt = conn
        .prepare(
            "SELECT r.task_id, t.key FROM board_ruling_tasks r JOIN tasks t ON t.id = r.task_id
              WHERE r.ruling_id = ?1 ORDER BY r.rowid",
        )
        .map_err(map_err)?;
    let rows = stmt
        .query_map(params![uuid_blob(ruling)], |r| Ok((get_uuid(r, 0)?, r.get(1)?)))
        .map_err(map_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_err)?;
    Ok(rows)
}

fn ruling_in(conn: &Connection, id: Uuid) -> Result<Ruling> {
    let mut ruling = conn
        .query_row(&format!("SELECT {COLS} FROM board_rulings WHERE id = ?1"), params![uuid_blob(id)], row_to_ruling)
        .optional()
        .map_err(map_err)?
        .ok_or(DomainError::NotFound)?;
    (ruling.tasks, ruling.task_keys) = tasks_of(conn, id)?.into_iter().unzip();
    Ok(ruling)
}

/// A theme named on a ruling must be on the ruling's board.
fn check_theme(conn: &Connection, workspace: Uuid, theme: Uuid) -> Result<()> {
    let board: Option<Vec<u8>> = conn
        .query_row("SELECT workspace_id FROM board_themes WHERE id = ?1", params![uuid_blob(theme)], |r| r.get(0))
        .optional()
        .map_err(map_err)?;
    match board {
        None => Err(DomainError::NotFound),
        Some(b) if b != uuid_blob(workspace) => Err(invalid("other_board")),
        Some(_) => Ok(()),
    }
}

impl Store {
    /// Record a ruling on `workspace`'s board, standing, touching `tasks`.
    /// It takes the board's next number.
    pub fn add_ruling(&self, workspace: Uuid, new: &NewRuling, tasks: &[Uuid], actor: Actor) -> Result<Ruling> {
        let decision = required(&new.decision, DECISION_MAX, "decision")?;
        let why = required(&new.why, WHY_MAX, "why")?;
        let reversal = required(&new.reversal, REVERSAL_MAX, "reversal")?;
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        board_exists(&tx, workspace)?;
        check_cards(&tx, workspace, tasks)?;
        if let Some(theme) = new.theme_id {
            check_theme(&tx, workspace, theme)?;
        }
        let number: i64 = tx
            .query_row(
                "SELECT coalesce(max(number), 0) + 1 FROM board_rulings WHERE workspace_id = ?1",
                params![uuid_blob(workspace)],
                |r| r.get(0),
            )
            .map_err(map_err)?;
        let id = Uuid::now_v7();
        tx.execute(
            "INSERT INTO board_rulings (id, workspace_id, number, decision, why, reversal, theme_id, state, actor,
                                        created_at, resource_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'standing', ?8, ?9, 1)",
            params![
                uuid_blob(id),
                uuid_blob(workspace),
                number,
                decision,
                why,
                reversal,
                new.theme_id.map(uuid_blob),
                actor.to_string(),
                now_millis(),
            ],
        )
        .map_err(map_err)?;
        for task in tasks {
            tx.execute(
                "INSERT OR IGNORE INTO board_ruling_tasks (ruling_id, task_id) VALUES (?1, ?2)",
                params![uuid_blob(id), uuid_blob(*task)],
            )
            .map_err(map_err)?;
        }
        let ruling = ruling_in(&tx, id)?;
        tx.commit().map_err(map_err)?;
        Ok(ruling)
    }

    /// Confirm or reverse a ruling, with the owner's words if there are any.
    pub fn set_ruling(&self, ruling: Uuid, state: RulingState, note: Option<&str>, actor: Actor) -> Result<Ruling> {
        let note = note.map(str::trim).unwrap_or_default();
        if note.chars().count() > NOTE_MAX {
            return Err(invalid("note"));
        }
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let before = ruling_in(&tx, ruling)?;
        if !before.state.can_move_to(state) {
            return Err(invalid("ruling_state"));
        }
        tx.execute(
            "UPDATE board_rulings SET state = ?2, note = ?3, settled_by = ?4, settled_at = ?5,
                    resource_version = resource_version + 1
              WHERE id = ?1",
            params![uuid_blob(ruling), state.as_str(), note, actor.to_string(), now_millis()],
        )
        .map_err(map_err)?;
        let after = ruling_in(&tx, ruling)?;
        tx.commit().map_err(map_err)?;
        Ok(after)
    }

    /// A ruling by id.
    pub fn ruling(&self, ruling: Uuid) -> Result<Ruling> {
        ruling_in(&self.conn(), ruling)
    }
}

/// A board's rulings for the plan read: every standing one, newest first, then
/// the ones settled at or after `settled_since_ms`, most recently settled
/// first. A card now on another board is left out of a ruling's cards.
pub(crate) fn rulings_of(
    conn: &Connection,
    workspace: Uuid,
    settled_since_ms: i64,
    on_board: &HashMap<Uuid, CardRef>,
) -> Result<Vec<Ruling>> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {COLS} FROM board_rulings
              WHERE workspace_id = ?1 AND (state = 'standing' OR coalesce(settled_at, 0) >= ?2)
              ORDER BY state <> 'standing', CASE WHEN state = 'standing' THEN created_at ELSE settled_at END DESC,
                       number DESC"
        ))
        .map_err(map_err)?;
    let mut rulings = stmt
        .query_map(params![uuid_blob(workspace), settled_since_ms], row_to_ruling)
        .map_err(map_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_err)?;
    for ruling in &mut rulings {
        (ruling.tasks, ruling.task_keys) =
            tasks_of(conn, ruling.id)?.into_iter().filter(|(t, _)| on_board.contains_key(t)).unzip();
    }
    Ok(rulings)
}

#[cfg(test)]
#[path = "rulings_tests.rs"]
mod tests;
