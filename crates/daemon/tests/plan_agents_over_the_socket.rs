//! The agents on the plan (ov-503) as a client meets them: turns the runner
//! recorded, read back from `plan.get` through the real dispatch table, with
//! the capability the runner advertises for them.

#[path = "support/in_process.rs"]
mod in_process;

use farcooler_core::usage::TokenCounts;
use farcooler_protocol::v1::{self as pb, Scope, request as payload, result};
use farcooler_store::models::TerminalRole;
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

/// The orchestrator, and a subagent nothing names, come back over the wire
/// with their tokens, roles and terminal; the capability is advertised.
#[tokio::test]
async fn the_agents_are_read_back_over_the_wire() {
    let h = start(Scope::HostAdmin).await;
    let repo = a_repository(&h);
    let orchestrator = a_pane(&h, repo.worktree, None);
    h.service.store.set_terminal_role(orchestrator, TerminalRole::Orchestrator).unwrap();
    h.service.store.record_turn(&turn("claude-log:t1", Some(orchestrator), &repo, TurnKind::Turn, 700)).unwrap();
    h.service.store.record_turn(&turn("claude-log:agent:ghost", Some(orchestrator), &repo, TurnKind::Subagent, 300)).unwrap();

    let mut link = connect(&h).await;
    let mut r = request("plan.get");
    // Naming the capability makes the runner refuse if it doesn't advertise it.
    r.required_capabilities =
        vec![farcooler_protocol::capability::BOARD_PLAN.into(), farcooler_protocol::capability::BOARD_AGENTS_LIST.into()];
    r.payload = Some(payload::Payload::PlanGet(pb::PlanGetRequest {
        workspace_id: bytes::Bytes::copy_from_slice(repo.workspace.as_bytes()),
        include_closed: false,
    }));
    let result::Value::Plan(plan) = link.call(r).await.unwrap().value.unwrap() else { panic!("wrong result") };

    let by_id = |id: &str| plan.agents.iter().find(|a| a.id == id).unwrap_or_else(|| panic!("no {id}: {:?}", plan.agents));
    let o = by_id(&format!("pane:{orchestrator}"));
    assert_eq!(o.role, pb::PlanAgentRole::Orchestrator as i32);
    assert_eq!(o.spend.as_ref().unwrap().input_tokens, 700, "its own turns, not the subagent filed to its pane");
    assert_eq!(o.terminal_id.as_deref(), Some(orchestrator.as_bytes().as_slice()));
    let g = by_id("agent:ghost");
    assert_eq!(g.role, pb::PlanAgentRole::Subagent as i32);
    assert_eq!((g.lane_id.clone(), g.train_id.clone(), g.task_id.clone()), (None, None, None));
    assert_eq!(g.spend.as_ref().unwrap().input_tokens, 300);
    assert_eq!(g.spend.as_ref().unwrap().cost_micros, Some(2_000));

    // The bytes survive a trip, and an app from before reads the plan it knew.
    let back = pb::Plan::decode(plan.encode_to_vec().as_slice()).unwrap();
    assert_eq!(back.agents, plan.agents);
}
