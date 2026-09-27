//! Workspaces: workstreams that own a board and, once claimed, worktrees.
//!
//! A workspace is a name, a task prefix, a board, at most one live
//! orchestrator, and the worktrees it owns. Every repository has one called
//! Main, made with it and never deleted; the others are split off it. A
//! workspace lives in one repository.
//!
//! Three rules this module keeps:
//!
//! - **A prefix is unique on the runner, ignoring case.** A task keeps its key
//!   when it moves between workspaces, and a key has to resolve without naming
//!   one. Checked here so the refusal can say what it is
//!   (`task_prefix_taken`), and held by `workspaces_one_prefix` besides.
//! - **A claim sticks.** The first signal to say whose a worktree is wins;
//!   only an explicit assignment moves it afterwards.
//! - **One live orchestrator per workspace.** The row can say only that an
//!   orchestrator hasn't ended: intent neither stopped nor failed, and no
//!   exit observed. Whether its pane is really there is tmux's to say, which
//!   the store can't read, so a caller names the unended ones it found not
//!   running (`Vacated`), each at the version it read. Every seat is taken by
//!   one transaction that checks and writes together: `set_terminal_role_with`,
//!   `set_terminal_workspace_with` and `reseat_orchestrator`.
//!
//! Every refusal is a `DomainError::InvalidArgument` whose `what` names it,
//! so a caller can say each one in its own sentence:
//!
//! | `what` | when |
//! |---|---|
//! | `task_prefix` | not `^[a-z][a-z0-9]{0,7}$` after lowercasing |
//! | `task_prefix_taken` | another workspace holds it, in any case |
//! | `name` | an empty name |
//! | `main_workspace` | deleting Main |
//! | `workspace_not_empty` | deleting a workspace a task, worktree or terminal still names |
//! | `other_repository` | moving a task, or giving a worktree or terminal, to a workspace in another repository |
//! | `main_checkout` | assigning a repository's main checkout to a workspace other than Main |
//! | `orchestrator_taken` | a second live orchestrator for one workspace |
//! | `workspace` | making a terminal with no workspace an orchestrator |

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;
use uuid::Uuid;

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::TerminalIntent;

use crate::error::map_err;
use crate::models::{
    Actor, ClaimSource, NoteKind, Task, Terminal, TerminalRole, Workspace, Worktree, get_uuid,
    row_to_workspace, uuid_blob,
};
use crate::store::Store;
use crate::tasks::{derive_prefix, insert_note, now_millis};

/// Whether `prefix` is one a person may choose: a lowercase letter, then up to
/// seven lowercase letters or digits.
///
/// Short because a person types it and says it; a letter first so a key never
/// reads as a number or a flag. Callers lowercase first (`bil` and `BIL` are
/// one prefix), so this sees only what would be stored.
pub fn valid_prefix(prefix: &str) -> bool {
    let bytes = prefix.as_bytes();
    (1..=8).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes[1..].iter().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// A prefix as a person typed it, as it will be stored: trimmed and
/// lowercased, then held to `valid_prefix`.
fn chosen_prefix(raw: &str) -> Result<String> {
    let prefix = raw.trim().to_ascii_lowercase();
    if valid_prefix(&prefix) {
        Ok(prefix)
    } else {
        Err(DomainError::InvalidArgument { what: "task_prefix" })
    }
}

/// The workspace holding `prefix`, ignoring case, if one does.
fn prefix_holder(conn: &Connection, prefix: &str) -> rusqlite::Result<Option<Uuid>> {
    conn.query_row(
        "SELECT id FROM workspaces WHERE task_prefix = ?1 COLLATE NOCASE",
        params![prefix],
        |r| get_uuid(r, 0),
    )
    .optional()
}

/// `base`, or `base2`, `base3`, … : the first no workspace holds, ignoring
/// case. The rule migration 0012 used for repositories, over workspaces.
///
/// For a prefix nobody chose: Main's, derived from its repository's name, and
/// a held prefix migration 0015 carries over. Not held to `valid_prefix`, see
/// `derive_prefix`.
pub(crate) fn free_prefix(conn: &Connection, base: &str) -> rusqlite::Result<String> {
    let mut candidate = base.to_string();
    let mut attempt = 1u32;
    while prefix_holder(conn, &candidate)?.is_some() {
        attempt += 1;
        candidate = format!("{base}{attempt}");
    }
    Ok(candidate)
}

/// Main's prefix for a repository called `name`: `derive_prefix`, held to
/// `valid_prefix`, and free on the runner.
///
/// `derive_prefix` can start with a digit (`3tier-app` gives `3a`) or run past
/// eight characters (one initial per word), and a prefix a person could not
/// set back with `set_workspace_prefix` is not one to hand them. So leading
/// digits are dropped, the rest is cut to eight, and an empty result is `t`.
/// A taken one gets a digit, as `free_prefix` does, but cut short enough that
/// the digit still fits in eight.
fn main_prefix(conn: &Connection, name: &str) -> rusqlite::Result<String> {
    const MAX: usize = 8;
    let derived = derive_prefix(name);
    let mut base: String =
        derived.trim_start_matches(|c: char| c.is_ascii_digit()).chars().take(MAX).collect();
    if base.is_empty() {
        base = "t".to_string();
    }
    let mut candidate = base.clone();
    let mut attempt = 1u32;
    while prefix_holder(conn, &candidate)?.is_some() {
        attempt += 1;
        let suffix = attempt.to_string();
        let keep = MAX.saturating_sub(suffix.len()).max(1);
        candidate = format!("{}{suffix}", base.chars().take(keep).collect::<String>());
    }
    Ok(candidate)
}

/// Every column `row_to_workspace` reads, in its order.
const WORKSPACE_COLUMNS: &str =
    "id, repository_id, name, task_prefix, is_main, ordinal, resource_version";

/// A terminal is an unended orchestrator while its role says so and nothing
/// on the row says it has ended: intent neither stopped nor failed, and no
/// exit observed. The durable row is all a store can read; tmux is not
/// consulted, which is why a caller can name some of these `Vacated`.
const UNENDED_ORCHESTRATOR: &str =
    "role = 2 AND intent NOT IN (?2, ?3) AND exit_code IS NULL AND exit_signal IS NULL";

/// An unended orchestrator the caller found not running, as of the version
/// of its row it read: its pane is lost, or exited with the exit not yet on
/// the row. It doesn't hold its workspace's seat.
///
/// By version, because the finding is about that row. A row written since
/// may be the terminal coming back (`reseat_orchestrator` bumps it), and a
/// finding from before that must not seat a second orchestrator beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vacated {
    pub terminal: Uuid,
    pub resource_version: u64,
}

fn ended_intents() -> (i32, i32) {
    (TerminalIntent::Stopped as i32, TerminalIntent::Failed as i32)
}

/// The orchestrator other than `except` holding `workspace`'s seat, if one
/// does: unended, and not `vacated` at the version its row has now.
fn other_live_orchestrator(
    conn: &Connection,
    workspace: Uuid,
    except: Option<Uuid>,
    vacated: &[Vacated],
) -> rusqlite::Result<Option<Uuid>> {
    let (stopped, failed) = ended_intents();
    let mut stmt = conn.prepare(&format!(
        "SELECT id, resource_version FROM terminals
          WHERE workspace_id = ?1 AND {UNENDED_ORCHESTRATOR} AND (?4 IS NULL OR id != ?4)
          ORDER BY rowid"
    ))?;
    let rows = stmt.query_map(params![uuid_blob(workspace), stopped, failed, except.map(uuid_blob)], |r| {
        Ok(Vacated { terminal: get_uuid(r, 0)?, resource_version: r.get::<_, i64>(1)? as u64 })
    })?;
    for row in rows {
        let row = row?;
        if !vacated.contains(&row) {
            return Ok(Some(row.terminal));
        }
    }
    Ok(None)
}

/// The repository a workspace is in, or `NotFound`.
fn repository_of(conn: &Connection, workspace: Uuid) -> Result<Uuid> {
    conn.query_row(
        "SELECT repository_id FROM workspaces WHERE id = ?1",
        params![uuid_blob(workspace)],
        |r| get_uuid(r, 0),
    )
    .map_err(map_err)
}

/// Whether `workspace` is its repository's Main.
fn is_main(conn: &Connection, workspace: Uuid) -> Result<bool> {
    conn.query_row("SELECT is_main FROM workspaces WHERE id = ?1", params![uuid_blob(workspace)], |r| r.get(0))
        .map_err(map_err)
}

impl Store {
    // ---- the workspace row ----

    /// A new workspace in `repository`, after every one already there.
    ///
    /// The prefix is lowercased and must be `valid_prefix` and free on the
    /// runner. Its board starts empty; tasks arrive by `create_task` or
    /// `move_tasks`.
    pub fn create_workspace(&self, repository: Uuid, name: &str, task_prefix: &str) -> Result<Workspace> {
        let name = name.trim();
        if name.is_empty() {
            return Err(DomainError::InvalidArgument { what: "name" });
        }
        let prefix = chosen_prefix(task_prefix)?;
        let id = Uuid::now_v7();
        {
            let mut conn = self.conn();
            let tx = conn.transaction().map_err(map_err)?;
            // A missing repository would fail the foreign key, which reads
            // as a conflict; it is a missing repository.
            let exists: Option<i64> = tx
                .query_row("SELECT 1 FROM repositories WHERE id = ?1", params![uuid_blob(repository)], |r| {
                    r.get(0)
                })
                .optional()
                .map_err(map_err)?;
            if exists.is_none() {
                return Err(DomainError::NotFound);
            }
            if prefix_holder(&tx, &prefix).map_err(map_err)?.is_some() {
                return Err(DomainError::InvalidArgument { what: "task_prefix_taken" });
            }
            tx.execute(
                "INSERT INTO workspaces (id, repository_id, name, task_prefix, is_main, ordinal, created_at)
                 VALUES (?1, ?2, ?3, ?4, 0,
                         (SELECT COALESCE(MAX(ordinal), 0) + 1 FROM workspaces WHERE repository_id = ?2),
                         ?5)",
                params![uuid_blob(id), uuid_blob(repository), name, prefix, now_millis()],
            )
            .map_err(map_err)?;
            tx.commit().map_err(map_err)?;
        }
        self.get_workspace(id)
    }

    /// `repository`'s Main, made if it has none yet: named `Main`, with the
    /// prefix derived from the repository's name (`main_prefix`), or that
    /// prefix with a digit when another workspace holds it. Always one
    /// `valid_prefix` accepts.
    ///
    /// Idempotent, so registration can call it and so can anything that finds
    /// a repository without one. The derivation happens once, here, and is
    /// stored: renaming the repository later changes nothing.
    pub fn ensure_main_workspace(&self, repository: Uuid) -> Result<Workspace> {
        {
            let mut conn = self.conn();
            let tx = conn.transaction().map_err(map_err)?;
            let existing: Option<Uuid> = tx
                .query_row(
                    "SELECT id FROM workspaces WHERE repository_id = ?1 AND is_main = 1",
                    params![uuid_blob(repository)],
                    |r| get_uuid(r, 0),
                )
                .optional()
                .map_err(map_err)?;
            if existing.is_none() {
                let name: String = tx
                    .query_row(
                        "SELECT display_name FROM repositories WHERE id = ?1",
                        params![uuid_blob(repository)],
                        |r| r.get(0),
                    )
                    .map_err(map_err)?;
                let prefix = main_prefix(&tx, &name).map_err(map_err)?;
                tx.execute(
                    "INSERT INTO workspaces (id, repository_id, name, task_prefix, is_main, ordinal, created_at)
                     VALUES (?1, ?2, 'Main', ?3, 1, 0, ?4)",
                    params![uuid_blob(Uuid::now_v7()), uuid_blob(repository), prefix, now_millis()],
                )
                .map_err(map_err)?;
                tx.commit().map_err(map_err)?;
            }
        }
        self.main_workspace(repository)
    }

    pub fn get_workspace(&self, id: Uuid) -> Result<Workspace> {
        self.conn()
            .query_row(
                &format!("SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE id = ?1"),
                params![uuid_blob(id)],
                row_to_workspace,
            )
            .map_err(map_err)
    }

    /// `repository`'s Main, or `NotFound` if it has none yet (see
    /// `ensure_main_workspace`).
    pub fn main_workspace(&self, repository: Uuid) -> Result<Workspace> {
        self.conn()
            .query_row(
                &format!(
                    "SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE repository_id = ?1 AND is_main = 1"
                ),
                params![uuid_blob(repository)],
                row_to_workspace,
            )
            .map_err(map_err)
    }

    /// Every workspace in `repository`, or on the runner, in order: grouped
    /// by repository, then Main first and the rest as they were made.
    pub fn list_workspaces(&self, repository: Option<Uuid>) -> Result<Vec<Workspace>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {WORKSPACE_COLUMNS} FROM workspaces
                  WHERE ?1 IS NULL OR repository_id = ?1
                  ORDER BY repository_id, ordinal, rowid"
            ))
            .map_err(map_err)?;
        let rows = stmt.query_map(params![repository.map(uuid_blob)], row_to_workspace).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    pub fn rename_workspace(&self, id: Uuid, expected_version: u64, name: &str) -> Result<Workspace> {
        let name = name.trim();
        if name.is_empty() {
            return Err(DomainError::InvalidArgument { what: "name" });
        }
        self.run_versioned(
            "UPDATE workspaces SET name = ?1, resource_version = resource_version + 1
              WHERE id = ?2 AND resource_version = ?3",
            &[&name, &uuid_blob(id), &(expected_version as i64)],
            "SELECT 1 FROM workspaces WHERE id = ?1",
            &[&uuid_blob(id)],
        )?;
        self.get_workspace(id)
    }

    /// Change what new keys on this board start with.
    ///
    /// Only tasks created afterwards take it: keys are stored whole, so every
    /// key already issued keeps resolving. Setting the prefix a workspace
    /// already holds, in any case, is allowed and stores the new spelling.
    pub fn set_workspace_prefix(&self, id: Uuid, expected_version: u64, prefix: &str) -> Result<Workspace> {
        let prefix = chosen_prefix(prefix)?;
        {
            let mut conn = self.conn();
            let tx = conn.transaction().map_err(map_err)?;
            if let Some(holder) = prefix_holder(&tx, &prefix).map_err(map_err)?
                && holder != id
            {
                return Err(DomainError::InvalidArgument { what: "task_prefix_taken" });
            }
            let changed = tx
                .execute(
                    "UPDATE workspaces SET task_prefix = ?1, resource_version = resource_version + 1
                      WHERE id = ?2 AND resource_version = ?3",
                    params![prefix, uuid_blob(id), expected_version as i64],
                )
                .map_err(map_err)?;
            if changed != 1 {
                let exists: Option<i64> = tx
                    .query_row("SELECT 1 FROM workspaces WHERE id = ?1", params![uuid_blob(id)], |r| r.get(0))
                    .optional()
                    .map_err(map_err)?;
                return Err(match exists {
                    Some(_) => DomainError::ResourceConflict,
                    None => DomainError::NotFound,
                });
            }
            tx.commit().map_err(map_err)?;
        }
        self.get_workspace(id)
    }

    /// Delete a workspace that holds nothing.
    ///
    /// Refused for Main (`main_workspace`), and while any task, worktree or
    /// terminal still names it (`workspace_not_empty`): move them first. The
    /// schema refuses the same delete on its own (see migration 0015); this
    /// asks first so the refusal can say which it is.
    pub fn delete_workspace(&self, id: Uuid) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let is_main: bool = tx
            .query_row("SELECT is_main FROM workspaces WHERE id = ?1", params![uuid_blob(id)], |r| r.get(0))
            .map_err(map_err)?;
        if is_main {
            return Err(DomainError::InvalidArgument { what: "main_workspace" });
        }
        let holds: i64 = tx
            .query_row(
                "SELECT (SELECT count(*) FROM tasks WHERE workspace_id = ?1)
                      + (SELECT count(*) FROM worktrees WHERE workspace_id = ?1)
                      + (SELECT count(*) FROM terminals WHERE workspace_id = ?1)",
                params![uuid_blob(id)],
                |r| r.get(0),
            )
            .map_err(map_err)?;
        if holds > 0 {
            return Err(DomainError::InvalidArgument { what: "workspace_not_empty" });
        }
        tx.execute("DELETE FROM workspaces WHERE id = ?1", params![uuid_blob(id)]).map_err(|e| {
            match map_err(e) {
                // Only the foreign keys can refuse this delete.
                DomainError::ResourceConflict => DomainError::InvalidArgument { what: "workspace_not_empty" },
                other => other,
            }
        })?;
        tx.commit().map_err(map_err)
    }

    // ---- what a workspace holds ----

    /// Move tasks to another board in the same repository.
    ///
    /// All or nothing: a task that is missing (`NotFound`) or in another
    /// repository (`other_repository`) moves none of them. Each task keeps
    /// its key, and its record gets a `Comment` from `actor` saying where it
    /// went (`Moved from Main to Billing.`), with both workspace ids in
    /// `extra`. A task already on `to` is left alone and writes nothing.
    ///
    /// Returns the tasks as they now read, in the order given.
    pub fn move_tasks(&self, tasks: &[Uuid], to: Uuid, actor: Actor) -> Result<Vec<Task>> {
        {
            let mut conn = self.conn();
            let tx = conn.transaction().map_err(map_err)?;
            let (to_repository, to_name): (Uuid, String) = tx
                .query_row(
                    "SELECT repository_id, name FROM workspaces WHERE id = ?1",
                    params![uuid_blob(to)],
                    |r| Ok((get_uuid(r, 0)?, r.get(1)?)),
                )
                .map_err(map_err)?;
            for &task in tasks {
                let (repository, from, from_name): (Uuid, Uuid, String) = tx
                    .query_row(
                        "SELECT t.repository_id, t.workspace_id, w.name
                           FROM tasks t JOIN workspaces w ON w.id = t.workspace_id
                          WHERE t.id = ?1",
                        params![uuid_blob(task)],
                        |r| Ok((get_uuid(r, 0)?, get_uuid(r, 1)?, r.get(2)?)),
                    )
                    .map_err(map_err)?;
                if repository != to_repository {
                    return Err(DomainError::InvalidArgument { what: "other_repository" });
                }
                if from == to {
                    continue;
                }
                tx.execute(
                    "UPDATE tasks SET workspace_id = ?1, resource_version = resource_version + 1
                      WHERE id = ?2",
                    params![uuid_blob(to), uuid_blob(task)],
                )
                .map_err(map_err)?;
                insert_note(
                    &tx,
                    task,
                    NoteKind::Comment,
                    actor,
                    &format!("Moved from {from_name} to {to_name}."),
                    &json!({ "from_workspace": from.to_string(), "to_workspace": to.to_string() }),
                    None,
                )?;
            }
            tx.commit().map_err(map_err)?;
        }
        tasks.iter().map(|&task| self.get_task(task)).collect()
    }

    /// Claim an unclaimed worktree for `workspace`, saying which signal did.
    ///
    /// Sticky: a worktree that already has an owner is left alone and the
    /// answer is `None`, however strong this signal is. Only
    /// `assign_worktree` moves an existing claim. A main checkout is left
    /// alone the same way for any workspace but Main, which alone may
    /// claim it.
    ///
    /// Terminals in the worktree that have no workspace yet take this one;
    /// a terminal that already has one keeps it.
    pub fn claim_worktree(
        &self,
        worktree: Uuid,
        workspace: Uuid,
        source: ClaimSource,
    ) -> Result<Option<Worktree>> {
        {
            let mut conn = self.conn();
            let tx = conn.transaction().map_err(map_err)?;
            let (repository, owner, is_main_checkout): (Uuid, Option<Vec<u8>>, bool) = tx
                .query_row(
                    "SELECT repository_id, workspace_id, is_main_checkout FROM worktrees WHERE id = ?1",
                    params![uuid_blob(worktree)],
                    |r| Ok((get_uuid(r, 0)?, r.get(1)?, r.get(2)?)),
                )
                .map_err(map_err)?;
            if repository_of(&tx, workspace)? != repository {
                return Err(DomainError::InvalidArgument { what: "other_repository" });
            }
            if owner.is_some() || (is_main_checkout && !is_main(&tx, workspace)?) {
                return Ok(None);
            }
            give_worktree(&tx, worktree, workspace, source)?;
            tx.commit().map_err(map_err)?;
        }
        self.get_worktree(worktree).map(Some)
    }

    /// Give a worktree to `workspace` whoever owned it, as an explicit claim.
    /// The one way ownership moves once claimed.
    ///
    /// A repository's main checkout belongs to Main: giving it to any other
    /// workspace is refused (`main_checkout`). Giving it back to Main is not.
    pub fn assign_worktree(&self, worktree: Uuid, workspace: Uuid) -> Result<Worktree> {
        {
            let mut conn = self.conn();
            let tx = conn.transaction().map_err(map_err)?;
            let (repository, is_main_checkout): (Uuid, bool) = tx
                .query_row(
                    "SELECT repository_id, is_main_checkout FROM worktrees WHERE id = ?1",
                    params![uuid_blob(worktree)],
                    |r| Ok((get_uuid(r, 0)?, r.get(1)?)),
                )
                .map_err(map_err)?;
            if repository_of(&tx, workspace)? != repository {
                return Err(DomainError::InvalidArgument { what: "other_repository" });
            }
            if is_main_checkout && !is_main(&tx, workspace)? {
                return Err(DomainError::InvalidArgument { what: "main_checkout" });
            }
            give_worktree(&tx, worktree, workspace, ClaimSource::Explicit)?;
            tx.commit().map_err(map_err)?;
        }
        self.get_worktree(worktree)
    }

    /// `set_terminal_role_with`, counting every unended orchestrator as live.
    pub fn set_terminal_role(&self, terminal: Uuid, role: TerminalRole) -> Result<Terminal> {
        self.set_terminal_role_with(terminal, role, &[])
    }

    /// Make a terminal a shell, an agent, or its workspace's orchestrator.
    ///
    /// An orchestrator needs a workspace (`workspace`), and a workspace has at
    /// most one live (`orchestrator_taken`): any unended orchestrator but this
    /// one and the `vacated` holds the seat. Checked and written in one
    /// transaction.
    pub fn set_terminal_role_with(
        &self,
        terminal: Uuid,
        role: TerminalRole,
        vacated: &[Vacated],
    ) -> Result<Terminal> {
        {
            let mut conn = self.conn();
            let tx = conn.transaction().map_err(map_err)?;
            let workspace: Option<Vec<u8>> = tx
                .query_row(
                    "SELECT workspace_id FROM terminals WHERE id = ?1",
                    params![uuid_blob(terminal)],
                    |r| r.get(0),
                )
                .map_err(map_err)?;
            if role == TerminalRole::Orchestrator {
                let Some(workspace) = workspace else {
                    return Err(DomainError::InvalidArgument { what: "workspace" });
                };
                let workspace = Uuid::from_slice(&workspace).map_err(|_| DomainError::OperationFailed)?;
                if other_live_orchestrator(&tx, workspace, Some(terminal), vacated).map_err(map_err)?.is_some() {
                    return Err(DomainError::InvalidArgument { what: "orchestrator_taken" });
                }
            }
            tx.execute(
                "UPDATE terminals SET role = ?1, resource_version = resource_version + 1 WHERE id = ?2",
                params![role.as_i64(), uuid_blob(terminal)],
            )
            .map_err(map_err)?;
            tx.commit().map_err(map_err)?;
        }
        self.get_terminal(terminal)
    }

    /// `set_terminal_workspace_with`, counting every unended orchestrator as
    /// live.
    pub fn set_terminal_workspace(&self, terminal: Uuid, workspace: Uuid) -> Result<Terminal> {
        self.set_terminal_workspace_with(terminal, workspace, &[])
    }

    /// Say whose work a terminal is doing, whichever worktree it runs in.
    ///
    /// The workspace must be in the terminal's repository
    /// (`other_repository`), and an orchestrator cannot join a workspace that
    /// already has a live one (`orchestrator_taken`), with `vacated` read as
    /// in `set_terminal_role_with`.
    pub fn set_terminal_workspace_with(
        &self,
        terminal: Uuid,
        workspace: Uuid,
        vacated: &[Vacated],
    ) -> Result<Terminal> {
        {
            let mut conn = self.conn();
            let tx = conn.transaction().map_err(map_err)?;
            let (repository, role): (Uuid, i64) = tx
                .query_row(
                    "SELECT w.repository_id, t.role
                       FROM terminals t JOIN worktrees w ON w.id = t.worktree_id
                      WHERE t.id = ?1",
                    params![uuid_blob(terminal)],
                    |r| Ok((get_uuid(r, 0)?, r.get(1)?)),
                )
                .map_err(map_err)?;
            if repository_of(&tx, workspace)? != repository {
                return Err(DomainError::InvalidArgument { what: "other_repository" });
            }
            if TerminalRole::from_i64(role) == TerminalRole::Orchestrator
                && other_live_orchestrator(&tx, workspace, Some(terminal), vacated).map_err(map_err)?.is_some()
            {
                return Err(DomainError::InvalidArgument { what: "orchestrator_taken" });
            }
            tx.execute(
                "UPDATE terminals SET workspace_id = ?1, resource_version = resource_version + 1 WHERE id = ?2",
                params![uuid_blob(workspace), uuid_blob(terminal)],
            )
            .map_err(map_err)?;
            tx.commit().map_err(map_err)?;
        }
        self.get_terminal(terminal)
    }

    /// The workspace's orchestrators whose rows don't say they've ended,
    /// oldest first. Which of them is live is the caller's to decide against
    /// the panes; `Vacated` says so back.
    pub fn unended_orchestrators(&self, workspace: Uuid) -> Result<Vec<Terminal>> {
        let (stopped, failed) = ended_intents();
        let ids: Vec<Uuid> = {
            let conn = self.conn();
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT id FROM terminals
                      WHERE workspace_id = ?1 AND {UNENDED_ORCHESTRATOR} ORDER BY rowid"
                ))
                .map_err(map_err)?;
            stmt.query_map(params![uuid_blob(workspace), stopped, failed], |r| get_uuid(r, 0))
                .map_err(map_err)?
                .collect::<rusqlite::Result<_>>()
                .map_err(map_err)?
        };
        ids.into_iter().map(|id| self.get_terminal(id)).collect()
    }

    /// Bring an orchestrator back into its seat before its pane is restarted:
    /// intent running, not yet confirmed, no exit. Refused with
    /// `orchestrator_taken` if another holds the seat (`vacated` read as in
    /// `set_terminal_role_with`), and with `ResourceConflict` if the row isn't
    /// at `expected_version`. Checked and written in one transaction, so two
    /// restarts, or a restart and a start, can't both take the seat.
    ///
    /// Written before the pane, not after, so the seat is held while the pane
    /// comes up: an unconfirmed row is starting, and holds it. A terminal
    /// that isn't an orchestrator with a workspace has no seat to check, and
    /// is only written.
    pub fn reseat_orchestrator(
        &self,
        terminal: Uuid,
        expected_version: u64,
        vacated: &[Vacated],
    ) -> Result<Terminal> {
        {
            let mut conn = self.conn();
            let tx = conn.transaction().map_err(map_err)?;
            let (workspace, role, version): (Option<Vec<u8>>, i64, i64) = tx
                .query_row(
                    "SELECT workspace_id, role, resource_version FROM terminals WHERE id = ?1",
                    params![uuid_blob(terminal)],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .map_err(map_err)?;
            if version as u64 != expected_version {
                return Err(DomainError::ResourceConflict);
            }
            if TerminalRole::from_i64(role) == TerminalRole::Orchestrator
                && let Some(workspace) = workspace
            {
                let workspace = Uuid::from_slice(&workspace).map_err(|_| DomainError::OperationFailed)?;
                if other_live_orchestrator(&tx, workspace, Some(terminal), vacated).map_err(map_err)?.is_some() {
                    return Err(DomainError::InvalidArgument { what: "orchestrator_taken" });
                }
            }
            tx.execute(
                "UPDATE terminals SET intent = ?1, runtime_confirmed = 0, exit_code = NULL, exit_signal = NULL,
                        resource_version = resource_version + 1
                  WHERE id = ?2",
                params![TerminalIntent::Running as i32, uuid_blob(terminal)],
            )
            .map_err(map_err)?;
            tx.commit().map_err(map_err)?;
        }
        self.get_terminal(terminal)
    }
}

/// Write a claim, and hand the worktree's unowned terminals to the claimant.
fn give_worktree(conn: &Connection, worktree: Uuid, workspace: Uuid, source: ClaimSource) -> Result<()> {
    conn.execute(
        "UPDATE worktrees SET workspace_id = ?1, claim_source = ?2, resource_version = resource_version + 1
          WHERE id = ?3",
        params![uuid_blob(workspace), source.as_str(), uuid_blob(worktree)],
    )
    .map_err(map_err)?;
    conn.execute(
        "UPDATE terminals SET workspace_id = ?1, resource_version = resource_version + 1
          WHERE worktree_id = ?2 AND workspace_id IS NULL",
        params![uuid_blob(workspace), uuid_blob(worktree)],
    )
    .map_err(map_err)?;
    Ok(())
}

/// Fixtures other crates' tests need too, such as the daemon's claiming
/// tests: built under `cfg(test)` or the `testing` feature only.
#[cfg(any(test, feature = "testing"))]
impl Store {
    /// A worktree in `repository` at `path` that no workspace has claimed.
    pub fn create_unclaimed_worktree_for_test(&self, repository: Uuid, path: &str) -> Uuid {
        let worktree = self.create_worktree(repository, "branch", path, false).expect("worktree");
        assert_eq!(worktree.workspace_id, None, "a new worktree starts unclaimed");
        worktree.id
    }

    /// Give `worktree` to `workspace` as an explicit claim, past every rule
    /// `assign_worktree` holds it to: how a row reads that a runner from
    /// before the rule wrote, such as a main checkout outside Main.
    pub fn give_worktree_for_test(&self, worktree: Uuid, workspace: Uuid) {
        give_worktree(&self.conn(), worktree, workspace, ClaimSource::Explicit).expect("given");
    }

    /// A running `claude` terminal in `worktree`, doing `workspace`'s work.
    pub fn create_terminal_for_test(&self, worktree: Uuid, workspace: Uuid) -> Uuid {
        let terminal = self
            .create_terminal(worktree, "agent", "claude", TerminalIntent::Running, 80, 24)
            .expect("terminal");
        self.set_terminal_workspace(terminal.id, workspace).expect("its workspace").id
    }
}

#[cfg(test)]
impl Store {
    /// A task on `workspace` carrying a key and a former key the caller
    /// chooses, the way migration 0012 left renamed boards.
    pub(crate) fn insert_task_with_former_key_for_test(
        &self,
        workspace: Uuid,
        key: &str,
        former: &str,
    ) -> Uuid {
        let repository = self.get_workspace(workspace).expect("workspace").repository_id;
        let id = Uuid::now_v7();
        self.conn()
            .execute(
                "INSERT INTO tasks (id, repository_id, workspace_id, key, former_key, title, status,
                                    status_since, created_at, resource_version)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'renamed', 'backlog', 0, 0, 1)",
                params![uuid_blob(id), uuid_blob(repository), uuid_blob(workspace), key, former],
            )
            .expect("insert a renamed task");
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::tasks::TaskScope;

    fn refused(what: &'static str) -> impl Fn(&DomainError) -> bool {
        move |e| matches!(e, DomainError::InvalidArgument { what: w } if *w == what)
    }

    #[test]
    fn a_prefix_is_a_letter_then_up_to_seven_letters_or_digits() {
        for good in ["b", "bil", "b1", "abcdefgh", "x9y8"] {
            assert!(valid_prefix(good), "{good}");
        }
        for bad in ["", "1x", "Bil", "bi-l", "abcdefghi", "bil ", "é"] {
            assert!(!valid_prefix(bad), "{bad:?}");
        }
    }

    #[test]
    fn registering_a_repository_makes_its_main() {
        let store = Store::open_in_memory().unwrap();
        let repo = store.register_repository_for_test("billing service");
        let main = store.ensure_main_workspace(repo).unwrap();
        assert!(main.is_main);
        assert_eq!(main.name, "Main");
        assert_eq!(main.task_prefix, "bs");
        assert_eq!(main.ordinal, 0);
        assert_eq!(store.ensure_main_workspace(repo).unwrap().id, main.id, "idempotent");
        assert_eq!(store.main_workspace(repo).unwrap(), main);
        assert_eq!(store.list_workspaces(Some(repo)).unwrap(), vec![main]);
    }

    /// A prefix derived from a repository's name is one a person could have
    /// typed: a letter first, at most eight, and still that when a digit is
    /// added to make it free.
    #[test]
    fn a_derived_prefix_is_always_a_valid_one() {
        let store = Store::open_in_memory().unwrap();
        let digit = store.register_repository_for_test("3tier-app-with-a-long-name");
        let long = store.register_repository_for_test("a-b-c-d-e-f-g-h-i-j");
        let again = store.register_repository_for_test("a b c d e f g h i j k");
        let numeric = store.register_repository_for_test("2024");

        let digit = store.ensure_main_workspace(digit).unwrap().task_prefix;
        let long = store.ensure_main_workspace(long).unwrap().task_prefix;
        let again = store.ensure_main_workspace(again).unwrap().task_prefix;
        let numeric = store.ensure_main_workspace(numeric).unwrap().task_prefix;

        assert_eq!(digit, "awaln", "the leading digit is dropped");
        assert_eq!(long, "abcdefgh", "cut to eight");
        assert_eq!(again, "abcdefg2", "the digit that makes it free still fits in eight");
        assert_eq!(numeric, "t", "nothing but digits falls back");
        for prefix in [digit, long, again, numeric] {
            assert!(valid_prefix(&prefix), "{prefix:?} is not a prefix a person could set");
        }
    }

    #[test]
    fn prefixes_are_unique_on_the_runner_ignoring_case() {
        let store = Store::open_in_memory().unwrap();
        let repo = store.register_repository_for_test("r");
        let other = store.register_repository_for_test("o");
        store.ensure_main_workspace(repo).unwrap();
        let billing = store.create_workspace(repo, "Billing", "bil").unwrap();

        // Lowercased before anything else, so `Bil` is `bil`, which is taken:
        // refused as taken, not as malformed.
        let err = store.create_workspace(repo, "Other", "Bil").unwrap_err();
        assert!(refused("task_prefix_taken")(&err), "{err:?}");
        let err = store.create_workspace(other, "Elsewhere", "bil").unwrap_err();
        assert!(refused("task_prefix_taken")(&err), "per runner, not per repository: {err:?}");
        let err = store.set_workspace_prefix(billing.id, billing.resource_version, "r").unwrap_err();
        assert!(refused("task_prefix_taken")(&err), "Main holds `r`: {err:?}");

        let err = store.create_workspace(repo, "Bad", "1x").unwrap_err();
        assert!(refused("task_prefix")(&err), "must start with a letter: {err:?}");
        let err = store.create_workspace(repo, "Long", "abcdefghi").unwrap_err();
        assert!(refused("task_prefix")(&err), "at most 8: {err:?}");
        let err = store.create_workspace(repo, "  ", "ok").unwrap_err();
        assert!(refused("name")(&err), "{err:?}");

        // A workspace may take its own prefix back, in any case.
        let again = store.set_workspace_prefix(billing.id, billing.resource_version, "BIL").unwrap();
        assert_eq!(again.task_prefix, "bil");
        assert_eq!(again.resource_version, billing.resource_version + 1);
    }

    #[test]
    fn a_stale_version_is_a_conflict() {
        let store = Store::open_in_memory().unwrap();
        let repo = store.register_repository_for_test("r");
        let ws = store.create_workspace(repo, "Billing", "bil").unwrap();
        let renamed = store.rename_workspace(ws.id, ws.resource_version, "Payments").unwrap();
        assert_eq!(renamed.name, "Payments");
        assert!(matches!(
            store.rename_workspace(ws.id, ws.resource_version, "Again"),
            Err(DomainError::ResourceConflict)
        ));
        assert!(matches!(
            store.set_workspace_prefix(ws.id, ws.resource_version, "pay"),
            Err(DomainError::ResourceConflict)
        ));
    }

    #[test]
    fn a_key_is_never_issued_twice_across_prefix_renames() {
        let store = Store::open_in_memory().unwrap();
        let repo = store.register_repository_for_test("r");
        let ws = store.create_workspace(repo, "Billing", "bil").unwrap();
        let a = store.create_task(ws.id, "a", Actor::User).unwrap();
        let ws = store.set_workspace_prefix(ws.id, ws.resource_version, "pay").unwrap();
        let b = store.create_task(ws.id, "b", Actor::User).unwrap();
        let ws = store.set_workspace_prefix(ws.id, ws.resource_version, "bil").unwrap();
        let c = store.create_task(ws.id, "c", Actor::User).unwrap();
        assert_eq!((a.key.as_str(), b.key.as_str(), c.key.as_str()), ("bil-1", "pay-1", "bil-2"));
    }

    /// A prefix given up in one repository and taken in another continues
    /// its numbering: "anywhere on the runner".
    #[test]
    fn numbering_spans_the_runner() {
        let store = Store::open_in_memory().unwrap();
        let one = store.register_repository_for_test("one");
        let two = store.register_repository_for_test("two");
        let first = store.create_workspace(one, "Billing", "bil").unwrap();
        store.create_task(first.id, "a", Actor::User).unwrap();
        store.create_task(first.id, "b", Actor::User).unwrap();
        store.set_workspace_prefix(first.id, first.resource_version, "pay").unwrap();
        let second = store.create_workspace(two, "Billing", "bil").unwrap();
        assert_eq!(store.create_task(second.id, "c", Actor::User).unwrap().key, "bil-3");
    }

    #[test]
    fn numbering_counts_former_keys() {
        let store = Store::open_in_memory().unwrap();
        let repo = store.register_repository_for_test("r");
        let main = store.ensure_main_workspace(repo).unwrap();
        store.insert_task_with_former_key_for_test(main.id, "zz-9", "r-7");
        let t = store.create_task(main.id, "next", Actor::User).unwrap();
        assert_eq!(t.key, format!("{}-8", main.task_prefix));
    }

    #[test]
    fn a_moved_task_keeps_its_key_and_leaves_a_note() {
        let store = Store::open_in_memory().unwrap();
        let repo = store.register_repository_for_test("r");
        let main = store.ensure_main_workspace(repo).unwrap();
        let billing = store.create_workspace(repo, "Billing", "bil").unwrap();
        let t = store.create_task(main.id, "t", Actor::User).unwrap();

        let moved = store.move_tasks(&[t.id], billing.id, Actor::Manager).unwrap();
        assert_eq!(moved[0].key, t.key);
        assert_eq!(moved[0].workspace_id, billing.id);
        assert_eq!(moved[0].resource_version, t.resource_version + 1);
        assert_eq!(store.tasks_with_key(None, &t.key).unwrap().len(), 1);
        assert!(store.list_tasks(TaskScope::Workspace(main.id), None).unwrap().is_empty());
        assert_eq!(store.list_tasks(TaskScope::Workspace(billing.id), None).unwrap()[0].id, t.id);
        assert_eq!(store.list_tasks(TaskScope::Repository(repo), None).unwrap().len(), 1);

        let notes = store.notes_for(t.id, None).unwrap();
        let last = notes.last().unwrap();
        assert_eq!(
            (last.kind, last.actor, last.body.as_str()),
            (NoteKind::Comment, Actor::Manager, "Moved from Main to Billing."),
            "a move is recorded in the task's history"
        );
        assert_eq!(last.extra["to_workspace"], billing.id.to_string());

        // Moving it where it already is writes nothing.
        let again = store.move_tasks(&[t.id], billing.id, Actor::Manager).unwrap();
        assert_eq!(again[0].resource_version, moved[0].resource_version);
        assert_eq!(store.notes_for(t.id, None).unwrap().len(), notes.len());
    }

    /// And a refused batch moves none of it.
    #[test]
    fn a_move_across_repositories_is_refused() {
        let store = Store::open_in_memory().unwrap();
        let a = store.register_repository_for_test("a");
        let b = store.register_repository_for_test("b");
        let a_main = store.ensure_main_workspace(a).unwrap();
        let a_side = store.create_workspace(a, "Side", "side").unwrap();
        let here = store.create_task(a_side.id, "here", Actor::User).unwrap();
        let b_main = store.ensure_main_workspace(b).unwrap();
        let there = store.create_task(b_main.id, "there", Actor::User).unwrap();

        let err = store.move_tasks(&[here.id, there.id], a_main.id, Actor::User).unwrap_err();
        assert!(refused("other_repository")(&err), "{err:?}");
        assert_eq!(store.get_task(here.id).unwrap().workspace_id, a_side.id, "nothing moved");
        assert!(matches!(
            store.move_tasks(&[Uuid::now_v7()], a_main.id, Actor::User),
            Err(DomainError::NotFound)
        ));
    }

    #[test]
    fn deleting_refuses_main_and_anything_still_holding_work() {
        let store = Store::open_in_memory().unwrap();
        let repo = store.register_repository_for_test("r");
        let main = store.ensure_main_workspace(repo).unwrap();
        let err = store.delete_workspace(main.id).unwrap_err();
        assert!(refused("main_workspace")(&err), "{err:?}");

        let ws = store.create_workspace(repo, "Billing", "bil").unwrap();
        let t = store.create_task(ws.id, "t", Actor::User).unwrap();
        let err = store.delete_workspace(ws.id).unwrap_err();
        assert!(refused("workspace_not_empty")(&err), "a task: {err:?}");
        store.move_tasks(&[t.id], main.id, Actor::User).unwrap();

        let wt = store.create_unclaimed_worktree_for_test(repo, "/r/.worktrees/x");
        store.assign_worktree(wt, ws.id).unwrap();
        let err = store.delete_workspace(ws.id).unwrap_err();
        assert!(refused("workspace_not_empty")(&err), "a worktree: {err:?}");
        store.assign_worktree(wt, main.id).unwrap();

        let term = store.create_terminal_for_test(wt, ws.id);
        let err = store.delete_workspace(ws.id).unwrap_err();
        assert!(refused("workspace_not_empty")(&err), "a terminal: {err:?}");
        store.set_terminal_workspace(term, main.id).unwrap();

        store.delete_workspace(ws.id).unwrap();
        assert!(matches!(store.get_workspace(ws.id), Err(DomainError::NotFound)));
        assert!(matches!(store.delete_workspace(ws.id), Err(DomainError::NotFound)));
    }

    #[test]
    fn a_claim_sticks_and_an_assignment_overrides_it() {
        let store = Store::open_in_memory().unwrap();
        let repo = store.register_repository_for_test("r");
        let main = store.ensure_main_workspace(repo).unwrap();
        let billing = store.create_workspace(repo, "Billing", "bil").unwrap();
        let wt = store.create_unclaimed_worktree_for_test(repo, "/r/.worktrees/x");
        let shell = store.create_terminal(wt, "s", "shell", TerminalIntent::Running, 80, 24).unwrap();
        assert_eq!((shell.workspace_id, shell.role), (None, TerminalRole::Shell));

        let claimed = store.claim_worktree(wt, billing.id, ClaimSource::Hook).unwrap().expect("claimed");
        assert_eq!((claimed.workspace_id, claimed.claim_source), (Some(billing.id), Some(ClaimSource::Hook)));
        assert_eq!(
            store.get_terminal(shell.id).unwrap().workspace_id,
            Some(billing.id),
            "a terminal with no workspace takes the claimant's"
        );
        assert!(store.claim_worktree(wt, main.id, ClaimSource::Hook).unwrap().is_none(), "sticky");
        assert!(store.claim_worktree(wt, main.id, ClaimSource::Explicit).unwrap().is_none(), "even explicitly");
        assert_eq!(store.get_worktree(wt).unwrap().workspace_id, Some(billing.id));

        let wt = store.assign_worktree(wt, main.id).unwrap();
        assert_eq!((wt.workspace_id, wt.claim_source), (Some(main.id), Some(ClaimSource::Explicit)));
        assert_eq!(
            store.get_terminal(shell.id).unwrap().workspace_id,
            Some(billing.id),
            "a terminal's workspace is whose work it is, and does not follow the worktree"
        );
        let later = store.create_terminal(wt.id, "a", "codex", TerminalIntent::Running, 80, 24).unwrap();
        assert_eq!((later.workspace_id, later.role), (Some(main.id), TerminalRole::Agent));

        let other = store.register_repository_for_test("o");
        let elsewhere = store.ensure_main_workspace(other).unwrap();
        let err = store.assign_worktree(wt.id, elsewhere.id).unwrap_err();
        assert!(refused("other_repository")(&err), "{err:?}");
    }

    /// A repository's main checkout is Main's: the runner refuses to give it
    /// to another workspace, whoever is asking, and leaves it where it was.
    /// One some other workspace already holds (a claim made before this
    /// rule) can still be given back to Main.
    #[test]
    fn a_main_checkout_cannot_be_assigned_away_from_main() {
        let store = Store::open_in_memory().unwrap();
        let repo = store.register_repository_for_test("r");
        let main = store.ensure_main_workspace(repo).unwrap();
        let billing = store.create_workspace(repo, "Billing", "bil").unwrap();
        let checkout = store.create_worktree(repo, "main", "/r", true).unwrap().id;
        store.claim_worktree(checkout, main.id, ClaimSource::Explicit).unwrap().expect("claimed");

        let err = store.assign_worktree(checkout, billing.id).unwrap_err();
        assert!(refused("main_checkout")(&err), "{err:?}");
        assert_eq!(store.get_worktree(checkout).unwrap().workspace_id, Some(main.id), "left with Main");
        store.assign_worktree(checkout, main.id).unwrap();

        let other = store.create_worktree(repo, "main", "/s", true).unwrap().id;
        store.give_worktree_for_test(other, billing.id);
        let ops = store.create_workspace(repo, "Ops", "ops").unwrap();
        let err = store.assign_worktree(other, ops.id).unwrap_err();
        assert!(refused("main_checkout")(&err), "{err:?}");
        let back = store.assign_worktree(other, main.id).unwrap();
        assert_eq!((back.workspace_id, back.claim_source), (Some(main.id), Some(ClaimSource::Explicit)));

        // A linked worktree still goes wherever it is sent.
        let wt = store.create_unclaimed_worktree_for_test(repo, "/r/.worktrees/x");
        assert_eq!(store.assign_worktree(wt, billing.id).unwrap().workspace_id, Some(billing.id));
    }

    /// A signal from another workspace skips an unclaimed main checkout, as
    /// it skips one already claimed: only Main may claim it.
    #[test]
    fn only_main_can_claim_a_main_checkout() {
        let store = Store::open_in_memory().unwrap();
        let repo = store.register_repository_for_test("r");
        let main = store.ensure_main_workspace(repo).unwrap();
        let billing = store.create_workspace(repo, "Billing", "bil").unwrap();
        let checkout = store.create_worktree(repo, "main", "/r", true).unwrap().id;

        for source in [ClaimSource::Hook, ClaimSource::Process, ClaimSource::Explicit] {
            assert!(store.claim_worktree(checkout, billing.id, source).unwrap().is_none(), "{source:?}");
        }
        assert_eq!(store.get_worktree(checkout).unwrap().workspace_id, None, "still unclaimed");
        let claimed = store.claim_worktree(checkout, main.id, ClaimSource::Hook).unwrap().expect("Main's");
        assert_eq!(claimed.workspace_id, Some(main.id));
    }

    /// The first orchestrator whose row doesn't say it has ended.
    fn unended(store: &Store, workspace: Uuid) -> Option<Uuid> {
        store.unended_orchestrators(workspace).unwrap().first().map(|t| t.id)
    }

    /// An orchestrator the caller found not running doesn't hold the seat,
    /// but only as the row was when it looked: a row written since may be
    /// that orchestrator coming back.
    #[test]
    fn a_vacated_orchestrator_frees_its_seat_only_as_it_was_read() {
        let store = Store::open_in_memory().unwrap();
        let repo = store.register_repository_for_test("r");
        let main = store.ensure_main_workspace(repo).unwrap();
        let wt = store.create_unclaimed_worktree_for_test(repo, "/r");
        let [a, b, c] = [(); 3].map(|_| store.create_terminal_for_test(wt, main.id));
        let found = |t: Uuid| Vacated { terminal: t, resource_version: store.get_terminal(t).unwrap().resource_version };

        store.set_terminal_role(a, TerminalRole::Orchestrator).unwrap();
        let a_lost = found(a);
        let err = store.set_terminal_role_with(b, TerminalRole::Orchestrator, &[]).unwrap_err();
        assert!(refused("orchestrator_taken")(&err), "{err:?}");
        store.set_terminal_role_with(b, TerminalRole::Orchestrator, &[a_lost]).expect("a lost one's seat is free");

        // `a` restarted while `b` holds the seat is refused, and left as it was.
        let before = store.get_terminal(a).unwrap();
        let err = store.reseat_orchestrator(a, before.resource_version, &[]).unwrap_err();
        assert!(refused("orchestrator_taken")(&err), "{err:?}");
        assert_eq!(store.get_terminal(a).unwrap().resource_version, before.resource_version);

        // `b` lost in turn: `a` comes back, starting. (`a` confirmed first,
        // as a lost one was.)
        let b_lost = found(b);
        let before = store
            .update_terminal(
                a,
                before.resource_version,
                crate::models::TerminalUpdate {
                    title: before.title.clone(),
                    command_preset: before.command_preset.clone(),
                    intent: TerminalIntent::Running,
                    runtime_confirmed: true,
                    exit_code: None,
                    exit_signal: None,
                    lease_generation: before.lease_generation,
                    epoch: before.epoch,
                    columns: before.columns,
                    rows: before.rows,
                },
            )
            .unwrap();
        let back = store.reseat_orchestrator(a, before.resource_version, &[b_lost]).unwrap();
        assert_eq!(back.intent, TerminalIntent::Running);
        assert!(!back.runtime_confirmed, "unconfirmed until its pane is seen");
        assert_eq!((back.exit_code, back.exit_signal), (None, None));
        assert_eq!(back.resource_version, before.resource_version + 1);

        // What was found before `a` came back doesn't vacate it now.
        let err = store.set_terminal_role_with(c, TerminalRole::Orchestrator, &[a_lost, b_lost]).unwrap_err();
        assert!(refused("orchestrator_taken")(&err), "a stale finding seated a second: {err:?}");
        let err = store.set_terminal_workspace_with(b, main.id, &[a_lost]).unwrap_err();
        assert!(refused("orchestrator_taken")(&err), "{err:?}");
        store.set_terminal_role_with(c, TerminalRole::Orchestrator, &[found(a), b_lost]).expect("a fresh one does");

        let err = store.reseat_orchestrator(a, before.resource_version, &[]).unwrap_err();
        assert!(matches!(err, DomainError::ResourceConflict), "a restart from an old read: {err:?}");
    }

    #[test]
    fn one_orchestrator_per_workspace() {
        let store = Store::open_in_memory().unwrap();
        let repo = store.register_repository_for_test("r");
        let main = store.ensure_main_workspace(repo).unwrap();
        let billing = store.create_workspace(repo, "Billing", "bil").unwrap();
        let wt = store.create_unclaimed_worktree_for_test(repo, "/r");
        let a = store.create_terminal_for_test(wt, main.id);
        let b = store.create_terminal_for_test(wt, main.id);
        assert_eq!(unended(&store, main.id), None);

        store.set_terminal_role(a, TerminalRole::Orchestrator).unwrap();
        let err = store.set_terminal_role(b, TerminalRole::Orchestrator).unwrap_err();
        assert!(refused("orchestrator_taken")(&err), "{err:?}");
        assert_eq!(unended(&store, main.id), Some(a));
        let again = store.set_terminal_role(a, TerminalRole::Orchestrator).unwrap();
        assert_eq!(again.role, TerminalRole::Orchestrator, "re-asserting its own seat is allowed");

        // Billing's orchestrator may run in Main's worktree, and cannot then
        // be handed to Main.
        store.set_terminal_workspace(b, billing.id).unwrap();
        store.set_terminal_role(b, TerminalRole::Orchestrator).unwrap();
        assert_eq!(unended(&store, billing.id), Some(b));
        let err = store.set_terminal_workspace(b, main.id).unwrap_err();
        assert!(refused("orchestrator_taken")(&err), "{err:?}");

        // Once `a` has stopped it is no longer live, and the seat is free.
        let stopped = store.get_terminal(a).unwrap();
        store
            .update_terminal(
                a,
                stopped.resource_version,
                crate::models::TerminalUpdate {
                    title: stopped.title,
                    command_preset: stopped.command_preset,
                    intent: TerminalIntent::Stopped,
                    runtime_confirmed: stopped.runtime_confirmed,
                    exit_code: Some(0),
                    exit_signal: None,
                    lease_generation: stopped.lease_generation,
                    epoch: stopped.epoch,
                    columns: stopped.columns,
                    rows: stopped.rows,
                },
            )
            .unwrap();
        assert_eq!(unended(&store, main.id), None);
        store.set_terminal_workspace(b, main.id).unwrap();
        assert_eq!(unended(&store, main.id), Some(b));

        let unowned = store.create_unclaimed_worktree_for_test(repo, "/r/.worktrees/free");
        let loose = store.create_terminal(unowned, "t", "claude", TerminalIntent::Running, 80, 24).unwrap();
        let err = store.set_terminal_role(loose.id, TerminalRole::Orchestrator).unwrap_err();
        assert!(refused("workspace")(&err), "an orchestrator of nothing: {err:?}");
    }

    #[test]
    fn every_role_and_claim_source_round_trips() {
        for role in [TerminalRole::Shell, TerminalRole::Agent, TerminalRole::Orchestrator] {
            assert_eq!(TerminalRole::from_i64(role.as_i64()), role);
        }
        assert_eq!(TerminalRole::from_i64(7), TerminalRole::Agent, "never guessed to be an orchestrator");
        for source in [ClaimSource::Explicit, ClaimSource::Hook, ClaimSource::Process, ClaimSource::Migration] {
            assert_eq!(ClaimSource::parse(source.as_str()), Some(source));
        }
    }
}
