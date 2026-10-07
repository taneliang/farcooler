//! Codex's rollout and hooks, folded into the same rows as claude's (ov-378).
//!
//! The design is `.claude/agent/reports/codex-projection/design.md`. A codex
//! rollout (`$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<ts>-<thread>.jsonl`)
//! says outright where a turn starts and ends (`task_started`,
//! `task_complete`, `turn_aborted`, each naming its `turn_id`), and codex's
//! hooks name the same `turn_id` and, for a tool, the record's `call_id` as
//! their `tool_use_id`. So a hook's provisional row and the record that
//! confirms it share an id, and nothing is matched by its words.
//!
//! Two shapes of the same facts are on disk, and both are read: before
//! codex 0.147 a step is an `event_msg` of its own (`user_message`,
//! `agent_message`, `patch_apply_end`); since, an `event_msg/item_completed`
//! whose `item.type` says what it was. `response_item`, the model's own
//! history, is in both. An item and its `response_item` carry one id
//! (`msg_…`, `rs_…`, a call's id), so whichever comes first makes the row
//! and the other finds it.
//!
//! Measured on codex-cli 0.153.4 against a stand-in API (design, "What the
//! spike measured"): codex flushes each step within about 50 ms, in
//! `ordinal` order; an Enter while a turn runs steers that turn (a second
//! `UserMessage` under the same `turn_id`); `!cmd` is a turn with a
//! `CommandExecution` from `user_shell` and no hook at all; a denied approval
//! ends the turn as `turn_aborted`.
//!
//! **Activity** comes from here, not from a registry: the newest turn is
//! `Busy` while it is open, `Waiting` while a held permission is unanswered,
//! and `Idle` once a record or a hook has closed it.

use std::collections::HashSet;

use serde_json::Value;

use super::fold::{clamp_end, clip, squeeze, Projection, LINE_CHARS, PROMPT_CHARS, SUMMARY_CHARS};
use super::hooks::HookEffect;
use super::record::{Input, Str};
use super::rows::*;
use crate::session_log::claude::parse_iso8601_millis;

/// What a codex fold keeps beyond the rows.
#[derive(Debug, Default)]
pub(crate) struct CodexState {
    /// Turns whose first prompt the rollout has written: a later one in the
    /// same turn is a steer.
    prompted: HashSet<usize>,
    /// The previous record's time, which is when a reasoning step began.
    last_at: Option<i64>,
    /// Each reply folded, by its row id and words.
    said: HashSet<u64>,
}

/// Top-level record types that carry nothing a row is made of. Any other
/// unknown top-level type is a `Gap`. `event_msg` payloads this fold doesn't
/// read are silent instead: codex has dozens, and most repeat a fact the
/// fold already has.
const SILENT_TYPES: &[&str] =
    &["session_meta", "turn_context", "world_state", "compacted", "token_usage_record", "inter_agent_communication_metadata"];

/// `response_item` payloads with no row of their own.
const SILENT_ITEMS: &[&str] = &["agent_message", "ghost_snapshot", "compaction", "compaction_summary", "other"];

fn str_at<'v>(value: &'v Value, key: &str) -> Option<&'v str> {
    value.get(key).and_then(Value::as_str)
}

/// The tool as a person reads it, and its one-line summary. codex's shell
/// tools are all `Bash` here, as its own hooks name them, so a
/// `PermissionRequest` (which names no call) ties to its call by command.
fn tool_of(name: &str, args: &Value) -> (String, String, Option<String>) {
    let line = |s: &str| squeeze(s, SUMMARY_CHARS);
    match name {
        "exec_command" | "shell" | "shell_command" | "local_shell_call" | "container.exec" => {
            ("Bash".into(), command_of(args).map(|c| line(&c)).unwrap_or_default(), None)
        }
        "apply_patch" => {
            let patch = args.as_str().or_else(|| str_at(args, "input")).or_else(|| str_at(args, "patch")).unwrap_or_default();
            let path = patched_file(patch);
            ("Edit".into(), path.as_deref().map(line).unwrap_or_default(), path)
        }
        _ => {
            let said = ["description", "title", "task_name", "cmd", "command", "query", "path", "message"]
                .iter()
                .find_map(|k| str_at(args, k))
                .map(line)
                .unwrap_or_default();
            (name.to_string(), said, None)
        }
    }
}

/// A shell call's command: `cmd` (`exec_command`), a string `command`
/// (`shell_command`), or an argv whose last word is the script
/// (`["bash", "-lc", "…"]`).
fn command_of(args: &Value) -> Option<String> {
    if let Some(cmd) = str_at(args, "cmd").or_else(|| str_at(args, "command")) {
        return Some(cmd.to_string());
    }
    let argv = args.get("command").or_else(|| args.pointer("/action/command"))?.as_array()?;
    let words: Vec<&str> = argv.iter().filter_map(Value::as_str).collect();
    match words.as_slice() {
        [shell, flag, script] if flag.starts_with('-') && flag.ends_with('c') && !shell.is_empty() => Some(script.to_string()),
        _ if !words.is_empty() => Some(words.join(" ")),
        _ => None,
    }
}

/// The first file an `apply_patch` envelope names.
fn patched_file(patch: &str) -> Option<String> {
    patch.lines().find_map(|l| {
        ["*** Update File: ", "*** Add File: ", "*** Delete File: "].iter().find_map(|p| l.strip_prefix(p)).map(|p| p.trim().to_string())
    })
}

/// Whether a call's output says it failed: a shell that exited non-zero, or
/// a call the person stopped.
fn output_failed(output: &str) -> bool {
    let head = &output[..output.len().min(400)];
    if head.contains("aborted by user") {
        return true;
    }
    head.lines()
        .find_map(|l| l.strip_prefix("Process exited with code "))
        .and_then(|code| code.trim().parse::<i64>().ok())
        .is_some_and(|code| code != 0)
}

impl Projection {
    // -----------------------------------------------------------------
    // The rollout
    // -----------------------------------------------------------------

    /// Fold one complete line of a codex rollout.
    pub fn fold_codex_line(&mut self, line: &[u8]) {
        self.count_line(line.len());
        let record = match serde_json::from_slice::<Value>(line) {
            Ok(record) if record.is_object() => record,
            _ => {
                self.gap(GapReason::Unparsed);
                return;
            }
        };
        // Read again after a rewrite or a rebind back to this file.
        if self.seen("codex", line) {
            return;
        }
        let at = str_at(&record, "timestamp").and_then(parse_iso8601_millis);
        let payload = record.get("payload").unwrap_or(&Value::Null);
        match str_at(&record, "type") {
            Some("event_msg") => self.codex_event(payload, at),
            Some("response_item") => self.codex_item(payload, at),
            Some(kind) if SILENT_TYPES.contains(&kind) => {}
            Some(kind) => self.gap(GapReason::Unknown(kind.to_string())),
            None => self.gap(GapReason::Unknown(String::new())),
        }
        if at.is_some() {
            self.codex.last_at = at;
        }
        self.codex_activity();
    }

    fn codex_event(&mut self, payload: &Value, at: Option<i64>) {
        match str_at(payload, "type") {
            Some("task_started") => {
                let started = at.or_else(|| payload.get("started_at").and_then(Value::as_i64).map(|s| s * 1000));
                let id = match str_at(payload, "turn_id") {
                    Some(turn) => format!("turn:{turn}"),
                    None => format!("turn:resumed:{}", self.next_seq()),
                };
                self.open_turn(id, TurnOrigin::Other, "", started, false);
            }
            Some("task_complete") => self.codex_turn_end(payload, at, TurnOutcome::Finished),
            Some("turn_aborted") => self.codex_turn_end(payload, at, TurnOutcome::Interrupted),
            Some("user_message") => {
                if let Some(text) = str_at(payload, "message") {
                    self.codex_prompt(text, None, at);
                }
            }
            Some("item_completed") => self.codex_completed(payload, at),
            Some("context_compacted") => {
                self.notice(NoticeKind::Compacted, "Context compacted".to_string(), at, false);
            }
            Some("error" | "stream_error") => {
                let text = str_at(payload, "message").map(|m| squeeze(m, LINE_CHARS)).unwrap_or_else(|| "The request failed".into());
                self.notice(NoticeKind::ApiError, text, at, false);
            }
            Some("patch_apply_end") => {
                if let Some(call) = str_at(payload, "call_id") {
                    let failed = payload.get("success").and_then(Value::as_bool) == Some(false);
                    self.codex_call_done(call, failed, at);
                }
            }
            _ => {}
        }
    }

    /// The turn a turn-scoped record or hook names, else the rollout's own.
    fn codex_turn_of(&self, payload: &Value) -> Option<usize> {
        str_at(payload, "turn_id").and_then(|t| self.index.get(&format!("turn:{t}")).copied()).or(self.turn)
    }

    fn codex_turn_end(&mut self, payload: &Value, at: Option<i64>, outcome: TurnOutcome) {
        let Some(i) = self.codex_turn_of(payload) else { return };
        let interrupted = outcome == TurnOutcome::Interrupted;
        self.end_turn(i, at, outcome.clone());
        let duration = payload.get("duration_ms").and_then(Value::as_i64);
        let row = &mut self.rows[i];
        let was_provisional = std::mem::replace(&mut row.provisional, false);
        let mut moved = was_provisional;
        if let RowKind::Turn(turn) = &mut row.kind {
            // A hook ended it first: the record's end and codex's own
            // duration are the record. Interrupted outranks a `Stop`.
            let end = clamp_end(turn.started_ms, at.or(turn.ended_ms));
            if turn.ended_ms != end || (duration.is_some() && turn.duration_ms != duration) {
                turn.ended_ms = end;
                turn.duration_ms = duration.or(turn.duration_ms);
                moved = true;
            }
            if interrupted && turn.outcome != Some(TurnOutcome::Interrupted) {
                turn.outcome = Some(TurnOutcome::Interrupted);
                moved = true;
            }
        }
        if moved {
            self.touch(i);
        }
        // A call the turn left running never comes back.
        let turn_id = self.rows[i].id.clone();
        let running: Vec<usize> = self
            .since_turn(i)
            .filter(|&j| self.rows[j].turn.as_deref() == Some(&turn_id) && matches!(&self.rows[j].kind, RowKind::Tool(t) if t.status == ToolStatus::Running))
            .collect();
        for j in running {
            if let RowKind::Tool(tool) = &mut self.rows[j].kind {
                tool.status = if interrupted { ToolStatus::Failed } else { ToolStatus::Done };
                tool.ended_ms = clamp_end(tool.started_ms, at);
            }
            self.touch(j);
        }
    }

    /// What the person typed: the turn's prompt, or a steer into it.
    fn codex_prompt(&mut self, text: &str, item: Option<&str>, at: Option<i64>) {
        let turn = match self.turn {
            Some(turn) => turn,
            None => {
                let id = format!("turn:resumed:{}", self.next_seq());
                self.open_turn(id, TurnOrigin::Other, "", at, false)
            }
        };
        if self.codex.prompted.insert(turn) {
            if let Some(t) = self.turn_mut(turn) {
                t.prompt = clip(text, PROMPT_CHARS);
                t.origin = TurnOrigin::Typed;
            }
            self.touch(turn);
            return;
        }
        let id = match item {
            Some(item) => format!("steer:{item}"),
            None => format!("steer:{}", self.next_seq()),
        };
        if self.index.contains_key(&id) {
            return;
        }
        let queued = Queued { text: squeeze(text, LINE_CHARS), state: QueuedState::Sent, at_ms: at };
        self.push(id, Some(turn), false, RowKind::Queued(queued));
    }

    /// `event_msg/item_completed`, codex 0.147's shape for every step.
    fn codex_completed(&mut self, payload: &Value, at: Option<i64>) {
        let Some(item) = payload.get("item") else { return };
        let id = str_at(item, "id");
        let started = payload.get("started_at_ms").and_then(Value::as_i64);
        let ended = payload.get("completed_at_ms").and_then(Value::as_i64).or(at);
        match str_at(item, "type") {
            Some("UserMessage") => {
                let text: Vec<&str> =
                    item.get("content").and_then(Value::as_array).into_iter().flatten().filter_map(|c| str_at(c, "text")).collect();
                self.codex_prompt(&text.join("\n"), id, at);
            }
            Some("AgentMessage") => {
                let Some(id) = id else { return };
                let text: Vec<&str> =
                    item.get("content").and_then(Value::as_array).into_iter().flatten().filter_map(|c| str_at(c, "text")).collect();
                self.codex_prose(id, &text.join("\n"), str_at(item, "phase") == Some("final_answer"), at);
            }
            Some("Reasoning") => {
                let Some(id) = id else { return };
                self.codex_thinking(id, started.or(self.codex.last_at), ended);
            }
            Some("CommandExecution") => {
                let Some(id) = id else { return };
                let failed = item.get("exit_code").and_then(Value::as_i64).is_some_and(|c| c != 0)
                    || matches!(str_at(item, "status"), Some("failed" | "declined"));
                let command = item
                    .pointer("/parsed_cmd/0/cmd")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| command_of(item))
                    .unwrap_or_default();
                // `!cmd`: the person's own command is the turn's prompt.
                let unprompted = self.turn.is_some_and(|t| !self.codex.prompted.contains(&t));
                if str_at(item, "source") == Some("user_shell") && unprompted {
                    self.codex_prompt(&format!("!{command}"), None, at);
                }
                let args = serde_json::json!({ "cmd": command });
                self.codex_call(self.turn, id, "exec_command", &args, started.or(at), false);
                self.codex_call_done(id, failed, ended);
            }
            Some(kind @ ("FileChange" | "McpToolCall" | "Extension")) => {
                let Some(id) = id else { return };
                let (name, args) = match kind {
                    "FileChange" => {
                        let path = item.get("changes").and_then(Value::as_object).and_then(|c| c.keys().next().cloned());
                        ("apply_patch".to_string(), Value::String(path.map(|p| format!("*** Update File: {p}")).unwrap_or_default()))
                    }
                    "McpToolCall" => (str_at(item, "tool").unwrap_or("mcp").to_string(), item.get("arguments").cloned().unwrap_or_default()),
                    _ => (str_at(item, "kind").unwrap_or("extension").to_string(), item.clone()),
                };
                let failed = matches!(str_at(item, "status"), Some("failed" | "declined"));
                self.codex_call(self.turn, id, &name, &args, started.or(at), false);
                self.codex_call_done(id, failed, ended);
            }
            _ => {}
        }
    }

    /// `response_item`: the model's own history, in every codex version.
    fn codex_item(&mut self, payload: &Value, at: Option<i64>) {
        match str_at(payload, "type") {
            Some("message") => {
                // The user's side carries codex's injected context too; the
                // prompt comes from the `UserMessage` item instead.
                if str_at(payload, "role") != Some("assistant") {
                    return;
                }
                let text: Vec<&str> = payload
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|c| str_at(c, "type") == Some("output_text"))
                    .filter_map(|c| str_at(c, "text"))
                    .collect();
                let id = match str_at(payload, "id") {
                    Some(id) => id.to_string(),
                    None => format!("anon:{}", self.next_seq()),
                };
                self.codex_prose(&id, &text.join("\n"), str_at(payload, "phase") == Some("final_answer"), at);
            }
            Some("reasoning") => {
                let id = match str_at(payload, "id") {
                    Some(id) => id.to_string(),
                    None => format!("anon:{}", self.next_seq()),
                };
                self.codex_thinking(&id, self.codex.last_at, at);
            }
            Some(kind @ ("function_call" | "custom_tool_call" | "local_shell_call" | "web_search_call")) => {
                let call = str_at(payload, "call_id").or_else(|| str_at(payload, "id"));
                let Some(call) = call else { return };
                let (name, args) = match kind {
                    "function_call" => {
                        let raw = str_at(payload, "arguments").unwrap_or("{}");
                        (str_at(payload, "name").unwrap_or("tool"), serde_json::from_str::<Value>(raw).unwrap_or_default())
                    }
                    "custom_tool_call" => (str_at(payload, "name").unwrap_or("tool"), payload.get("input").cloned().unwrap_or_default()),
                    "local_shell_call" => ("local_shell_call", payload.clone()),
                    _ => ("web_search", payload.get("action").cloned().unwrap_or_default()),
                };
                self.codex_call(self.turn, call, name, &args, at, false);
                if kind == "web_search_call" && str_at(payload, "status") == Some("completed") {
                    self.codex_call_done(call, false, at);
                }
            }
            Some("function_call_output" | "custom_tool_call_output") => {
                let Some(call) = str_at(payload, "call_id") else { return };
                let output = match payload.get("output") {
                    Some(Value::String(s)) => s.as_str(),
                    Some(Value::Array(parts)) => parts.first().and_then(|p| str_at(p, "text")).unwrap_or_default(),
                    Some(other) => other.get("content").and_then(Value::as_str).unwrap_or_default(),
                    None => "",
                };
                self.codex_call_done(call, output_failed(output), at);
            }
            Some(kind) if SILENT_ITEMS.contains(&kind) => {}
            Some(kind) => self.gap(GapReason::Unknown(format!("response_item/{kind}"))),
            None => self.gap(GapReason::Unknown("response_item".into())),
        }
    }

    /// A row id for an item of the rollout's current turn. An item's own id
    /// is unique in practice, and scoped to its turn all the same: a reused
    /// one would otherwise fold a whole reply into an older turn's row.
    fn codex_item_id(&self, kind: &str, id: &str) -> String {
        match self.turn {
            Some(turn) => format!("{kind}:{}:{id}", self.rows[turn].ord),
            None => format!("{kind}:-:{id}"),
        }
    }

    /// A reply: once, though codex writes it as an item and again in its
    /// history. The two copies share an id and their words; a second message
    /// under the same id with other words (seen in a scrubbed fixture) is a
    /// row of its own.
    fn codex_prose(&mut self, id: &str, text: &str, conclusion: bool, at: Option<i64>) {
        let mut row_id = self.codex_item_id("prose", id);
        if text.trim().is_empty() {
            return;
        }
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            (&row_id, squeeze(text, usize::MAX)).hash(&mut hasher);
            hasher.finish()
        };
        if !self.codex.said.insert(key) {
            return;
        }
        if self.index.contains_key(&row_id) {
            row_id = format!("{row_id}:{}", self.next_seq());
        }
        let turn = self.turn;
        self.push(row_id, turn, false, RowKind::Prose(Prose { text: text.to_string(), conclusion, at_ms: at }));
    }

    fn codex_thinking(&mut self, id: &str, started: Option<i64>, ended: Option<i64>) {
        let row_id = self.codex_item_id("think", id);
        if self.index.contains_key(&row_id) {
            return;
        }
        let turn = self.turn;
        let thinking = Thinking { started_ms: started, ended_ms: clamp_end(started, ended) };
        self.push(row_id, turn, false, RowKind::Thinking(thinking));
    }

    /// A call is in: a running `Tool` row, or the confirmation of the one a
    /// `PreToolUse` put up for it.
    fn codex_call(&mut self, turn: Option<usize>, call: &str, name: &str, args: &Value, at: Option<i64>, provisional: bool) {
        if name == "request_user_input" {
            return self.codex_question(turn, call, args, at, provisional);
        }
        let row_id = format!("tool:{call}");
        if let Some(&i) = self.index.get(&row_id) {
            if !provisional && self.rows[i].provisional {
                self.rows[i].provisional = false;
                if let RowKind::Tool(tool) = &mut self.rows[i].kind {
                    tool.started_ms = at.or(tool.started_ms);
                }
                self.touch(i);
                self.tool_confirmed(call);
            }
            return;
        }
        let Some(turn) = turn.or(self.newest_turn) else { return };
        let (name, summary, file_path) = tool_of(name, args);
        let tool = Tool {
            name: name.clone(),
            summary: summary.clone(),
            status: ToolStatus::Running,
            started_ms: at,
            ended_ms: None,
            diff: Vec::new(),
            file_path,
        };
        self.push(row_id, Some(turn), provisional, RowKind::Tool(tool));
        self.link_tool(None, call, &name, &summary, !provisional);
    }

    /// `request_user_input`, codex's question to the person: an `Ask`, as
    /// claude's `AskUserQuestion` is.
    fn codex_question(&mut self, turn: Option<usize>, call: &str, args: &Value, at: Option<i64>, provisional: bool) {
        let row_id = format!("ask:{call}");
        if let Some(&i) = self.index.get(&row_id) {
            if !provisional && self.rows[i].provisional {
                self.rows[i].provisional = false;
                self.touch(i);
            }
            return;
        }
        let Some(turn) = turn.or(self.newest_turn) else { return };
        let question = args.pointer("/questions/0/question").and_then(Value::as_str).map(|q| squeeze(q, LINE_CHARS)).unwrap_or_default();
        let ask = Ask {
            kind: AskKind::Question,
            text: question,
            tool: Some("request_user_input".into()),
            asked_ms: at,
            answered_ms: None,
            answered: false,
            // Answered at the terminal only: no option is offered, and no
            // hook holds it for a view to answer.
            held: None,
            questions: Vec::new(),
            plan: None,
            answered_by: None,
        };
        self.push(row_id, Some(turn), provisional, RowKind::Ask(ask));
    }

    /// A call came back.
    fn codex_call_done(&mut self, call: &str, failed: bool, at: Option<i64>) {
        if let Some(&i) = self.index.get(&format!("ask:{call}")) {
            let row = &mut self.rows[i];
            if let RowKind::Ask(ask) = &mut row.kind {
                if !ask.answered || row.provisional {
                    ask.answered = true;
                    ask.answered_ms = ask.answered_ms.or(at);
                    row.provisional = false;
                    self.touch(i);
                }
            }
            return;
        }
        let Some(&i) = self.index.get(&format!("tool:{call}")) else { return };
        let row = &mut self.rows[i];
        if let RowKind::Tool(tool) = &mut row.kind {
            let status = if failed { ToolStatus::Failed } else { ToolStatus::Done };
            if tool.status == status && !row.provisional {
                return;
            }
            tool.status = status;
            tool.ended_ms = clamp_end(tool.started_ms, at.or(tool.ended_ms));
        }
        row.provisional = false;
        self.touch(i);
        self.tool_confirmed(call);
        self.tool_done(call, at);
    }

    /// The newest turn's activity, from what the rollout and the hooks say.
    fn codex_activity(&mut self) {
        let Some(newest) = self.newest_turn else { return };
        let open = matches!(&self.rows[newest].kind, RowKind::Turn(t) if t.outcome.is_none());
        let activity = match open {
            false => Activity::Idle,
            true if !self.tool_asks.is_empty() || !self.perm_waiting.is_empty() => Activity::Waiting,
            true => Activity::Busy,
        };
        if self.activity != Some(activity) {
            self.set_activity(activity);
        }
    }

    // -----------------------------------------------------------------
    // Hooks
    // -----------------------------------------------------------------

    /// Fold one codex hook, received at `now` (ms since the epoch, the
    /// daemon's clock). codex runs a hook only once a person has trusted it
    /// (design, finding 9), so none of this is relied on: the rollout says
    /// all of it a moment later.
    pub fn codex_hook(&mut self, event: &str, payload: &Value, now: i64) -> HookEffect {
        let at = Some(now);
        let effect = match event {
            "UserPromptSubmit" => {
                if let Some(turn) = str_at(payload, "turn_id") {
                    let id = format!("turn:{turn}");
                    let prompt = str_at(payload, "prompt").unwrap_or_default();
                    // Known already: a steer, or a prompt the rollout has
                    // written. The rollout brings either in a moment.
                    if !self.index.contains_key(&id) {
                        self.open_turn(id, TurnOrigin::Typed, prompt, at, true);
                    }
                }
                HookEffect::None
            }
            "PreToolUse" => {
                if let Some(call) = str_at(payload, "tool_use_id") {
                    let name = str_at(payload, "tool_name").unwrap_or("Tool");
                    let input = payload.get("tool_input").cloned().unwrap_or_default();
                    // The hook says `Bash` with a `command`, as `tool_of`
                    // names a shell call.
                    let (name, args) = match name {
                        "Bash" => ("exec_command", input),
                        other => (other, input),
                    };
                    let turn = self.codex_turn_of(payload);
                    self.codex_call(turn, call, name, &args, at, true);
                }
                HookEffect::None
            }
            "PostToolUse" => {
                // Done, but not the record: the output, read next, says
                // whether it failed. The ask it held is answered now.
                if let Some(call) = str_at(payload, "tool_use_id") {
                    if let Some(&i) = self.index.get(&format!("tool:{call}")) {
                        if let RowKind::Tool(tool) = &mut self.rows[i].kind {
                            if tool.status == ToolStatus::Running {
                                tool.status = ToolStatus::Done;
                                tool.ended_ms = clamp_end(tool.started_ms, at);
                                self.touch(i);
                            }
                        }
                    }
                    self.tool_done(call, at);
                }
                HookEffect::None
            }
            "PermissionRequest" => {
                let turn = self.codex_turn_of(payload).unwrap_or_else(|| self.hook_turn(payload, now));
                let command = payload.pointer("/tool_input/command").and_then(Value::as_str).map(|c| c.to_string().into());
                // Only the command: the request's `description` is codex's
                // reason, and the call's summary is its command.
                let input = Input { command: Str(command), ..Input::default() };
                self.permission_request(turn, payload, &input, now);
                HookEffect::None
            }
            "Stop" | "Interrupt" => {
                if let Some(turn) = self.codex_turn_of(payload) {
                    let outcome = if event == "Stop" { TurnOutcome::Finished } else { TurnOutcome::Interrupted };
                    self.end_turn(turn, at, outcome);
                }
                HookEffect::None
            }
            "SessionStart" => self.session_start(payload, now),
            _ => HookEffect::None,
        };
        self.codex_activity();
        effect
    }
}
