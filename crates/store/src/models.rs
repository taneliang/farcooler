//! Durable row shapes.
//!
//! Every struct here mirrors a table exactly. `Terminal` in particular has no
//! field that could encode whether the process is alive right now: that is
//! `overnight_core::derive`'s job, computed fresh against tmux on every read,
//! never stored. See the crate root docs and `store::tests::terminals_table_has_no_runtime_state_column`.

use rusqlite::Row;
use rusqlite::types::Type;
use uuid::Uuid;

use overnight_protocol::v1::TerminalIntent;

pub(crate) fn uuid_blob(id: Uuid) -> Vec<u8> {
    id.as_bytes().to_vec()
}

pub(crate) fn get_uuid(row: &Row, idx: usize) -> rusqlite::Result<Uuid> {
    let bytes: Vec<u8> = row.get(idx)?;
    Uuid::from_slice(&bytes)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(idx, Type::Blob, Box::new(e)))
}

pub(crate) fn get_intent(row: &Row, idx: usize) -> rusqlite::Result<TerminalIntent> {
    let raw: i32 = row.get(idx)?;
    TerminalIntent::try_from(raw)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(idx, Type::Integer, Box::new(e)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryRoot {
    pub id: Uuid,
    pub host_id: Uuid,
    pub path: String,
    /// Unix milliseconds.
    pub created_at: i64,
    pub resource_version: u64,
}

pub(crate) fn row_to_repository_root(row: &Row) -> rusqlite::Result<RepositoryRoot> {
    Ok(RepositoryRoot {
        id: get_uuid(row, 0)?,
        host_id: get_uuid(row, 1)?,
        path: row.get(2)?,
        created_at: row.get(3)?,
        resource_version: row.get::<_, i64>(4)? as u64,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    pub id: Uuid,
    pub host_id: Uuid,
    pub repository_root_id: Uuid,
    pub display_name: String,
    pub canonical_git_dir: String,
    pub remote_summary: String,
    pub resource_version: u64,
}

pub(crate) fn row_to_repository(row: &Row) -> rusqlite::Result<Repository> {
    Ok(Repository {
        id: get_uuid(row, 0)?,
        host_id: get_uuid(row, 1)?,
        repository_root_id: get_uuid(row, 2)?,
        display_name: row.get(3)?,
        canonical_git_dir: row.get(4)?,
        remote_summary: row.get(5)?,
        resource_version: row.get::<_, i64>(6)? as u64,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub id: Uuid,
    pub repository_id: Uuid,
    pub task_name: String,
    pub branch: String,
    pub worktree_path: String,
    pub archived: bool,
    pub creation_failed: bool,
    pub resource_version: u64,
}

pub(crate) fn row_to_workspace(row: &Row) -> rusqlite::Result<Workspace> {
    Ok(Workspace {
        id: get_uuid(row, 0)?,
        repository_id: get_uuid(row, 1)?,
        task_name: row.get(2)?,
        branch: row.get(3)?,
        worktree_path: row.get(4)?,
        archived: row.get(5)?,
        creation_failed: row.get(6)?,
        resource_version: row.get::<_, i64>(7)? as u64,
    })
}

/// The durable half of a terminal. Deliberately has no runtime-state field:
/// see the crate root docs for why that omission is the whole point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terminal {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub title: String,
    pub command_preset: String,
    pub intent: TerminalIntent,
    pub runtime_confirmed: bool,
    pub exit_code: Option<i32>,
    pub exit_signal: Option<i32>,
    pub loss_dismissed: bool,
    pub lease_generation: u64,
    pub epoch: u64,
    pub columns: u32,
    pub rows: u32,
    pub resource_version: u64,
}

pub(crate) fn row_to_terminal(row: &Row) -> rusqlite::Result<Terminal> {
    Ok(Terminal {
        id: get_uuid(row, 0)?,
        workspace_id: get_uuid(row, 1)?,
        title: row.get(2)?,
        command_preset: row.get(3)?,
        intent: get_intent(row, 4)?,
        runtime_confirmed: row.get(5)?,
        exit_code: row.get(6)?,
        exit_signal: row.get(7)?,
        loss_dismissed: row.get(8)?,
        lease_generation: row.get::<_, i64>(9)? as u64,
        epoch: row.get::<_, i64>(10)? as u64,
        columns: row.get::<_, i64>(11)? as u32,
        rows: row.get::<_, i64>(12)? as u32,
        resource_version: row.get::<_, i64>(13)? as u64,
    })
}

/// Every field of `Terminal` a caller may legitimately change in place. `id`,
/// `workspace_id`, and `resource_version` are excluded: identity never moves
/// and the version is the store's own bookkeeping, not an input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalUpdate {
    pub title: String,
    pub command_preset: String,
    pub intent: TerminalIntent,
    pub runtime_confirmed: bool,
    pub exit_code: Option<i32>,
    pub exit_signal: Option<i32>,
    pub loss_dismissed: bool,
    pub lease_generation: u64,
    pub epoch: u64,
    pub columns: u32,
    pub rows: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdempotencyRecord {
    pub key: String,
    pub client_id: Uuid,
    pub request_hash: String,
    /// Unix milliseconds.
    pub created_at: i64,
}
