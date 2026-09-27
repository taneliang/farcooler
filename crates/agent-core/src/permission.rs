//! What a claude permission ask offers, whichever door it came in by.
//!
//! Claude asks for permission two ways: as a `can_use_tool` control request on
//! the ACP path, and as a `PermissionRequest` hook while it runs as a TUI. Both
//! become the same `AgentEvent::Permission`, and a phone answers either by the
//! option ids built here. One builder is what keeps the two from drifting: a
//! hook ask whose ids differed from the ACP ask's would be answered with an id
//! nothing on the daemon side recognizes.

use crate::event::PermissionOption;

/// What a tool row should say it is doing.
///
/// The command for a shell, the path for a file operation, the tool's own name
/// otherwise. A row reading "Bash" tells you less than the command it ran.
///
/// The task tools are here because of what the fallback did to them: a session
/// that called `TaskCreate` five times drew five rows every one of which said
/// literally "TaskCreate", which is the worst version of this — five rows that
/// are not merely uninformative but indistinguishable. `TaskUpdate` has the
/// same shape of problem and less to work with, since an update that only moves
/// a status carries nothing but `taskId`.
///
/// `TaskList` deliberately gets no case: it takes no parameters at all, so its
/// own name is the whole truth about it.
pub fn claude_tool_title(name: &str, input: &serde_json::Value) -> String {
    let field = |key: &str| input[key].as_str().map(str::to_string);
    match name {
        "Bash" | "BashOutput" => field("command").unwrap_or_else(|| name.to_string()),
        "Read" | "Write" | "Edit" | "NotebookEdit" => {
            field("file_path").unwrap_or_else(|| name.to_string())
        }
        "Glob" | "Grep" => field("pattern").unwrap_or_else(|| name.to_string()),
        "WebFetch" => field("url").unwrap_or_else(|| name.to_string()),
        "WebSearch" => field("query").unwrap_or_else(|| name.to_string()),
        // `Task` and `Agent` are the SUBAGENT dispatch, unrelated to the
        // `Task*` tools below despite the shared prefix.
        "Task" | "Agent" => field("description").unwrap_or_else(|| name.to_string()),
        "TaskCreate" => field("subject").unwrap_or_else(|| name.to_string()),
        "TaskUpdate" => task_update_title(input).unwrap_or_else(|| name.to_string()),
        _ => name.to_string(),
    }
}

/// A `TaskUpdate` row, named for whatever it actually says.
///
/// A new `subject` is a rename and IS the row. Otherwise the only required
/// field is `taskId`, so the row is the task it moved plus the state it moved
/// it to — because the update most often made is a status change, and without
/// the status three of those in a row would read identically, which is the
/// complaint that brought this whole function here.
fn task_update_title(input: &serde_json::Value) -> Option<String> {
    if let Some(subject) = input["subject"].as_str().filter(|s| !s.is_empty()) {
        return Some(subject.to_string());
    }
    let id = claude_task_key(input["taskId"].as_str()?);
    Some(match input["status"].as_str().filter(|s| !s.is_empty()) {
        Some(status) => format!("Task #{id}: {status}"),
        None => format!("Task #{id}"),
    })
}

/// A task id as claude's task tools key tasks by.
///
/// `#` is display sugar. The result sentence writes `Task #2` and `TaskUpdate`
/// is documented to take `"2"`, so both are trimmed to the same key. That costs
/// nothing and means a model that writes `taskId: "#2"` still hits the task it
/// meant, instead of being discarded as an id nobody created.
pub fn claude_task_key(id: &str) -> String {
    id.trim().trim_start_matches('#').trim().to_string()
}

/// The buttons a claude permission ask offers: allow this once, or deny it.
///
/// The ids `allow` and `deny` are the contract. A phone echoes one back, and
/// the daemon turns it into claude's own answer, so they are the same on both
/// paths. The allow button is named for what the tool will do, by
/// `claude_tool_title`, because "Allow Bash" is not something a person can
/// decide on.
pub fn permission_options(tool_name: &str, input: &serde_json::Value) -> Vec<PermissionOption> {
    vec![
        PermissionOption {
            id: "allow".into(),
            name: format!("Allow {}", claude_tool_title(tool_name, input)),
            kind: "allow_once".into(),
        },
        PermissionOption {
            id: "deny".into(),
            name: "Deny".into(),
            kind: "reject_once".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_options_are_allow_and_deny_by_those_ids() {
        let options = permission_options("Read", &serde_json::json!({ "file_path": "/a" }));
        let ids: Vec<&str> = options.iter().map(|o| o.id.as_str()).collect();
        let kinds: Vec<&str> = options.iter().map(|o| o.kind.as_str()).collect();
        assert_eq!(ids, ["allow", "deny"]);
        assert_eq!(kinds, ["allow_once", "reject_once"]);
        assert_eq!(options[1].name, "Deny");
    }

    #[test]
    fn a_bash_ask_is_named_for_its_command() {
        let options = permission_options("Bash", &serde_json::json!({ "command": "touch x" }));
        assert_eq!(options.first().map(|o| o.name.as_str()), Some("Allow touch x"));
    }
}
