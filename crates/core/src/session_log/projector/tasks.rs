//! The agent's task list, as a checklist row (ov-452).
//!
//! Claude keeps a list two ways. `TodoWrite` sends the whole list on every
//! call. `TaskCreate` and `TaskUpdate` build one a call at a time, and a
//! created task's id exists only in its result (`toolUseResult.task.id`,
//! else the sentence `Task #1 created successfully: …`), which is why the
//! task tools change the list when their result lands, never on the call: an
//! update applied before the create it follows was confirmed would name an id
//! not yet learned, and a denied call changes nothing. The port of
//! `crates/claude/src/normalize.rs`'s `Tasks`, which the old chat drew as its
//! plan panel.
//!
//! A completed task stays on the list, checked: the tool's own `TaskList`
//! drops it, but a list that empties as the agent succeeds reads as the plan
//! being lost rather than done.
//!
//! None of these calls is a `Tool` row: a turn's changes to the list are one
//! `Tasks` row, at its first change, updated by each later one.

use std::collections::HashMap;

use serde_json::Value;

use super::fold::{squeeze, Projection, LINE_CHARS};
use super::record::ToolUseResult;
use super::rows::*;

/// The tools that write the list. `TaskList` and `TaskGet` only read it, and
/// stay tool rows.
pub(super) fn writes_tasks(name: &str) -> bool {
    matches!(name, "TodoWrite" | "TaskCreate" | "TaskUpdate")
}

/// The list as the calls so far left it, and the task calls waiting on their
/// results.
#[derive(Debug, Default)]
pub(super) struct TaskState {
    /// Each task by the id `TaskUpdate` names it with (a `TodoWrite` item by
    /// its place), in the order made.
    items: Vec<(String, TaskItem)>,
    /// `TaskCreate` and `TaskUpdate` calls by `tool_use` id: name and input.
    calls: HashMap<String, (String, Value)>,
}

fn status_of(word: &str) -> Option<TaskStatus> {
    match word {
        "pending" => Some(TaskStatus::Pending),
        "in_progress" => Some(TaskStatus::InProgress),
        "completed" => Some(TaskStatus::Completed),
        _ => None,
    }
}

fn text_at<'v>(input: &'v Value, key: &str) -> Option<&'v str> {
    input.get(key).and_then(Value::as_str).filter(|s| !s.trim().is_empty())
}

/// `TaskUpdate` takes `"1"` where the result says `#1`.
fn task_key(id: &str) -> String {
    id.trim().trim_start_matches('#').to_string()
}

/// The id a `TaskCreate`'s result sentence names: `Task #2 created
/// successfully: …`. Anchored on the words after the id too, since a
/// `Write` answers "File created successfully".
fn created_id(result: &str) -> Option<String> {
    let (_, after) = result.split_once("Task #")?;
    let (id, rest) = after.split_once(' ')?;
    (!id.is_empty() && rest.starts_with("created successfully")).then(|| task_key(id))
}

impl Projection {
    /// A call to a tool that writes the list. A hook's announcement of one
    /// is ignored: the list moves on the transcript's record alone.
    pub(super) fn task_call(&mut self, turn: usize, id: &str, name: &str, raw: Option<&str>, provisional: bool) {
        if provisional {
            return;
        }
        let input = raw.and_then(|r| serde_json::from_str::<Value>(r).ok()).unwrap_or_default();
        if name != "TodoWrite" {
            self.tasks.calls.insert(id.to_string(), (name.to_string(), input));
            return;
        }
        // The whole list, every time: it replaces whatever was there.
        let Some(todos) = input.get("todos").and_then(Value::as_array) else { return };
        self.tasks.items = todos
            .iter()
            .enumerate()
            .map(|(n, t)| {
                let subject = text_at(t, "content").or_else(|| text_at(t, "subject")).unwrap_or_default();
                let status = text_at(t, "status").and_then(status_of).unwrap_or(TaskStatus::Pending);
                (format!("todo:{n}"), TaskItem { subject: squeeze(subject, LINE_CHARS), status })
            })
            .collect();
        self.show_tasks(turn);
    }

    /// A result came back: if it is a task call's, apply it. Whether it was.
    pub(super) fn task_result(&mut self, id: &str, failed: bool, text: Option<&str>, result: Option<&ToolUseResult<'_>>) -> bool {
        let Some((name, input)) = self.tasks.calls.remove(id) else { return false };
        // A failed or denied call did nothing to the list.
        if failed {
            return true;
        }
        match name.as_str() {
            "TaskCreate" => {
                let subject = text_at(&input, "subject").or_else(|| text_at(&input, "description")).unwrap_or_default();
                // An unreadable result costs the task its id, never its row.
                let key = result
                    .and_then(|r| r.task.0.as_ref())
                    .and_then(|t| t.id.get())
                    .map(task_key)
                    .or_else(|| text.and_then(created_id))
                    .unwrap_or_else(|| format!("unidentified:{id}"));
                let item = TaskItem { subject: squeeze(subject, LINE_CHARS), status: TaskStatus::Pending };
                match self.tasks.items.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, existing)) => existing.subject = item.subject,
                    None => self.tasks.items.push((key, item)),
                }
            }
            _ => {
                // An id this list never saw created (a subagent's, or one
                // made before the transcript was read) is ignored, not made up.
                let Some(key) = text_at(&input, "taskId").map(task_key) else { return true };
                let Some(at) = self.tasks.items.iter().position(|(k, _)| *k == key) else { return true };
                if text_at(&input, "status") == Some("deleted") {
                    self.tasks.items.remove(at);
                } else {
                    let item = &mut self.tasks.items[at].1;
                    if let Some(subject) = text_at(&input, "subject") {
                        item.subject = squeeze(subject, LINE_CHARS);
                    }
                    if let Some(status) = text_at(&input, "status").and_then(status_of) {
                        item.status = status;
                    }
                }
            }
        }
        if let Some(turn) = self.turn {
            self.show_tasks(turn);
        }
        true
    }

    /// The list on turn `turn`'s `Tasks` row: made at its first change, and
    /// changed in place after.
    fn show_tasks(&mut self, turn: usize) {
        let items: Vec<TaskItem> = self.tasks.items.iter().map(|(_, t)| t.clone()).collect();
        let id = format!("tasks:{}", self.rows[turn].id);
        match self.index.get(&id).copied() {
            Some(i) => {
                if let RowKind::Tasks(tasks) = &mut self.rows[i].kind {
                    if tasks.items == items {
                        return;
                    }
                    tasks.items = items;
                }
                self.touch(i);
            }
            None => {
                self.push(id, Some(turn), false, RowKind::Tasks(Tasks { items }));
            }
        }
    }
}
