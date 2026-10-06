//! Hooks as provisional rows, which the transcript then confirms.
//!
//! A hook reaches the daemon within milliseconds; the transcript line for the
//! same moment can trail it by a flush. So a hook puts a row up at once,
//! marked provisional, under the id the transcript will later use for it
//! (`turn:<promptId>`, `tool:<tool_use_id>`), and the transcript's record
//! confirms it in place. Prose is the one row with no shared id: a
//! `MessageDisplay` names a message the transcript does not (its `message_id`
//! is a fresh uuid, "not the API msg_ id"), so the transcript confirms the
//! oldest waiting prose whose words its own begin with, and when the turn
//! closes any still waiting is paired off or stands (`settle_prose`).
//!
//! Registered for every claude pane Far Cooler launches: `SessionStart`,
//! `UserPromptSubmit`, `Stop`, `StopFailure`, `MessageDisplay`,
//! `PermissionRequest`, `Notification`, the tool hooks (`PreToolUse`,
//! `PostToolUse`, `PostToolUseFailure`) and the subagent hooks
//! (`SubagentStart`, `SubagentStop`). `Notification` changes no row.
//!
//! Every hook finds its turn by its own `prompt_id`, never by the
//! transcript's current turn: the hook for turn 2 routinely arrives while the
//! transcript is still writing turn 1's last lines.

use std::path::PathBuf;

use serde_json::Value;

use super::fold::{clip, squeeze, Projection, PROMPT_CHARS};
use super::record::{Block, Bool, Input, List, Obj, Str};
use super::rows::*;

/// What a hook asks of whoever holds this projection, beyond its rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookEffect {
    None,
    /// The pane's conversation moved to another session: `/clear` starts a
    /// new one, and a `SessionStart` naming a session this projection is not
    /// is the same news however it came about. The holder starts reading the
    /// new transcript; the rows so far stay.
    Rebind { session_id: String, transcript_path: Option<PathBuf>, source: String },
}

fn text<'v>(payload: &'v Value, key: &str) -> Option<&'v str> {
    payload.get(key).and_then(Value::as_str)
}

/// A hook's `tool_input`, read into the same `Input` a transcript block has,
/// so a summary is the same words from either source.
fn input_of(payload: &Value) -> Input<'static> {
    let field = |key: &str| Str(payload.pointer(&format!("/tool_input/{key}")).and_then(Value::as_str).map(|s| s.to_string().into()));
    Input {
        description: field("description"),
        command: field("command"),
        file_path: field("file_path"),
        pattern: field("pattern"),
        path: field("path"),
        subject: field("subject"),
        url: field("url"),
        query: field("query"),
        subagent_type: field("subagent_type"),
        run_in_background: Bool(payload.pointer("/tool_input/run_in_background").and_then(Value::as_bool)),
        questions: List(Vec::new()),
        plan: field("plan"),
    }
}

impl Projection {
    /// The turn a hook belongs to: its own `prompt_id`'s row, else the newest.
    fn hook_turn(&mut self, payload: &Value, now: i64) -> usize {
        if let Some(i) = text(payload, "prompt_id").and_then(|p| self.index.get(&format!("turn:{p}")).copied()) {
            return i;
        }
        if let Some(i) = self.newest_turn {
            return i;
        }
        let id = format!("turn:resumed:{}", self.next_seq());
        self.open_turn(id, TurnOrigin::Other, "", Some(now), true)
    }

    /// Fold one hook firing, received at `now` (ms since the epoch, the
    /// daemon's clock).
    pub fn hook(&mut self, event: &str, payload: &Value, now: i64) -> HookEffect {
        let at = Some(now);
        match event {
            "UserPromptSubmit" => {
                let Some(prompt_id) = text(payload, "prompt_id") else { return HookEffect::None };
                let prompt = text(payload, "prompt").unwrap_or_default();
                let id = format!("turn:{prompt_id}");
                if !self.index.contains_key(&id) {
                    self.open_turn(id, TurnOrigin::Typed, prompt, at, true);
                } else if let Some(&i) = self.index.get(&id) {
                    // The hook carries the prompt as typed, pastes expanded; the
                    // transcript's copy may be the same or shorter.
                    if let Some(turn) = self.turn_mut(i) {
                        if turn.prompt.len() < prompt.trim().len() {
                            turn.prompt = clip(prompt, PROMPT_CHARS);
                            self.touch(i);
                        }
                    }
                }
            }
            "Stop" | "StopFailure" if payload.get("agent_id").is_some() => {}
            "Stop" => {
                let turn = self.hook_turn(payload, now);
                self.end_turn(turn, at, TurnOutcome::Finished);
            }
            "StopFailure" => {
                let turn = self.hook_turn(payload, now);
                let detail = text(payload, "last_assistant_message").or(text(payload, "error")).unwrap_or("The turn failed");
                self.fail_turn(turn, detail, at);
            }
            "MessageDisplay" => self.message_display(payload, now),
            "PermissionRequest" => {
                let turn = self.hook_turn(payload, now);
                self.permission_request(turn, payload, &input_of(payload), now);
            }
            // A subagent's own tool calls carry its `agent_id`; they are its
            // row's business, not the main turn's. Its transcript counts
            // them; a `PreToolUse` names what it's doing now, ahead of it.
            "PreToolUse" if payload.get("agent_id").is_some() => self.subagent_acting(payload, now),
            "PreToolUse" | "PostToolUse" | "PostToolUseFailure" if payload.get("agent_id").is_some() => {}
            "SubagentStart" => self.subagent_started(payload),
            "PreToolUse" => {
                let turn = self.hook_turn(payload, now);
                let block = Block {
                    kind: Str(Some("tool_use".into())),
                    id: Str(text(payload, "tool_use_id").map(|s| s.to_string().into())),
                    name: Str(text(payload, "tool_name").map(|s| s.to_string().into())),
                    input: Obj(Some(input_of(payload))),
                    ..Block::default()
                };
                self.tool_use(turn, &block, at, true);
            }
            "PostToolUse" | "PostToolUseFailure" => {
                let Some(id) = text(payload, "tool_use_id") else { return HookEffect::None };
                let Some(&i) = self.index.get(&format!("tool:{id}")) else { return HookEffect::None };
                if let RowKind::Tool(tool) = &mut self.rows[i].kind {
                    if tool.status == ToolStatus::Running {
                        tool.status = if event == "PostToolUse" { ToolStatus::Done } else { ToolStatus::Failed };
                        tool.ended_ms = tool.started_ms.map_or(at, |s| Some(now.max(s)));
                        self.touch(i);
                    }
                }
                self.tool_done(id, at);
            }
            "SubagentStop" => {
                let Some(agent) = text(payload, "agent_id") else { return HookEffect::None };
                match self.agents.get(agent).copied() {
                    Some(i) => {
                        if matches!(&self.rows[i].kind, RowKind::Subagent(s) if s.status == SubagentState::Running) {
                            self.end_subagent(i, SubagentState::Completed, at);
                        }
                    }
                    None => self.stop_orphan(agent),
                }
            }
            "SessionStart" => return self.session_start(payload, now),
            _ => {}
        }
        HookEffect::None
    }

    /// `SubagentStart`: tie its `agent_id` to the `Agent` call's row, when
    /// exactly one running row of its type is still untied. The hook names
    /// no `tool_use_id`, so two of one type launched together wait for their
    /// meta files instead (`join_by_meta`).
    fn subagent_started(&mut self, payload: &Value) {
        let Some(agent) = text(payload, "agent_id") else { return };
        if self.agents.contains_key(agent) {
            return;
        }
        let kind = text(payload, "agent_type");
        let mut untied = self.rows.iter().enumerate().filter(|(_, row)| {
            matches!(&row.kind, RowKind::Subagent(s)
                if s.status == SubagentState::Running
                    && s.agent_id.is_none()
                    && kind.is_none_or(|k| s.agent_type == k))
        });
        if let (Some((i, _)), None) = (untied.next(), untied.next()) {
            self.join_agent(agent, i);
        }
    }

    /// A subagent's `PreToolUse`: its row's current action, now.
    fn subagent_acting(&mut self, payload: &Value, now: i64) {
        let Some(&i) = text(payload, "agent_id").and_then(|a| self.agents.get(a)) else { return };
        let summary = super::fold::summarize(&input_of(payload));
        let name = text(payload, "tool_name").unwrap_or("Tool");
        if let RowKind::Subagent(sub) = &mut self.rows[i].kind {
            sub.current_action = if summary.is_empty() { name.to_string() } else { format!("{name} {summary}") };
            sub.last_ms = sub.last_ms.max(Some(now));
        }
        self.touch(i);
    }

    fn message_display(&mut self, payload: &Value, now: i64) {
        let Some(message) = text(payload, "message_id") else { return };
        let delta = text(payload, "delta").unwrap_or_default();
        let index = payload.get("index").and_then(Value::as_u64);
        let turn = self.hook_turn(payload, now);
        if let Some(&(i, last)) = self.hook_messages.get(message) {
            // A repeat or a straggler (`index` at or below the last one
            // applied), the assembler's own rule.
            if index.is_some_and(|n| last.is_some_and(|l| n <= l)) {
                return;
            }
            self.hook_messages.insert(message.to_string(), (i, index.or(last)));
            if self.rows[i].provisional {
                if let RowKind::Prose(p) = &mut self.rows[i].kind {
                    p.text.push_str(delta);
                }
                self.touch(i);
            }
            return;
        }
        if delta.trim().is_empty() {
            return;
        }
        // The transcript may already have written this message; then the hook
        // has nothing to add, and later flushes of it are dropped too. Its row
        // is the first in this turn that the transcript wrote, that no other
        // message has claimed, and whose words BEGIN with these: hooks and
        // records each arrive in order. Matching these words anywhere inside
        // a row bound a short first flush ("I'll") to an earlier row that
        // happened to contain it (ov-363 review 1, finding 10).
        let turn_id = self.rows[turn].id.clone();
        let words = squeeze(delta, usize::MAX);
        let claimed: std::collections::HashSet<usize> = self.hook_messages.values().map(|&(i, _)| i).collect();
        let written = (0..self.rows.len()).find(|&i| {
            let row = &self.rows[i];
            !row.provisional
                && !row.retracted
                && row.turn.as_deref() == Some(&turn_id)
                && !claimed.contains(&i)
                && matches!(&row.kind, RowKind::Prose(p) if squeeze(&p.text, usize::MAX).starts_with(&words))
        });
        // A turn the transcript has closed has all its words written: a flush
        // that matches none of them is not news, and a row for it would wait
        // for a record that never comes.
        let closed = !self.rows[turn].provisional && matches!(&self.rows[turn].kind, RowKind::Turn(t) if t.outcome.is_some());
        let i = match written {
            Some(i) => i,
            None if closed => return,
            None => {
                let id = format!("hprose:{message}");
                let prose = Prose { text: delta.to_string(), conclusion: false, at_ms: Some(now) };
                self.push(id, Some(turn), true, RowKind::Prose(prose))
            }
        };
        self.hook_messages.insert(message.to_string(), (i, index));
    }

    /// The transcript has closed `turn`, so every word it will write for it
    /// is in, and a hook's prose still waiting will never be confirmed.
    ///
    /// Each such row is paired with a row the transcript wrote in the turn
    /// that no hook message has claimed: the one whose words hold its words
    /// or begin them, else the oldest, since both arrive in order. That
    /// covers a display that changed the words (claude's `MessageDisplay`
    /// delta is what was drawn, not what was stored). A paired row is
    /// retracted as a copy. One left over was shown and never written, as
    /// when a reply is cut off, and stands as the only record of it.
    pub(super) fn settle_prose(&mut self, turn: usize) {
        let turn_id = self.rows[turn].id.clone();
        let in_turn = |row: &Row| row.turn.as_deref() == Some(turn_id.as_str()) && !row.retracted && matches!(row.kind, RowKind::Prose(_));
        let waiting: Vec<usize> = (0..self.rows.len()).filter(|&i| self.rows[i].provisional && in_turn(&self.rows[i])).collect();
        if waiting.is_empty() {
            return;
        }
        let claimed: std::collections::HashSet<usize> = self.hook_messages.values().map(|&(i, _)| i).collect();
        let mut unclaimed: Vec<usize> =
            (0..self.rows.len()).filter(|&i| !self.rows[i].provisional && in_turn(&self.rows[i]) && !claimed.contains(&i)).collect();
        let words = |row: &Row| match &row.kind {
            RowKind::Prose(p) => squeeze(&p.text, usize::MAX),
            _ => String::new(),
        };
        for i in waiting {
            let shown = words(&self.rows[i]);
            let by_words = unclaimed.iter().position(|&j| {
                let written = words(&self.rows[j]);
                !written.is_empty() && (written.contains(&shown) || shown.starts_with(&written))
            });
            match by_words.or((!unclaimed.is_empty()).then_some(0)) {
                Some(n) => {
                    let written = unclaimed.remove(n);
                    self.retract(i);
                    for entry in self.hook_messages.values_mut().filter(|(row, _)| *row == i) {
                        entry.0 = written;
                    }
                }
                None => {
                    self.rows[i].provisional = false;
                    self.touch(i);
                }
            }
        }
    }

    fn session_start(&mut self, payload: &Value, now: i64) -> HookEffect {
        let source = text(payload, "source").unwrap_or("startup");
        let session = text(payload, "session_id");
        let at = Some(now);
        match source {
            "resume" => {
                self.notice(NoticeKind::Resumed, "Conversation resumed".to_string(), at, false);
            }
            "clear" => {
                self.notice(NoticeKind::Cleared, "Conversation cleared".to_string(), at, false);
            }
            "compact" => {
                self.notice(NoticeKind::Compacted, "Context compacted".to_string(), at, true);
            }
            _ => {}
        }
        let moved = match (session, self.session_id.as_deref()) {
            (Some(new), Some(old)) => new != old,
            (Some(_), None) => source == "clear",
            _ => false,
        };
        let Some(session) = session.filter(|_| moved) else { return HookEffect::None };
        self.session_id = Some(session.to_string());
        // The old session's open turn will never be written to again.
        if let Some(turn) = self.turn.take() {
            self.end_turn(turn, at, TurnOutcome::Finished);
        }
        HookEffect::Rebind {
            session_id: session.to_string(),
            transcript_path: text(payload, "transcript_path").map(PathBuf::from),
            source: source.to_string(),
        }
    }
}
