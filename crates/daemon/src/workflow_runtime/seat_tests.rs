//! ov-1267, security review finding 1: the orchestrator publishes its workspace's workflow, so who holds the seat
//! decides who publishes. An agent never takes the seat or steps the orchestrator down, in an owner-led workspace as
//! much as a member-led one, and moving a worktree to another workspace carries no seat with it.
use super::*;
use super::admission_tests::{factory, mutation, ok, refused, sent};
use crate::{rpc::RpcFactory, service::Service};
use std::sync::Arc;
use farcooler_protocol::{v1::{self as pb, request, response, Scope}, workflow_commands::Action};
use farcooler_store::models::{TerminalRole, TerminalUpdate};
use farcooler_transport::{Handler, Peer};

/// A pane of the owner's in `worktree`, in `workspace` as `role`: on the runner's own login, in a workspace nobody
/// leads. An agent (a builder's or a reviewer's pane) runs Claude.
async fn owners_pane(svc: &Arc<Service>, worktree: Uuid, workspace: Uuid, role: TerminalRole) -> (Uuid, i32) {
    let t = svc.create_terminal(worktree, "the owner's pane", "shell").await.unwrap();
    svc.store.update_terminal(t.id, t.resource_version, TerminalUpdate {
        title: t.title, command_preset: "claude".into(), intent: t.intent,
        runtime_confirmed: t.runtime_confirmed, exit_code: t.exit_code, exit_signal: t.exit_signal,
        lease_generation: t.lease_generation, epoch: t.epoch, columns: t.columns, rows: t.rows,
    }).unwrap();
    svc.store.set_terminal_workspace(t.id, workspace).unwrap();
    svc.store.set_terminal_role(t.id, role).unwrap();
    svc.inventory.refresh().await;
    let pid = svc.inventory_snapshot().panes.iter().find(|p| p.terminal_id == t.id).unwrap().stamp.pid as i32;
    assert_eq!(crate::caller_seat::caller_seat(svc, Some(pid)).await, crate::caller_seat::Seat::Agent(Some(t.id)));
    (t.id, pid)
}

fn set_role(terminal: Uuid, role: pb::TerminalRole) -> pb::Request {
    let mut req = farcooler_transport::request("terminal.set_role");
    req.target_resource_id = Some(bytes::Bytes::copy_from_slice(terminal.as_bytes()));
    req.payload = Some(request::Payload::TerminalSetRole(pb::TerminalSetRole { role: role as i32 }));
    req
}

fn answered(reply: &pb::Response) -> bool { matches!(reply.outcome, Some(response::Outcome::Result(_))) }

fn publish(id: &str, sequence: u64) -> Action {
    Action::Publish { reason: Some("Track the board".into()), notifications: None, adapter: None,
        mutation: mutation(id, 0, sequence), expected_pin: None, definition: Definition::tracker("owner") }
}

/// A builder or a reviewer in a workspace the owner leads (nobody leads it) neither seats itself as orchestrator nor
/// steps the live one down, even once the seat is empty; and so never publishes. The owner, in person, does both.
#[tokio::test]
async fn ov1267_an_agent_never_moves_the_orchestrator_seat_in_an_owner_led_workspace() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let ws = svc.store.ensure_main_workspace(repo).unwrap().id;
    assert_eq!(svc.store.workspace_lead(ws).unwrap(), None, "the owner leads it");
    let main = svc.store.list_worktrees_for_repository(repo).unwrap().remove(0).id;
    let (orchestrator, _) = owners_pane(&svc, main, ws, TerminalRole::Orchestrator).await;
    let (builder, builders_pid) = owners_pane(&svc, main, ws, TerminalRole::Agent).await;
    let (reviewer, reviewers_pid) = owners_pane(&svc, main, ws, TerminalRole::Agent).await;
    let mut runtime = Runtime::open_service(&svc.root_dir().join("seat-test"), svc.store.clone()).unwrap();
    let peer = Peer { client_id: None, scope: Scope::Control };
    let owner = factory(&svc, &runtime, peer.clone(), None);
    let agents: [(&str, Uuid, RpcFactory); 2] = [
        ("a builder", builder, factory(&svc, &runtime, peer.clone(), Some(builders_pid))),
        ("a reviewer", reviewer, factory(&svc, &runtime, peer.clone(), Some(reviewers_pid))),
    ];
    let role = |id| svc.store.get_terminal(id).unwrap().role;

    for (who, pane, rpc) in &agents {
        assert_eq!(refused(&rpc.handle(set_role(orchestrator, pb::TerminalRole::Agent)).await), "scope-denied", "{who} demoted the orchestrator");
        assert_eq!(refused(&rpc.handle(set_role(orchestrator, pb::TerminalRole::Shell)).await), "scope-denied", "{who} demoted the orchestrator");
        assert_eq!(role(orchestrator), TerminalRole::Orchestrator, "{who}");
        assert_eq!(refused(&rpc.handle(set_role(*pane, pb::TerminalRole::Orchestrator)).await), "scope-denied", "{who} seated itself");
        assert_eq!(role(*pane), TerminalRole::Agent, "{who}");
    }

    // The owner steps the orchestrator down; the empty seat is still not an agent's to take.
    assert!(answered(&owner.handle(set_role(orchestrator, pb::TerminalRole::Shell)).await), "the owner steps it down");
    for (n, (who, pane, rpc)) in agents.iter().enumerate() {
        assert_eq!(refused(&rpc.handle(set_role(*pane, pb::TerminalRole::Orchestrator)).await), "scope-denied", "{who} took the empty seat");
        assert_eq!(role(*pane), TerminalRole::Agent, "{who}");
        let before = super::admission_tests::snapshot(&runtime, ws);
        let (reply, _) = sent(&mut runtime, rpc, ws, &publish(&format!("stray-{n}"), n as u64 + 1)).await;
        assert_eq!(refused(&reply), "scope-denied", "{who} published");
        assert_eq!(super::admission_tests::snapshot(&runtime, ws), before, "{who} wrote");
    }

    // The owner seats an orchestrator again, which then publishes: the refusals above are the seat's, not the setup's.
    assert!(answered(&owner.handle(set_role(orchestrator, pb::TerminalRole::Orchestrator)).await), "the owner seats it");
    let (_, pid) = svc.inventory_snapshot().panes.iter().find(|p| p.terminal_id == orchestrator)
        .map(|p| (p.terminal_id, p.stamp.pid as i32)).unwrap();
    let seated = factory(&svc, &runtime, peer, Some(pid));
    ok(&sent(&mut runtime, &seated, ws, &publish("seated", 9)).await.0);
}

/// `worktree.assign` moves a builder's worktree, and its agents with it, into another workspace the owner leads. The
/// builder arrives as an agent and stays one: it can't take that workspace's seat, and can't publish there.
#[tokio::test]
async fn ov1267_a_builder_that_moves_its_worktree_takes_no_seat() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let ws = svc.store.ensure_main_workspace(repo).unwrap().id;
    let other = svc.store.create_workspace(repo, "Other", "oth").unwrap().id;
    let feature = svc.create_worktree(repo, "feature", "feature", "HEAD").await.unwrap().id;
    svc.store.assign_worktree(feature, ws).unwrap();
    let (builder, pid) = owners_pane(&svc, feature, ws, TerminalRole::Agent).await;
    let mut runtime = Runtime::open_service(&svc.root_dir().join("seat-test"), svc.store.clone()).unwrap();
    let rpc = factory(&svc, &runtime, Peer { client_id: None, scope: Scope::Control }, Some(pid));

    let mut assign = farcooler_transport::request("worktree.assign");
    assign.target_resource_id = Some(bytes::Bytes::copy_from_slice(feature.as_bytes()));
    assign.payload = Some(request::Payload::WorktreeAssign(pb::WorktreeAssign {
        workspace_id: bytes::Bytes::copy_from_slice(other.as_bytes()) }));
    let reply = rpc.handle(assign).await;
    assert!(answered(&reply), "the builder moves its worktree: {reply:?}");
    let moved = svc.store.get_terminal(builder).unwrap();
    assert_eq!((moved.workspace_id, moved.role), (Some(other), TerminalRole::Agent), "it went along as an agent");

    assert_eq!(refused(&rpc.handle(set_role(builder, pb::TerminalRole::Orchestrator)).await), "scope-denied", "it took the seat");
    assert_eq!(svc.store.get_terminal(builder).unwrap().role, TerminalRole::Agent);
    let before = super::admission_tests::snapshot(&runtime, other);
    let (reply, _) = sent(&mut runtime, &rpc, other, &publish("moved", 1)).await;
    assert_eq!(refused(&reply), "scope-denied", "it published in the workspace it moved to");
    assert_eq!(super::admission_tests::snapshot(&runtime, other), before);
}
