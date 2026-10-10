//! What a native view draws, one row per thing that happened.
//!
//! Every row has an id that never changes once it is handed out, so a client
//! can apply a change to the row it already drew rather than re-diffing the
//! list. A provisional row (one a hook announced and the transcript has not
//! yet confirmed) keeps its id when it is confirmed: the client sees the same
//! row become firm, never a second row beside it.
//!
//! Nothing is ever renumbered (ov-358 found today's ring renumbering its
//! window on trim, which froze every reader at 4096 events). A row's `ord` is
//! its position at insertion and `rev` the revision that last changed it; both
//! only grow, so a page cursor (`ord`) and a follow cursor (`rev`) stay valid
//! for the life of the session. A row a hook put up that the transcript then
//! showed to be a copy is retracted rather than removed, so `ord` stays its
//! index (`Row::retracted`).

use serde::Serialize;

/// One row, where it sits, and whether the transcript has confirmed it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Row {
    /// Insertion order, from 0. Rows are never removed (a retracted one stays
    /// in place), so this is also the row's index, and a page is a range of it.
    pub ord: u64,
    /// The projection's revision when this row last changed. Monotonic.
    pub rev: u64,
    /// Stable for the life of the session: `turn:<promptId>`,
    /// `tool:<tool_use_id>`, `sub:<tool_use_id>`, and so on.
    pub id: String,
    /// The `Turn` row this one belongs to. `None` for a turn itself, and for
    /// anything that happened before the first prompt this projection saw.
    pub turn: Option<String>,
    /// Announced by a hook and not yet confirmed by a transcript record.
    pub provisional: bool,
    /// Taken back: a hook's row the transcript showed to be a copy of one it
    /// wrote. Kept in place so `ord` stays the row's index; a page skips it,
    /// and a follow sends its removal (ov-366).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub retracted: bool,
    /// The revision the row was added at, so a follow can tell a row a
    /// client has never seen (an insert) from one it may hold (an update).
    #[serde(skip)]
    pub born: u64,
    pub kind: RowKind,
}

/// One entry of a follow: what happened to a row after the follower's
/// revision. Rows are only ever added at the end, so an insert is always
/// below every row the client holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Change<'r> {
    Insert(&'r Row),
    Update(&'r Row),
    /// A row the client may hold is gone (`Row::retracted`), by id.
    Remove { id: &'r str, rev: u64 },
}

impl Change<'_> {
    pub fn id(&self) -> &str {
        match self {
            Change::Insert(row) | Change::Update(row) => &row.id,
            Change::Remove { id, .. } => id,
        }
    }

    pub fn rev(&self) -> u64 {
        match self {
            Change::Insert(row) | Change::Update(row) => row.rev,
            Change::Remove { rev, .. } => *rev,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum RowKind {
    Turn(Turn),
    Prose(Prose),
    Thinking(Thinking),
    Tool(Tool),
    Subagent(Subagent),
    Ask(Ask),
    Queued(Queued),
    Notice(Notice),
    Handoff(Handoff),
    Gap(Gap),
    Hint(Hint),
    Tasks(Tasks),
}

/// One prompt and everything the agent did about it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Turn {
    /// What was asked, in full (to `PROMPT_CHARS`), line breaks kept: the
    /// person's own message is a row every device draws, not a title. Today's
    /// chat path records no user message at all (ov-358, finding 4).
    pub prompt: String,
    pub origin: TurnOrigin,
    pub started_ms: Option<i64>,
    pub ended_ms: Option<i64>,
    /// Claude's own `turn_duration` when it wrote one, else the span from the
    /// prompt to the end.
    pub duration_ms: Option<i64>,
    /// `None` while the turn is open.
    pub outcome: Option<TurnOutcome>,
    /// Background subagents this turn launched that have not ended. A turn
    /// can be over with agents still running; the view says both.
    pub background_running: u32,
    /// What claude's session registry last said about the process, for the
    /// newest turn only: busy, idle, or running a shell command.
    pub activity: Option<Activity>,
    /// The prompt claude's empty box suggests after this turn, for the newest
    /// turn only: the words its composer shows dim, and takes into the draft
    /// on Tab (ov-409). Read off the screen, the one place claude puts it;
    /// absent when there is none. Never sent by anything but a person.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    /// The images the prompt carried, in its order: pasted in the terminal
    /// (`[Image #N]`) or sent from a composer (ov-454). Their bytes stay in
    /// the transcript, fetched one at a time through `agent.image`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<PromptImage>,
    /// Where the prompt's record is, for `agent.image` to read its images
    /// back. Never sent.
    #[serde(skip)]
    pub source: Option<RecordAt>,
    /// The tokens the turn's newest model call used, its context and its
    /// answer together (`Message::tokens`): what the agent panel shows beside
    /// "main" (ov-453). 0 until a call says.
    #[serde(skip_serializing_if = "is_zero")]
    pub tokens: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// One image a prompt carried: its type, as the transcript says it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PromptImage {
    pub mime: String,
}

/// A transcript record's place: its file and the byte its line starts at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordAt {
    pub path: std::sync::Arc<std::path::Path>,
    pub at: u64,
}

/// Who started a turn: claude's own `promptSource` / `origin.kind`, folded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TurnOrigin {
    /// A person, at the terminal or through Far Cooler typing for them.
    Typed,
    /// A message written while the agent was busy, sent from claude's queue.
    Queued,
    /// A background task finished and claude woke itself to read the result.
    Notification,
    /// A program driving claude, not a person.
    Sdk,
    /// Claude woke itself for something no person typed: another session's
    /// message, a continuation (`promptSource: system`).
    System,
    /// A scheduled task fired (`CronCreate`, `/loop`): `turnOrigin:
    /// scheduled`, with the task's `scheduledTaskId` (ov-452).
    Scheduled,
    /// A turn whose start this projection never saw (it began before the file
    /// was read, or the source said something new).
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum TurnOutcome {
    Finished,
    Interrupted,
    Failed { detail: String },
    /// A hook saw the prompt submitted and the transcript moved on to a later
    /// turn without ever writing it. Seen live: two of seven prompts in the
    /// recorded sandbox session.
    Unrecorded,
}

/// The registry's `status`, which claude rewrites as the process moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Activity {
    Busy,
    Idle,
    /// Running a `!` shell command. Seen live as `"shell"`; not in the brief.
    Shell,
    /// A permission dialog up: claude 2.1.290 writes `"waiting"` (ov-368).
    /// A view offers no Stop then: one Esc would answer the dialog No.
    Waiting,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Prose {
    pub text: String,
    /// The turn's closing answer (`stop_reason == "end_turn"`) rather than
    /// narration on the way there.
    pub conclusion: bool,
    pub at_ms: Option<i64>,
}

/// That the agent thought, and for how long. Its words are not kept: claude
/// writes them empty or signed, and the view shows only the duration.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Thinking {
    pub started_ms: Option<i64>,
    pub ended_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Tool {
    pub name: String,
    /// The one input field a person reads first: a description, a command, a
    /// path or a pattern.
    pub summary: String,
    pub status: ToolStatus,
    pub started_ms: Option<i64>,
    pub ended_ms: Option<i64>,
    /// An edit's hunks, from the result's `structuredPatch`.
    pub diff: Vec<Hunk>,
    pub file_path: Option<String>,
    /// What it was called with, a `key: value` line per field, cut short
    /// (`detail`): what the row opens to, with `result` (ov-452).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// What it answered, cut likewise. Absent until it answers, and for an
    /// answer with no text (an image).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ToolStatus {
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    /// Unified-diff lines, each starting with ` `, `-` or `+`.
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Subagent {
    /// The `Agent` call that launched it: the join key onto its meta file.
    pub tool_use_id: String,
    /// `agentId`, once the launch result or the meta file names it.
    pub agent_id: Option<String>,
    pub agent_type: String,
    pub description: String,
    pub background: bool,
    pub status: SubagentState,
    pub started_ms: Option<i64>,
    pub ended_ms: Option<i64>,
    /// Tool calls seen in its own transcript.
    pub tool_count: u32,
    /// Its latest tool call, as `Name summary`.
    pub current_action: String,
    /// The newest record in its own transcript, for a live run time.
    pub last_ms: Option<i64>,
    /// The tokens its newest model call used, context and answer together,
    /// as claude's agent panel counts them (ov-453). 0 until its transcript
    /// says.
    #[serde(skip_serializing_if = "is_zero")]
    pub tokens: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SubagentState {
    Running,
    Completed,
    Failed,
    Killed,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Ask {
    pub kind: AskKind,
    /// The question, or the tool asking permission and what it would touch.
    pub text: String,
    pub tool: Option<String>,
    pub asked_ms: Option<i64>,
    pub answered_ms: Option<i64>,
    pub answered: bool,
    /// The id a view answers it with (`terminal.agent_answer`) while the
    /// runner's hook holds it, and only then (ov-370). Absent once the hold
    /// ends, however it ended: then only the terminal can answer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub held: Option<String>,
    /// An `AskUserQuestion`'s questions, whole, so a view can offer their
    /// options. Empty for any other ask.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<AskQuestion>,
    /// An `ExitPlanMode`'s plan, its line breaks kept, up to `PLAN_CHARS`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// The device whose answer the hook took ("iPhone", "Mac"), when one
    /// did; absent when the keyboard answered, or nobody has.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answered_by: Option<String>,
}

/// One question of an `AskUserQuestion`, as claude asked it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AskQuestion {
    /// The words claude asked, which its answer is keyed by.
    pub question: String,
    /// Claude's short label for it ("Color").
    pub header: String,
    pub options: Vec<AskOption>,
    /// Several options may be chosen; claude reads them joined by ", ".
    pub multi_select: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AskOption {
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AskKind {
    /// `AskUserQuestion`.
    Question,
    /// A held `PermissionRequest`.
    Permission,
    /// `ExitPlanMode`.
    PlanExit,
}

/// A message written while claude was busy, waiting in claude's own queue.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Queued {
    pub text: String,
    pub state: QueuedState,
    pub at_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum QueuedState {
    Waiting,
    /// Taken into the turn that was running (`absorbed_mid_turn`), or handed
    /// to an agent. One a dequeue sent as a turn of its own is retracted
    /// instead: that turn shows it (ov-452).
    Sent,
    /// Taken back before it was sent.
    Withdrawn,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Notice {
    pub kind: NoticeKind,
    pub text: String,
    pub at_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum NoticeKind {
    /// The context was compacted (`compact_boundary`, or `SessionStart`
    /// with `source: compact`).
    Compacted,
    /// `/clear`: the conversation continues in a new session file.
    Cleared,
    /// `claude --resume` picked this conversation up again.
    Resumed,
    /// A slash command claude ran locally.
    Command,
    /// A request failed and claude is retrying, or gave up.
    ApiError,
}

/// Something only the terminal can show: a panel, a trust or MCP dialog.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Handoff {
    pub reason: String,
    pub at_ms: Option<i64>,
}

/// The row `Hint` has: one per session, never renumbered, found by this id.
pub const HINT_ID: &str = "hint:composer";

/// What claude's empty box shows as a generic example, `Try "…"` (ov-409):
/// not part of the conversation, so a view keeps this row out of its
/// transcript and shows its words as the composer's placeholder. `text` is
/// empty once the box shows something else (a row is never removed).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hint {
    pub text: String,
}

/// The agent's task list as this turn left it (ov-452): a `TodoWrite`'s
/// list, or the one `TaskCreate` and `TaskUpdate` build. One row per turn
/// that changed the list, where its first change was; each later change in
/// the turn updates it, so a run of updates is one checklist, not a row each.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Tasks {
    pub items: Vec<TaskItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TaskItem {
    pub subject: String,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
}

/// Where the projection cannot say what happened.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Gap {
    pub reason: GapReason,
    /// Consecutive gaps of one reason fold into one row.
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum GapReason {
    /// A complete line that is not a JSON object.
    Unparsed,
    /// A line over the reader's cap, skipped unread.
    TooLarge,
    /// A record of a type this build does not know, named.
    Unknown(String),
    /// The file shrank or was replaced, and was read again from its start.
    Rewritten,
}
