//! The agents on the plan (ov-503, ov-872) as a client meets them: agents the
//! runner recorded, with their turns, tickets and what each is doing now, read
//! back from `plan.get` through the real dispatch table, with the capability
//! the runner advertises for them.

#[path = "support/in_process.rs"]
mod in_process;

use farcooler_core::usage::TokenCounts;
use farcooler_protocol::v1::{self as pb, Scope, request as payload, result};
use farcooler_store::agents::{AgentPlace, AgentRole, NewAgent};
use farcooler_store::models::{Actor, TerminalRole};
use farcooler_store::ticket_work::{LinkedBy, Verdict, WorkRole};
use farcooler_store::usage::{NewTurn, Surface, TurnKind, TurnModel};
use farcooler_transport::request;
use in_process::*;
use prost::Message;
use uuid::Uuid;

fn turn(key: &str, terminal: Option<Uuid>, repo: &Repo, kind: TurnKind, input: u64) -> NewTurn {
    NewTurn {
        key: key.into(),
        terminal_id: terminal,
        worktree_id: Some(repo.worktree),
        repository_id: Some(repo.id),
        workspace_id: None,
        task_id: None,
        harness: "claude".into(),
        surface: Surface::Terminal,
        started_at: None,
        ended_at: now_millis(),
        active_ms: None,
        usage: "reported",
        models: vec![TurnModel::priced(
            Some("claude-opus-5".into()),
            TokenCounts { input, output: 0, cache_read: 0, cache_write: 0, cache_write_1h: 0, fast: false },
            Some(2_000),
        )],
        kind,
    }
}

/// The orchestrator's pane and a subagent it runs come back over the wire with
/// their tokens, roles, terminal, tickets and what each is doing now; the
/// capability is advertised.
#[tokio::test]
async fn the_agents_are_read_back_over_the_wire() {
    let h = start(Scope::HostAdmin).await;
    let repo = a_repository(&h);
    let store = &h.service.store;
    let orchestrator = a_pane(&h, repo.worktree, None);
    store.set_terminal_role(orchestrator, TerminalRole::Orchestrator).unwrap();
    let task = store.create_task(repo.workspace, "Fix the gate", Actor::User).unwrap();
    let new = |role, place| NewAgent {
        role,
        place,
        harness: "claude".into(),
        session_id: None,
        session_cwd: None,
        model: None,
        title: None,
    };
    let o = store.record_agent(repo.workspace, &new(AgentRole::Orchestrator, AgentPlace::Pane { terminal: orchestrator })).unwrap();
    let parent = Some(orchestrator);
    let g = store.record_agent(repo.workspace, &new(AgentRole::Builder, AgentPlace::Subagent { id: "ghost".into(), parent })).unwrap();
    store.ensure_on_line(task.id, Actor::Manager).unwrap();
    store.start_work(g.id, task.id, WorkRole::Build, LinkedBy::Orchestrator, Actor::Manager).unwrap();
    store.record_turn(&turn("claude-log:t1", Some(orchestrator), &repo, TurnKind::Turn, 700)).unwrap();
    store.record_turn(&turn("claude-log:agent:ghost", Some(orchestrator), &repo, TurnKind::Subagent, 300)).unwrap();
    h.service.worker_seen.note("ghost", Some(now_millis()), Some("Running cargo test".into()), None);

    let mut link = connect(&h).await;
    let mut r = request("plan.get");
    // Naming the capability makes the runner refuse if it doesn't advertise it.
    r.required_capabilities =
        vec![farcooler_protocol::capability::BOARD_PLAN.into(), farcooler_protocol::capability::BOARD_LINE.into()];
    r.payload = Some(payload::Payload::PlanGet(pb::PlanGetRequest {
        workspace_id: bytes::Bytes::copy_from_slice(repo.workspace.as_bytes()),
        include_closed: false,
    }));
    let result::Value::Plan(plan) = link.call(r).await.unwrap().value.unwrap() else { panic!("wrong result") };

    let by_id = |id: Uuid| {
        plan.agents.iter().find(|a| a.id == id.as_bytes().as_slice()).unwrap_or_else(|| panic!("no {id}: {:?}", plan.agents))
    };
    let wire_o = by_id(o.id);
    assert_eq!(wire_o.role, pb::AgentRole::Orchestrator as i32);
    assert_eq!(wire_o.spend.as_ref().unwrap().input_tokens, 700, "its own turns, not the subagent filed to its pane");
    assert_eq!(wire_o.terminal_id.as_deref(), Some(orchestrator.as_bytes().as_slice()));
    assert!(wire_o.tickets.is_empty());
    let wire_g = by_id(g.id);
    assert_eq!(wire_g.role, pb::AgentRole::Builder as i32);
    assert_eq!((wire_g.terminal_id.clone(), wire_g.subagent_id.as_str()), (None, "ghost"));
    assert_eq!(wire_g.parent_terminal_id.as_deref(), Some(orchestrator.as_bytes().as_slice()));
    assert_eq!(wire_g.tickets.len(), 1, "{:?}", wire_g.tickets);
    assert_eq!(wire_g.tickets[0].task_id, task.id.as_bytes().as_slice());
    assert_eq!(wire_g.tickets[0].role, pb::WorkRole::Build as i32);
    assert_eq!(wire_g.doing_now, "Running cargo test", "a subagent's last tool call, as the runner saw it");
    assert_eq!(wire_g.spend.as_ref().unwrap().input_tokens, 300);
    assert_eq!(wire_g.spend.as_ref().unwrap().cost_micros, Some(2_000));
    assert_eq!(plan.agents_spend.as_ref().unwrap().input_tokens, 1_000, "the day's spend counts every turn");

    // Its ticket is on the line, building, with the subagent's work on it.
    let line = plan.lines.iter().find(|l| l.task_id == task.id.as_bytes().as_slice()).expect("the ticket on the line");
    assert_eq!(line.work.len(), 1);
    assert_eq!(line.work[0].agent_id, g.id.as_bytes().as_slice());

    // An agent that ended leaves the read.
    store.end_agent(g.id, Verdict::Finished).unwrap();
    let mut again = request("plan.get");
    again.payload = Some(payload::Payload::PlanGet(pb::PlanGetRequest {
        workspace_id: bytes::Bytes::copy_from_slice(repo.workspace.as_bytes()),
        include_closed: false,
    }));
    let result::Value::Plan(after) = link.call(again).await.unwrap().value.unwrap() else { panic!("wrong result") };
    assert!(after.agents.iter().all(|a| a.id != g.id.as_bytes().as_slice()), "{:?}", after.agents);

    // The bytes survive a trip.
    let back = pb::Plan::decode(plan.encode_to_vec().as_slice()).unwrap();
    assert_eq!(back.agents, plan.agents);
}
