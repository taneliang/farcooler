//! Asks the runner's hook is holding, on the rows a view answers them from
//! (ov-370).
//!
//! `hook_ingress` holds a claude `PermissionRequest` while a device may
//! answer it, and folds the request here with the hold's id added to its
//! payload (`HELD_ASK`). The row it lands on carries that id (`Ask::held`),
//! which is what a view answers with; the hold's end, however it came
//! (`ASK_ENDED`), takes it off again, and names the device when one answered.
//!
//! A question's or a plan's request makes no row of its own. Its
//! `PreToolUse` (or its transcript record) already put up the `Ask` row,
//! `ask:<tool_use_id>`, and the request names no `tool_use_id`, so it goes
//! to the newest unanswered ask row of its tool: claude raises one such
//! dialog at a time. A request that comes before its row waits for it.

use std::collections::VecDeque;

use serde_json::Value;

use super::fold::{clip, Projection};
use super::record::Input;
use super::rows::*;

/// The key `hook_ingress` adds to a held `PermissionRequest`'s payload: the
/// id a device answers the ask with.
pub const HELD_ASK: &str = "farcooler_held_ask";

/// A hook event no agent sends: the hold of `ask_id` ended. `by` names the
/// device whose answer ended it, when one did.
pub const ASK_ENDED: &str = "FarCoolerAskEnded";

/// The longest plan a row carries.
pub(super) const PLAN_CHARS: usize = 16_000;

/// Ended holds remembered, so a request folded after its own end (they are
/// folded on different tasks) doesn't put a dead id back up.
const ENDED_KEPT: usize = 32;

/// The projection's side of held asks.
#[derive(Debug, Default)]
pub(crate) struct HeldAsks {
    ended: VecDeque<String>,
    /// A question's or a plan's held id whose row isn't up yet: tool, id,
    /// and what the row must say (`asked`).
    waiting: Vec<(String, String, String)>,
}

/// What a question or a plan asks, from its request's `tool_input`: the
/// plan's words, or the questions' words. A hold goes only on a row that
/// asks the same (review 1 M2): an older row of the same tool left
/// unanswered (claude killed mid-dialog, then resumed) must never show its
/// plan beside an Approve that approves the new one.
fn asked_in(tool: &str, input: &Value) -> String {
    if tool == "ExitPlanMode" {
        return input.get("plan").and_then(Value::as_str).map(|p| clip(p, PLAN_CHARS)).unwrap_or_default();
    }
    let questions = input.get("questions").and_then(Value::as_array).cloned().unwrap_or_default();
    questions.iter().filter_map(|q| q.get("question").and_then(Value::as_str)).collect::<Vec<_>>().join("\n")
}

/// `asked_in`, as an `Ask` row says it.
fn asked_by(ask: &Ask) -> String {
    match ask.tool.as_deref() {
        Some("ExitPlanMode") => ask.plan.clone().unwrap_or_default(),
        _ => ask.questions.iter().map(|q| q.question.as_str()).collect::<Vec<_>>().join("\n"),
    }
}

/// The tools whose dialog is answered by more than yes or no, and whose row
/// is an `Ask` of its own: a question and a plan's approval.
pub(super) fn is_dialog_tool(tool: &str) -> bool {
    matches!(tool, "AskUserQuestion" | "ExitPlanMode")
}

/// An `AskUserQuestion`'s questions, whole.
pub(super) fn questions_of(input: &Input<'_>) -> Vec<AskQuestion> {
    let words = |s: &super::record::Str<'_>| s.get().unwrap_or_default().to_string();
    input
        .questions
        .0
        .iter()
        .filter_map(|q| q.0.as_ref())
        .map(|q| AskQuestion {
            question: words(&q.question),
            header: words(&q.header),
            options: q
                .options
                .0
                .iter()
                .filter_map(|o| o.0.as_ref())
                .map(|o| AskOption { label: words(&o.label), description: words(&o.description) })
                .collect(),
            multi_select: q.multi_select.yes(),
        })
        .collect()
}

/// An `ExitPlanMode`'s plan, its line breaks kept.
pub(super) fn plan_of(input: &Input<'_>) -> Option<String> {
    input.plan.get().map(|p| clip(p, PLAN_CHARS)).filter(|p| !p.is_empty())
}

impl Projection {
    /// The hold's id a `PermissionRequest` was folded with, unless that hold
    /// has ended already.
    pub(super) fn held_id(&self, payload: &Value) -> Option<String> {
        let id = payload.get(HELD_ASK).and_then(Value::as_str)?;
        (!self.held.ended.iter().any(|e| e == id)).then(|| id.to_string())
    }

    /// A question's or a plan's `PermissionRequest`: its hold goes on its row.
    pub(super) fn dialog_request(&mut self, tool: &str, payload: &Value) {
        let Some(id) = self.held_id(payload) else { return };
        let asked = asked_in(tool, payload.get("tool_input").unwrap_or(&Value::Null));
        match self.open_dialog_row(tool, &asked) {
            Some(i) => self.set_held(i, Some(id), None),
            None => self.held.waiting.push((tool.to_string(), id, asked)),
        }
    }

    /// The newest unanswered `Ask` row of `tool` that holds nothing yet and
    /// asks what the request does.
    fn open_dialog_row(&self, tool: &str, asked: &str) -> Option<usize> {
        self.rows.iter().rposition(|row| {
            matches!(&row.kind, RowKind::Ask(a)
                if !a.answered && a.held.is_none() && a.tool.as_deref() == Some(tool) && asked_by(a) == asked)
                && row.id.starts_with("ask:")
        })
    }

    /// Ask row `i`, of `tool`, was just put up: a hold that came first lands
    /// on it.
    pub(super) fn dialog_row_up(&mut self, i: usize, tool: &str) {
        let asked = match &self.rows[i].kind {
            RowKind::Ask(a) => asked_by(a),
            _ => return,
        };
        if let Some(n) = self.held.waiting.iter().position(|(t, _, a)| t == tool && *a == asked) {
            let (_, id, _) = self.held.waiting.remove(n);
            self.set_held(i, Some(id), None);
        }
    }

    /// `ASK_ENDED`: the hold's id comes off its row, which says who answered
    /// when a device did.
    pub(super) fn ask_ended(&mut self, payload: &Value) {
        let Some(id) = payload.get("ask_id").and_then(Value::as_str) else { return };
        if self.held.ended.len() >= ENDED_KEPT {
            self.held.ended.pop_front();
        }
        self.held.ended.push_back(id.to_string());
        self.held.waiting.retain(|(_, w, _)| w != id);
        let by = payload.get("by").and_then(Value::as_str).filter(|b| !b.is_empty()).map(str::to_string);
        let row = self.rows.iter().rposition(|row| matches!(&row.kind, RowKind::Ask(a) if a.held.as_deref() == Some(id)));
        if let Some(i) = row {
            self.set_held(i, None, by);
        }
    }

    fn set_held(&mut self, i: usize, held: Option<String>, by: Option<String>) {
        if let RowKind::Ask(ask) = &mut self.rows[i].kind {
            ask.held = held;
            if by.is_some() {
                ask.answered_by = by;
            }
            self.touch(i);
        }
    }
}
