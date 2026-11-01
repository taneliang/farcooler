//! A pane opened as an agent (ov-457, ov-872): `terminal.create` with an
//! `agent`.
//!
//! An agent is recorded when it starts: an orchestrator, a builder, a reviewer
//! or a lander, working one or more tickets. `task dispatch` opens a pane on
//! one card; `agent dispatch` opens it as an agent in a role on its tickets,
//! the first as the pane's task, and the runner does the rest:
//!
//! - **refuses** a ticket that isn't on the first one's board (`resolve`),
//!   before anything is made;
//! - **briefs** the agent on its role and every ticket after the first, after
//!   the task's opening prompt (`brief`);
//! - **records** the pane as the agent, its tickets on the line and in its
//!   work, and a builder's first ticket on the pane's worktree and branch,
//!   building (`start`, `Store::start_pane_agent`). The pane's turns are then
//!   its tickets' spend;
//! - **exports** `FARCOOLER_ROLE` on every launch while the pane is that agent
//!   (`env`), read off the agent's record, never off the terminal's row.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1 as pb;
use farcooler_store::Store;
use farcooler_store::agent_panes::PaneStart;
use farcooler_store::agents::AgentRole;
use farcooler_store::models::{Actor, Task, Terminal, Worktree};
use uuid::Uuid;

/// The widest a card's title is quoted in a brief.
const TITLE_WIDTH: usize = 60;

/// A dispatch, resolved: the role, and every ticket in order, the pane's own
/// task first.
#[derive(Debug, Clone)]
pub(crate) struct Dispatch {
    pub role: AgentRole,
    pub tickets: Vec<Task>,
    pub title: String,
}

/// `agent` on `task`'s board: its role and its tickets, the task first.
/// Refused as `task_key` when there is no task to put it on, or a ticket isn't
/// on the task's board; as `role` without a role. `None` when no agent was
/// asked for.
pub(crate) fn resolve(store: &Store, task: Option<Uuid>, agent: Option<&pb::AgentDispatch>) -> Result<Option<Dispatch>> {
    let Some(agent) = agent else { return Ok(None) };
    let refused = DomainError::InvalidArgument { what: "task_key" };
    let first = store.get_task(task.ok_or(refused.clone())?)?;
    let role = match pb::AgentRole::try_from(agent.role) {
        Ok(pb::AgentRole::Orchestrator) => AgentRole::Orchestrator,
        Ok(pb::AgentRole::Builder) => AgentRole::Builder,
        Ok(pb::AgentRole::Reviewer) => AgentRole::Reviewer,
        Ok(pb::AgentRole::Lander) => AgentRole::Lander,
        _ => return Err(DomainError::InvalidArgument { what: "role" }),
    };
    let mut tickets = vec![first.clone()];
    for key in &agent.more_tickets {
        let found: Vec<Task> =
            store.tasks_with_key(None, key.trim())?.into_iter().filter(|t| t.workspace_id == first.workspace_id).collect();
        let [ticket] = found.as_slice() else { return Err(refused) };
        if !tickets.iter().any(|t| t.id == ticket.id) {
            tickets.push(ticket.clone());
        }
    }
    Ok(Some(Dispatch { role, tickets, title: agent.title.trim().to_string() }))
}

/// The role as a person reads it in a brief.
fn role_word(role: AgentRole) -> &'static str {
    match role {
        AgentRole::Orchestrator => "the orchestrator",
        AgentRole::Builder => "a builder",
        AgentRole::Reviewer => "a reviewer",
        AgentRole::Lander => "the lander",
    }
}

/// What the agent is told about its role and its tickets, after its task's
/// opening prompt: the tickets in order, and how to move on to the next. `cli`
/// is quoted as `opening_prompt`'s is.
pub(crate) fn brief(dispatch: &Dispatch, cli: &str) -> String {
    let list: Vec<String> = dispatch
        .tickets
        .iter()
        .map(|t| format!("{} ({})", t.key, crate::watch::answer_wake::one_line(&t.title, TITLE_WIDTH)))
        .collect();
    let role = role_word(dispatch.role);
    let work = match dispatch.role {
        AgentRole::Builder => {
            " Work them one at a time, each on a branch of its own, stacked on the one before when it builds on it. \
             When one is done, move it the way it says, then start the next"
        }
        AgentRole::Reviewer => " Review them one at a time, and say what you found on each before the next",
        AgentRole::Lander => " Land them in this order",
        AgentRole::Orchestrator => "",
    };
    format!(
        "This pane is {role}, working {list}.{work}: read each with {cli} task show <key>. \
         {cli} agent list shows who else is at work.",
        list = list.join(", then "),
    )
}

/// `prompt`, the task's opening prompt, with the dispatch's brief after it,
/// the CLI named as `opening_prompt` names it. `prompt` unchanged with none.
pub(crate) fn with_brief(dispatch: Option<&Dispatch>, prompt: Option<String>) -> Option<String> {
    let Some(dispatch) = dispatch else { return prompt };
    let cli = crate::service::shell_quote(&crate::service::cli_binary(std::env::current_exe().ok().as_deref()));
    let brief = brief(dispatch, &cli);
    Some(match prompt {
        Some(opening) => format!("{opening}\n\n{brief}"),
        None => brief,
    })
}

/// Record `term`'s pane, just made in `ws` running `preset`, as the agent
/// `dispatch` asked for: see this module's docs. The runner's own write.
pub(crate) fn start(store: &Store, dispatch: &Dispatch, term: &Terminal, preset: &str, ws: &Worktree) -> Result<()> {
    let workspace = dispatch.tickets[0].workspace_id;
    let start = PaneStart {
        terminal: term.id,
        preset: preset.to_string(),
        role: dispatch.role,
        tickets: dispatch.tickets.iter().map(|t| t.id).collect(),
        worktree: Some(ws.id),
        worktree_path: ws.worktree_path.clone(),
        branch: ws.branch.clone(),
        title: dispatch.title.clone(),
    };
    store.start_pane_agent(workspace, &start, Actor::Runner).map(|_| ())
}

impl crate::service::Service {
    /// `create_terminal_with_prompt` for `task`, opened as the agent `agent`
    /// asks for: see this module, which refuses, briefs, records and exports.
    pub async fn create_terminal_as_agent(
        &self,
        worktree_id: Uuid,
        title: &str,
        command_preset: &str,
        prompt: Option<&str>,
        task: Option<Uuid>,
        agent: Option<&pb::AgentDispatch>,
    ) -> Result<farcooler_store::models::Terminal> {
        self.open_terminal(worktree_id, title, command_preset, prompt, task, None, agent).await
    }
}

/// Record a pane just made: as the agent `dispatch` asked for, or as its
/// workspace's orchestrator when it took that seat (`orchestrator_of`). A
/// refusal is the caller's to undo the pane for.
pub(crate) fn started(
    store: &Store,
    dispatch: Option<&Dispatch>,
    orchestrator_of: Option<Uuid>,
    term: &Terminal,
    preset: &str,
    ws: &Worktree,
) -> Result<()> {
    if let Some(dispatch) = dispatch {
        start(store, dispatch, term, preset, ws)?;
    }
    record_orchestrator(store, orchestrator_of, term, preset);
    Ok(())
}

/// The workspace a new pane takes the orchestrator's seat of, if it does.
pub(crate) fn orchestrator_of(seat: Option<&crate::reps::Seat>) -> Option<Uuid> {
    match seat {
        Some(crate::reps::Seat::Orchestrator(workspace)) => Some(*workspace),
        _ => None,
    }
}

/// Record a workspace's orchestrator pane (`workspace`, when the pane took
/// that seat) as its orchestrator agent, so the plan lists it from the start.
/// Logged, not returned: the pane is made, and only the plan's row of it would
/// be missing.
pub(crate) fn record_orchestrator(store: &Store, workspace: Option<Uuid>, term: &Terminal, preset: &str) {
    let Some(workspace) = workspace else { return };
    let (harness, model) = match preset.split_once(':') {
        Some((h, m)) => (h.to_string(), Some(m.to_string())),
        None => (preset.to_string(), None),
    };
    let new = farcooler_store::agents::NewAgent {
        role: AgentRole::Orchestrator,
        place: farcooler_store::agents::AgentPlace::Pane { terminal: term.id },
        harness,
        session_id: None,
        session_cwd: None,
        model,
        title: None,
    };
    if let Err(e) = store.record_agent(workspace, &new) {
        tracing::warn!(terminal = %term.id, error = %e, "the orchestrator's pane was not recorded as an agent");
    }
}

/// `FARCOOLER_ROLE` and the agent's role word, for a pane recorded as an
/// agent. Nothing for any other pane.
pub(crate) fn env(store: &Store, term: &Terminal) -> Option<(String, String)> {
    let agent = store.pane_agent(term.id).ok().flatten()?;
    Some((farcooler_core::pane_env::ROLE.to_string(), agent.role.as_str().to_string()))
}

#[cfg(test)]
#[path = "agent_panes_tests.rs"]
mod tests;
