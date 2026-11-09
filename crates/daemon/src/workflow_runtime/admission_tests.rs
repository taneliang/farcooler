//! Hold the actual RPC inbox before consumption; no timing sleeps or providers.
use super::*;
use std::sync::{Arc, OnceLock};
use crate::{rpc::RpcFactory, service::Service, watch::Watcher};
use farcooler_protocol::{v1::{self as pb, request, response, Scope}, workflow_commands::{Action, Mutation}};
use farcooler_transport::{Handler, Peer};

pub(super) fn request(workspace: Uuid, action: &Action) -> pb::Request {
    let mut req = farcooler_transport::request(action.method());
    req.target_resource_id = Some(bytes::Bytes::copy_from_slice(workspace.as_bytes()));
    req.payload = Some(request::Payload::WorkflowCommand(pb::WorkflowCommand {
        schema: 1, json: serde_json::to_string(action).unwrap(),
    }));
    req
}
pub(super) fn mutation(id: &str, revision: u64, sequence: u64) -> Mutation {
    Mutation { id: id.into(), revision, epoch: 1, sequence }
}
pub(super) fn factory(svc: &Arc<Service>, runtime: &Runtime, peer: Peer, pid: Option<i32>) -> RpcFactory {
    RpcFactory::new(svc.clone(), Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer)
        .with_caller_pid(pid).with_workflow_fixture(runtime.command_handle(&runtime.controller()).unwrap())
}
// Snapshot every durable admission surface, including inactive outbox entries.
pub(super) fn snapshot(runtime: &Runtime, workspace: Uuid) -> serde_json::Value {
    let w = workspace.to_string();
    serde_json::json!({"configuration": runtime.store.workflow_configuration(&w).unwrap(),
        "state": runtime.state(&w).unwrap(), "events": runtime.events(&w).unwrap(),
        "outbox": format!("{:?}", runtime.store.workflow_outbox(&w).unwrap())})
}
pub(super) async fn held(
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

/// One change to the device's authority while a command is held, published
/// or enqueued. A test per change and kind (ov-993): fourteen fixtures in one
/// test took 9 s; side by side they take one fixture's time.
async fn device_authority_change(change: u8, publication: bool) {
    use farcooler_store::access::Cap;
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    svc.store.bind_owner_device("phone", None).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
    let definition = coding_definition(false);
    let pin = definition.pin();
    let publish = Action::Publish { notifications:None, adapter:None, mutation: mutation("p", 0, 1), expected_pin: None, definition };
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
macro_rules! device_authority_changes {
    ($($name:ident = $change:literal, $publication:literal;)*) => {
        mod workflow_held_queue_device_authority_changes_refuse_without_writes {
            $(#[tokio::test] async fn $name() { super::device_authority_change($change, $publication).await; })*
        }
    };
}
device_authority_changes! {
    viewer_cap_publish = 0, true; viewer_cap_enqueue = 0, false;
    revoked_publish = 1, true; revoked_enqueue = 1, false;
    reconnected_publish = 2, true; reconnected_enqueue = 2, false;
    made_a_member_publish = 3, true; made_a_member_enqueue = 3, false;
    revoked_and_readmitted_publish = 4, true; revoked_and_readmitted_enqueue = 4, false;
    rows_cleared_publish = 5, true; rows_cleared_enqueue = 5, false;
    unchanged_publish = 6, true; unchanged_enqueue = 6, false;
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
        runtime.wire_command(&workspace.to_string(), "owner", Action::Publish { notifications:None, adapter:None,
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

#[test]
fn only_a_person_may_publish_or_settle_a_push() {
    use farcooler_protocol::workflow_commands::{parse, Action};
    let mutation = r#""mutation":{"id":"s","revision":3,"epoch":1,"sequence":2}"#;
    let settle = parse(&format!(r#"{{"action":"Settle",{mutation},"item":"a","run":"run","pin":"pin","landed":true}}"#)).unwrap();
    assert!(matches!(settle, Action::Settle { .. }));
    assert!(super::admission::person_only(&settle), "an orchestrator must not settle a push");
    let queue = parse(&format!(r#"{{"action":"Queue",{mutation},"item":"a","run":"run","pin":"pin"}}"#)).unwrap();
    assert!(!super::admission::person_only(&queue), "the orchestrator still queues");
}

/// ov-999: a pane in `workspace` on the login of Alice, who leads it, as a
/// real shell in a tagged tmux pane: its id and root pid. As `orchestrator`,
/// the workspace's orchestrator, marked as an agent's; else her own shell.
pub(super) async fn alices_pane(svc: &Arc<Service>, repo: Uuid, workspace: Uuid, orchestrator: bool) -> (Uuid, i32) {
    use farcooler_store::{models::{TerminalRole, TerminalUpdate}, claude_logins::TerminalLogin};
    let worktree = svc.store.list_worktrees_for_repository(repo).unwrap().remove(0);
    let t = svc.create_terminal(worktree.id, "alice's pane", "shell").await.unwrap();
    if orchestrator {
        svc.store.update_terminal(t.id, t.resource_version, TerminalUpdate {
            title: t.title, command_preset: "claude".into(), intent: t.intent,
            runtime_confirmed: t.runtime_confirmed, exit_code: t.exit_code, exit_signal: t.exit_signal,
            lease_generation: t.lease_generation, epoch: t.epoch, columns: t.columns, rows: t.rows,
        }).unwrap();
        svc.store.set_terminal_role(t.id, TerminalRole::Orchestrator).unwrap();
    }
    svc.store.set_terminal_workspace(t.id, workspace).unwrap();
    svc.store.set_member("alice", "Alice", &[]).unwrap();
    let login_id = svc.store.ensure_member_login("alice").unwrap();
    svc.store.set_terminal_login(t.id, &TerminalLogin {
        handle: "alice".into(), login_id, config_dir: "/fixture/login".into(), shared: false,
    }).unwrap();
    svc.store.set_workspace_lead(workspace, Some("alice")).unwrap();
    svc.inventory.refresh().await;
    let pid = svc.inventory_snapshot().panes.iter().find(|p| p.terminal_id == t.id).unwrap().stamp.pid as i32;
    let seat = crate::caller_seat::caller_seat(svc, Some(pid)).await;
    assert_eq!(seat, if orchestrator { crate::caller_seat::Seat::Agent(Some(t.id)) } else { crate::caller_seat::Seat::Person });
    (t.id, pid)
}

/// One `workflow.*` call through dispatch, handed to the runtime if it got
/// that far. `None` in the second place when nothing reached the inbox.
pub(super) async fn sent(runtime: &mut Runtime, rpc: &RpcFactory, workspace: Uuid, action: &Action) -> (pb::Response, bool) {
    let pending = rpc.handle(request(workspace, action));
    tokio::pin!(pending);
    let reached = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::select! {
            message = runtime.inbox.recv() => Ok(message.unwrap()),
            response = &mut pending => Err(response),
        }
    }).await.expect("an answer or the inbox");
    match reached {
        Err(response) => (response, false),
        Ok(message) => { runtime.receive(message).await; (pending.await, true) }
    }
}

/// The engine's receipt in a dispatch reply; panics on a refusal.
pub(super) fn receipt(response: &pb::Response) -> serde_json::Value {
    match &response.outcome {
        Some(response::Outcome::Result(r)) => match &r.value {
            Some(pb::result::Value::WorkflowReply(reply)) => serde_json::from_str(&reply.json).unwrap(),
            other => panic!("not a workflow reply: {other:?}"),
        },
        other => panic!("refused: {other:?}"),
    }
}
pub(super) fn ok(response: &pb::Response) {
    let value = receipt(response);
    assert!(value.pointer("/receipt/result/Ok").is_some(), "{value}");
}
pub(super) fn refused(response: &pb::Response) -> String {
    match &response.outcome {
        Some(response::Outcome::Error(e)) => farcooler_core::error::word_for(e.code).to_string(),
        other => panic!("admitted: {other:?}"),
    }
}

/// ov-999: the orchestrator a member leads enqueues, starts and reviews its
/// own workspace's tickets through `workflow.*` dispatch, as the
/// `orchestrator` principal; a cutover stays the owner's.
#[tokio::test]
async fn workflow_a_member_led_orchestrator_drives_its_own_workspace() {
    use farcooler_protocol::workflow_config::AdapterConfiguration;
    use farcooler_workflow::Reviewer;
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let (_pane, pid) = alices_pane(&svc, repo, workspace, true).await;
    assert_eq!(crate::caller_person::caller_person(&svc, &Peer { client_id: None, scope: Scope::Control }, Some(pid),
        &OnceLock::new()).unwrap(), crate::people::Person::Member("alice".into()), "the pane is Alice's");
    let w = workspace.to_string();
    let peer = Peer { client_id: None, scope: Scope::Control };

    // Enqueue, and review where the orchestrator is the reviewer.
    let dir = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
    let mut definition = coding_definition(true);
    definition.stages.iter_mut().find(|s| matches!(s.kind, StageKind::Review(_))).unwrap().kind =
        StageKind::Review(Reviewer::Orchestrator("orchestrator".into()));
    let pin = definition.pin();
    runtime.wire_command(&w, "owner", Action::Publish { notifications: None, adapter: None,
        mutation: mutation("p", 0, 1), expected_pin: None, definition }).unwrap();
    let rpc = factory(&svc, &runtime, peer.clone(), Some(pid));
    let (reply, _) = sent(&mut runtime, &rpc, workspace, &Action::Enqueue { mutation: mutation("e", 0, 1), item: "a".into(), pin: pin.clone() }).await;
    ok(&reply);
    // The receipt names Alice, and the pane it came from (ov-1001).
    assert_eq!(receipt(&reply).pointer("/receipt/actor"), Some(&serde_json::json!({"person": "member:alice", "pane": _pane, "orchestrator": true})));
    for time in 1..15 {
        runtime.pump(time).await.unwrap();
        if runtime.state(&w).unwrap().unwrap().items["a"].phase == Phase::WaitingReview { break; }
    }
    let state = runtime.state(&w).unwrap().unwrap(); let item = &state.items["a"];
    assert_eq!(item.phase, Phase::WaitingReview);
    let (reply, _) = sent(&mut runtime, &rpc, workspace, &Action::Review { mutation: mutation("r", state.revision, 2), item: "a".into(),
        run: item.run.clone(), pin: pin.clone(), evidence: item.last_evidence.clone().unwrap(), passed: true, reason: None }).await;
    ok(&reply);
    assert_ne!(runtime.state(&w).unwrap().unwrap().items["a"].phase, Phase::WaitingReview, "the review moved it on");

    // Start a tracked ticket under the published coding workflow.
    let (_coding, mut runtime) = super::coding_tests::configured(false, "pass").await;
    let published = runtime.coding.clone().unwrap();
    let definition = published.definition().unwrap();
    let adapter = Some(Box::new(AdapterConfiguration::Coding(published)));
    let start_pin = farcooler_store::workflow_publication::configuration_pin(&definition, adapter.as_deref());
    runtime.wire_command(&w, "owner", Action::Publish { notifications: None, adapter, mutation: mutation("p", 0, 1),
        expected_pin: None, definition }).unwrap();
    assert_eq!(runtime.track_tickets(&[(w.clone(), "ov-7".into(), "todo".into())]).unwrap(), 1);
    let state = runtime.state(&w).unwrap().unwrap(); let item = &state.items["ov-7"];
    let rpc = factory(&svc, &runtime, peer.clone(), Some(pid));
    let (reply, _) = sent(&mut runtime, &rpc, workspace, &Action::Start { mutation: mutation("s", state.revision, 1), item: "ov-7".into(),
        run: item.run.clone(), pin: item.pin.clone(), start_pin, models: Default::default(), worktree: None, with: vec![] }).await;
    ok(&reply);

    // Admitted to the engine is not admitted to a cutover.
    let cutover = crate::cutover_delegate::admit(&svc, peer, svc.sessions().open(None), Some(pid), Arc::new(OnceLock::new()), "a-1").await;
    assert!(matches!(cutover, Err(farcooler_core::DomainError::ScopeDenied { needed: "owner" })), "{:?}", cutover.err());
}

/// ov-999: the same pane is refused on any other workspace: one nobody leads
/// (dispatch's owner check), and one Alice leads too (admission: it is not
/// that workspace's orchestrator). Neither reaches the engine.
#[tokio::test]
async fn workflow_a_member_led_orchestrator_is_refused_on_another_workspace() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let (_pane, pid) = alices_pane(&svc, repo, workspace, true).await;
    let unled = svc.store.create_workspace(repo, "Unled", "unl").unwrap().id;
    let also_alices = svc.store.create_workspace(repo, "Also Alice's", "als").unwrap().id;
    svc.store.set_workspace_lead(also_alices, Some("alice")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
    let definition = coding_definition(false); let pin = definition.pin();
    for other in [unled, also_alices] {
        runtime.wire_command(&other.to_string(), "owner", Action::Publish { notifications: None, adapter: None,
            mutation: mutation("p", 0, 1), expected_pin: None, definition: definition.clone() }).unwrap();
    }
    let rpc = factory(&svc, &runtime, Peer { client_id: None, scope: Scope::Control }, Some(pid));
    for (other, word) in [(unled, "scope-denied"), (also_alices, "scope-denied")] {
        let before = snapshot(&runtime, other);
        let (reply, reached) = sent(&mut runtime, &rpc, other, &Action::Enqueue { mutation: mutation("e", 0, 1), item: "a".into(), pin: pin.clone() }).await;
        assert_eq!(refused(&reply), word, "{other}");
        assert!(!reached, "refused before the engine");
        assert_eq!(snapshot(&runtime, other), before);
    }
}

/// ov-999: `workflow.*` dispatch lets a member's pane reach the engine only
/// on a workspace they lead (or as a Maintainer, R-107), before admission is
/// asked. Alice's own shell, an agent nowhere, asking to enqueue: refused
/// there where she doesn't lead (`scope-denied`); where she does, admitted,
/// as `member:alice`. Remove the check in `Rpc::engine_caller` and the first
/// is refused by admission instead (`not-yours`).
#[tokio::test]
async fn workflow_dispatch_admits_a_member_only_where_they_lead() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let (_pane, pid) = alices_pane(&svc, repo, workspace, false).await;
    let dir = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
    let definition = coding_definition(false); let pin = definition.pin();
    runtime.wire_command(&workspace.to_string(), "owner", Action::Publish { notifications: None, adapter: None,
        mutation: mutation("p", 0, 1), expected_pin: None, definition }).unwrap();
    let rpc = factory(&svc, &runtime, Peer { client_id: None, scope: Scope::Control }, Some(pid));
    let enqueue = Action::Enqueue { mutation: mutation("e", 0, 1), item: "a".into(), pin: pin.clone() };
    svc.store.set_workspace_lead(workspace, None).unwrap();
    let before = snapshot(&runtime, workspace);
    let (reply, reached) = sent(&mut runtime, &rpc, workspace, &enqueue).await;
    assert_eq!(refused(&reply), "scope-denied", "where she doesn't lead");
    assert!(!reached);
    assert_eq!(snapshot(&runtime, workspace), before);
    svc.store.set_workspace_lead(workspace, Some("alice")).unwrap();
    let (reply, _) = sent(&mut runtime, &rpc, workspace, &enqueue).await;
    ok(&reply);
    assert!(runtime.events(&workspace.to_string()).unwrap().iter().any(|e| e.source == "wire:member:alice"), "as Alice, not the owner");
}
