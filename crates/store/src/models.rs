//! Durable row shapes.
//!
//! Every struct here mirrors a table exactly. `Terminal` in particular has no
//! field that could encode whether the process is alive right now: that is
//! `farcooler_core::derive`'s job, computed fresh against tmux on every read,
//! never stored. See the crate root docs and `store::tests::terminals_table_has_no_runtime_state_column`.

use rusqlite::Row;
use rusqlite::types::Type;
use uuid::Uuid;

use farcooler_protocol::v1::TerminalIntent;

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
    /// The task key prefix this repository was assigned, once, at
    /// registration -- `''` until `Store::assign_task_key_prefix` sets it.
    ///
    /// Stored, not derived from `display_name`: see `farcooler_store::tasks`
    /// for why a prefix computed on every read would break every task key
    /// ever written down the moment the repository was renamed.
    pub task_key_prefix: String,
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
        task_key_prefix: row.get(7)?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub id: Uuid,
    pub repository_id: Uuid,
    pub branch: String,
    pub worktree_path: String,
    /// The user asked not to see it. Never touches git.
    pub hidden: bool,
    pub creation_failed: bool,
    /// The repository's own checkout, as git reports it. Never removable.
    pub is_main_checkout: bool,
    /// git no longer lists this worktree, but the row carries terminals worth
    /// keeping. Set by the reconciler; cleared if the worktree comes back.
    pub worktree_missing: bool,
    /// Where this workspace sits in the list, across the whole runner.
    ///
    /// The user's, and nothing else's. It is set once when the row is created —
    /// after every row that already exists — and after that only a reorder ever
    /// moves it. Deliberately not derived from anything about the work: a rank
    /// that answered to activity or attention would rearrange the layout under
    /// someone who is reading it, which is the one thing this must never do.
    pub ordinal: u32,
    pub resource_version: u64,
}

impl Workspace {
    /// What this workspace is called.
    ///
    /// Derived from the worktree path on every read rather than stored, for the
    /// same reason no terminal stores whether it is running: a second copy of a
    /// fact someone else owns is a copy that can be wrong. There is deliberately
    /// no column a stale name could occupy. See `farcooler_core::names`.
    pub fn name(&self) -> String {
        farcooler_core::names::display(&self.worktree_path)
    }
}

pub(crate) fn row_to_workspace(row: &Row) -> rusqlite::Result<Workspace> {
    Ok(Workspace {
        id: get_uuid(row, 0)?,
        repository_id: get_uuid(row, 1)?,
        branch: row.get(2)?,
        worktree_path: row.get(3)?,
        hidden: row.get(4)?,
        creation_failed: row.get(5)?,
        resource_version: row.get::<_, i64>(6)? as u64,
        is_main_checkout: row.get(7)?,
        worktree_missing: row.get(8)?,
        ordinal: row.get::<_, i64>(9)? as u32,
    })
}

/// What a terminal's pane is hosting.
///
/// Distinct from a terminal's VT `mode` and from the ACP `agent_mode`. Three
/// unrelated things would otherwise all be called "mode" in one pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneMode {
    /// A TUI, exactly as before this feature existed. The default.
    Terminal,
    /// `farcooler agent-host`, bridging an ACP agent.
    Agent,
    /// `farcooler pane-host`, holding the rectangle a client draws this
    /// worktree's diff into.
    ///
    /// Set when the pane is created and never changed: unlike the two above
    /// there is no TUI underneath to switch back to, so this is what the pane
    /// IS rather than a posture it is currently in.
    Changes,
}

impl PaneMode {
    pub fn as_i64(self) -> i64 {
        match self {
            PaneMode::Terminal => 0,
            PaneMode::Agent => 1,
            PaneMode::Changes => 2,
        }
    }

    pub fn from_i64(raw: i64) -> Self {
        match raw {
            1 => PaneMode::Agent,
            2 => PaneMode::Changes,
            // Anything unrecognized is the mode that always works.
            _ => PaneMode::Terminal,
        }
    }
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
    pub lease_generation: u64,
    pub epoch: u64,
    pub columns: u32,
    pub rows: u32,
    pub resource_version: u64,
    pub pane_mode: PaneMode,
    pub agent_session_id: Option<String>,
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
        lease_generation: row.get::<_, i64>(8)? as u64,
        epoch: row.get::<_, i64>(9)? as u64,
        columns: row.get::<_, i64>(10)? as u32,
        rows: row.get::<_, i64>(11)? as u32,
        resource_version: row.get::<_, i64>(12)? as u64,
        pane_mode: PaneMode::from_i64(row.get(13)?),
        agent_session_id: row.get(14)?,
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

// ---- the board ----
//
// The split this whole surface rests on: `Task` is current understanding and
// every field of it may be revised, while `TaskNote` is the record of how that
// understanding was reached and no field of any note may ever change. There is
// deliberately no `TaskNoteUpdate` here, and `task_notes` has a trigger that
// refuses one anyway. Correcting the record is a new note carrying
// `supersedes`.

/// Where a task sits, in the order work moves through the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Backlog,
    Todo,
    /// Waiting on a person. A `Question` note puts a task here and the
    /// `Answer` takes it back out.
    NeedsDecision,
    InProgress,
    InReview,
    Done,
    Cancelled,
}

impl TaskStatus {
    /// The stored form, which is also the form a `StatusChange` note's `from`
    /// and `to` carry. One vocabulary, so a note reads the same as the column
    /// it describes.
    pub fn as_str(self) -> &'static str {
        match self {
            TaskStatus::Backlog => "backlog",
            TaskStatus::Todo => "todo",
            TaskStatus::NeedsDecision => "needs_decision",
            TaskStatus::InProgress => "in_progress",
            TaskStatus::InReview => "in_review",
            TaskStatus::Done => "done",
            TaskStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(raw: &str) -> Option<TaskStatus> {
        Some(match raw {
            "backlog" => TaskStatus::Backlog,
            "todo" => TaskStatus::Todo,
            "needs_decision" => TaskStatus::NeedsDecision,
            "in_progress" => TaskStatus::InProgress,
            "in_review" => TaskStatus::InReview,
            "done" => TaskStatus::Done,
            "cancelled" => TaskStatus::Cancelled,
            _ => return None,
        })
    }
}

/// What a note is.
///
/// The list is a guess at what a manager needs and the guess will be wrong;
/// adding a kind is a line here and an arm below, never a migration, which is
/// why `extra` is JSON rather than columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteKind {
    /// What was decided, why, and what was rejected. The reason the board is
    /// worth keeping.
    Decision,
    /// Something learned about the code or the problem.
    Finding,
    /// Something only the user can answer.
    Question,
    /// The answer to a question.
    Answer,
    /// What a dispatched agent is doing. High volume; the one kind a reader
    /// usually wants to skip.
    Progress,
    /// A person talking.
    Comment,
    /// A task moved. Recorded rather than inferred, so history reads without
    /// a join.
    StatusChange,
    /// A task came into being, and who made it.
    Created,
}

impl NoteKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NoteKind::Decision => "decision",
            NoteKind::Finding => "finding",
            NoteKind::Question => "question",
            NoteKind::Answer => "answer",
            NoteKind::Progress => "progress",
            NoteKind::Comment => "comment",
            NoteKind::StatusChange => "status_change",
            NoteKind::Created => "created",
        }
    }

    pub fn parse(raw: &str) -> Option<NoteKind> {
        Some(match raw {
            "decision" => NoteKind::Decision,
            "finding" => NoteKind::Finding,
            "question" => NoteKind::Question,
            "answer" => NoteKind::Answer,
            "progress" => NoteKind::Progress,
            "comment" => NoteKind::Comment,
            "status_change" => NoteKind::StatusChange,
            "created" => NoteKind::Created,
            _ => return None,
        })
    }
}

/// Who did something.
///
/// `Agent` carries the terminal rather than a name because that is the thing
/// you can open and read: an entry saying an agent decided something is only
/// useful if you can go and see which pane said it.
///
/// One string column (`user`, `manager`, `agent:<uuid>`) rather than a kind
/// plus a nullable id, because two columns are two things that can disagree --
/// a row claiming `manager` with a terminal attached has no meaning, and
/// nothing would stop one being written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    User,
    Manager,
    Agent { terminal: Uuid },
}

impl std::fmt::Display for Actor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Actor::User => f.write_str("user"),
            Actor::Manager => f.write_str("manager"),
            Actor::Agent { terminal } => write!(f, "agent:{terminal}"),
        }
    }
}

impl Actor {
    pub fn parse(raw: &str) -> Option<Actor> {
        match raw {
            "user" => Some(Actor::User),
            "manager" => Some(Actor::Manager),
            _ => raw
                .strip_prefix("agent:")
                .and_then(|id| Uuid::parse_str(id).ok())
                .map(|terminal| Actor::Agent { terminal }),
        }
    }
}

/// One checkable thing, and whether it holds.
///
/// Structured rather than a markdown checklist inside `intent`, because a
/// dispatched agent is asked "are you done" and review is asked "is this
/// right", and both questions are answered item by item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptanceItem {
    pub id: Uuid,
    pub text: String,
    pub met: bool,
}

impl AcceptanceItem {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({ "id": self.id.to_string(), "text": self.text, "met": self.met })
    }

    fn from_json(value: &serde_json::Value) -> Option<AcceptanceItem> {
        Some(AcceptanceItem {
            id: Uuid::parse_str(value.get("id")?.as_str()?).ok()?,
            text: value.get("text")?.as_str()?.to_string(),
            met: value.get("met")?.as_bool()?,
        })
    }
}

/// Current understanding, and nothing about how it was reached.
///
/// Every field here may be revised freely; see `TaskNote` for the half that
/// may not. `created_at` is on the table but not here: it orders a listing,
/// and the question the board itself asks is `status_since`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub id: Uuid,
    /// Short and typeable, for a person and for a prompt: `fc-42`. Per
    /// repository, not global.
    pub key: String,
    pub repository_id: Uuid,
    /// One line, for the board.
    pub title: String,
    pub status: TaskStatus,
    /// Unix milliseconds, from when the status last changed.
    ///
    /// Stored rather than derived from the note stream, because the board's
    /// most important column is "how long has this been sitting like this"
    /// and a list view must not read every task's history to render it.
    pub status_since: i64,
    /// Why this exists, in the user's terms. Revised as understanding
    /// improves; never a log.
    pub intent: String,
    /// How anyone knows it is finished.
    pub acceptance: Vec<AcceptanceItem>,
    /// What must hold. Separate from intent because constraints outlive the
    /// reason and are what a fix round most often violates.
    pub constraints: Vec<String>,
    /// The lane this task is using, when it has one. A task exists in
    /// `backlog` long before any worktree does.
    pub workspace_id: Option<Uuid>,
    pub labels: Vec<String>,
    pub resource_version: u64,
}

/// Every field of `Task` a caller may legitimately revise in place.
///
/// `status` is deliberately absent: it moves through `Store::set_task_status`,
/// which is what writes the `StatusChange` note. A status settable here would
/// be a status that could move without the log noticing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskUpdate {
    pub title: String,
    pub intent: String,
    pub acceptance: Vec<AcceptanceItem>,
    pub constraints: Vec<String>,
    pub labels: Vec<String>,
    pub workspace_id: Option<Uuid>,
}

/// One entry in the record. Never edited, by this crate or by anything else:
/// `task_notes` carries triggers that refuse an UPDATE or a DELETE while the
/// note's task is still there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskNote {
    pub id: Uuid,
    pub task_id: Uuid,
    pub kind: NoteKind,
    pub actor: Actor,
    /// Unix milliseconds.
    pub at: i64,
    pub body: String,
    /// Kind-specific structure: a decision's rejected alternatives, a
    /// question's options, a status change's `from` and `to`.
    ///
    /// JSON and not columns, deliberately -- see `NoteKind`. The cost is that
    /// these fields are a convention rather than a constraint, which is the
    /// right trade this early and the wrong one later.
    pub extra: serde_json::Value,
    /// The note this replaces, when a later understanding replaced an earlier
    /// one. Superseding rather than editing is what makes "we changed our
    /// minds" legible: an edited decision is indistinguishable from a decision
    /// that was always that way.
    pub supersedes: Option<Uuid>,
}

/// A column that should have decoded and did not.
///
/// Every one of these columns is written only by this crate, so reaching here
/// means the row was written by something else or by an older shape of this
/// code. Failing is the point: unlike `PaneMode::from_i64`, there is no status
/// or kind that "always works" -- defaulting an unreadable status to `Backlog`
/// would quietly move a finished task back onto the board.
fn decode_failure(idx: usize, what: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(idx, Type::Text, Box::new(std::io::Error::other(what)))
}

fn get_status(row: &Row, idx: usize) -> rusqlite::Result<TaskStatus> {
    let raw: String = row.get(idx)?;
    TaskStatus::parse(&raw).ok_or_else(|| decode_failure(idx, format!("unknown status {raw:?}")))
}

fn get_note_kind(row: &Row, idx: usize) -> rusqlite::Result<NoteKind> {
    let raw: String = row.get(idx)?;
    NoteKind::parse(&raw).ok_or_else(|| decode_failure(idx, format!("unknown note kind {raw:?}")))
}

fn get_actor(row: &Row, idx: usize) -> rusqlite::Result<Actor> {
    let raw: String = row.get(idx)?;
    Actor::parse(&raw).ok_or_else(|| decode_failure(idx, format!("unreadable actor {raw:?}")))
}

fn get_optional_uuid(row: &Row, idx: usize) -> rusqlite::Result<Option<Uuid>> {
    let bytes: Option<Vec<u8>> = row.get(idx)?;
    bytes
        .map(|b| {
            Uuid::from_slice(&b).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(idx, Type::Blob, Box::new(e))
            })
        })
        .transpose()
}

fn get_json(row: &Row, idx: usize) -> rusqlite::Result<serde_json::Value> {
    let raw: String = row.get(idx)?;
    serde_json::from_str(&raw).map_err(|e| decode_failure(idx, format!("{e}")))
}

/// A JSON text column holding an array, decoded strictly: an element of the
/// wrong shape fails the read rather than being dropped, so a caller never
/// silently receives fewer constraints than the row holds.
fn get_json_array(row: &Row, idx: usize) -> rusqlite::Result<Vec<serde_json::Value>> {
    match get_json(row, idx)? {
        serde_json::Value::Array(items) => Ok(items),
        other => Err(decode_failure(idx, format!("expected a JSON array, got {other}"))),
    }
}

fn get_strings(row: &Row, idx: usize) -> rusqlite::Result<Vec<String>> {
    get_json_array(row, idx)?
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_string)
                .ok_or_else(|| decode_failure(idx, format!("expected a string, got {item}")))
        })
        .collect()
}

fn get_acceptance(row: &Row, idx: usize) -> rusqlite::Result<Vec<AcceptanceItem>> {
    get_json_array(row, idx)?
        .iter()
        .map(|item| {
            AcceptanceItem::from_json(item)
                .ok_or_else(|| decode_failure(idx, format!("unreadable acceptance item {item}")))
        })
        .collect()
}

pub(crate) fn strings_to_json(values: &[String]) -> String {
    serde_json::Value::Array(values.iter().map(|v| serde_json::Value::String(v.clone())).collect())
        .to_string()
}

pub(crate) fn acceptance_to_json(items: &[AcceptanceItem]) -> String {
    serde_json::Value::Array(items.iter().map(AcceptanceItem::to_json).collect()).to_string()
}

pub(crate) fn row_to_task(row: &Row) -> rusqlite::Result<Task> {
    Ok(Task {
        id: get_uuid(row, 0)?,
        repository_id: get_uuid(row, 1)?,
        key: row.get(2)?,
        title: row.get(3)?,
        status: get_status(row, 4)?,
        status_since: row.get(5)?,
        intent: row.get(6)?,
        acceptance: get_acceptance(row, 7)?,
        constraints: get_strings(row, 8)?,
        labels: get_strings(row, 9)?,
        workspace_id: get_optional_uuid(row, 10)?,
        resource_version: row.get::<_, i64>(11)? as u64,
    })
}

pub(crate) fn row_to_task_note(row: &Row) -> rusqlite::Result<TaskNote> {
    Ok(TaskNote {
        id: get_uuid(row, 0)?,
        task_id: get_uuid(row, 1)?,
        kind: get_note_kind(row, 2)?,
        actor: get_actor(row, 3)?,
        at: row.get(4)?,
        body: row.get(5)?,
        extra: get_json(row, 6)?,
        supersedes: get_optional_uuid(row, 7)?,
    })
}
