//! How a workspace lands its work (ov-313, ov-305 section 6.1): straight on its
//! base branch, or through pull requests, with the few numbers that go with it.
//!
//! # Additive and removable
//!
//! One new table (migration 0029, `Older::Welcome`), `workspace_landing`, one
//! row per workspace that has said anything, going with its workspace by
//! cascade. This is the workspace-settings pattern (`wake_on_answer`, the
//! `workspace.set_settings` write, the `Workspace` wire message) with one
//! difference: the setting lives in a table of its own rather than a column
//! of `workspaces`, so a build from before it reads `workspaces` as it always
//! did and a rollback keeps a working database. Saving a setting still moves
//! the workspace's `resource_version`, as every other setting does, so two
//! clients saving at once conflict rather than overwrite.
//!
//! A workspace with no row has chosen nothing: its mode is `None`, which an
//! app reads as "not chosen yet" and the runner's detection fills in as a
//! suggestion, never as a choice (`farcooler repo landing`).

use rusqlite::{OptionalExtension, Transaction, params};
use uuid::Uuid;

use farcooler_core::{DomainError, Result};

use crate::error::map_err;
use crate::models::{Workspace, uuid_blob};
use crate::store::Store;

/// How a workspace lands its work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LandingMode {
    /// Push to the base branch.
    Direct,
    /// Open a pull request per card and merge it there.
    PullRequests,
}

impl LandingMode {
    /// The stored word.
    pub fn as_str(self) -> &'static str {
        match self {
            LandingMode::Direct => "direct",
            LandingMode::PullRequests => "pull_requests",
        }
    }

    /// The mode a stored word names.
    pub fn parse(raw: &str) -> Option<LandingMode> {
        Some(match raw {
            "direct" => LandingMode::Direct,
            "pull_requests" => LandingMode::PullRequests,
            _ => return None,
        })
    }
}

/// What a workspace has said about landing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Landing {
    /// `None` until somebody chooses.
    pub mode: Option<LandingMode>,
    /// The branch work lands on, when it isn't the repository's default.
    pub base: Option<String>,
    /// How many changed lines a pull request should stay under, when said.
    pub budget_lines: Option<u32>,
    /// Whether a pull request's description carries a cost line. Off by
    /// default: cost is the owner's business until the owner opts in.
    pub pr_cost_line: bool,
}

/// What a `workspace.set_settings` changes: each field present is set, each
/// absent is left alone. An empty `base` and a `budget_lines` of 0 take the
/// value away.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LandingPatch {
    pub mode: Option<LandingMode>,
    pub base: Option<String>,
    pub budget_lines: Option<u32>,
    pub pr_cost_line: Option<bool>,
}

impl LandingPatch {
    /// Whether it changes nothing.
    pub fn is_empty(&self) -> bool {
        *self == LandingPatch::default()
    }
}

/// A branch name, as far as a setting needs one: not empty after trimming
/// (that is a clear), no spaces, no `..`, at most 255 bytes.
fn branch_ok(base: &str) -> bool {
    base.len() <= 255 && !base.contains(char::is_whitespace) && !base.contains("..") && !base.starts_with('-')
}

pub(crate) fn migration_0029_workspace_landing(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        CREATE TABLE workspace_landing (
            workspace_id BLOB PRIMARY KEY NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            -- `direct` or `pull_requests`; NULL until somebody chooses.
            mode TEXT,
            base TEXT,
            budget_lines INTEGER,
            pr_cost_line INTEGER NOT NULL DEFAULT 0
        );
        "#,
    )
}

impl Store {
    /// What `workspace` has said about landing. Nothing said is `Landing::default()`.
    pub fn get_landing(&self, workspace: Uuid) -> Result<Landing> {
        let conn = self.conn();
        read(&conn, workspace)
    }

    /// Set what `patch` names, moving the workspace's version as any setting does.
    ///
    /// Refused as `InvalidArgument` with `base` for a branch name that can't
    /// be one, before anything is written. `ResourceConflict` when
    /// `expected_version` isn't the workspace's, `NotFound` when it isn't there.
    pub fn set_landing(&self, id: Uuid, expected_version: u64, patch: &LandingPatch) -> Result<Workspace> {
        if let Some(base) = &patch.base
            && !base.trim().is_empty()
            && !branch_ok(base.trim())
        {
            return Err(DomainError::InvalidArgument { what: "base" });
        }
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let changed = tx
            .execute(
                "UPDATE workspaces SET resource_version = resource_version + 1
                  WHERE id = ?1 AND resource_version = ?2",
                params![uuid_blob(id), expected_version as i64],
            )
            .map_err(map_err)?;
        if changed != 1 {
            let exists: Option<i64> = tx
                .query_row("SELECT 1 FROM workspaces WHERE id = ?1", params![uuid_blob(id)], |r| r.get(0))
                .optional()
                .map_err(map_err)?;
            return Err(if exists.is_some() { DomainError::ResourceConflict } else { DomainError::NotFound });
        }
        let mut now = read(&tx, id)?;
        if let Some(mode) = patch.mode {
            now.mode = Some(mode);
        }
        if let Some(base) = &patch.base {
            now.base = Some(base.trim().to_string()).filter(|b| !b.is_empty());
        }
        if let Some(lines) = patch.budget_lines {
            now.budget_lines = Some(lines).filter(|n| *n > 0);
        }
        if let Some(on) = patch.pr_cost_line {
            now.pr_cost_line = on;
        }
        tx.execute(
            "INSERT INTO workspace_landing (workspace_id, mode, base, budget_lines, pr_cost_line)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(workspace_id) DO UPDATE SET mode = excluded.mode, base = excluded.base,
                 budget_lines = excluded.budget_lines, pr_cost_line = excluded.pr_cost_line",
            params![
                uuid_blob(id),
                now.mode.map(LandingMode::as_str),
                now.base,
                now.budget_lines.map(i64::from),
                now.pr_cost_line
            ],
        )
        .map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        drop(conn);
        self.get_workspace(id)
    }
}

fn read(conn: &rusqlite::Connection, workspace: Uuid) -> Result<Landing> {
    let row = conn
        .query_row(
            "SELECT mode, base, budget_lines, pr_cost_line FROM workspace_landing WHERE workspace_id = ?1",
            params![uuid_blob(workspace)],
            |r| {
                Ok(Landing {
                    mode: r.get::<_, Option<String>>(0)?.as_deref().and_then(LandingMode::parse),
                    base: r.get(1)?,
                    budget_lines: r.get::<_, Option<i64>>(2)?.map(|n| n as u32),
                    pr_cost_line: r.get(3)?,
                })
            },
        )
        .optional()
        .map_err(map_err)?;
    Ok(row.unwrap_or_default())
}

#[cfg(test)]
#[path = "landing_tests.rs"]
mod tests;
