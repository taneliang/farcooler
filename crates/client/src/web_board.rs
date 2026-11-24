//! Browser board and ticket presentation derived from the runner's board records.
use super::STATUSES;
use farcooler_protocol::v1::{self as pb, TerminalRole, TerminalState, WorktreeState};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Intent {
    Create { draft: Draft },
    Edit { task: Uuid, version: String, draft: Draft },
    Mark { task: Option<Uuid>, through: i64 },
    Start { task: Uuid, guard: String },
}

#[derive(Deserialize)]
pub struct Draft {
    pub title: String,
    pub intent: String,
    pub acceptance: String,
    pub constraints: String,
    pub labels: String,
}

pub fn can_write(scope: &str) -> bool {
    matches!(scope, "control" | "host_admin")
}

fn movement(task: &Value) -> i64 {
    ["created_at", "updated_at", "status_since"].iter()
        .filter_map(|name| task[name].as_i64()).max().unwrap_or_default()
}

pub fn board(raw: &Value, scope: &str) -> Value {
    let tasks = raw["tasks"].as_array().map(Vec::as_slice).unwrap_or_default();
    let reads = &raw["reads"];
    let floor = reads["floor_ms"].as_i64();
    let marks: HashMap<_, _> = reads["opened"].as_array().into_iter().flatten()
        .filter_map(|mark| Some((mark["task_id"].as_str()?, mark["opened_ms"].as_i64()?))).collect();
    let mut columns: Vec<Vec<Value>> = vec![Vec::new(); STATUSES.len()];
    let mut through = 0;
    let mut unread = 0;
    for task in tasks {
        let Some(i) = STATUSES.iter().position(|status| task["status"] == status.wire) else { continue };
        let at = movement(task);
        through = through.max(at);
        let fresh = floor.is_some_and(|floor| {
            at > floor.max(task["id"].as_str().and_then(|id| marks.get(id).copied()).unwrap_or_default())
        });
        unread += usize::from(fresh);
        columns[i].push(json!({
            "id": task["id"], "key": task["key"], "title": task["title"],
            "unread": fresh.then_some("Unread"),
        }));
    }
    let write = can_write(scope);
    json!({
        "title": "Board", "refresh": "Refresh", "empty": "No tickets",
        "unread": floor.map(|_| format!("{unread} unread")),
        "create": write.then_some("New Ticket"),
        "markAll": (write && unread > 0).then(|| json!({"label":"Mark Board Read", "through":through})),
        "columns": STATUSES.iter().zip(columns).map(|(status, rows)| json!({
            "title": status.apple, "rows": rows,
        })).collect::<Vec<_>>(),
    })
}

pub fn form(task: Option<&Value>) -> Value {
    let lines = |key: &str, field: Option<&str>| task.and_then(|t| t[key].as_array()).into_iter()
        .flatten().filter_map(|line| field.map_or(line, |name| &line[name]).as_str())
        .collect::<Vec<_>>().join("\n");
    let text = |key: &str| task.and_then(|t| t[key].as_str()).unwrap_or_default();
    json!({
        "heading": if task.is_some() { "Edit Ticket" } else { "New Ticket" },
        "submit": if task.is_some() { "Save Ticket" } else { "Create Ticket" },
        "cancel": "Cancel",
        "fields": [
            {"name":"title", "label":"Title", "multiline":false, "required":true, "value":text("title")},
            {"name":"intent", "label":"Intent", "multiline":true, "value":text("intent")},
            {"name":"acceptance", "label":"Acceptance criteria", "multiline":true, "value":lines("acceptance", Some("text"))},
            {"name":"constraints", "label":"Constraints", "multiline":true, "value":lines("constraints", None)},
            {"name":"labels", "label":"Labels", "multiline":true, "value":lines("labels", None)}
        ],
    })
}

pub fn destinations(task: &pb::Task, terminals: &[pb::Terminal], trees: &[pb::Worktree]) -> (Option<Uuid>, Option<Uuid>) {
    let agent = terminals.iter().filter(|t| t.task_id.as_ref() == Some(&task.id))
        .filter(|t| matches!(TerminalRole::try_from(t.role), Ok(TerminalRole::Agent)))
        .filter(|t| matches!(TerminalState::try_from(t.state), Ok(TerminalState::Running | TerminalState::Starting)))
        .max_by_key(|t| &t.id);
    let tree = task.worktree_id.as_ref().or_else(|| agent.map(|t| &t.worktree_id))
        .and_then(|id| trees.iter().find(|tree| tree.id == id))
        .filter(|tree| matches!(WorktreeState::try_from(tree.state), Ok(WorktreeState::Ready | WorktreeState::Active)));
    (agent.and_then(|t| Uuid::from_slice(&t.id).ok()), tree.and_then(|t| Uuid::from_slice(&t.id).ok()))
}

pub fn ticket(
    value: &Value, scope: &str, shared_reads: bool, start: Option<&str>,
    (agent, tree): (Option<Uuid>, Option<Uuid>),
) -> Value {
    let write = can_write(scope);
    json!({
        "task": {"id":value["id"], "key":value["key"], "title":value["title"]},
        "version": value["resource_version"].to_string(), "back": "Back to Board", "activity": "Activity and Notes",
        "status": STATUSES.iter().find(|s| value["status"] == s.wire).map(|s| s.apple),
        "edit": write.then(|| form(Some(value))),
        "start": start.filter(|_| write).map(|guard| json!({"label":"Start Ticket", "guard":guard})),
        "mark": (write && shared_reads).then(|| movement(value)),
        "agent": agent.filter(|_| write).map(|id| json!({"id":id, "label":"Open Agent"})),
        "worktree": tree.map(|id| json!({"id":id, "label":"Open Worktree"})),
        "sections": [
            {"label":"Intent", "lines":value["intent"].as_str().filter(|s| !s.is_empty()).into_iter().collect::<Vec<_>>()},
            {"label":"Acceptance criteria", "criteria":value["acceptance"]},
            {"label":"Constraints", "lines":value["constraints"]},
            {"label":"Labels", "lines":value["labels"]},
        ],
    })
}

pub fn failure(word: &str, code: &str, what: &str) -> &'static str {
    match (word, code, what) {
        (_, "scope-denied", _) => "This action needs control of the runner.",
        (_, _, "ticket_version") => "This ticket changed while you were editing. Copy your draft, then reopen the ticket before saving.",
        (_, _, "ticket_start") => "The workflow changed or refused to start this ticket. Reopen the ticket to see its current state.",
        (_, _, "ticket_title" | "title") => "Enter a ticket title with 200 characters or fewer.",
        (_, "not-found", _) => "This ticket is no longer on this board.",
        ("runner_too_old", _, _) => "Update Far Cooler on the runner to open this board.",
        ("closed" | "no_answer", _, _) => "The runner didn’t answer. Refresh to check its current state.",
        _ => "The runner couldn’t complete this request. Refresh to check its current state.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_columns_use_shared_read_marks_and_runner_clocks() {
        let task = |id: &str, status: &str, at| json!({"id":id, "status":status, "updated_at":at});
        let raw = json!({
            "tasks": [task("seen", "backlog", 20), task("new", "backlog", 21), task("old", "in_review", 10)],
            "reads": {"floor_ms":10, "opened":[{"task_id":"seen","opened_ms":20}]}
        });
        let view = board(&raw, "control");
        assert_eq!(view["unread"], "1 unread");
        assert_eq!(view["markAll"]["through"], 21);
        let backlog = &view["columns"][1];
        assert_eq!(backlog["title"], "Backlog");
        assert_eq!(backlog["rows"][0]["id"], "seen");
        assert!(backlog["rows"][0]["unread"].is_null());
        assert_eq!(backlog["rows"][1]["unread"], "Unread");
        let read = board(&raw, "read");
        assert_eq!(read["unread"], "1 unread");
        assert!(read["create"].is_null());
        assert!(read["markAll"].is_null());
        assert!(board(&json!({"tasks":raw["tasks"]}), "control")["unread"].is_null());
    }

    #[test]
    fn detail_uses_exact_agent_and_worktree_and_does_not_offer_read_only_writes() {
        let (id, other, terminal, checkout) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let bytes = |id: Uuid| bytes::Bytes::copy_from_slice(id.as_bytes());
        let task = pb::Task { id: bytes(id), status: pb::TaskStatus::Todo as i32,
            resource_version: u64::MAX, updated_at: 91, ..Default::default() };
        let agent = pb::Terminal { task_id: Some(bytes(id)), id: bytes(terminal),
            worktree_id: bytes(checkout), role: TerminalRole::Agent as i32,
            state: TerminalState::Running as i32, ..Default::default() };
        let unrelated = pb::Terminal { task_id: Some(bytes(other)), id: bytes(Uuid::now_v7()), ..agent.clone() };
        let tree = pb::Worktree { id: bytes(checkout), state: WorktreeState::Ready as i32, ..Default::default() };
        let links = destinations(&task, &[agent, unrelated], &[tree]);
        assert_eq!(links, (Some(terminal), Some(checkout)));
        let raw = crate::tasks_json::task_json(&task, 100);
        let control = ticket(&raw, "control", true, Some("guard"), links);
        assert_eq!(control["version"], u64::MAX.to_string());
        assert_eq!(control["agent"]["id"], json!(terminal));
        assert_eq!(control["worktree"]["id"], json!(checkout));
        assert_eq!(control["mark"], 91);
        let read = ticket(&raw, "read", true, Some("guard"), links);
        for key in ["edit", "start", "mark", "agent"] {
            assert!(read[key].is_null(), "{key}");
        }
    }
}
