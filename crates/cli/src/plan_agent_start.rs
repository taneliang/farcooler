//! `agent dispatch` on a tracked ticket (ov-962): the engine starts the work.
//!
//! A tracked ticket belongs to the engine, which refuses a pane linked to it by hand. So a builder
//! dispatched on tracked tickets is a `workflow start` on each, under the workspace's published
//! workflow, and the run provisions its own worktree and starts its own agent. Nothing is opened
//! here. Tickets the engine doesn't track dispatch a pane as before.

use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, result};
use farcooler_protocol::workflow_commands::{Action, Mutation};
use serde_json::{Value, json};
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

/// The state and published pin one `workflow get` answered.
struct Snapshot {
    revision: u64,
    epoch: u64,
    published: Option<String>,
    items: Value,
    /// Items that finished and left `items` (a closed ticket is here from the step after it closes).
    archive: Value,
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
    }))
}

impl Snapshot {
    fn hold(&self, key: &str) -> Hold {
        let Some(item) = self.items.as_object().and_then(|m| m.iter().find(|(k, _)| k.eq_ignore_ascii_case(key))).map(|(_, v)| v) else {
            // A ticket that closed has left `items` by the next step, but the engine still owns its name.
            let closed = self.archive.as_object().is_some_and(|m| m.keys().any(|k| k.eq_ignore_ascii_case(key)));
            return if closed { Hold::Taken("has closed under the engine") } else { Hold::None };
        };
        let ticket = item.pointer("/definition/source").and_then(Value::as_str) == Some("ticket/v1");
        match (ticket, item.get("phase").and_then(Value::as_str)) {
            (true, Some("WaitingApproval")) => Hold::AtRest {
                run: item.get("run").and_then(Value::as_str).unwrap_or_default().into(),
                pin: item.get("pin").and_then(Value::as_str).unwrap_or_default().into(),
            },
            (true, _) => Hold::Taken("has closed under the engine"),
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

/// How the engine's refusal reads to the person who asked.
fn refusal_words(refusal: &str) -> &'static str {
    match refusal {
        "Stale" => "the board moved while it was starting",
        "Phase" => "it isn't waiting to be started any more",
        "Resource" => "something else holds what it needs",
        "Blocked" => "it's waiting on another ticket",
        "Limit" => "the board's limit on runs is reached",
        "Unauthorized" => "this seat can't start it",
        "Missing" => "the engine doesn't have it",
        "Duplicate" => "it was already started",
        "Definition" | "Invalid" => "the published workflow doesn't fit it",
        "Evidence" => "its evidence doesn't match",
        _ => "the engine refused it",
    }
}

/// Starts a builder's tickets through the engine when it holds them. A pane when it holds none. A mix,
/// a ticket the engine can't start, a reviewer, a workspace with nothing published, or a flag the engine
/// would drop is refused whole before anything starts. `dropped` names the flags the person gave that a
/// start can't honor: it starts its own agent in its own worktree under the published workflow.
pub(super) async fn through_engine<L: DispatchLink>(
    link: &mut L,
    tickets: &[pb::Task],
    builder: bool,
    role: &str,
    json: bool,
    dropped: &[&str],
) -> Result<Engine, Failed> {
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
        return Err(format!("{} {why}, so it can't be dispatched. Move it with `farcooler workflow`.", t.key).into());
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
                item: ticket.key.clone(), run, pin, start_pin,
            };
            let request = crate::workflow::request(workspace, &action).map_err(|_| unreadable())?;
            let reply = link.call(request).await.map_err(|_| Failed::from(format!("{} couldn't be started: the engine didn't answer.", ticket.key)))?;
            let Some(result::Value::WorkflowReply(reply)) = reply.value else { return Err(unreadable()) };
            let value: Value = serde_json::from_str(&reply.json).map_err(|_| unreadable())?;
            let Some(refusal) = value.pointer("/receipt/result/Err") else { break };
            let said = refusal.as_str().map_or_else(|| refusal.to_string(), String::from);
            if said == "Stale" && !retried { retried = true; continue; }
            let done = if started.is_empty() { String::new() } else { format!(" {} already started.", started.join(", ")) };
            return Err(format!("The engine didn't start {}: {}.{done}", ticket.key, refusal_words(&said)).into());
        }
        started.push(ticket.key.clone());
    }
    let list = started.join(", then ");
    if json {
        return Ok(Engine::Started(json!({ "through": "engine", "role": "builder", "tickets": started }).to_string()));
    }
    Ok(Engine::Started(format!(
        "The engine is starting {list}. Each run makes its own worktree and starts its own agent under the board's published workflow, so no pane was opened here.\nWatch it with `farcooler workflow get {workspace}`."
    )))
}
