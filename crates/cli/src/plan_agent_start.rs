//! `agent dispatch` on a tracked ticket (ov-962): the engine starts the work.
//!
//! A tracked ticket belongs to the engine, which refuses a pane linked to it by hand. So a builder
//! dispatched on tracked tickets is a `workflow start` on each, under the workspace's published
//! workflow, and the run provisions its own worktree and starts its own agent. Nothing is opened
//! here. Tickets the engine doesn't track dispatch a pane as before.

use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, result};
use farcooler_protocol::workflow_commands::{Action, Mutation, NamedTree};
use farcooler_transport::ClientError;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

use super::{Failed, unreadable};
use crate::tasks::DispatchLink;

/// What the engine holds for one ticket.
#[derive(Debug, PartialEq, Eq)]
enum Hold {
    /// Not an engine item.
    None,
    /// A tracker-only ticket at rest: the one thing `Start` takes.
    AtRest { run: String, pin: String },
    /// An item with a run of its own, or one that has closed.
    Taken(&'static str),
}

/// Why a closed ticket can't be dispatched.
const CLOSED: &str = "has closed under the engine";

/// The state and published pin one `workflow get` answered.
struct Snapshot {
    revision: u64,
    epoch: u64,
    published: Option<String>,
    items: Value,
    /// Items that finished and left `items` (a closed ticket is here from the step after it closes).
    archive: Value,
    /// The published workflow's build steps that list models, each with its list, the first being the default (ov-966).
    models: Vec<(String, Vec<String>)>,
}

/// What `workflow get` said: the board's state, nothing held yet, or no answer at all (a seat the
/// engine turns away, a runner that can't be reached).
enum Seen {
    Held(Snapshot),
    Nothing,
    Unanswered,
}

async fn snapshot<L: DispatchLink>(link: &mut L, workspace: Uuid) -> Seen {
    let Ok(request) = crate::workflow::request(workspace, &Action::Get) else { return Seen::Unanswered };
    let Ok(reply) = link.call(request).await else { return Seen::Unanswered };
    match read(reply) {
        Some(seen) => seen.map_or(Seen::Nothing, Seen::Held),
        None => Seen::Unanswered,
    }
}

/// `None` when the reply isn't one; `Some(None)` when the board holds nothing.
fn read(reply: pb::Result) -> Option<Option<Snapshot>> {
    let Some(result::Value::WorkflowReply(reply)) = reply.value else { return None };
    let value: Value = serde_json::from_str(&reply.json).ok()?;
    let Some(state) = value.get("state").filter(|s| !s.is_null()) else { return Some(None) };
    Some(Some(Snapshot {
        revision: state.get("revision")?.as_u64()?,
        epoch: state.get("epoch")?.as_u64()?,
        published: value.pointer("/configuration/pin").and_then(Value::as_str).map(String::from),
        items: state.get("items").cloned().unwrap_or(Value::Null),
        archive: state.get("archive").cloned().unwrap_or(Value::Null),
        models: value.pointer("/configuration/definition/models").and_then(Value::as_object).map(|m| m.iter().map(|(step, list)| {
            (step.clone(), list.as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect())
        }).collect()).unwrap_or_default(),
    }))
}

impl Snapshot {
    fn hold(&self, key: &str) -> Hold {
        let Some(item) = self.items.as_object().and_then(|m| m.iter().find(|(k, _)| k.eq_ignore_ascii_case(key))).map(|(_, v)| v) else {
            // A ticket that closed has left `items` by the next step, but the engine still owns its name.
            let closed = self.archive.as_object().is_some_and(|m| m.keys().any(|k| k.eq_ignore_ascii_case(key)));
            return if closed { Hold::Taken(CLOSED) } else { Hold::None };
        };
        let ticket = item.pointer("/definition/source").and_then(Value::as_str) == Some("ticket/v1");
        match (ticket, item.get("phase").and_then(Value::as_str)) {
            (true, Some("WaitingApproval")) => Hold::AtRest {
                run: item.get("run").and_then(Value::as_str).unwrap_or_default().into(),
                pin: item.get("pin").and_then(Value::as_str).unwrap_or_default().into(),
            },
            (true, _) => Hold::Taken(CLOSED),
            (false, _) => Hold::Taken("already has a run under the engine"),
        }
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_millis() as u64).max(1)
}

/// What dispatch did with the tickets.
pub(super) enum Engine {
    /// The engine started them; this is what to print.
    Started(String),
    /// It holds none, so a pane opens as always. `unanswered` when the engine couldn't be asked, so a
    /// pane that fails may be failing on a ticket the engine holds.
    Pane { unanswered: bool },
}

/// How the engine's refusal reads to the person who asked. `key` is the ticket, for a command to run.
fn refusal_words(refusal: &str, key: &str) -> String {
    String::from(match refusal {
        "Stale" => "the board moved while it was starting",
        "Phase" => "it isn't waiting to be started any more",
        // The store refuses a Start this way while the card has open subagent records or blocks, which a run would
        // fence and so never settle (ov-965). The orchestrator settles those, so say how.
        "Resource" => return format!("something else holds what it needs, or a subagent still works it or it has a block. \
            End its subagents with `farcooler agent end <agent> --verdict stopped`, clear its blocks with \
            `farcooler task block {key} --on <key> --clear`, and dispatch again"),
        "Blocked" => "it's waiting on another ticket",
        "Limit" => "the board's limit on runs is reached",
        "Unauthorized" => "this seat can't start it",
        "Missing" => "the engine doesn't have it",
        "Duplicate" => "it was already started",
        "Definition" | "Invalid" => "the published workflow doesn't fit it",
        "Evidence" => "its evidence doesn't match",
        _ => "the engine refused it",
    })
}

/// A Start the runner refused because the worktree it names is taken (ov-967), said as that.
fn taken(error: &ClientError, key: &str, worktree: Option<&NamedTree>) -> Option<Failed> {
    let (ClientError::Daemon { code, .. }, Some(tree)) = (error, worktree) else { return None };
    match pb::ErrorCode::try_from(*code).ok()? {
        pb::ErrorCode::BranchExists => Some(format!("{key} wasn't started: the branch {} already exists. Name a new one with --branch.", tree.branch).into()),
        pb::ErrorCode::WorktreeExists => Some(format!("{key} wasn't started: a worktree named {} already exists. Name another with --new.", tree.name).into()),
        pb::ErrorCode::InvalidArgument => Some(format!("{key} wasn't started: the runner won't take {} as a worktree name or {} as a branch.", tree.name, tree.branch).into()),
        _ => None,
    }
}

/// What the person asked of `agent dispatch` that the engine reads.
pub(super) struct Asked<'a> {
    pub builder: bool,
    pub role: &'a str,
    /// The flags the person gave that a start can't honor.
    pub dropped: &'a [&'a str],
    /// `--harness`, when given: the build step's model (ov-966).
    pub harness: Option<&'a str>,
    /// `--new` and `--branch`: the worktree the run makes, which the app lists (ov-967).
    pub worktree: Option<&'a NamedTree>,
}

/// The model picks a Start sends for `--harness`: the one build step that lists models, if it lists this one.
fn picks(seen: &Snapshot, harness: Option<&str>) -> Result<BTreeMap<String, String>, Failed> {
    let Some(model) = harness else { return Ok(BTreeMap::new()) };
    match seen.models.as_slice() {
        [] => Err("The published workflow doesn't let the orchestrator pick a model, so --harness would be ignored. Leave it out, or publish a workflow whose build step lists the models it may run with.".into()),
        [(step, models)] if models.iter().any(|m| m == model) => Ok([(step.clone(), model.to_string())].into()),
        [(step, models)] => Err(format!("The published workflow's {step} step runs {}, not {model}. Pick one of those, or leave --harness out for {}.",
            models.join(" or "), models.first().map_or("its default", String::as_str)).into()),
        several => Err(format!("The published workflow lists models for more than one step ({}). Start each ticket with `farcooler workflow start --model <step>=<model>`.",
            several.iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>().join(" and ")).into()),
    }
}

/// Starts a builder's tickets through the engine when it holds them. A pane when it holds none. A mix,
/// a ticket the engine can't start, a reviewer, a workspace with nothing published, or a flag the engine
/// would drop is refused whole before anything starts, as is a `--harness` the workflow can't take. The engine
/// starts its own agent in its own worktree under the published workflow, with the model `--harness` picks.
pub(super) async fn through_engine<L: DispatchLink>(
    link: &mut L,
    tickets: &[pb::Task],
    asked: Asked<'_>,
    json: bool,
) -> Result<Engine, Failed> {
    let Asked { builder, role, dropped, harness, worktree } = asked;
    let pane = |unanswered| Ok(Engine::Pane { unanswered });
    if !link.capabilities().iter().any(|c| c == capability::WORKFLOW_COMMANDS) { return pane(false); }
    let Some(first) = tickets.first() else { return pane(false) };
    let Some(workspace) = Uuid::from_slice(&first.workspace_id).ok() else { return pane(false) };
    let seen = match snapshot(link, workspace).await {
        Seen::Held(seen) => seen,
        Seen::Nothing => return pane(false),
        Seen::Unanswered => return pane(true),
    };
    let held: Vec<Hold> = tickets.iter().map(|t| seen.hold(&t.key)).collect();
    if held.iter().all(|h| *h == Hold::None) { return pane(false); }
    if let Some((t, Hold::Taken(why))) = tickets.iter().zip(&held).find(|(_, h)| matches!(h, Hold::Taken(_))) {
        // A closed ticket comes back through the engine (ov-968); one with a run is sent back, not dispatched again.
        let next = if *why == CLOSED { format!("Reopen it with `farcooler workflow reopen {workspace} {}`, then dispatch again", t.key) }
            else { "Send it back to its build step with `farcooler workflow send-back`".into() };
        return Err(format!("{} {why}, so it can't be dispatched. {next}.", t.key).into());
    }
    if let Some((t, _)) = tickets.iter().zip(&held).find(|(_, h)| **h == Hold::None) {
        return Err(format!("{} isn't tracked by the engine and the others are. Dispatch it on its own.", t.key).into());
    }
    if !builder {
        return Err(format!("A {role} can't be dispatched on a tracked ticket: the engine runs its review and landing. Dispatch a builder to start it.").into());
    }
    if seen.published.is_none() {
        return Err("This board has no published workflow, so the engine has nothing to start. Publish one first.".into());
    }
    if !dropped.is_empty() {
        let (them, flags) = (if dropped.len() == 1 { "it" } else { "them" }, dropped.join(" and "));
        return Err(format!("The engine starts a tracked ticket's agent itself, in a worktree of its own, so {flags} would be ignored. Leave {them} out and dispatch again.").into());
    }
    let models = picks(&seen, harness)?;
    if worktree.is_some() && tickets.len() > 1 {
        return Err("Each run makes its own worktree, so --new and --branch name one ticket's. Dispatch them one at a time, each with its own.".into());
    }
    let mut started = Vec::new();
    for ticket in tickets {
        // Each start moves the revision, so every ticket reads the state again. A start the board moved
        // under is read and sent once more; a second miss is the person's to look at.
        let mut retried = false;
        loop {
            let now = if started.is_empty() && !retried { None } else { match snapshot(link, workspace).await { Seen::Held(s) => Some(s), _ => None } };
            let seen = now.as_ref().unwrap_or(&seen);
            let (Hold::AtRest { run, pin }, Some(start_pin)) = (seen.hold(&ticket.key), seen.published.clone()) else {
                return Err(format!("{} changed under the engine while the others started. Look at the board and dispatch again.", ticket.key).into());
            };
            let action = Action::Start {
                mutation: Mutation { id: format!("dispatch-{}-{}", ticket.key, seen.revision), revision: seen.revision, epoch: seen.epoch, sequence: now_millis() },
                item: ticket.key.clone(), run, pin, start_pin, models: models.clone(), worktree: worktree.cloned(),
            };
            let request = crate::workflow::request(workspace, &action).map_err(|_| unreadable())?;
            let reply = link.call(request).await.map_err(|e| taken(&e, &ticket.key, worktree)
                .unwrap_or_else(|| Failed::from(format!("{} couldn't be started: the engine didn't answer.", ticket.key))))?;
            let Some(result::Value::WorkflowReply(reply)) = reply.value else { return Err(unreadable()) };
            let value: Value = serde_json::from_str(&reply.json).map_err(|_| unreadable())?;
            let Some(refusal) = value.pointer("/receipt/result/Err") else { break };
            let said = refusal.as_str().map_or_else(|| refusal.to_string(), String::from);
            if said == "Stale" && !retried { retried = true; continue; }
            let done = if started.is_empty() { String::new() } else { format!(" {} already started.", started.join(", ")) };
            return Err(format!("The engine didn't start {}: {}.{done}", ticket.key, refusal_words(&said, &ticket.key)).into());
        }
        started.push(ticket.key.clone());
    }
    let list = started.join(", then ");
    if let (Some(tree), false) = (worktree, json) {
        return Ok(Engine::Started(format!(
            "The engine is starting {list} in the worktree {}, on the branch {}. The run makes the worktree, which the app lists, from main as it is now, and starts its own agent under the board's published workflow, so no pane was opened here.\nWatch it with `farcooler workflow get {workspace}`.",
            tree.name, tree.branch)));
    }
    if json {
        return Ok(Engine::Started(json!({ "through": "engine", "role": "builder", "tickets": started }).to_string()));
    }
    Ok(Engine::Started(format!(
        "The engine is starting {list}. Each run makes its own worktree and starts its own agent under the board's published workflow, so no pane was opened here.\nWatch it with `farcooler workflow get {workspace}`."
    )))
}

/// `task dispatch` opens a pane, which a tracked ticket refuses after any worktree is made. So it asks
/// first, and sends the person to `agent dispatch`, which starts the ticket through the engine (ov-962).
/// When the engine can't be asked (a member's orchestrator), a new worktree is not made on a guess: it
/// would be left behind if the card turns out to be tracked. An existing worktree makes nothing, so it goes on.
pub(crate) async fn refuse_tracked<L: DispatchLink>(link: &mut L, ticket: &pb::Task, makes_worktree: bool) -> Result<(), Failed> {
    if !link.capabilities().iter().any(|c| c == capability::WORKFLOW_COMMANDS) { return Ok(()); }
    let Some(workspace) = Uuid::from_slice(&ticket.workspace_id).ok() else { return Ok(()) };
    match snapshot(link, workspace).await {
        Seen::Held(seen) if seen.hold(&ticket.key) != Hold::None => Err(format!(
            "{} is tracked by the engine, which refuses a pane. Start it with `farcooler agent dispatch --role builder --ticket {}`.",
            ticket.key, ticket.key).into()),
        Seen::Unanswered if makes_worktree => Err(format!(
            "Couldn't ask the engine whether {} is tracked, so no worktree was made. Dispatch into one that exists with --worktree, or ask the workspace owner's orchestrator.",
            ticket.key).into()),
        _ => Ok(()),
    }
}
