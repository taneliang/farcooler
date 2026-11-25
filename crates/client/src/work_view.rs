//! Presentation for starting and recovering work on a paired runner.
use farcooler_protocol::v1::{self as wire, AgentActivity, TerminalRole, TerminalState, WorktreeState};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Inventory {
    pub scope: String,
    pub workspaces: Vec<Workspace>,
    pub repositories: Vec<Choice>,
    pub worktrees: Vec<Worktree>,
    pub terminals: Vec<Terminal>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub is_main: bool,
    pub repository: String,
}

#[derive(Serialize, Deserialize)]
pub struct Choice {
    pub id: String,
    pub label: String,
}

#[derive(Serialize, Deserialize)]
pub struct Worktree {
    pub id: String,
    pub label: String,
    pub repository: String,
    pub workspace: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct Terminal {
    pub id: String,
    pub title: String,
    pub worktree: String,
    pub workspace: Option<String>,
    pub agent: bool,
    pub state: String,
    pub columns: u32,
    pub rows: u32,
}

#[derive(Default, Deserialize)]
#[serde(default)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub workspace: Option<String>,
    pub preferred_workspace: Option<String>,
    pub repository: Option<String>,
    pub worktree: Option<String>,
    pub terminal: Option<String>,
    pub stream_ended: bool,
}

pub fn inventory(
    scope: &str,
    workspaces: Vec<wire::Workspace>,
    repositories: Vec<wire::Repository>,
    worktrees: Vec<wire::Worktree>,
    terminals: Vec<wire::Terminal>,
) -> Inventory {
    Inventory {
        scope: scope.into(),
        workspaces: workspaces.into_iter().filter_map(|w| Some(Workspace {
            id: id(&w.id)?,
            name: w.name,
            is_main: w.is_main,
            repository: id(&w.repository_id)?,
        })).collect(),
        repositories: repositories.into_iter().filter_map(|r| Some(Choice {
            id: id(&r.id)?,
            label: r.display_name,
        })).collect(),
        worktrees: worktrees.into_iter().filter(|w| {
            matches!(WorktreeState::try_from(w.state), Ok(WorktreeState::Ready | WorktreeState::Active))
        }).filter_map(|w| Some(Worktree {
            id: id(&w.id)?,
            label: if w.branch.is_empty() || w.branch == w.task_name { w.task_name } else { format!("{} · {}", w.task_name, w.branch) },
            repository: id(&w.repository_id)?,
            workspace: w.workspace_id.as_deref().and_then(id),
        })).collect(),
        terminals: terminals.into_iter().filter_map(terminal).collect(),
    }
}

fn id(bytes: &[u8]) -> Option<String> {
    Uuid::from_slice(bytes).ok().map(|u| u.to_string())
}

pub fn terminal(t: wire::Terminal) -> Option<Terminal> {
    let agent = matches!(TerminalRole::try_from(t.role), Ok(TerminalRole::Agent | TerminalRole::Orchestrator))
        || !matches!(AgentActivity::try_from(t.activity), Ok(AgentActivity::None | AgentActivity::Unspecified) | Err(_));
    Some(Terminal {
        id: id(&t.id)?,
        worktree: id(&t.worktree_id).unwrap_or_default(),
        title: if t.title.is_empty() { "Terminal".into() } else { t.title },
        workspace: t.workspace_id.as_deref().and_then(id),
        state: TerminalState::try_from(t.state).map(|s| s.as_str_name().trim_start_matches("TERMINAL_STATE_").to_ascii_lowercase()).unwrap_or_else(|_| "unknown".into()),
        columns: t.columns,
        rows: t.rows,
        agent,
    })
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Intent {
    Create { repository: Uuid, workspace: Option<Uuid>, name: String, branch: String },
    Start { worktree: Uuid, preset: Preset },
    Rename { terminal: Uuid, name: String },
    Close { terminal: Uuid },
    Restart { terminal: Uuid },
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    Shell,
    Codex,
    Claude,
}

impl Preset {
    pub fn label(self) -> &'static str {
        match self { Self::Shell => "Shell", Self::Codex => "Codex", Self::Claude => "Claude Code" }
    }

    pub fn command(self) -> &'static str {
        match self { Self::Shell => "shell", Self::Codex => "codex", Self::Claude => "claude" }
    }
}

pub const TAKE_OVER_TERMINAL: &str = "Take Over";

pub const READ_ONLY: &str = "This browser has read-only access to the runner. A device with control access can open the terminal and make changes.";

pub fn failure(word: &str, code: &str, what: &str) -> &'static str {
    match (word, code, what) {
        (_, "scope-denied", _) => READ_ONLY,
        (_, "not-yours", _) => "Your account can’t manage this terminal on the runner.",
        (_, "not-found", _) => "The selected worktree or terminal is no longer on the runner.",
        (_, "branch-exists", _) => "That branch already exists. Pick a different branch name or select its worktree.",
        (_, "worktree-exists", _) => "A worktree already uses that name. Choose it from the list or enter another name.",
        (_, "invalid-argument", "worktree_name") => "Enter a worktree name between 1 and 60 characters that includes a letter or number.",
        (_, "invalid-argument", "branch_name") => "Enter a valid Git branch name, such as feature/fix-build.",
        (_, "invalid-argument", _) => "Check the name or branch and try again.",
        (_, "running-processes", _) => "The terminal is running. Refresh its state before trying again.",
        (_, _, reason @ ("writer_busy" | "writer_taken_over" | "writer_available")) =>
            farcooler_core::error::sentence(reason).unwrap_or_default(),
        (_, _, "terminal_state_unknown" | "terminal_state_changed") => "The terminal’s state changed. Refresh it before trying again.",
        ("closed", _, _) => "The runner connection was lost. Reconnect and check whether the action finished before trying again.",
        ("runner_too_old", _, _) => "Update Far Cooler on the runner to use this action.",
        _ => "The runner couldn’t finish this action. Refresh the list and check its current state before trying again.",
    }
}

pub fn view(inventory: &Inventory, selection: &Selection) -> Value {
    let Inventory { scope, workspaces, repositories, worktrees, terminals } = inventory;
    // Control grants terminal operations. Input ownership comes from the
    // runner's separate terminal.writer response.
    let control = matches!(scope.as_str(), "control" | "host_admin");
    let readable = control || scope != "read";
    let requested = selection.terminal.as_deref().and_then(|id| terminals.iter().find(|t| t.id == id));
    let main = |repository: Option<&str>| {
        let in_repo = |w: &&Workspace| repository.is_none_or(|r| w.repository == r);
        workspaces.iter().filter(in_repo).find(|w| w.is_main)
            .or_else(|| workspaces.iter().find(in_repo))
    };
    let tree_for = |terminal: &Terminal| worktrees.iter().find(|w| w.id == terminal.worktree);
    let by_id = |id: &str| workspaces.iter().find(|w| w.id == id);
    let owner = |t: &Terminal| {
        t.workspace.as_deref().and_then(by_id)
            .or_else(|| main(tree_for(t).map(|w| w.repository.as_str())))
    };
    let tree_owner = |tree: &Worktree| {
        tree.workspace.as_deref().and_then(by_id).or_else(|| main(Some(&tree.repository)))
    };
    let requested_tree = selection.worktree.as_deref().and_then(|id| worktrees.iter().find(|tree| tree.id == id));
    let linked_workspace = requested.and_then(owner).or_else(|| requested_tree.and_then(tree_owner));
    let requested_workspace = selection.workspace.as_deref().and_then(|id| workspaces.iter().find(|w| w.id == id));
    let missing_workspace = selection.workspace.is_some() && requested_workspace.is_none() && linked_workspace.is_none();
    let workspace = if missing_workspace { None } else {
        linked_workspace.or(requested_workspace).or_else(|| {
            selection.preferred_workspace.as_deref()
                .and_then(|id| workspaces.iter().find(|w| w.id == id && selection.repository.as_ref().is_none_or(|r| w.repository == *r)))
        }).or_else(|| main(selection.repository.as_deref()))
    };
    let repository = workspace.map(|w| w.repository.as_str())
        .or(selection.repository.as_deref()).or_else(|| repositories.first().map(|r| r.id.as_str()));
    let owned_trees: Vec<_> = worktrees.iter().filter(|w| {
        let owner = tree_owner(w);
        !missing_workspace && Some(w.repository.as_str()) == repository
            && owner.map(|w| &w.id) == workspace.map(|w| &w.id)
    }).collect();
    let visible: Vec<_> = terminals.iter().filter(|t| {
        !missing_workspace && (workspaces.is_empty() || owner(t).map(|w| &w.id) == workspace.map(|w| &w.id))
    }).collect();
    let selected = requested.or_else(|| {
        if selection.terminal.is_some() { return None; }
        visible.iter().copied().find(|t| t.state == "running").or_else(|| visible.first().copied())
    });
    let chosen_tree = if selection.worktree.is_some() {
        requested_tree.filter(|w| owned_trees.iter().any(|owned| owned.id == w.id))
    } else {
        selected.and_then(tree_for).filter(|w| owned_trees.iter().any(|owned| owned.id == w.id))
            .or_else(|| owned_trees.first().copied())
    };

    let terminal_rows: Vec<_> = visible.into_iter().map(|t| {
        let running = t.state == "running";
        let recover = if t.state == "lost" && control {
            json!({"action": "restart", "label": "Recover Terminal"})
        } else if running && readable && selected.is_some_and(|s| s.id == t.id) && selection.stream_ended {
            json!({"action": "reattach", "label": "Reconnect Terminal"})
        } else {
            Value::Null
        };
        let state = match t.state.as_str() {
            "running" => "Running", "starting" => "Starting", "exited" => "Exited",
            "error" => "Error", "lost" => "Lost", _ => "State unknown",
        };
        let close = control && matches!(t.state.as_str(), "running" | "starting" | "exited" | "error" | "lost");
        json!({
            "id": t.id, "title": t.title, "agent": t.agent, "state": state, "running": running,
            "attach": readable && running,
            "attachNote": if !readable { READ_ONLY } else if t.state == "lost" {
                "This terminal’s process was lost. Recover it to continue."
            } else if !running { "This terminal is currently unavailable." } else { "" },
            "access": if readable { "" } else { "Read-only" },
            "rename": control.then_some(json!({"label": "Rename", "prompt": "Terminal name"})),
            "close": close.then(|| json!({
                "label": "Close Terminal",
                "prompt": if matches!(t.state.as_str(), "running" | "starting") {
                    format!("Close “{}” and stop its running process?", t.title)
                } else { format!("Close “{}”?", t.title) },
            })),
            "recover": recover,
        })
    }).collect();

    let notice = if missing_workspace {
        "This workspace is no longer on the runner."
    } else if selected.is_none() && selection.terminal.is_some() {
        "This terminal is no longer on the runner."
    } else if selection.worktree.is_some() && chosen_tree.is_none() {
        "This checkout is no longer available."
    } else if !readable {
        READ_ONLY
    } else if selection.stream_ended {
        "This terminal’s connection ended. Reconnect it to continue."
    } else if selected.is_none() {
        "No terminals here yet."
    } else {
        ""
    };
    json!({
        "workspaces": workspaces, "repositories": repositories, "worktrees": owned_trees,
        "workspace": workspace.map(|w| &w.id), "repository": repository,
        "worktree": chosen_tree.map(|w| &w.id), "terminal": selected.map(|t| &t.id), "terminals": terminal_rows,
        "notice": notice,
        "missingTerminal": selected.is_none() && selection.terminal.is_some(),
        "labels": {
            "open": "Start Work", "link": "Workspace Link", "repository": "Repository", "worktree": "Worktree",
            "close": "Done", "refresh": "Refresh", "emptyTree": "No worktrees available",
        },
        "create": (control && repository.is_some() && !missing_workspace).then_some(json!({
            "open": "New Worktree", "submit": "Create Worktree", "name": "Worktree Name", "branch": "Branch",
        })),
        "start": (control && chosen_tree.is_some()).then(|| json!({
            "preset": "Terminal Type", "submit": "Start Terminal",
            "presets": ([Preset::Shell, Preset::Codex, Preset::Claude].map(|preset| json!({"id": preset.command(), "label": preset.label()}))),
        })),
    })
}

#[cfg(test)]
mod tests;
