//! `agent.stop` (ov-872): the orchestrator stops an agent because a person
//! asked it to. No screen has a Stop button; stopping is the orchestrator's
//! tool, `farcooler agent stop`.
//!
//! - **A pane**, of any harness, is stopped as `terminal.stop` stops it, under
//!   `terminal.stop`'s own gate: only the lead stops the orchestrator, and a
//!   rep only its own pane. Its agent ends as stopped.
//! - **A Claude subagent** is asked: the ask is recorded on the agent, and its
//!   next tool call is refused with a message telling it to stop and hand back
//!   (`hook_ingress::stopping`). A tool call already running finishes first,
//!   so it reads "Stopping" until its `SubagentStop` ends it.
//! - **Any other harness's subagent** is refused `stop_unsupported`: the
//!   runner installs waiting hooks only for Claude (`hook_install`), so it
//!   can't reach one. Its orchestrator stops it with its own harness's tool.
//!
//! Who may: anyone who can dispatch an agent (`Ruling::Members`, at
//! `control`), so never a Viewer.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{Request, request, result};
use farcooler_store::agents::AgentPlace;
use farcooler_store::ticket_work::Verdict;
use farcooler_transport::Peer;

use crate::service::Service;
use crate::task_ops::{actor_from_wire, required_id};
use crate::watch::Watcher;
use crate::wire::id_bytes;

/// The longest reason, in characters: one line the agent reads.
const REASON_MAX: usize = 200;

/// Stop the agent `req` names, answering it as it now is.
pub(crate) async fn dispatch(
    svc: &Service,
    watcher: &Watcher,
    peer: &Peer,
    caller_pid: Option<i32>,
    req: Request,
) -> Result<result::Value> {
    let Some(request::Payload::AgentStop(p)) = req.payload else {
        return Err(DomainError::InvalidArgument { what: "payload" });
    };
    let id = required_id(&p.agent_id)?;
    let actor = actor_from_wire(&p.actor)?;
    let reason = p.reason.as_deref().map(str::trim).filter(|r| !r.is_empty()).map(str::to_string);
    if reason.as_deref().is_some_and(|r| r.chars().count() > REASON_MAX || r.chars().any(char::is_control)) {
        return Err(DomainError::InvalidArgument { what: "reason" });
    }
    let agent = svc.store.agent(id)?;
    if agent.ended_at.is_some() {
        return Err(DomainError::InvalidArgument { what: "agent_ended" });
    }
    match agent.place {
        AgentPlace::Pane { terminal } => {
            let as_stop = Request {
                method: "terminal.stop".into(),
                target_resource_id: Some(id_bytes(terminal)),
                ..Default::default()
            };
            crate::people::gate(svc, peer, caller_pid, &as_stop)?;
            svc.store.ask_agent_stop(id, actor)?;
            let worktree = svc.store.get_terminal(terminal)?.worktree_id;
            svc.stop_terminal(terminal).await?;
            // Stopping kills the pane, so its window's grid just changed.
            if let Ok(groups) = svc.layout(worktree).await {
                watcher.publish_layout(worktree, &groups);
            }
            svc.store.end_agent(id, Verdict::Stopped)?;
        }
        AgentPlace::Subagent { .. } if agent.harness == "claude" => {
            svc.store.ask_agent_stop(id, actor)?;
            svc.hooks().stop_reason(id, reason);
        }
        AgentPlace::Subagent { .. } => return Err(DomainError::Conflict { what: "stop_unsupported" }),
    }
    watcher.announce_plan_changed(agent.workspace_id, actor);
    Ok(result::Value::Agent(crate::rpc_plan::pb_agent_row(&svc.store.agent(id)?)))
}
