//! Hooks as provisional rows, which the transcript then confirms.
//!
//! A hook reaches the daemon within milliseconds; the transcript line for the
//! same moment can trail it by a flush. So a hook puts a row up at once,
//! marked provisional, under the id the transcript will later use for it
//! (`turn:<promptId>`, `tool:<tool_use_id>`), and the transcript's record
//! confirms it in place. Prose is the one row with no shared id: a
//! `MessageDisplay` names a message the transcript does not, so the transcript
//! confirms the oldest waiting prose whose words its own begin with.
//!
//! Today's registered set is `SessionStart`, `UserPromptSubmit`, `Stop`,
//! `MessageDisplay` and `PermissionRequest`. ov-364 registers the tool,
//! subagent and failure hooks; they are read here already, so registering
//! them is the whole of that change on this side.
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
                let tool = text(payload, "tool_name").unwrap_or("Tool");
                let summary = super::fold::summarize(&input_of(payload));
                let ask = Ask {
                    kind: AskKind::Permission,
                    text: if summary.is_empty() { tool.to_string() } else { format!("{tool} {summary}") },
                    tool: Some(tool.to_string()),
                    asked_ms: at,
                    answered_ms: None,
                    answered: false,
                };
                let id = format!("perm:{}", self.next_seq());
                self.push(id, Some(turn), true, RowKind::Ask(ask));
            }
            // A subagent's own tool calls carry its `agent_id`; they are its
            // row's business (from its transcript), not the main turn's.
            "PreToolUse" | "PostToolUse" | "PostToolUseFailure" if payload.get("agent_id").is_some() => {}
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
        // has nothing to add, and later flushes of it are dropped too.
        let turn_id = self.rows[turn].id.clone();
        let words = squeeze(delta, usize::MAX);
        let written = (0..self.rows.len()).rev().find(|&i| {
            let row = &self.rows[i];
            !row.provisional
                && row.turn.as_deref() == Some(&turn_id)
                && matches!(&row.kind, RowKind::Prose(p) if squeeze(&p.text, usize::MAX).contains(&words))
        });
        let i = match written {
            Some(i) => i,
            None => {
                let id = format!("hprose:{message}");
                let prose = Prose { text: delta.to_string(), conclusion: false, at_ms: Some(now) };
                self.push(id, Some(turn), true, RowKind::Prose(prose))
            }
        };
        self.hook_messages.insert(message.to_string(), (i, index));
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
