//! `farcooler agent dispatch` (ov-457, ov-872): open a pane as an agent on
//! one or more tickets.
//!
//! The pane opens on the first ticket exactly as `task dispatch` would (same
//! refusals, warnings and board moves: it IS `task dispatch`), with
//! `TerminalCreate.agent` naming its role and the tickets after the first. The
//! runner does the rest (`farcooler_daemon::agent_panes`): it refuses a
//! ticket that isn't on the first one's board, briefs the agent on its role
//! and every ticket, records the pane as the agent with its tickets on the
//! line, starts a builder building the first on the pane's branch, and
//! exports `FARCOOLER_ROLE`. Nothing is recorded by hand.
//!
//! A reviewer or a lander works a ticket a builder already built, so its pane
//! leaves the card where it is: only a builder's dispatch puts the card on
//! the pane's worktree and into progress.
//!
//! **Seen to start** (ov-493). Then it watches the pane for up to
//! `FIRST_TURN_LOOKS` seconds and says whether the agent began its first
//! turn, or what stopped it: claude's folder-trust prompt, which is the
//! owner's to answer, another prompt, or nothing yet. The runner watches on
//! past that (`farcooler_daemon::watch::agent_start`) and messages the
//! orchestrator if it never starts.

use clap::Args;
use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, result};

use super::{Failed, RoleArg, board_in, card, unreadable};
use crate::tasks::{Board, DispatchLink, Lane, actor_for};

/// `agent dispatch`'s arguments.
#[derive(Debug, Clone, Args)]
pub(crate) struct AgentDispatchArgs {
    /// Builder, reviewer, lander or orchestrator.
    #[arg(long, value_enum)]
    role: RoleArg,
    /// A ticket it works, by key, in order: the pane opens on the first. Repeat for more.
    #[arg(long = "ticket", value_name = "KEY", required = true)]
    tickets: Vec<String>,
    /// The worktree to work in, by name or id.
    #[arg(long, required_unless_present = "new", conflicts_with = "new")]
    worktree: Option<String>,
    /// Make a new worktree with this name for it.
    #[arg(long, requires = "branch")]
    new: Option<String>,
    /// The new worktree's branch.
    #[arg(long, requires = "new")]
    branch: Option<String>,
    /// What the new worktree's branch starts from.
    #[arg(long, default_value = "HEAD")]
    base: String,
    /// The agent to start: claude, codex or cursor, with an optional `:model`.
    #[arg(long, default_value = "claude")]
    harness: String,
    /// What it's for, for people: "Phones show the plan".
    #[arg(long)]
    title: Option<String>,
    /// Dispatch even though the first ticket already has an agent.
    #[arg(long)]
    again: bool,
}

/// `agent dispatch`, answering what to print.
pub(super) async fn dispatch<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    args: AgentDispatchArgs,
    actor: &str,
    json: bool,
) -> Result<String, Failed> {
    if !link.capabilities().iter().any(|c| c == capability::BOARD_LINE) {
        return Err("This runner's Far Cooler can't open a pane as an agent yet. Update it and try again.".into());
    }
    let items = board_in(link, board, None, None).await?.items;
    let mut tickets: Vec<pb::Task> = Vec::new();
    for key in &args.tickets {
        let task = card(link, board, &items, key).await?;
        if !tickets.iter().any(|t| t.id == task.id) {
            tickets.push(task);
        }
    }
    let Some((first, rest)) = tickets.split_first() else { return Err("Name a ticket with --ticket.".into()) };
    let worktree = match (args.worktree, args.new, args.branch) {
        (Some(named), _, _) => Lane::Existing(named),
        (None, Some(name), Some(branch)) => Lane::New { name, branch, base: args.base },
        _ => return Err("Name a worktree with --worktree, or a new one with --new and --branch.".into()),
    };
    let agent = pb::AgentDispatch {
        role: args.role.wire() as i32,
        more_tickets: rest.iter().map(|t| t.key.clone()).collect(),
        title: args.title.clone().unwrap_or_default(),
    };
    let asked = crate::tasks::Dispatch {
        task: first,
        lane: worktree,
        preset: &args.harness,
        actor: actor_for(Some(actor))?,
        again: args.again,
        agent: Some(agent),
    };
    let done = crate::tasks::dispatch(link, asked, &mut |w| eprintln!("{w}")).await?;
    let said = crate::tasks::dispatched_output(&first.key, &done, json);
    let turn = if done.confirmed() { first_turn(link, done.worktree, done.terminal).await } else { FirstTurn::Unseen };
    let role = super::role_word(args.role.wire() as i32);
    if json {
        let mut out: serde_json::Value = serde_json::from_str(&said).map_err(|_| unreadable())?;
        out["role"] = serde_json::Value::String(role.to_lowercase());
        out["tickets"] = tickets.iter().map(|t| serde_json::Value::String(t.key.clone())).collect();
        out["first_turn"] = serde_json::Value::String(turn.word().into());
        if let FirstTurn::Stopped(q) = &turn {
            out["stopped_on"] = q.clone().map_or(serde_json::Value::Null, serde_json::Value::String);
        }
        return Ok(out.to_string());
    }
    let started = turn.said(&done.terminal_short).map(|s| format!("\n{s}")).unwrap_or_default();
    let then = if rest.is_empty() {
        String::new()
    } else {
        format!(", then {}", rest.iter().map(|t| t.key.as_str()).collect::<Vec<_>>().join(", "))
    };
    Ok(format!("{role} is starting on {}{then}.\n{said}{started}", first.key))
}

/// How many times dispatch looks for the agent's first turn, a second
/// apart: a claude given its opening prompt is working within a few.
const FIRST_TURN_LOOKS: usize = 20;

/// What dispatch saw of the agent's first turn.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FirstTurn {
    /// It's working, or already finished a turn.
    Began,
    /// It stopped on a prompt first, quoted when the runner read one.
    Stopped(Option<String>),
    /// Not yet, when dispatch stopped looking.
    NotYet,
    /// Its pane wasn't confirmed, so it wasn't looked for.
    Unseen,
}

impl FirstTurn {
    /// `first_turn` under `--json`.
    fn word(&self) -> &'static str {
        match self {
            FirstTurn::Began => "began",
            FirstTurn::Stopped(_) => "stopped",
            FirstTurn::NotYet => "not_yet",
            FirstTurn::Unseen => "unseen",
        }
    }

    /// The line it adds, naming the pane by `short`.
    fn said(&self, short: &str) -> Option<String> {
        let screen = format!("`farcooler terminal screen {short}`");
        Some(match self {
            FirstTurn::Began => "  its agent began its first turn".to_string(),
            FirstTurn::Stopped(Some(q)) if q == farcooler_core::activity::trust::QUESTION => {
                "  its agent hasn't started: Claude Code is asking whether to trust its folder. that's the \
                 owner's call, never yours: tell them it's waiting in Needs You"
                    .to_string()
            }
            FirstTurn::Stopped(Some(q)) => format!("  its agent hasn't started: it's stopped on a prompt, “{q}”. see {screen}"),
            FirstTurn::Stopped(None) => format!("  its agent hasn't started: it's stopped on a prompt. see {screen}"),
            FirstTurn::NotYet => format!(
                "  its agent hasn't begun its first turn yet: see {screen}. the runner messages you if it doesn't start"
            ),
            FirstTurn::Unseen => return None,
        })
    }
}

/// Look at `terminal` in `worktree` up to `FIRST_TURN_LOOKS` times until its
/// agent is working, has finished a turn, or is stopped on a prompt.
async fn first_turn<L: DispatchLink>(link: &mut L, worktree: uuid::Uuid, terminal: uuid::Uuid) -> FirstTurn {
    for look in 0..FIRST_TURN_LOOKS {
        if look > 0 {
            link.pause(std::time::Duration::from_secs(1)).await;
        }
        let Ok(r) = link.call(crate::req_for("terminal.list", worktree)).await else { continue };
        let Some(result::Value::TerminalList(l)) = r.value else { continue };
        let Some(t) = l.items.iter().find(|t| t.id.as_ref() == terminal.as_bytes()) else { continue };
        match pb::AgentActivity::try_from(t.activity) {
            Ok(pb::AgentActivity::Working | pb::AgentActivity::Done) => return FirstTurn::Began,
            Ok(pb::AgentActivity::Blocked) => return FirstTurn::Stopped(t.blocked_question.clone()),
            _ => {}
        }
    }
    FirstTurn::NotYet
}

#[cfg(test)]
#[path = "plan_agent_dispatch_tests.rs"]
mod tests;
