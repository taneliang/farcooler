//! `farcooler agent`: the agents at work on a board (ov-503, ov-872).
//!
//! ```text
//! farcooler agent list
//! farcooler agent dispatch --role builder --ticket KEY [--ticket KEY]... (--worktree W | --new N --branch B)
//!     [--harness claude] [--title T] [--again]
//! farcooler agent record --role reviewer --subagent ID [--parent TERMINAL] [--harness claude] [--model M]
//!     [--title T] [--ticket KEY]...
//! farcooler agent work AGENT --ticket KEY --role build|fix|review|land [--verdict V]
//! farcooler agent end AGENT --verdict finished|failed|stopped|handed_back
//! farcooler agent stop AGENT|TICKET [--reason R]
//! ```
//!
//! An agent is recorded when it starts: the orchestrator, a builder, a
//! reviewer or a lander, a pane or a subagent, working any number of tickets.
//! `dispatch` opens a pane as one (`plan_agent_dispatch.rs`); `record` records
//! a subagent an orchestrator started itself, its parent pane read from
//! `FARCOOLER_ACTOR` and its session from `CLAUDE_CODE_SESSION_ID` the way
//! `task worker` reads them. Each ticket an agent is given goes on the line if
//! it isn't there. An agent is named by its short id, as `agent list` and
//! `plan --json` print it, or its full id.
//!
//! `stop` is how the orchestrator stops an agent a person asked it to stop:
//! no screen has a Stop button. A pane stops. A Claude subagent's next tool
//! call is refused, telling it to stop and hand back; a command already running
//! finishes first, so it reads "stopping" until it ends. Another harness's
//! subagent can't be reached, and the refusal says so. It names the agent as
//! the others do, or by a ticket it works when one agent works it.
//!
//! `list` shows every agent still at work, working ones first, each with its
//! role, harness or model, title, tickets, what it's doing now and its spend.
//! The header's spend is the last 24 hours of every agent, those that ended
//! too. Tokens come first and dollars second, and every dollar is
//! API-equivalent, as everywhere else on the plan.
//!
//! A child of `plan.rs`, whose board and refusal helpers it shares.

use clap::{Args, Subcommand, ValueEnum};
use farcooler_core::usage_words::{dollars, tokens};
use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, request, result};
use serde_json::{Value, json};

use super::overview_text::{agent_name, key_of};
use super::{
    Common, Failed, ago, board_in, card, cards_only, count, get_plan, ids_of, needs_the_line, on_a_board, plan_json,
    role_word, send_saying, total, unreadable,
};
use crate::tasks::{Board, DispatchLink, refused};
use crate::{Unresolved, expect_value, id_bytes};

/// Said to a runner from before `agent.stop`, before anything is sent.
pub(super) const NEEDS_STOP: &str = "This runner needs an update to stop an agent.";

#[path = "plan_agent_dispatch.rs"]
mod dispatch;
#[path = "plan_agent_start.rs"]
mod start;

/// `farcooler agent`.
#[derive(Debug, Clone, Args)]
pub(crate) struct AgentArgs {
    #[command(flatten)]
    pub(super) common: Common,
    #[command(subcommand)]
    pub(super) cmd: AgentCmd,
}

#[derive(Debug, Clone, Subcommand)]
pub(crate) enum AgentCmd {
    /// The agents at work: their roles, tickets, what they're doing now and what they spent.
    List,
    /// Open a pane as an agent on one or more tickets: the pane works the first, briefed on them all.
    Dispatch(dispatch::AgentDispatchArgs),
    /// Record a subagent you started, and the tickets it works.
    ///
    /// Its parent pane is the one this runs in, read from FARCOOLER_ACTOR,
    /// unless --parent names another.
    Record {
        /// Orchestrator, builder, reviewer or lander.
        #[arg(long, value_enum)]
        role: RoleArg,
        /// The subagent's id, from its launch result.
        #[arg(long)]
        subagent: String,
        /// The pane it runs under, by terminal id.
        #[arg(long, value_name = "TERMINAL")]
        parent: Option<String>,
        /// `claude`, `codex`, or an adapter's word.
        #[arg(long, default_value = "claude")]
        harness: String,
        /// Its model: "sonnet", "opus".
        #[arg(long)]
        model: Option<String>,
        /// What it's for, for people: "Review the phones' plan".
        #[arg(long)]
        title: Option<String>,
        /// A ticket it works, by key, in order. Repeat for more.
        #[arg(long = "ticket", value_name = "KEY")]
        tickets: Vec<String>,
    },
    /// Start an agent's work on a ticket, or end it with --verdict.
    Work {
        /// The agent, by its short id.
        agent: String,
        /// The ticket, by key.
        #[arg(long, value_name = "KEY")]
        ticket: String,
        /// What it does: build, fix, review or land.
        #[arg(long, value_enum)]
        role: WorkArg,
        /// How the work ended. Without one, the work starts.
        #[arg(long, value_enum)]
        verdict: Option<VerdictArg>,
    },
    /// Stop an agent a person asked you to stop: a pane stops, a Claude subagent is told to.
    ///
    /// A Claude subagent's next tool call is refused with a message telling it
    /// to stop and hand back. A command it's already running finishes first.
    Stop {
        /// The agent, by its short id, or by a ticket key only it works.
        agent: String,
        /// Why, in a line: the subagent reads it.
        #[arg(long)]
        reason: Option<String>,
    },
    /// An agent has finished: every ticket it still works ends the same way.
    End {
        /// The agent, by its short id.
        agent: String,
        /// Finished, failed, stopped or handed_back.
        #[arg(long, value_enum)]
        verdict: EndArg,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum RoleArg {
    Orchestrator,
    Builder,
    Reviewer,
    Lander,
}

impl RoleArg {
    pub(super) fn wire(self) -> pb::AgentRole {
        match self {
            RoleArg::Orchestrator => pb::AgentRole::Orchestrator,
            RoleArg::Builder => pb::AgentRole::Builder,
            RoleArg::Reviewer => pb::AgentRole::Reviewer,
            RoleArg::Lander => pb::AgentRole::Lander,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum WorkArg {
    Build,
    Fix,
    Review,
    Land,
}

impl WorkArg {
    fn wire(self) -> pb::WorkRole {
        match self {
            WorkArg::Build => pb::WorkRole::Build,
            WorkArg::Fix => pb::WorkRole::Fix,
            WorkArg::Review => pb::WorkRole::Review,
            WorkArg::Land => pb::WorkRole::Land,
        }
    }

    fn doing(self) -> &'static str {
        match self {
            WorkArg::Build => "building",
            WorkArg::Fix => "fixing",
            WorkArg::Review => "reviewing",
            WorkArg::Land => "landing",
        }
    }
}

/// How work on a ticket ends: the runner's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum VerdictArg {
    Finished,
    Failed,
    Stopped,
    #[value(name = "handed_back", alias = "handed-back")]
    HandedBack,
    Approved,
    #[value(name = "changes_asked", alias = "changes-asked")]
    ChangesAsked,
}

impl VerdictArg {
    fn word(self) -> &'static str {
        match self {
            VerdictArg::Finished => "finished",
            VerdictArg::Failed => "failed",
            VerdictArg::Stopped => "stopped",
            VerdictArg::HandedBack => "handed_back",
            VerdictArg::Approved => "approved",
            VerdictArg::ChangesAsked => "changes_asked",
        }
    }

    fn said(self) -> &'static str {
        match self {
            VerdictArg::HandedBack => "handed back",
            VerdictArg::ChangesAsked => "asked for changes",
            other => other.word(),
        }
    }
}

/// How an agent ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum EndArg {
    Finished,
    Failed,
    Stopped,
    #[value(name = "handed_back", alias = "handed-back")]
    HandedBack,
}

impl EndArg {
    fn verdict(self) -> VerdictArg {
        match self {
            EndArg::Finished => VerdictArg::Finished,
            EndArg::Failed => VerdictArg::Failed,
            EndArg::Stopped => VerdictArg::Stopped,
            EndArg::HandedBack => VerdictArg::HandedBack,
        }
    }
}

/// What `agent record` reads from the session it runs in, as `task worker`
/// does: the pane (`FARCOOLER_ACTOR`, `agent:<terminal id>`) and claude's
/// session (`CLAUDE_CODE_SESSION_ID`, the same in its subagents).
#[derive(Debug, Clone, Default)]
pub(super) struct Session {
    pub(super) actor: Option<String>,
    pub(super) claude_session: Option<String>,
}

impl Session {
    pub(super) fn from_process() -> Session {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        Session { actor: var(farcooler_core::pane_env::ACTOR), claude_session: var("CLAUDE_CODE_SESSION_ID") }
    }

    /// The pane this runs in, when it's an agent's.
    fn pane(&self) -> Option<bytes::Bytes> {
        let id = self.actor.as_deref()?.trim().strip_prefix("agent:")?;
        uuid::Uuid::parse_str(id.trim()).ok().map(id_bytes)
    }
}

/// This module's sentences, said before `plan.rs`'s, where a word means
/// something of an agent's own.
fn said_here(what: &str) -> Option<&'static str> {
    Some(match what {
        "place" => "Name the subagent with --subagent, the id from its launch result.",
        "task_key" => "Those tickets aren't all on this board.",
        "stop_unsupported" => {
            "Far Cooler can stop a pane, or a Claude subagent, but not this harness's subagents. Stop it with your own harness's tool, or stop the orchestrator's pane it runs under."
        }
        "reason" => "A reason is one line, up to 200 characters.",
        _ => return None,
    })
}

/// An agent's write, refused in this module's words first.
async fn send_agent<L: DispatchLink>(link: &mut L, board: &Board, method: &str, p: request::Payload) -> Result<pb::Agent, Failed> {
    match send_saying(link, board, method, p, said_here).await? {
        result::Value::Agent(a) => Ok(a),
        _ => Err(unreadable()),
    }
}

/// One `agent` command on `board`, answering what to print.
pub(super) async fn run_on<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    cmd: AgentCmd,
    actor: &str,
    json: bool,
) -> Result<String, Failed> {
    run_in(link, board, cmd, actor, json, &Session::from_process()).await
}

/// `run_on`, with the session it runs in given.
pub(super) async fn run_in<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    cmd: AgentCmd,
    actor: &str,
    json: bool,
    session: &Session,
) -> Result<String, Failed> {
    needs_the_line(&link.capabilities())?;
    let (board, ws) = on_a_board(link, board).await?;
    let board = &board;
    match cmd {
        AgentCmd::List => {
            let plan = get_plan(link, &ws, false).await?;
            Ok(if json { agents_json(&plan).to_string() } else { agents_text(&plan) })
        }
        AgentCmd::Dispatch(args) => dispatch::dispatch(link, board, args, actor, json).await,
        AgentCmd::Record { role, subagent, parent, harness, model, title, tickets } => {
            let items = if tickets.is_empty() { Vec::new() } else { board_in(link, board, None, None).await?.items };
            let task_ids = ids_of(link, board, &items, &tickets).await?;
            let parent = match parent {
                Some(named) => Some(terminal_id(link, &named).await?),
                None => session.pane(),
            };
            let harness = harness.trim().to_lowercase();
            let p = request::Payload::AgentRecord(pb::AgentRecord {
                workspace_id: ws.clone(),
                role: role.wire() as i32,
                session_id: if harness == "claude" { session.claude_session.clone() } else { None },
                harness,
                terminal_id: None,
                subagent_id: subagent.trim().to_string(),
                parent_terminal_id: parent,
                model: model.filter(|m| !m.trim().is_empty()),
                title: title.filter(|t| !t.trim().is_empty()),
                task_ids,
                actor: actor.into(),
            });
            let agent = send_agent(link, board, "agent.record", p).await?;
            let plan = get_plan(link, &ws, false).await?;
            let plan = pb::Plan { agents: vec![agent.clone()], ..cards_only(&plan, &items) };
            Ok(if json {
                farcooler_client::plan_json::agent_json(&plan, &agent).to_string()
            } else {
                let on = keys_of(&plan, &agent);
                let on = if on.is_empty() { String::new() } else { format!(", working {on}") };
                format!("Recorded {}{on}.", agent_name(&plan, &agent.id))
            })
        }
        AgentCmd::Work { agent, ticket, role, verdict } => {
            let plan = get_plan(link, &ws, false).await?;
            let id = find_agent(&plan, &agent)?;
            let items = board_in(link, board, None, None).await?.items;
            let task = card(link, board, &items, &ticket).await?;
            let p = request::Payload::AgentWork(pb::AgentWork {
                agent_id: id.clone(),
                task_id: task.id.clone(),
                role: role.wire() as i32,
                verdict: verdict.map(|v| v.word().to_string()),
                actor: actor.into(),
            });
            let agent = send_agent(link, board, "agent.work", p).await?;
            let plan = pb::Plan { agents: vec![agent.clone()], ..cards_only(&plan, &items) };
            let name = agent_name(&plan, &agent.id);
            Ok(if json {
                farcooler_client::plan_json::agent_json(&plan, &agent).to_string()
            } else {
                match verdict {
                    None => format!("{name} is {} {}.", role.doing(), task.key),
                    Some(v) => format!("{name} is done {} {}: {}.", role.doing(), task.key, v.said()),
                }
            })
        }
        AgentCmd::Stop { agent, reason } => {
            if !link.capabilities().iter().any(|c| c == capability::AGENT_STOP) {
                let code = Some(pb::ErrorCode::CapabilityUnsupported as i32);
                return Err(Box::new(crate::tasks::Refused::new(NEEDS_STOP.to_string(), code)));
            }
            let plan = get_plan(link, &ws, false).await?;
            let id = agent_or_ticket(&plan, &agent)?;
            let name = agent_name(&plan, &id);
            let p = request::Payload::AgentStop(pb::AgentStop { agent_id: id, reason, actor: actor.into() });
            let stopped = send_agent(link, board, "agent.stop", p).await?;
            Ok(if json {
                farcooler_client::plan_json::agent_json(&plan, &stopped).to_string()
            } else if stopped.terminal_id.is_some() {
                format!("Stopped {name}: its pane has stopped.")
            } else {
                format!(
                    "Asked {name} to stop. Its next tool call is refused with a message telling it to stop and hand back. \
                     A command it's already running finishes first, so it shows as stopping until it ends."
                )
            })
        }
        AgentCmd::End { agent, verdict } => {
            let plan = get_plan(link, &ws, false).await?;
            let id = find_agent(&plan, &agent)?;
            let name = agent_name(&plan, &id);
            let p = request::Payload::AgentEnd(pb::AgentEnd {
                agent_id: id,
                verdict: verdict.verdict().word().into(),
                actor: actor.into(),
            });
            let ended = send_agent(link, board, "agent.end", p).await?;
            Ok(if json {
                farcooler_client::plan_json::agent_json(&plan, &ended).to_string()
            } else {
                format!("{name} has ended: {}.", verdict.verdict().said())
            })
        }
    }
}

/// An agent by its full id, its short id, or its subagent id.
pub(super) fn find_agent(plan: &pb::Plan, name: &str) -> Result<bytes::Bytes, Failed> {
    let wanted = name.trim();
    if let Ok(id) = uuid::Uuid::parse_str(wanted) {
        return Ok(id_bytes(id));
    }
    if let Some(a) = plan.agents.iter().find(|a| !a.subagent_id.is_empty() && a.subagent_id == wanted) {
        return Ok(a.id.clone());
    }
    match crate::resolve(&plan.agents, wanted, |a| &a.id, "agent") {
        Ok(a) if !wanted.is_empty() => Ok(a.id.clone()),
        Err(Unresolved::Ambiguous(_)) => Err(format!("{wanted:?} matches more than one agent. Give more of its id.").into()),
        _ => Err(format!("No agent at work here is {wanted:?}. `farcooler agent list` shows them.").into()),
    }
}

/// An agent by `find_agent`'s names, or by a ticket key when one agent works
/// that ticket.
fn agent_or_ticket(plan: &pb::Plan, name: &str) -> Result<bytes::Bytes, Failed> {
    let key = name.trim();
    let on: Vec<&pb::Agent> =
        plan.agents.iter().filter(|a| a.tickets.iter().any(|t| key_of(plan, &t.task_id).eq_ignore_ascii_case(key))).collect();
    match on.as_slice() {
        [] => find_agent(plan, name),
        [one] => Ok(one.id.clone()),
        many => {
            let names = many.iter().map(|a| agent_name(plan, &a.id)).collect::<Vec<_>>().join(", ");
            Err(format!("More than one agent works {key}: {names}. Name one by its short id.").into())
        }
    }
}

/// A terminal by its full or short id, for `--parent`.
async fn terminal_id<L: DispatchLink>(link: &mut L, given: &str) -> Result<bytes::Bytes, Failed> {
    if let Ok(id) = uuid::Uuid::parse_str(given.trim()) {
        return Ok(id_bytes(id));
    }
    let r = link.call(crate::req("terminal.list")).await.map_err(|e| refused(e, "The terminals couldn't be read."))?;
    let result::Value::TerminalList(list) = expect_value(r.value)? else { return Err(unreadable()) };
    match crate::resolve(&list.items, given, |t| &t.id, "terminal") {
        Ok(t) => Ok(t.id.clone()),
        Err(Unresolved::Ambiguous(_)) => Err(format!("{given:?} matches more than one pane. Give more of its id.").into()),
        Err(Unresolved::Missing(_)) => Err(format!("No pane here is {given:?}. `farcooler terminal list` shows them.").into()),
    }
}

/// The keys of the tickets an agent works now, in its order.
fn keys_of(plan: &pb::Plan, a: &pb::Agent) -> String {
    a.tickets.iter().map(|t| key_of(plan, &t.task_id)).collect::<Vec<_>>().join(" ")
}

/// What the agent works on, in a person's words: the plan for the
/// orchestrator, else its tickets' keys.
fn works_on(plan: &pb::Plan, a: &pb::Agent) -> String {
    if a.role == pb::AgentRole::Orchestrator as i32 && a.tickets.is_empty() {
        return "The plan".to_string();
    }
    match keys_of(plan, a) {
        keys if keys.is_empty() => "No ticket".to_string(),
        keys => keys,
    }
}

/// "2.1M tokens · about $14.20 API-equivalent", tokens first.
fn spend_line(s: &pb::Spend) -> String {
    match s.cost_micros {
        Some(m) => format!("{} tokens · about {} API-equivalent", tokens(total(s)), dollars(m)),
        None => format!("{} tokens", tokens(total(s))),
    }
}

/// One agent's row: whether it's working, who it is, what it works on, what
/// it's doing now, and its spend.
pub(super) fn agent_row(plan: &pb::Plan, a: &pb::Agent) -> String {
    let mut who = agent_name(plan, &a.id);
    if !a.title.is_empty() {
        who.push_str(&format!(" \u{201c}{}\u{201d}", a.title));
    }
    let state = if a.stop_asked_at.is_some() {
        "stopping".to_string()
    } else if a.working {
        "working".to_string()
    } else if a.last_at > 0 {
        format!("idle · active {}", ago(plan.now_ms - a.last_at))
    } else {
        "idle".to_string()
    };
    let doing = if a.doing_now.is_empty() { String::new() } else { format!(" · {}", a.doing_now) };
    format!(
        "  {} {who}  {}  {state}{doing}  {}",
        if a.working { "\u{25cf}" } else { "\u{25cb}" },
        works_on(plan, a),
        spend_line(&a.spend.unwrap_or_default())
    )
}

/// `agent list`, in words.
fn agents_text(plan: &pb::Plan) -> String {
    let all = plan.agents_spend.unwrap_or_default();
    // With none at work the day's spend still shows, as the apps' header does.
    if plan.agents.is_empty() && total(&all) == 0 {
        return "No agents at work.".to_string();
    }
    let working = plan.agents.iter().filter(|a| a.working).count();
    let cost = all.cost_micros.map(|m| format!(" · about {} API-equivalent", dollars(m))).unwrap_or_default();
    let lead = if plan.agents.is_empty() {
        "No agents at work".to_string()
    } else {
        format!("{working} working · {}", count(plan.agents.len(), "agent"))
    };
    let mut out = vec![format!("{lead} · {} tokens in the last 24 hours{cost}", tokens(total(&all)))];
    out.extend(plan.agents.iter().map(|a| agent_row(plan, a)));
    out.join("\n")
}

/// The `agent list --json` object: the same `agents` and `agents_spend` that
/// `plan --json` carries, which is what the apps read.
fn agents_json(plan: &pb::Plan) -> Value {
    let mut whole = plan_json(plan);
    json!({ "now_ms": plan.now_ms, "agents": whole["agents"].take(), "agents_spend": whole["agents_spend"].take() })
}
