//! The fold: transcript records in, rows out.
//!
//! A port of the spike's `project.py` (branch agent-architecture, 27bd9f83),
//! with what the spike left out: the rows the design names beyond turns, tools
//! and subagents; a gap where a record cannot be read; and the hook overlay in
//! `hooks.rs`, which needs the transcript's turn and the hooks' turn kept
//! apart (see `Projection::turn`).
//!
//! Line order is the only order trusted. `timestamp` is read for durations and
//! never to sort: claude's timestamps are not monotonic, and a duration that
//! would come out negative is clamped to zero instead.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use super::record::{decode, Block, Content, Input, Record, SubagentMeta, ToolUseResult};
use super::rows::*;
use crate::session_log::claude::parse_iso8601_millis;
use crate::session_log::SubagentStatus;

/// Record types claude writes that carry nothing a row is made of. Anything
/// else unrecognized becomes a `Gap`, so a new record type is seen, not lost.
const SILENT_TYPES: &[&str] = &[
    "frame-link",
    "artifact-autoreact-ledger",
    "artifact-comment-monitor",
    "mode",
    "permission-mode",
    "atis-latch",
    "last-prompt",
    "cost-state",
    "file-history-snapshot",
    "file-history-delta",
    "ai-title",
    "agent-name",
    "custom-title",
    "bridge-session",
    "pr-link",
    "summary",
    "tag",
];

/// The longest prompt a `Turn` row keeps, line breaks and all. Long enough
/// for any typed message and most pastes; a 1 MB paste is cut, with `…`.
pub(super) const PROMPT_CHARS: usize = 16_000;

/// The longest one-line text a queued message, notice or ask keeps.
const LINE_CHARS: usize = 200;

/// The longest summary a `Tool` row or a subagent's action line keeps.
const SUMMARY_CHARS: usize = 80;

/// What a subagent's own transcript said before anything joined it to the
/// `Agent` call that launched it.
#[derive(Debug, Default, Clone)]
pub(super) struct Orphan {
    tool_count: u32,
    current_action: String,
    last_ms: Option<i64>,
    /// `SubagentStop` arrived for it.
    stopped: bool,
}

/// One entry in claude's queue.
#[derive(Debug, Clone)]
struct QueueEntry {
    /// The enqueued text, as written, for `remove`, which names it whole.
    content: String,
    /// Its first words, for matching the prompt a dequeue delivered.
    key: String,
    /// Its `Queued` row, when it is a person's message.
    row: Option<usize>,
}

fn queue_key(text: &str) -> String {
    squeeze(text, 40)
}

/// Text claude queues for itself rather than for a person.
fn machine_queued(text: &str) -> bool {
    let text = text.trim_start();
    text.starts_with("<task-notification>") || text.starts_with("<agent-message")
}

/// Counts for the benchmark and for logs.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FoldStats {
    pub lines: u64,
    pub bytes: u64,
    pub gaps: u64,
}

/// One session's rows, folded from its transcripts and its hooks.
#[derive(Debug, Default)]
pub struct Projection {
    pub(super) rows: Vec<Row>,
    pub(super) index: HashMap<String, usize>,
    /// Parallel to `rows`: whether the row is already in `changed`.
    marked: Vec<bool>,
    changed: Vec<usize>,
    /// The turn the TRANSCRIPT is in. Hooks arrive ahead of the transcript, so
    /// a hook's turn is found by its own `prompt_id` rather than by this, or
    /// the tail of one turn would be filed under the next.
    pub(super) turn: Option<usize>,
    /// The newest `Turn` row, whichever source made it.
    pub(super) newest_turn: Option<usize>,
    /// The time of the transcript's previous record in this turn, which is
    /// when a thinking block began.
    turn_clock: Option<i64>,
    /// `agentId` to its `Subagent` row.
    pub(super) agents: HashMap<String, usize>,
    pub(super) orphans: HashMap<String, Orphan>,
    /// Tool calls counted in each subagent row's own transcript. The row shows
    /// the larger of this and the count its launch result reports, which
    /// covers the same calls: adding them would count each one twice.
    seen_tools: HashMap<usize, u32>,
    /// Claude's queue as its `queue-operation` records leave it, oldest
    /// first: every entry, including the ones that are not a person's message
    /// and so have no row.
    queue: Vec<QueueEntry>,
    /// Dequeues not yet matched to the prompt they delivered. A `dequeue`
    /// record names no entry, and claude does not take the oldest: replayed
    /// over 156 real files, a front-of-queue pop was wrong 1,506 times in
    /// 1,888. The prompt that follows says which it was.
    pending_dequeues: u32,
    /// Hashes of every line with a `uuid` already folded. A rewrite read again
    /// from its start, or a `/resume` back into a session already shown,
    /// re-reads lines this projection has; folding them twice would duplicate
    /// their rows. (Lines without a `uuid` are always folded; see `repeated`.)
    seen_lines: HashSet<u64>,
    /// Turns a `turn_duration` has already timed. A second one is not theirs.
    timed: HashSet<usize>,
    /// Hook `message_id` to the prose row it became or matched,
    /// and the last `index` applied to it.
    pub(super) hook_messages: HashMap<String, (usize, Option<u64>)>,
    /// The rows in `hook_messages`, so a match need not walk it.
    pub(super) claimed: HashSet<usize>,
    /// The turn row showing `activity`, so a new one clears just that one.
    activity_shown: Option<usize>,
    /// Turns a hook opened that the transcript has not yet written, so a
    /// confirmed turn retires them without walking every row.
    hook_turns: Vec<usize>,
    /// Uuid-less lines folded so far, by scope and line hash, as the most
    /// copies of each met in any one read of its file (see `repeated`).
    folded_copies: HashMap<u64, u32>,
    /// The same, for the read of each file now under way, by scope.
    read_copies: HashMap<String, HashMap<u64, u32>>,
    /// Held permissions not yet tied to the tool call they hold (`asks.rs`).
    pub(super) perm_waiting: Vec<super::asks::PermWait>,
    /// A tool call's id to the permission row that held it.
    pub(super) tool_asks: HashMap<String, usize>,
    /// Each subagent's tool calls that have not come back, as id, name and
    /// summary: what a permission one of its calls asks for is matched to.
    pub(super) sub_tools: HashMap<String, Vec<(String, String, String)>>,
    seq: u64,
    /// Bumped by every change; a row's `rev` is the value when it last moved.
    revision: u64,
    pub(super) activity: Option<Activity>,
    pub(super) session_id: Option<String>,
    stats: FoldStats,
}

/// Collapses whitespace, and cuts to `max` characters.
pub(super) fn squeeze(text: &str, max: usize) -> String {
    let mut out = String::with_capacity(text.len().min(max.saturating_mul(4)));
    for (n, word) in text.split_whitespace().enumerate() {
        if n > 0 {
            out.push(' ');
        }
        out.push_str(word);
        if out.len() > max.saturating_mul(4) {
            break;
        }
    }
    match out.char_indices().nth(max) {
        Some((cut, _)) => {
            out.truncate(cut);
            out.push('…');
            out
        }
        None => out,
    }
}

/// Cuts to `max` characters, keeping line breaks.
pub(super) fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

/// The one input field a person reads first, in the spike's order.
pub(super) fn summarize(input: &Input<'_>) -> String {
    let first = [
        &input.description,
        &input.command,
        &input.file_path,
        &input.pattern,
        &input.path,
        &input.subject,
        &input.url,
        &input.query,
    ]
    .into_iter()
    .find_map(|field| field.get());
    first.map(|s| squeeze(s, SUMMARY_CHARS)).unwrap_or_default()
}

/// The text between `<name>` and `</name>`.
fn tag<'t>(text: &'t str, name: &str) -> Option<&'t str> {
    let open = format!("<{name}>");
    let from = text.find(&open)? + open.len();
    let len = text[from..].find(&format!("</{name}>"))?;
    Some(&text[from..from + len])
}

fn clamp_end(started: Option<i64>, at: Option<i64>) -> Option<i64> {
    match (started, at) {
        (Some(s), Some(a)) => Some(a.max(s)),
        (_, a) => a,
    }
}

fn subagent_state(word: &str) -> SubagentState {
    match SubagentStatus::parse(word) {
        SubagentStatus::Completed => SubagentState::Completed,
        SubagentStatus::Failed => SubagentState::Failed,
        SubagentStatus::Killed => SubagentState::Killed,
        SubagentStatus::Stopped => SubagentState::Stopped,
    }
}

impl Projection {
    pub fn new() -> Projection {
        Projection::default()
    }

    /// A projection that knows which session it is, so a `SessionStart` for a
    /// different one can be told apart as a rebind.
    pub fn for_session(session_id: &str) -> Projection {
        Projection { session_id: Some(session_id.to_string()), ..Projection::default() }
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn row(&self, id: &str) -> Option<&Row> {
        self.index.get(id).map(|&i| &self.rows[i])
    }

    pub fn stats(&self) -> FoldStats {
        self.stats
    }

    /// The ids of every row added or changed since the last call, in row
    /// order. What a follower sends; the rows themselves are read by id.
    pub fn take_changed(&mut self) -> Vec<String> {
        let mut changed = std::mem::take(&mut self.changed);
        changed.sort_unstable();
        changed
            .into_iter()
            .map(|i| {
                self.marked[i] = false;
                self.rows[i].id.clone()
            })
            .collect()
    }

    /// The projection's current revision: every row with `rev` above a
    /// follower's last-seen value has changed since.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub(super) fn touch(&mut self, i: usize) {
        self.revision += 1;
        self.rows[i].rev = self.revision;
        if !self.marked[i] {
            self.marked[i] = true;
            self.changed.push(i);
        }
    }

    pub(super) fn push(&mut self, id: String, turn: Option<usize>, provisional: bool, kind: RowKind) -> usize {
        let i = self.rows.len();
        let turn = turn.map(|t| self.rows[t].id.clone());
        self.index.insert(id.clone(), i);
        self.rows.push(Row { ord: i as u64, rev: 0, id, turn, provisional, retracted: false, born: 0, kind });
        self.marked.push(false);
        self.touch(i);
        self.rows[i].born = self.revision;
        i
    }

    pub(super) fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// A gap: one row per reason per turn, counting. Unknown records are
    /// scattered among known ones (a coordinator session writes a
    /// `frame-link` between most of its turns' records), so folding only into
    /// the row just before would still draw hundreds of them.
    pub(super) fn gap(&mut self, reason: GapReason) {
        self.stats.gaps += 1;
        let turn = self.turn.map_or_else(|| "-".to_string(), |t| self.rows[t].id.clone());
        let id = format!("gap:{turn}:{reason:?}");
        if let Some(&i) = self.index.get(&id) {
            if let RowKind::Gap(gap) = &mut self.rows[i].kind {
                gap.count += 1;
            }
            self.touch(i);
            return;
        }
        self.push(id, self.turn, false, RowKind::Gap(Gap { reason, count: 1 }));
    }

    /// Whether `line` is a record already folded, noting it if not.
    ///
    /// Only a record with a `uuid` can be a repeat: claude gives every
    /// message, prompt and system record one, so a second copy of such a line
    /// is the same record read again. A record without one (`queue-operation`
    /// above all) can be written twice on purpose: two identical dequeues in
    /// the same millisecond are two dequeues, 44 times in the real corpus.
    ///
    /// Those are counted instead: the n-th copy met in a read of a file is a
    /// repeat when an earlier read already folded n of them. So a `/resume`
    /// back into a session already shown, which reads its file again from
    /// the start, adds no second Queued row for each enqueue (ov-366), and
    /// twin dequeues in one read still both fold.
    pub(super) fn repeated(&mut self, scope: &str, record: &Record<'_>, line: &[u8]) -> bool {
        if record.uuid.get().is_some() {
            return self.seen(scope, line);
        }
        let key = Self::line_key(scope, line);
        let met = self.read_copies.entry(scope.to_string()).or_default().entry(key).or_default();
        *met += 1;
        let folded = self.folded_copies.entry(key).or_default();
        if *met <= *folded {
            return true;
        }
        *folded = *met;
        false
    }

    /// A file of this scope is being read again from its start (`None`:
    /// every file is), so its uuid-less lines are counted afresh.
    pub fn reread(&mut self, scope: Option<&str>) {
        match scope {
            Some(scope) => {
                self.read_copies.remove(scope);
            }
            None => self.read_copies.clear(),
        }
    }

    fn line_key(scope: &str, line: &[u8]) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        scope.hash(&mut hasher);
        line.hash(&mut hasher);
        hasher.finish()
    }

    /// Whether `line` has been folded already, noting it if not.
    fn seen(&mut self, scope: &str, line: &[u8]) -> bool {
        !self.seen_lines.insert(Self::line_key(scope, line))
    }

    /// A turn that failed, whatever ended it first. The transcript writes a
    /// failure as a reply and a `turn_duration` like any other, so a
    /// `Finished` already there is overridden.
    pub(super) fn fail_turn(&mut self, i: usize, detail: &str, at: Option<i64>) {
        let failed = TurnOutcome::Failed { detail: squeeze(detail, LINE_CHARS) };
        let finished = matches!(self.turn_mut(i), Some(t) if t.outcome == Some(TurnOutcome::Finished));
        if finished {
            if let Some(t) = self.turn_mut(i) {
                t.outcome = Some(failed);
            }
            self.touch(i);
        } else {
            self.end_turn(i, at, failed);
        }
    }

    pub(super) fn notice(&mut self, kind: NoticeKind, text: String, at: Option<i64>, provisional: bool) -> usize {
        let id = format!("notice:{}", self.next_seq());
        self.push(id, self.turn, provisional, RowKind::Notice(Notice { kind, text, at_ms: at }))
    }

    /// A screen-only state the view must hand to the terminal.
    pub fn handoff(&mut self, reason: &str, at: Option<i64>) {
        let id = format!("handoff:{}", self.next_seq());
        let turn = self.newest_turn;
        self.push(id, turn, false, RowKind::Handoff(Handoff { reason: reason.to_string(), at_ms: at }));
    }

    /// What claude's registry says the process is doing, shown on the newest
    /// turn and cleared from the one before it.
    pub fn set_activity(&mut self, activity: Activity) {
        self.activity = Some(activity);
        self.show_activity();
    }

    pub(super) fn show_activity(&mut self) {
        let Some(newest) = self.newest_turn else { return };
        let activity = self.activity;
        let shown = self.activity_shown.replace(newest);
        for (i, want) in [(shown.filter(|&s| s != newest), None), (Some(newest), activity)] {
            let Some(i) = i else { continue };
            if let RowKind::Turn(turn) = &mut self.rows[i].kind {
                if turn.activity != want {
                    turn.activity = want;
                    self.touch(i);
                }
            }
        }
    }

    /// Rows that can belong to turn `turn`: a turn's rows are all added after
    /// it, so the walk starts there rather than at the session's start.
    pub(super) fn from_turn(&self, turn: usize) -> std::ops::Range<usize> {
        turn..self.rows.len()
    }

    pub(super) fn turn_mut(&mut self, i: usize) -> Option<&mut Turn> {
        match &mut self.rows[i].kind {
            RowKind::Turn(turn) => Some(turn),
            _ => None,
        }
    }

    /// End a turn that has not ended, with `outcome`, at `at`.
    pub(super) fn end_turn(&mut self, i: usize, at: Option<i64>, outcome: TurnOutcome) {
        let Some(turn) = self.turn_mut(i) else { return };
        if turn.outcome.is_some() {
            return;
        }
        turn.ended_ms = clamp_end(turn.started_ms, at);
        turn.duration_ms = match (turn.started_ms, turn.ended_ms) {
            (Some(s), Some(e)) => Some(e - s),
            _ => None,
        };
        turn.outcome = Some(outcome);
        self.touch(i);
        self.settle_asks(i, at);
    }

    /// Asks a turn left open are over when the turn is: the keyboard answered
    /// them, or the turn was cut short.
    fn settle_asks(&mut self, turn: usize, at: Option<i64>) {
        let id = self.rows[turn].id.clone();
        for i in self.from_turn(turn) {
            if self.rows[i].turn.as_deref() != Some(&id) {
                continue;
            }
            let row = &mut self.rows[i];
            if let RowKind::Ask(ask) = &mut row.kind {
                // Over with its turn, and no record will confirm it after:
                // the hook that held it is its record.
                if !ask.answered || row.provisional {
                    if !ask.answered {
                        ask.answered = true;
                        ask.answered_ms = at;
                    }
                    row.provisional = false;
                    self.touch(i);
                }
            }
        }
        self.perm_waiting.retain(|w| self.rows[w.ask].provisional);
    }

    /// Open a turn, or confirm the one a hook already opened for this prompt.
    pub(super) fn open_turn(
        &mut self,
        id: String,
        origin: TurnOrigin,
        prompt: &str,
        at: Option<i64>,
        provisional: bool,
    ) -> usize {
        if let Some(&i) = self.index.get(&id) {
            // The same prompt record a second time (a `/resume` into a
            // session already shown re-reads it) must not take the
            // transcript back to a turn it has left.
            let left = !self.rows[i].provisional && self.turn.is_some_and(|t| t != i);
            if !provisional && !left {
                let row = &mut self.rows[i];
                row.provisional = false;
                if let RowKind::Turn(turn) = &mut row.kind {
                    // Written after all: a rebuild that read the file after the
                    // hook arrived saw older turns first.
                    if turn.outcome == Some(TurnOutcome::Unrecorded) {
                        turn.outcome = None;
                        turn.ended_ms = None;
                        turn.duration_ms = None;
                    }
                    turn.started_ms = at.or(turn.started_ms);
                    turn.origin = origin;
                    if turn.prompt.is_empty() || origin != TurnOrigin::Other {
                        turn.prompt = clip(prompt, PROMPT_CHARS);
                    }
                }
                self.touch(i);
                self.enter_turn(i, at);
            }
            return i;
        }
        let turn = Turn {
            prompt: clip(prompt, PROMPT_CHARS),
            origin,
            started_ms: at,
            ended_ms: None,
            duration_ms: None,
            outcome: None,
            background_running: 0,
            activity: None,
        };
        let i = self.push(id, None, provisional, RowKind::Turn(turn));
        self.newest_turn = Some(i);
        self.show_activity();
        if provisional {
            self.hook_turns.push(i);
        }
        if !provisional {
            self.enter_turn(i, at);
        }
        i
    }

    /// Make `i` the transcript's turn, ending the one before it if nothing
    /// did. Most turns end with a `turn_duration`; SDK sessions and a turn cut
    /// off by a crash never write one, and a new prompt is proof it is over.
    fn enter_turn(&mut self, i: usize, at: Option<i64>) {
        if self.turn == Some(i) {
            return;
        }
        self.retire_unrecorded(i, at);
        if let Some(previous) = self.turn {
            let end = self.turn_clock.or(at);
            self.end_turn(previous, end, TurnOutcome::Finished);
            self.settle_prose(previous);
        }
        self.turn = Some(i);
        self.turn_clock = at;
    }

    /// Turns a hook opened before `confirmed` that the transcript has now
    /// passed without writing.
    fn retire_unrecorded(&mut self, confirmed: usize, at: Option<i64>) {
        let open = |rows: &[Row], i: usize| rows[i].provisional && matches!(&rows[i].kind, RowKind::Turn(t) if t.outcome.is_none());
        let passed: Vec<usize> = self.hook_turns.iter().copied().filter(|&i| i < confirmed && open(&self.rows, i)).collect();
        for i in passed {
            self.end_turn(i, at, TurnOutcome::Unrecorded);
        }
        let rows = &self.rows;
        self.hook_turns.retain(|&i| open(rows, i));
    }

    /// The transcript's turn, opening a placeholder when a record arrives with
    /// none: a file read from the middle, or a session that began before the
    /// first prompt was written.
    ///
    /// Also when the transcript's turn is over and timed but claude carries
    /// on with no prompt record (seen in real sessions: a run of replies and
    /// a second `turn_duration` with no prompt between them). That work is a
    /// turn of its own, and its `turn_duration` is not the last turn's.
    fn ensure_turn(&mut self, at: Option<i64>) -> usize {
        if let Some(i) = self.turn.filter(|i| !self.timed.contains(i)) {
            return i;
        }
        let id = format!("turn:resumed:{}", self.next_seq());
        self.open_turn(id, TurnOrigin::Other, "", at, false)
    }

    // -----------------------------------------------------------------
    // The main transcript
    // -----------------------------------------------------------------

    /// Fold one complete line of the main transcript.
    pub fn fold_line(&mut self, line: &[u8]) {
        self.stats.lines += 1;
        self.stats.bytes += line.len() as u64;
        let Some(record) = decode(line) else {
            self.gap(GapReason::Unparsed);
            return;
        };
        if self.repeated("", &record, line) {
            return;
        }
        let at = record.timestamp.get().and_then(parse_iso8601_millis);
        match record.kind.get() {
            Some("user") => self.user(&record, at),
            Some("assistant") => self.assistant(&record, at),
            Some("system") => self.system(&record, at),
            Some("queue-operation") => self.queue_operation(&record, at),
            Some("attachment") => self.attachment(&record, at),
            Some(kind) if SILENT_TYPES.contains(&kind) => {}
            Some(kind) => self.gap(GapReason::Unknown(kind.to_string())),
            None => self.gap(GapReason::Unknown(String::new())),
        }
    }

    /// A line the reader skipped for its size.
    pub fn fold_too_large(&mut self, bytes: u64) {
        self.stats.lines += 1;
        self.stats.bytes += bytes;
        self.gap(GapReason::TooLarge);
    }

    /// The file was replaced or shrank; what follows is read from its start.
    pub fn fold_rewritten(&mut self) {
        self.gap(GapReason::Rewritten);
    }

    fn user(&mut self, record: &Record<'_>, at: Option<i64>) {
        let message = record.message.0.as_ref();
        let content = message.map(|m| &m.content);
        let mut text: Option<&str> = None;
        let mut results = false;
        match content {
            Some(Content::Text(t)) => text = Some(t),
            Some(Content::Blocks(blocks)) => {
                for block in blocks.iter().filter_map(|b| b.0.as_ref()) {
                    match block.kind.get() {
                        Some("tool_result") => {
                            results = true;
                            self.tool_result(block, record.tool_use_result.0.as_ref(), at);
                        }
                        Some("text") if text.is_none() => text = block.text.get(),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        // A companion record (an image's source line) shares its prompt's
        // promptId and opens nothing. But claude also writes whole prompts as
        // isMeta: a scheduled heartbeat, a message from another session, each
        // with its own new promptId and `promptSource: system` (1,707 of them
        // in 156 real files), and those are turns.
        let companion = record.is_meta.yes()
            && record.prompt_id.get().is_none_or(|p| self.index.contains_key(&format!("turn:{p}")) || record.prompt_source.get().is_none());
        if results || companion || record.is_compact_summary.yes() {
            self.turn_clock = at.or(self.turn_clock);
            return;
        }
        let Some(text) = text else { return };
        let trimmed = text.trim_start();
        if trimmed.starts_with("[Request interrupted by user") {
            if let Some(turn) = self.turn {
                self.end_turn(turn, at, TurnOutcome::Interrupted);
                self.settle_prose(turn);
            }
            return;
        }
        if let Some(body) = trimmed.strip_prefix("<task-notification>") {
            self.task_notification(body, at);
        } else if let Some(name) = trimmed.starts_with("<command-name>").then(|| tag(trimmed, "command-name")).flatten() {
            self.notice(NoticeKind::Command, name.trim().to_string(), at, false);
            return;
        } else if trimmed.starts_with("<local-command-stdout>") || trimmed.starts_with("<local-command-caveat>") {
            return;
        }
        let Some(source) = record.prompt_source.get() else {
            if record.prompt_id.get().is_none() {
                return;
            }
            let id = format!("turn:{}", record.prompt_id.get().unwrap_or_default());
            self.open_turn(id, TurnOrigin::Other, text, at, false);
            return;
        };
        let origin = match source {
            "typed" | "suggestion_accepted" => TurnOrigin::Typed,
            "queued" => TurnOrigin::Queued,
            "system" if trimmed.starts_with("<task-notification>") => TurnOrigin::Notification,
            "system" => TurnOrigin::System,
            "sdk" => TurnOrigin::Sdk,
            _ => TurnOrigin::Other,
        };
        let id = match (record.prompt_id.get(), record.uuid.get()) {
            (Some(p), _) => format!("turn:{p}"),
            (None, Some(u)) => format!("turn:u:{u}"),
            (None, None) => format!("turn:{}", self.next_seq()),
        };
        let prompt = match origin {
            TurnOrigin::Notification => tag(trimmed, "summary").unwrap_or("A background task finished"),
            _ => text,
        };
        self.delivered(text);
        self.open_turn(id, origin, prompt, at, false);
    }

    /// A prompt arrived: if a dequeue is waiting to be matched, this is what
    /// it took. The entry whose first words are the prompt's, else (claude
    /// rewrites a peer's `<agent-message>` before delivering it) the newest
    /// entry that is no person's message.
    fn delivered(&mut self, prompt: &str) {
        if self.pending_dequeues == 0 {
            return;
        }
        self.pending_dequeues -= 1;
        let key = queue_key(prompt);
        let found = self
            .queue
            .iter()
            .position(|e| e.key == key)
            .or_else(|| self.queue.iter().rposition(|e| e.row.is_none()));
        if let Some(n) = found {
            let entry = self.queue.remove(n);
            if let Some(i) = entry.row {
                self.set_queued(i, QueuedState::Sent);
            }
        }
    }

    /// `queued_command`: a queued message claude took into the running turn.
    /// A background agent's notification is delivered this way when it lands
    /// mid-turn, and for 834 of 1,164 real async agents it was the only
    /// record of their end (claude.rs's `notified` reads it too).
    fn attachment(&mut self, record: &Record<'_>, at: Option<i64>) {
        let Some(attachment) = record.attachment.0.as_ref() else { return };
        if attachment.kind.get() != Some("queued_command") {
            return;
        }
        let prompt = attachment.prompt.get().unwrap_or_default().trim_start();
        if let Some(body) = prompt.strip_prefix("<task-notification>") {
            self.task_notification(body, at);
        }
    }

    fn assistant(&mut self, record: &Record<'_>, at: Option<i64>) {
        let turn = self.ensure_turn(at);
        let Some(message) = record.message.0.as_ref() else { return };
        // Claude reporting a failed request in the model's place: the turn
        // failed, and the text is why, not something the model said.
        if record.is_api_error.yes() {
            let detail = match &message.content {
                Content::Blocks(blocks) => blocks.iter().filter_map(|b| b.0.as_ref()).find_map(|b| b.text.get()),
                Content::Text(t) => Some(t.as_ref()),
                Content::None => None,
            };
            self.fail_turn(turn, detail.unwrap_or("The request failed"), at);
            self.turn_clock = at.or(self.turn_clock);
            return;
        }
        let ends = message.stop_reason.get() == Some("end_turn");
        let uuid = record.uuid.get().map(str::to_string).unwrap_or_else(|| format!("r{}", self.next_seq()));
        if let Content::Blocks(blocks) = &message.content {
            let last_text = blocks
                .iter()
                .rposition(|b| b.0.as_ref().is_some_and(|b| b.kind.get() == Some("text") && b.text.get().is_some_and(|t| !t.trim().is_empty())));
            for (n, block) in blocks.iter().enumerate() {
                let Some(block) = block.0.as_ref() else { continue };
                match block.kind.get() {
                    Some("text") => {
                        let Some(text) = block.text.get().filter(|t| !t.trim().is_empty()) else { continue };
                        self.prose(turn, format!("prose:{uuid}:{n}"), text, ends && Some(n) == last_text, at);
                    }
                    Some("thinking") | Some("redacted_thinking") => {
                        let started = self.turn_clock;
                        let kind = RowKind::Thinking(Thinking { started_ms: started, ended_ms: clamp_end(started, at) });
                        self.push(format!("think:{uuid}:{n}"), Some(turn), false, kind);
                    }
                    Some("tool_use") => self.tool_use(turn, block, at, false),
                    _ => {}
                }
            }
        }
        if ends {
            self.end_turn(turn, at, TurnOutcome::Finished);
        }
        self.turn_clock = at.or(self.turn_clock);
    }

    /// A text block, confirming the hook's provisional prose for it when one
    /// is waiting: the oldest unconfirmed row in this turn whose words are the
    /// start of these.
    fn prose(&mut self, turn: usize, id: String, text: &str, conclusion: bool, at: Option<i64>) {
        let turn_id = self.rows[turn].id.clone();
        let words = squeeze(text, usize::MAX);
        let waiting = self.from_turn(turn).find(|&i| {
            let row = &self.rows[i];
            row.provisional
                && row.turn.as_deref() == Some(&turn_id)
                && matches!(&row.kind, RowKind::Prose(p) if !p.text.is_empty() && words.starts_with(&squeeze(&p.text, usize::MAX)))
        });
        let prose = Prose { text: text.to_string(), conclusion, at_ms: at };
        match waiting {
            Some(i) => {
                self.rows[i].provisional = false;
                self.rows[i].kind = RowKind::Prose(prose);
                self.index.insert(id, i);
                self.touch(i);
            }
            None => {
                self.push(id, Some(turn), false, RowKind::Prose(prose));
            }
        }
    }

    /// An `Agent`, a question, a plan exit, or any other tool. `provisional`
    /// when a `PreToolUse` hook announced it ahead of the transcript.
    pub(super) fn tool_use(&mut self, turn: usize, block: &Block<'_>, at: Option<i64>, provisional: bool) {
        let Some(id) = block.id.get() else { return };
        let name = block.name.get().unwrap_or_default();
        let empty = Input::default();
        let input = block.input.0.as_ref().unwrap_or(&empty);
        let (row_id, kind) = match name {
            "Agent" | "Task" => {
                let sub = Subagent {
                    tool_use_id: id.to_string(),
                    agent_id: None,
                    agent_type: input.subagent_type.get().unwrap_or("general-purpose").to_string(),
                    description: input.description.get().map(|d| squeeze(d, SUMMARY_CHARS)).unwrap_or_default(),
                    background: input.run_in_background.yes(),
                    status: SubagentState::Running,
                    started_ms: at,
                    ended_ms: None,
                    tool_count: 0,
                    current_action: String::new(),
                    last_ms: at,
                };
                (format!("sub:{id}"), RowKind::Subagent(sub))
            }
            "AskUserQuestion" | "ExitPlanMode" => {
                let (kind, text) = if name == "ExitPlanMode" {
                    (AskKind::PlanExit, input.plan.get().map(|p| squeeze(p, LINE_CHARS)).unwrap_or_default())
                } else {
                    let question = input.questions.0.first().and_then(|q| q.0.as_ref()).and_then(|q| q.question.get());
                    (AskKind::Question, question.map(|q| squeeze(q, LINE_CHARS)).unwrap_or_default())
                };
                let ask = Ask { kind, text, tool: Some(name.to_string()), asked_ms: at, answered_ms: None, answered: false };
                (format!("ask:{id}"), RowKind::Ask(ask))
            }
            _ => {
                let tool = Tool {
                    name: name.to_string(),
                    summary: summarize(input),
                    status: ToolStatus::Running,
                    started_ms: at,
                    ended_ms: None,
                    diff: Vec::new(),
                    file_path: input.file_path.get().map(str::to_string),
                };
                (format!("tool:{id}"), RowKind::Tool(tool))
            }
        };
        if let Some(&i) = self.index.get(&row_id) {
            // A hook announced it; the transcript confirms it, and its own
            // time and input are the record.
            if !provisional && self.rows[i].provisional {
                self.rows[i].provisional = false;
                let keep_status = std::mem::replace(&mut self.rows[i].kind, kind);
                if let (RowKind::Tool(old), RowKind::Tool(new)) = (&keep_status, &mut self.rows[i].kind) {
                    if old.status != ToolStatus::Running {
                        new.status = old.status;
                        new.ended_ms = clamp_end(new.started_ms, old.ended_ms);
                    }
                }
                self.touch(i);
                self.tool_confirmed(id);
            }
            return;
        }
        let summary = match &kind {
            RowKind::Tool(tool) => Some((tool.name.clone(), tool.summary.clone())),
            _ => None,
        };
        self.push(row_id, Some(turn), provisional, kind);
        if let Some((name, summary)) = summary {
            self.link_tool(None, id, &name, &summary, !provisional);
        }
    }

    fn tool_result(&mut self, block: &Block<'_>, result: Option<&ToolUseResult<'_>>, at: Option<i64>) {
        let Some(id) = block.tool_use_id.get() else { return };
        let failed = block.is_error.yes();
        if let Some(&i) = self.index.get(&format!("tool:{id}")) {
            let is_tool = matches!(self.rows[i].kind, RowKind::Tool(_));
            if let RowKind::Tool(tool) = &mut self.rows[i].kind {
                tool.status = if failed { ToolStatus::Failed } else { ToolStatus::Done };
                tool.ended_ms = clamp_end(tool.started_ms, at);
                if let Some(result) = result {
                    tool.diff = result
                        .structured_patch
                        .0
                        .iter()
                        .filter_map(|h| h.0.as_ref())
                        .map(|h| Hunk {
                            old_start: h.old_start.int().unwrap_or(0).max(0) as u32,
                            old_lines: h.old_lines.int().unwrap_or(0).max(0) as u32,
                            new_start: h.new_start.int().unwrap_or(0).max(0) as u32,
                            new_lines: h.new_lines.int().unwrap_or(0).max(0) as u32,
                            lines: h.lines.0.iter().filter_map(|l| l.get().map(str::to_string)).collect(),
                        })
                        .collect();
                    if let Some(path) = result.file_path.get() {
                        tool.file_path = Some(path.to_string());
                    }
                }
            }
            self.rows[i].provisional = false;
            self.touch(i);
            if is_tool {
                self.tool_confirmed(id);
                self.tool_done(id, at);
            }
            return;
        }
        if let Some(&i) = self.index.get(&format!("sub:{id}")) {
            self.subagent_result(i, failed, result, at);
            return;
        }
        if let Some(&i) = self.index.get(&format!("ask:{id}")) {
            if let RowKind::Ask(ask) = &mut self.rows[i].kind {
                ask.answered = true;
                ask.answered_ms = at;
            }
            self.touch(i);
        }
    }

    /// The `Agent` call `i` came back: launched in the background, or over.
    pub(super) fn subagent_result(&mut self, i: usize, failed: bool, result: Option<&ToolUseResult<'_>>, at: Option<i64>) {
        if let Some(result) = result {
            if let Some(agent_id) = result.agent_id.get() {
                self.join_agent(agent_id, i);
            }
            if let RowKind::Subagent(sub) = &mut self.rows[i].kind {
                if let Some(count) = result.total_tool_use_count.int() {
                    sub.tool_count = sub.tool_count.max(count.max(0) as u32);
                }
                if let Some(kind) = result.agent_type.get() {
                    sub.agent_type = kind.to_string();
                }
            }
        }
        let status = result.and_then(|r| r.status.get());
        match status {
            Some("async_launched") => {
                if let RowKind::Subagent(sub) = &mut self.rows[i].kind {
                    sub.background = true;
                }
                self.touch(i);
            }
            _ => {
                let state = if failed { SubagentState::Failed } else { subagent_state(status.unwrap_or("completed")) };
                self.end_subagent(i, state, at);
            }
        }
        self.count_background(i);
    }

    pub(super) fn end_subagent(&mut self, i: usize, state: SubagentState, at: Option<i64>) {
        if let RowKind::Subagent(sub) = &mut self.rows[i].kind {
            // The first end stands: a notification can repeat, and claude
            // writes one ending as several records (queue, attachment, prompt).
            if sub.status != SubagentState::Running && sub.ended_ms.is_some() {
                return;
            }
            sub.status = state;
            sub.ended_ms = clamp_end(sub.started_ms, at);
        }
        self.touch(i);
        self.count_background(i);
        if let RowKind::Subagent(Subagent { agent_id: Some(agent), .. }) = &self.rows[i].kind {
            let agent = agent.clone();
            self.subagent_asks_over(&agent, at);
        }
    }

    /// Tie an `agentId` to its row, and give the row whatever its own
    /// transcript said before the tie was known.
    pub(super) fn join_agent(&mut self, agent_id: &str, i: usize) {
        self.agents.insert(agent_id.to_string(), i);
        let orphan = self.orphans.remove(agent_id);
        if let RowKind::Subagent(sub) = &mut self.rows[i].kind {
            sub.agent_id = Some(agent_id.to_string());
            if let Some(orphan) = &orphan {
                let seen = self.seen_tools.entry(i).or_default();
                *seen += orphan.tool_count;
                sub.tool_count = sub.tool_count.max(*seen);
                if !orphan.current_action.is_empty() {
                    sub.current_action = orphan.current_action.clone();
                }
                sub.last_ms = sub.last_ms.max(orphan.last_ms);
            }
        }
        self.touch(i);
        if orphan.is_some_and(|o| o.stopped) {
            self.end_subagent(i, SubagentState::Completed, None);
        }
    }

    /// Recount the background agents still running for the turn row `sub`
    /// belongs to.
    fn count_background(&mut self, sub: usize) {
        let Some(turn_id) = self.rows[sub].turn.clone() else { return };
        let Some(&t) = self.index.get(&turn_id) else { return };
        let running = self.rows[self.from_turn(t)]
            .iter()
            .filter(|r| r.turn.as_deref() == Some(&turn_id))
            .filter(|r| matches!(&r.kind, RowKind::Subagent(s) if s.background && s.status == SubagentState::Running))
            .count() as u32;
        if let Some(turn) = self.turn_mut(t) {
            if turn.background_running != running {
                turn.background_running = running;
                self.touch(t);
            }
        }
    }

    /// `<task-notification>`: a background agent (or shell) stopped. It may
    /// repeat for one agent, once per stop.
    pub(super) fn task_notification(&mut self, body: &str, at: Option<i64>) {
        let agent = tag(body, "task-id").map(str::trim);
        let tool = tag(body, "tool-use-id").map(str::trim);
        let status = tag(body, "status").unwrap_or("completed");
        let row = agent
            .and_then(|a| self.agents.get(a).copied())
            .or_else(|| tool.and_then(|t| self.index.get(&format!("sub:{t}")).copied()));
        if let Some(i) = row {
            if let (Some(agent), RowKind::Subagent(sub)) = (agent, &self.rows[i].kind) {
                if sub.agent_id.is_none() {
                    self.join_agent(agent, i);
                }
            }
            self.end_subagent(i, subagent_state(status), at);
        }
    }

    fn system(&mut self, record: &Record<'_>, at: Option<i64>) {
        match record.subtype.get() {
            Some("turn_duration") => {
                let Some(turn) = self.turn else { return };
                if !self.timed.insert(turn) {
                    return;
                }
                self.end_turn(turn, at, TurnOutcome::Finished);
                if let Some(ms) = record.duration_ms.int() {
                    if let Some(t) = self.turn_mut(turn) {
                        t.duration_ms = Some(ms.max(0));
                    }
                }
                self.rows[turn].provisional = false;
                self.touch(turn);
                self.settle_prose(turn);
            }
            Some("compact_boundary") => {
                let trigger = record.compact.0.as_ref().and_then(|c| c.trigger.get()).unwrap_or("");
                let text = if trigger == "auto" { "Context compacted automatically" } else { "Context compacted" };
                // A `SessionStart` hook with `source: compact` may already have
                // put a provisional notice up; this confirms it.
                let waiting = (0..self.rows.len()).rev().find(|&i| {
                    self.rows[i].provisional && matches!(&self.rows[i].kind, RowKind::Notice(n) if n.kind == NoticeKind::Compacted)
                });
                match waiting {
                    Some(i) => {
                        self.rows[i].provisional = false;
                        if let RowKind::Notice(n) = &mut self.rows[i].kind {
                            n.text = text.to_string();
                            n.at_ms = at;
                        }
                        self.touch(i);
                    }
                    None => {
                        self.notice(NoticeKind::Compacted, text.to_string(), at, false);
                    }
                }
            }
            Some("local_command") => {
                // Only the record that names the command; the one carrying its
                // output (`<local-command-stdout>`) is not a notice.
                let content = record.content.get().unwrap_or_default().trim_start();
                let name = content.starts_with("<command-name>").then(|| tag(content, "command-name")).flatten().unwrap_or("").trim();
                if !name.is_empty() {
                    self.notice(NoticeKind::Command, name.to_string(), at, false);
                }
            }
            Some("api_error") => {
                let text = record
                    .error
                    .0
                    .as_ref()
                    .and_then(|e| e.formatted.get().or(e.message.get()))
                    .unwrap_or("The request failed");
                self.notice(NoticeKind::ApiError, squeeze(text, LINE_CHARS), at, false);
            }
            _ => {}
        }
    }

    fn queue_operation(&mut self, record: &Record<'_>, at: Option<i64>) {
        let content = record.content.get().unwrap_or_default();
        match record.operation.get() {
            Some("enqueue") => {
                // 131 real files begin with an enqueue that carries nothing.
                if content.trim().is_empty() {
                    return;
                }
                let row = (!machine_queued(content)).then(|| {
                    let id = format!("queued:{}", self.next_seq());
                    let queued = Queued { text: squeeze(content, LINE_CHARS), state: QueuedState::Waiting, at_ms: at };
                    self.push(id, self.turn, false, RowKind::Queued(queued))
                });
                // claude.rs's `notified` reads an enqueued notification as the
                // agent's end; so does this.
                if let Some(body) = content.trim_start().strip_prefix("<task-notification>") {
                    self.task_notification(body, at);
                }
                self.queue.push(QueueEntry { content: content.to_string(), key: queue_key(content), row });
            }
            Some("dequeue") => self.pending_dequeues += 1,
            Some("remove") => {
                let Some(n) = self.queue.iter().position(|e| e.content == content) else { return };
                let entry = self.queue.remove(n);
                // Absorbed into the running turn, or handed to the agent: sent
                // either way. Only a remove with no such reason is taken back.
                let sent = matches!(record.reason.get(), Some("absorbed_mid_turn" | "delivered_to_agent"));
                if let Some(i) = entry.row {
                    self.set_queued(i, if sent { QueuedState::Sent } else { QueuedState::Withdrawn });
                }
            }
            // Everything waiting goes back to the input box.
            Some("popAll") => {
                for entry in std::mem::take(&mut self.queue) {
                    if let Some(i) = entry.row {
                        self.set_queued(i, QueuedState::Withdrawn);
                    }
                }
                self.pending_dequeues = 0;
            }
            _ => {}
        }
    }

    fn set_queued(&mut self, i: usize, state: QueuedState) {
        if let RowKind::Queued(q) = &mut self.rows[i].kind {
            q.state = state;
        }
        self.touch(i);
    }

    // -----------------------------------------------------------------
    // Subagent transcripts
    // -----------------------------------------------------------------

    /// Fold one line of `subagents/agent-<agent_id>.jsonl`. `meta` is its
    /// `.meta.json`, when it has been written: its `toolUseId` is the join
    /// onto the parent's `Agent` call, available before the parent's result
    /// names the `agentId` (which, for a foreground agent, is not until it
    /// ends).
    pub fn fold_subagent_line(&mut self, agent_id: &str, meta: Option<&SubagentMeta>, line: &[u8]) {
        self.stats.lines += 1;
        self.stats.bytes += line.len() as u64;
        if !self.agents.contains_key(agent_id) {
            if let Some(meta) = meta {
                self.join_by_meta(agent_id, meta);
            }
        }
        let Some(record) = decode(line) else {
            self.stats.gaps += 1;
            return;
        };
        if self.repeated(agent_id, &record, line) {
            return;
        }
        let at = record.timestamp.get().and_then(parse_iso8601_millis);
        self.subagent_record(agent_id, &record, at);
        let mut tools = 0u32;
        let mut action = None;
        if record.kind.get() == Some("assistant") {
            if let Some(Content::Blocks(blocks)) = record.message.0.as_ref().map(|m| &m.content) {
                for block in blocks.iter().filter_map(|b| b.0.as_ref()) {
                    if block.kind.get() == Some("tool_use") {
                        tools += 1;
                        let summary = block.input.0.as_ref().map(summarize).unwrap_or_default();
                        let name = block.name.get().unwrap_or("Tool");
                        action = Some(if summary.is_empty() { name.to_string() } else { format!("{name} {summary}") });
                    }
                }
            }
        }
        match self.agents.get(agent_id).copied() {
            Some(i) => {
                if let RowKind::Subagent(sub) = &mut self.rows[i].kind {
                    let seen = self.seen_tools.entry(i).or_default();
                    *seen += tools;
                    sub.tool_count = sub.tool_count.max(*seen);
                    if let Some(action) = action {
                        sub.current_action = action;
                    }
                    sub.last_ms = sub.last_ms.max(at);
                }
                self.touch(i);
            }
            None => {
                let orphan = self.orphans.entry(agent_id.to_string()).or_default();
                orphan.tool_count += tools;
                if let Some(action) = action {
                    orphan.current_action = action;
                }
                orphan.last_ms = orphan.last_ms.max(at);
            }
        }
    }

    /// Whether a subagent's `agentId` is tied to its row yet.
    pub fn is_joined(&self, agent_id: &str) -> bool {
        self.agents.contains_key(agent_id)
    }

    /// `SubagentStop` for an agent no row has claimed yet: remembered, and
    /// applied when the join arrives.
    pub(super) fn stop_orphan(&mut self, agent_id: &str) {
        self.orphans.entry(agent_id.to_string()).or_default().stopped = true;
    }

    /// Join an agent to its row through its meta file, and take what the meta
    /// says about it that the `Agent` call did not.
    pub fn join_by_meta(&mut self, agent_id: &str, meta: &SubagentMeta) {
        let Some(tool) = meta.tool_use_id.as_deref() else { return };
        let Some(&i) = self.index.get(&format!("sub:{tool}")) else { return };
        if let RowKind::Subagent(sub) = &mut self.rows[i].kind {
            if let Some(kind) = &meta.agent_type {
                sub.agent_type = kind.clone();
            }
            if sub.description.is_empty() {
                sub.description = meta.description.as_deref().map(|d| squeeze(d, SUMMARY_CHARS)).unwrap_or_default();
            }
            if meta.request_shape.as_deref() == Some("background") {
                sub.background = true;
            }
        }
        self.join_agent(agent_id, i);
        self.count_background(i);
    }
}

