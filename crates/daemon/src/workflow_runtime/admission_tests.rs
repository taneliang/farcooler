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
    let publish = Action::Publish { reason:None, notifications:None, adapter:None, mutation: mutation("p", 0, 1), expected_pin: None, definition };
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
        runtime.wire_command(&workspace.to_string(), "owner", Action::Publish { reason:None, notifications:None, adapter:None,
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
fn settling_a_push_stays_person_only_and_queue_stays_delegable() {
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
    use farcooler_store::models::TerminalRole;
    alices_pane_as(svc, repo, workspace, if orchestrator { TerminalRole::Orchestrator } else { TerminalRole::Shell }).await
}
/// `alices_pane` in `role`: an orchestrator, an agent (a builder's or a reviewer's pane), or her own shell.
pub(super) async fn alices_pane_as(svc: &Arc<Service>, repo: Uuid, workspace: Uuid, role: farcooler_store::models::TerminalRole) -> (Uuid, i32) {
    use farcooler_store::{models::{TerminalRole, TerminalUpdate}, claude_logins::TerminalLogin};
    let worktree = svc.store.list_worktrees_for_repository(repo).unwrap().remove(0);
    let t = svc.create_terminal(worktree.id, "alice's pane", "shell").await.unwrap();
    let agent = role != TerminalRole::Shell;
    if agent {
        svc.store.update_terminal(t.id, t.resource_version, TerminalUpdate {
            title: t.title, command_preset: "claude".into(), intent: t.intent,
            runtime_confirmed: t.runtime_confirmed, exit_code: t.exit_code, exit_signal: t.exit_signal,
            lease_generation: t.lease_generation, epoch: t.epoch, columns: t.columns, rows: t.rows,
        }).unwrap();
        svc.store.set_terminal_role(t.id, role).unwrap();
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
    assert_eq!(seat, if agent { crate::caller_seat::Seat::Agent(Some(t.id)) } else { crate::caller_seat::Seat::Person });
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
    runtime.wire_command(&w, "owner", Action::Publish { reason:None, notifications: None, adapter: None,
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
    runtime.wire_command(&w, "owner", Action::Publish { reason:None, notifications: None, adapter, mutation: mutation("p", 0, 1),
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
        runtime.wire_command(&other.to_string(), "owner", Action::Publish { reason:None, notifications: None, adapter: None,
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
    runtime.wire_command(&workspace.to_string(), "owner", Action::Publish { reason:None, notifications: None, adapter: None,
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

#[tokio::test]
async fn ov1179_only_the_granted_workspace_orchestrator_publishes() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let ws = svc.store.ensure_main_workspace(repo).unwrap().id;
    let (_pane, pid) = alices_pane(&svc, repo, ws, true).await;
    let mut runtime = Runtime::open_service(&svc.root_dir().join("publication-test"), svc.store.clone()).unwrap();
    let watcher = Watcher::new(svc.clone());
    let mut events = watcher.subscribe();
    let agent = RpcFactory::new(svc.clone(),watcher,Arc::new(tokio::sync::Notify::new()),Peer {client_id:None,scope:Scope::Control})
        .with_caller_pid(Some(pid)).with_workflow_fixture(runtime.command_handle(&runtime.controller()).unwrap());
    let owner = factory(&svc, &runtime, Peer { client_id: None, scope: Scope::Control }, None);
    let grant = |enabled, revision| Action::PublicationGrant { mutation: mutation(if enabled {"grant"} else {"revoke"}, revision, 1), enabled };
    let publication = Action::Publish { reason:Some("Make the board track work".into()), notifications:None, adapter:None,
        mutation:mutation("publish",0,1), expected_pin:None, definition:Definition::tracker("owner") };
    // On unless a person reserves it (ov-1267): the owner reserves it here, so the grant below is what lifts it.
    ok(&sent(&mut runtime, &owner, ws, &Action::PublicationGrant { mutation: mutation("reserve", 0, 1), enabled: false }).await.0);
    let (response, _) = sent(&mut runtime, &agent, ws, &publication).await;
    assert!(matches!(response.outcome, Some(response::Outcome::Error(_))), "published while reserved");
    let (response, _) = sent(&mut runtime, &agent, ws, &grant(true, 0)).await;
    assert!(matches!(response.outcome, Some(response::Outcome::Error(_))), "agent grants itself authority");
    let (response, _) = sent(&mut runtime, &owner, ws, &grant(true, 0)).await;
    ok(&response);
    let member = super::member_tests::alices_phone(&svc);
    svc.store.set_member_role("alice",farcooler_store::member_roles::Role::Maintainer,"user").unwrap();
    let member = factory(&svc,&runtime,member,None);
    let (response,_) = sent(&mut runtime,&member,ws,&grant(true,0)).await;
    assert!(matches!(response.outcome,Some(response::Outcome::Error(_))),"a member granted publication");
    let (response,_) = sent(&mut runtime,&member,ws,&publication).await;
    assert!(matches!(response.outcome,Some(response::Outcome::Error(_))),"a member in person used the orchestrator grant");
    let mut unexplained = publication.clone();
    if let Action::Publish {reason,..} = &mut unexplained { *reason=None; }
    let (response,_) = sent(&mut runtime,&agent,ws,&unexplained).await;
    assert!(matches!(response.outcome,Some(response::Outcome::Error(_))),"an agent published without a reason");
    let (response, _) = sent(&mut runtime, &agent, ws, &publication).await;
    ok(&response);
    let audit = receipt(&response);
    assert_eq!(audit["receipt"]["actor"]["person"],"member:alice");
    assert_eq!(audit["receipt"]["actor"]["orchestrator"],true);
    assert_eq!(audit["receipt"]["change"]["reason"],"Make the board track work");
    let notice = Uuid::parse_str(audit.pointer("/receipt/change/notice_task").unwrap().as_str().unwrap()).unwrap();
    assert_eq!(svc.store.get_task(notice).unwrap().status, farcooler_store::models::TaskStatus::InReview);
    let mut announced = false;
    while let Ok(event) = events.try_recv() {
        if let Some(pb::event::Payload::TaskChanged(changed)) = event.payload {
            announced |= changed.task_id.as_ref() == notice.as_bytes();
        }
    }
    assert!(announced,"owner's board never heard the publication notice");
    let other = svc.store.create_workspace(repo,"Other","oth").unwrap().id;
    let (response,_) = sent(&mut runtime,&agent,other,&publication).await;
    assert!(matches!(response.outcome,Some(response::Outcome::Error(_))),"grant escaped its workspace");
    let rollback = Action::Rollback {mutation:mutation("rollback",0,1),expected_pin:"pin".into(),reason:"Undo".into()};
    let (response,_) = sent(&mut runtime,&agent,ws,&rollback).await;
    assert!(matches!(response.outcome,Some(response::Outcome::Error(_))),"agent rolled back as the owner");
    let first = runtime.store.workflow_configuration(&ws.to_string()).unwrap().unwrap();
    let revision = runtime.state(&ws.to_string()).unwrap().map_or(0,|s|s.revision);
    let second = Action::Publish {reason:Some("Let the orchestrator track the board".into()),notifications:None,adapter:None,
        mutation:mutation("second",revision,2),expected_pin:Some(first.pin.clone()),definition:Definition::tracker("orchestrator")};
    ok(&sent(&mut runtime,&agent,ws,&second).await.0);
    let selected = runtime.store.workflow_configuration(&ws.to_string()).unwrap().unwrap();
    let revision = runtime.state(&ws.to_string()).unwrap().map_or(0,|s|s.revision);
    let undo = Action::Rollback {mutation:mutation("undo",revision,1),expected_pin:selected.pin,reason:"Restore the owner gate".into()};
    ok(&sent(&mut runtime,&owner,ws,&undo).await.0);
    assert_eq!(runtime.store.workflow_configuration(&ws.to_string()).unwrap(),Some(first));
    let revision = runtime.state(&ws.to_string()).unwrap().map_or(0,|s|s.revision);
    let (response, _) = sent(&mut runtime, &agent, ws, &grant(false, revision)).await;
    assert!(matches!(response.outcome, Some(response::Outcome::Error(_))), "agent revokes the owner grant");
    let (response, _) = sent(&mut runtime, &owner, ws, &grant(false, revision)).await;
    ok(&response);
    let (response, _) = sent(&mut runtime, &agent, ws, &publication).await;
    assert!(matches!(response.outcome, Some(response::Outcome::Error(_))), "revocation stops replay too");
}

#[tokio::test]
async fn ov1179_revocation_at_consumption_stops_a_queued_publish() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let ws = svc.store.ensure_main_workspace(repo).unwrap().id;
    let (_, pid) = alices_pane(&svc, repo, ws, true).await;
    let mut runtime = Runtime::open_service(&svc.root_dir().join("publication-test"), svc.store.clone()).unwrap();
    let agent = factory(&svc, &runtime, Peer { client_id:None,scope:Scope::Control }, Some(pid));
    let owner = factory(&svc, &runtime, Peer { client_id:None,scope:Scope::Control }, None);
    let grant = Action::PublicationGrant {mutation:mutation("grant",0,1),enabled:true};
    ok(&sent(&mut runtime,&owner,ws,&grant).await.0);
    let publish = Action::Publish {reason:Some("Track the workspace".into()),notifications:None,adapter:None,
        mutation:mutation("publish",0,1),expected_pin:None,definition:Definition::tracker("owner")};
    let store = svc.store.clone();
    held(&mut runtime,&agent,ws,publish,true,|| {
        store.workflow_wire_write_by(&ws.to_string(),"revoke","revoke",None,
            farcooler_store::workflow_publication::Write::PublicationGrant {enabled:false,revision:0}).unwrap();
    }).await;
    assert!(runtime.store.workflow_configuration(&ws.to_string()).unwrap().is_none());
}

/// ov-1179: a real publication grant must not bypass the runner's admin fences.
/// Use HostAdmin scope so a scope failure cannot mask a missing person check.
/// These are runner administration checks, not tests of external deployment or secret permissions.
#[tokio::test]
async fn ov1179_publication_grant_preserves_runner_admin_operation_boundaries() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let ws = svc.store.ensure_main_workspace(repo).unwrap().id;
    let (_pane, pid) = alices_pane(&svc, repo, ws, true).await;
    let mut runtime = Runtime::open_service(&svc.root_dir().join("publication-test"), svc.store.clone()).unwrap();
    let owner = factory(&svc, &runtime, Peer { client_id: None, scope: Scope::HostAdmin }, None);
    let agent = factory(&svc, &runtime, Peer { client_id: None, scope: Scope::HostAdmin }, Some(pid));
    ok(&sent(&mut runtime, &owner, ws, &Action::PublicationGrant {
        mutation: mutation("grant", 0, 1), enabled: true,
    }).await.0);
    ok(&sent(&mut runtime, &agent, ws, &Action::Publish {
        reason: Some("Track the board".into()), notifications: None, adapter: None,
        mutation: mutation("publish", 0, 1), expected_pin: None, definition: Definition::tracker("owner"),
    }).await.0);
    svc.store.set_member("bob", "Bob", &["phone-b".into()]).unwrap();
    let before = svc.store.stored_access().unwrap();
    let login_policy = svc.store.member_login_policy().unwrap();
    for (method, payload, refused_as) in [
        ("member.set_access", request::Payload::MemberSetAccess(pb::MemberSetAccess { handle: "bob".into(), access: "admin".into() }), "person_only"),
        ("client.set_cap", request::Payload::ClientSetCap(pb::ClientSetCap { client_id: "phone-b".into(), cap: "viewer".into() }), "person_only"),
        ("member.set", request::Payload::MemberSet(pb::MemberSet { handle: "carol".into(), name: "Carol".into(), ..Default::default() }), "person_only"),
        ("member.remove", request::Payload::MemberRemove(pb::MemberRemove { handle: "bob".into() }), "person_only"),
        ("member.set_own_login", request::Payload::MemberOwnLoginSet(pb::MemberOwnLoginSet { harness: "codex".into(), on: true }), "in_pane"),
    ] {
        let mut req = farcooler_transport::request(method);
        req.payload = Some(payload);
        let reply = agent.handle(req).await;
        assert!(matches!(reply.outcome, Some(response::Outcome::Error(ref e)) if e.what == refused_as), "{method}: {reply:?}");
        assert_eq!(svc.store.stored_access().unwrap(), before, "{method} changed access");
        assert_eq!(svc.store.member_login_policy().unwrap(), login_policy, "{method} changed credential sharing");
        assert!(svc.store.member("bob").is_ok());
        assert!(svc.store.member("carol").is_err());
    }
}

/// ov-1267 (owner, Nov 15: "you may absolutely publish workflows"): a workspace's own orchestrator publishes its
/// workflow from its pane with no grant asked for first. Each publish needs a reason and records who sent it, why,
/// and the pins before and after. A builder's or a reviewer's agent pane in the same workspace, and another
/// workspace's orchestrator, stay refused, as does every orchestrator once the owner reserves publishing.
#[tokio::test]
async fn ov1267_the_workspace_orchestrator_publishes_and_no_other_agent_does() {
    use farcooler_store::models::TerminalRole;
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let ws = svc.store.ensure_main_workspace(repo).unwrap().id;
    let w = ws.to_string();
    let other = svc.store.create_workspace(repo, "Other", "oth").unwrap().id;
    let (_, builder) = alices_pane_as(&svc, repo, ws, TerminalRole::Agent).await;
    let (_, reviewer) = alices_pane_as(&svc, repo, ws, TerminalRole::Agent).await;
    let (_, elsewhere) = alices_pane_as(&svc, repo, other, TerminalRole::Orchestrator).await;
    let (pane, orchestrator) = alices_pane_as(&svc, repo, ws, TerminalRole::Orchestrator).await;
    let mut runtime = Runtime::open_service(&svc.root_dir().join("publication-test"), svc.store.clone()).unwrap();
    let peer = Peer { client_id: None, scope: Scope::Control };
    let rpc = |pid| factory(&svc, &runtime, peer.clone(), Some(pid));
    let (agent, builder, reviewer, elsewhere) = (rpc(orchestrator), rpc(builder), rpc(reviewer), rpc(elsewhere));
    let owner = factory(&svc, &runtime, peer.clone(), None);
    let sequence = std::cell::Cell::new(1);
    let publish = |id: &str, revision, expected: Option<String>, reason: Option<&str>, definition| Action::Publish {
        reason: reason.map(String::from), notifications: None, adapter: None,
        mutation: mutation(id, revision, sequence.replace(sequence.get() + 1)), expected_pin: expected, definition };

    // Builder, reviewer and another workspace's orchestrator: refused before anything is written.
    for (who, rpc) in [("a builder", &builder), ("a reviewer", &reviewer), ("another workspace's orchestrator", &elsewhere)] {
        let before = snapshot(&runtime, ws);
        let (reply, _) = sent(&mut runtime, rpc, ws, &publish("stray", 0, None, Some("Track the board"), Definition::tracker("owner"))).await;
        assert_eq!(refused(&reply), "scope-denied", "{who} published");
        assert_eq!(snapshot(&runtime, ws), before, "{who} wrote");
    }
    // No reason: refused, nothing written.
    let before = snapshot(&runtime, ws);
    let (reply, _) = sent(&mut runtime, &agent, ws, &publish("bare", 0, None, None, Definition::tracker("owner"))).await;
    assert!(matches!(reply.outcome, Some(response::Outcome::Error(_))), "published with no reason");
    assert_eq!(snapshot(&runtime, ws), before);

    // The orchestrator publishes, with no grant asked for first.
    let first = Definition::tracker("owner");
    let reply = sent(&mut runtime, &agent, ws, &publish("first", 0, None, Some("Track the board"), first.clone())).await.0;
    ok(&reply);
    let audit = receipt(&reply)["receipt"].clone();
    assert_eq!(audit["actor"], serde_json::json!({"person": "member:alice", "pane": pane, "orchestrator": true}));
    assert_eq!(audit["change"]["reason"], "Track the board");
    assert_eq!(audit["change"]["previous"], serde_json::Value::Null);
    let first_pin = runtime.store.workflow_configuration(&w).unwrap().unwrap().pin;
    assert_eq!(audit["change"]["pin"], serde_json::json!(first_pin));
    // A second publish records the pin it replaced and the one it set.
    let revision = runtime.state(&w).unwrap().map_or(0, |s| s.revision);
    let reply = sent(&mut runtime, &agent, ws, &publish("second", revision, Some(first_pin.clone()), Some("Let the orchestrator decide"),
        Definition::tracker("orchestrator"))).await.0;
    ok(&reply);
    let audit = receipt(&reply)["receipt"].clone();
    let second_pin = runtime.store.workflow_configuration(&w).unwrap().unwrap().pin;
    assert_ne!(first_pin, second_pin);
    assert_eq!((audit["change"]["previous"].clone(), audit["change"]["pin"].clone()), (serde_json::json!(first_pin), serde_json::json!(second_pin)));
    assert!(audit["change"]["notice_task"].is_string(), "the owner is shown the change: {audit}");

    // The owner reserves publishing to a person; the orchestrator is refused from then on.
    let revision = runtime.state(&w).unwrap().map_or(0, |s| s.revision);
    ok(&sent(&mut runtime, &owner, ws, &Action::PublicationGrant { mutation: mutation("reserve", revision, 1), enabled: false }).await.0);
    let before = snapshot(&runtime, ws);
    let (reply, _) = sent(&mut runtime, &agent, ws, &publish("third", revision, Some(second_pin), Some("Once more"), Definition::tracker("owner"))).await;
    assert_eq!(refused(&reply), "scope-denied", "published past the owner's reservation");
    assert_eq!(snapshot(&runtime, ws), before);
}
