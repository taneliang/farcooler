//! Hold the actual RPC inbox before consumption; no timing sleeps or providers.
use super::*;
use std::sync::Arc;
use crate::{rpc::RpcFactory, service::Service, watch::Watcher};
use farcooler_protocol::{v1::{self as pb, request, response, Scope}, workflow_commands::{Action, Mutation}};
use farcooler_transport::{Handler, Peer};

fn request(workspace: Uuid, action: &Action) -> pb::Request {
    let mut req = farcooler_transport::request(action.method());
    req.target_resource_id = Some(bytes::Bytes::copy_from_slice(workspace.as_bytes()));
    req.payload = Some(request::Payload::WorkflowCommand(pb::WorkflowCommand {
        schema: 1, json: serde_json::to_string(action).unwrap(),
    }));
    req
}
fn mutation(id: &str, revision: u64, sequence: u64) -> Mutation {
    Mutation { id: id.into(), revision, epoch: 1, sequence }
}
fn factory(svc: &Arc<Service>, runtime: &Runtime, peer: Peer, pid: Option<i32>) -> RpcFactory {
    RpcFactory::new(svc.clone(), Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer)
        .with_caller_pid(pid).with_workflow_fixture(runtime.command_handle(&runtime.controller()).unwrap())
}
// Snapshot every durable admission surface, including inactive outbox entries.
fn snapshot(runtime: &Runtime, workspace: Uuid) -> serde_json::Value {
    let w = workspace.to_string();
    serde_json::json!({"configuration": runtime.store.workflow_configuration(&w).unwrap(),
        "state": runtime.state(&w).unwrap(), "events": runtime.events(&w).unwrap(),
        "outbox": format!("{:?}", runtime.store.workflow_outbox(&w).unwrap())})
}
async fn held(
    runtime: &mut Runtime, factory: &RpcFactory, workspace: Uuid, action: Action,
    refuse: bool, change: impl FnOnce(),
) -> pb::Response {
    let pending = factory.handle(request(workspace, &action));
    tokio::pin!(pending);
    let message = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::select! {
            message = runtime.inbox.recv() => message.unwrap(),
            response = &mut pending => panic!("RPC did not reach held inbox: {response:?}"),
        }
    }).await.expect("RPC reached inbox");
    let before = snapshot(runtime, workspace);
    change();
    runtime.receive(message).await;
    let response = pending.await;
    if refuse {
        assert!(matches!(response.outcome, Some(response::Outcome::Error(_))), "changed authority committed: {response:?}");
        assert_eq!(snapshot(runtime, workspace), before, "changed authority wrote durable intent");
    } else {
        assert!(matches!(response.outcome, Some(response::Outcome::Result(_))), "unchanged authority refused: {response:?}");
        assert_ne!(snapshot(runtime, workspace), before, "unchanged authority did not commit");
    }
    response
}

#[tokio::test]
async fn workflow_held_queue_device_authority_changes_refuse_without_writes() {
    use farcooler_store::access::Cap;
    for change in 0..7 {
        for publication in [true, false] {
            let (_dir, svc, repo) = crate::test_support::fixture().await;
            let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
            svc.store.bind_owner_device("phone", None).unwrap();
            let dir = tempfile::tempdir().unwrap();
            let mut runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
            let definition = coding_definition(false);
            let pin = definition.pin();
            let publish = Action::Publish { adapter:None, mutation: mutation("p", 0, 1), expected_pin: None, definition };
            let action = if publication { publish } else {
                runtime.wire_command(&workspace.to_string(), "owner", publish).unwrap();
                Action::Enqueue { mutation: mutation("e", 0, 1), item: "a".into(), pin }
            };
            let rpc = factory(&svc, &runtime, Peer { client_id: Some("phone".into()), scope: Scope::Control }, None);
            eprintln!("device change={change} publication={publication}");
            held(&mut runtime, &rpc, workspace, action, change != 6, || match change {
                0 => { svc.store.set_device_cap("phone", Some(Cap::Viewer)).unwrap(); },
                1 => { svc.sessions().mark_revoked("phone"); },
                2 => { svc.sessions().reconnect("phone"); },
                3 => { svc.store.set_member("alice", "Alice", &["phone".into()]).unwrap(); },
                4 => { svc.sessions().mark_revoked("phone"); svc.sessions().readmit("phone").unwrap(); },
                5 => { svc.store.clear_device_rows("phone").unwrap(); },
                _ => {}, // Unchanged authority must still be usable.
            }).await;
        }
    }
}

#[tokio::test]
async fn workflow_held_queue_orchestrator_proof_changes_refuse_without_writes() {
    use farcooler_store::{models::{TerminalRole, TerminalUpdate}, claude_logins::TerminalLogin};
    for change in 0..5 {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
        let worktree = svc.store.list_worktrees_for_repository(repo).unwrap().remove(0);
        // A real shell process in an isolated tagged tmux pane; mark the launch
        // preset as agent without starting an agent or provider.
        let t = svc.create_terminal(worktree.id, "queue proof", "shell").await.unwrap();
        svc.store.update_terminal(t.id, t.resource_version, TerminalUpdate {
            title: t.title, command_preset: "claude".into(), intent: t.intent,
            runtime_confirmed: t.runtime_confirmed, exit_code: t.exit_code, exit_signal: t.exit_signal,
            lease_generation: t.lease_generation, epoch: t.epoch, columns: t.columns, rows: t.rows,
        }).unwrap();
        svc.store.set_terminal_role(t.id, TerminalRole::Orchestrator).unwrap();
        svc.inventory.refresh().await;
        let pid = svc.inventory_snapshot().panes.iter().find(|p| p.terminal_id == t.id).unwrap().stamp.pid as i32;
        assert_eq!(crate::caller_seat::caller_seat(&svc, Some(pid)).await, crate::caller_seat::Seat::Agent(Some(t.id)));
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        let definition = coding_definition(false); let pin = definition.pin();
        runtime.wire_command(&workspace.to_string(), "owner", Action::Publish { adapter:None,
            mutation: mutation("p", 0, 1), expected_pin: None, definition,
        }).unwrap();
        let rpc = factory(&svc, &runtime, Peer { client_id: None, scope: Scope::Control }, Some(pid));
        eprintln!("orchestrator change={change}");
        held(&mut runtime, &rpc, workspace, Action::Enqueue {
            mutation: mutation("e", 0, 1), item: "a".into(), pin,
        }, change != 4, || match change {
            0 => { svc.store.set_terminal_role(t.id, TerminalRole::Agent).unwrap(); },
            1 => {
                let other = svc.store.create_workspace(repo, "Other", "oth").unwrap();
                svc.store.set_terminal_workspace(t.id, other.id).unwrap();
            },
            2 => {
                svc.store.set_member("alice", "Alice", &[]).unwrap();
                let login_id = svc.store.ensure_member_login("alice").unwrap();
                svc.store.set_terminal_login(t.id, &TerminalLogin {
                    handle: "alice".into(), login_id, config_dir: "/fixture/login".into(), shared: false,
                }).unwrap();
            },
            3 => {
                svc.store.set_member("alice", "Alice", &[]).unwrap();
                svc.store.set_workspace_lead(workspace, Some("alice")).unwrap();
            },
            _ => {},
        }).await;
    }
}
