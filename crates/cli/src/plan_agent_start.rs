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
    /// A ticket that goes where another ticket's run goes (ov-970): the lead's key.
    Follows(String),
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
        // It goes where another ticket's run goes (ov-970), so it isn't free to start.
        if let Some(lead) = item.get("follows").and_then(Value::as_str) { return Hold::Follows(lead.into()); }
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

/// "a, b and c".
fn listed(items: &[impl AsRef<str>]) -> String {
    let items: Vec<&str> = items.iter().map(AsRef::as_ref).collect();
    match items.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => items.join(""),
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
fn refusal_words(refusal: &str, keys: &[&str]) -> String {
    // A Start on several tickets is refused for whichever of them caused it, and the engine doesn't say which.
    let several = keys.len() > 1;
    String::from(match refusal {
        "Stale" => "the board moved while it was starting",
        "Phase" if several => "one of them isn't waiting to be started any more",
        "Phase" => "it isn't waiting to be started any more",
        // The store refuses a Start this way while the card has open subagent records or blocks, which a run would
        // fence and so never settle (ov-965). The orchestrator settles those, so say how.
        "Resource" if several => return format!("something else holds what they need, or a subagent still works one of them or one \
            has a block. End their subagents with `farcooler agent end <agent> --verdict stopped`, clear their blocks with {}, \
            and dispatch again", listed(&keys.iter().map(|k| format!("`farcooler task block {k} --on <key> --clear`")).collect::<Vec<_>>())),
        "Resource" => return format!("something else holds what it needs, or a subagent still works it or it has a block. \
            End its subagents with `farcooler agent end <agent> --verdict stopped`, clear its blocks with \
            `farcooler task block {} --on <key> --clear`, and dispatch again", keys[0]),
        "Blocked" if several => "one of them is waiting on another ticket",
        "Blocked" => "it's waiting on another ticket",
        "Limit" => "the board's limit on runs is reached",
        "Unauthorized" => "this seat can't start it",
        "Missing" if several => "the engine doesn't have one of them",
        "Missing" => "the engine doesn't have it",
        "Duplicate" => "it was already started",
        "Definition" | "Invalid" => "the published workflow doesn't fit it",
        "Evidence" => "its evidence doesn't match",
        _ => "the engine refused it",
    })
}

/// A Start the runner refused because the worktree it names is taken (ov-967), said as that.
fn taken(error: &ClientError, key: &str, worktree: Option<&NamedTree>) -> Option<Failed> {
    let (ClientError::Daemon { code, what, .. }, Some(tree)) = (error, worktree) else { return None };
    // A branch the runner won't adopt (ov-968) says why and what to do instead.
    if let Some(said) = what.starts_with("adopt_").then(|| farcooler_core::error::sentence(what)).flatten() {
        return Some(format!("{key} wasn't started on {}. {said}", tree.branch).into());
    }
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
    if let Some((t, Hold::Follows(lead))) = tickets.iter().zip(&held).find(|(_, h)| matches!(h, Hold::Follows(_))) {
        return Err(format!("{t} follows {lead}'s run, so it can't be dispatched. Its card goes where that run goes. If it needs more \
            work, send {lead} back with `farcooler workflow send-back {workspace} {lead} --run <run> --pin <pin> --reason <why>`. To work on \
            {t} alone, cancel {lead}'s run with `farcooler workflow cancel {workspace} {lead} --run <run> --pin <pin>`, which hands {t} back.",
            t = t.key).into());
    }
    if let Some((t, Hold::Taken(why))) = tickets.iter().zip(&held).find(|(_, h)| matches!(h, Hold::Taken(_))) {
        // A closed ticket comes back through the engine (ov-968); one with a run is sent back, not dispatched again.
        let next = match *why {
            CLOSED => format!("Reopen it with `farcooler workflow reopen {workspace} {}`, then dispatch again", t.key),
            _ => "Send it back to its build step with `farcooler workflow send-back`".into(),
        };
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
    // One builder on several tickets is one run (ov-970): the first leads, the rest follow it, so they share one
    // agent, one worktree, one branch and one pull request. A start the board moved under is read and sent once more;
    // a second miss is the person's to look at.
    let (lead, rest) = tickets.split_first().expect("a ticket, checked above");
    let with: Vec<String> = rest.iter().map(|t| t.key.clone()).collect();
    let mut retried = false;
    loop {
        let now = if !retried { None } else { match snapshot(link, workspace).await { Seen::Held(s) => Some(s), _ => None } };
        let seen = now.as_ref().unwrap_or(&seen);
        let (Hold::AtRest { run, pin }, Some(start_pin)) = (seen.hold(&lead.key), seen.published.clone()) else {
            return Err(format!("{} changed under the engine while it started. Look at the board and dispatch again.", lead.key).into());
        };
        let action = Action::Start {
            mutation: Mutation { id: format!("dispatch-{}-{}", lead.key, seen.revision), revision: seen.revision, epoch: seen.epoch, sequence: now_millis() },
            item: lead.key.clone(), run, pin, start_pin, models: models.clone(), worktree: worktree.cloned(), with: with.clone(),
        };
        let request = crate::workflow::request(workspace, &action).map_err(|_| unreadable())?;
        let reply = link.call(request).await.map_err(|e| taken(&e, &lead.key, worktree)
            .unwrap_or_else(|| Failed::from(format!("{} couldn't be started: the engine didn't answer.", lead.key))))?;
        let Some(result::Value::WorkflowReply(reply)) = reply.value else { return Err(unreadable()) };
        let value: Value = serde_json::from_str(&reply.json).map_err(|_| unreadable())?;
        let Some(refusal) = value.pointer("/receipt/result/Err") else { break };
        let said = refusal.as_str().map_or_else(|| refusal.to_string(), String::from);
        if said == "Stale" && !retried { retried = true; continue; }
        let keys: Vec<&str> = tickets.iter().map(|t| t.key.as_str()).collect();
        return Err(format!("The engine didn't start {}: {}.", listed(&keys), refusal_words(&said, &keys)).into());
    }
    let started: Vec<String> = tickets.iter().map(|t| t.key.clone()).collect();
    let list = if started.len() > 1 { format!("{} together, as one run", listed(&started)) } else { listed(&started) };
    if let (Some(tree), false) = (worktree, json) {
        return Ok(Engine::Started(format!(
            "The engine is starting {list} in the worktree {}, on the branch {}. The run makes the worktree, which the app lists, from main as it is now, and starts its own agent under the board's published workflow, so no pane was opened here.\nWatch it with `farcooler workflow get {workspace}`.",
            tree.name, tree.branch)));
    }
    if json {
        return Ok(Engine::Started(json!({ "through": "engine", "role": "builder", "tickets": started }).to_string()));
    }
    Ok(Engine::Started(format!(
        "The engine is starting {list}. The run makes its own worktree and starts its own agent under the board's published workflow, so no pane was opened here.\nWatch it with `farcooler workflow get {workspace}`."
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
