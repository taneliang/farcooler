//! ov-1162: a board read must not turn a working remote runner into a reconnect loop.
use std::time::{Duration, Instant};
use farcooler_daemon::service::Service;
use farcooler_protocol::{v1::{self as pb, request, result, Scope}, workflow_commands::Action};
use farcooler_store::models::Actor;
use farcooler_transport::{ClientError, request as rpc};
use farcooler_workflow::{Archived, Command, Definition, Grant, Grouping, Input, Phase, Progression, State, step};
use uuid::Uuid;

mod common;

/// A real daemon and its real stdio relay, with no sshd/account files involved.
/// The 750 KiB state is larger than the reported 695 KB workflow reply. A board
/// with 1,200 descriptions independently crosses the 1 MiB envelope ceiling.
#[tokio::test]
async fn a_large_first_sync_does_not_disconnect_the_control_session() {
    exercise(750 * 1024, 0).await;
}

/// Released Plan clients use an unmarked workflow read to enrich the board.
#[tokio::test]
async fn released_plan_loads_when_live_workflow_items_exceed_the_frame() {
    exercise(1200 * 1024, 0).await;
}

#[tokio::test]
async fn released_plan_keeps_live_runs_when_workflow_metadata_exceeds_the_frame() {
    exercise(8 * 1024, 1200 * 1024).await;
}

async fn exercise(item_bytes: usize, metadata_bytes: usize) {
    let dir = tempfile::Builder::new().prefix("fc1162").tempdir_in("/tmp").unwrap();
    let service = Service::open_in(dir.path().to_path_buf()).await.unwrap();
    let root = service.store.create_repository_root(service.host_id, "/fixture", 0).unwrap();
    let repo = service.store.create_repository(service.host_id, root.id, "fixture", "/fixture/.git", "").unwrap();
    let workspace = service.store.ensure_main_workspace(repo.id).unwrap().id;
    let mut state = large_state(workspace, item_bytes);
    state.source_sequences.insert("wire:owner".into(), 42);
    if metadata_bytes > 0 {
        state.source_sequences.extend((0..metadata_bytes / 100)
            .map(|n| (format!("old-source-{n}-{}", "x".repeat(100)), 1)));
    }
    let cap = farcooler_protocol::MAX_CONTROL_ENVELOPE_BYTES;
    assert_eq!(serde_json::to_vec(&state.items).unwrap().len() > cap, item_bytes > cap);
    assert_eq!(serde_json::to_vec(&state.source_sequences).unwrap().len() > cap, metadata_bytes > cap);
    let json = serde_json::to_string(&state).unwrap();
    eprintln!("scratch workflow state: {} bytes, {} items", json.len(), state.items.len());
    // Fixture-only seed, before the daemon opens the database. The state is a
    // collection of approval-waiting runs, with repeated definitions as on the
    // reported runner; nothing can launch an effect or touch a real repository.
    farcooler_store::testing::exec_for_tests(&service.store, &format!(
        "INSERT INTO workflow_states(workspace, revision, epoch, state) VALUES('{}', {}, 1, '{}')",
        workspace, state.revision, json.replace('\'', "''")));
    for i in 0..1200 {
        let task = service.store.create_task(workspace, &format!("ticket {i}"), Actor::User).unwrap();
        farcooler_store::testing::exec_for_tests(&service.store, &format!(
            "UPDATE tasks SET intent = '{}' WHERE id = X'{}'", "x".repeat(1024),
            task.id.simple()));
    }
    drop(service);
    let _daemon = common::listening_daemon(dir.path()).await;
    let began = Instant::now();
    let (_relay, mut client) = tokio::time::timeout(Duration::from_secs(5),
        common::spawn_with(dir.path(), &["--scope", "control"])).await.expect("stdio hello within five seconds");
    assert_eq!(client.server_hello().granted_scope, Scope::Control as i32);
    let mut get = rpc("workflow.get");
    get.target_resource_id = Some(workspace.as_bytes().to_vec().into());
    get.required_capabilities.push(farcooler_protocol::capability::WORKFLOW_COMMANDS.into());
    get.payload = Some(request::Payload::WorkflowCommand(pb::WorkflowCommand {
        schema: 1, json: serde_json::to_string(&Action::Get).unwrap(),
    }));
    let read = tokio::time::timeout(Duration::from_secs(5), client.call(get.clone())).await.unwrap();
    let workflow_oversized = json.len() > cap;
    let reply = read.expect("released workflow read").value.unwrap();
    if workflow_oversized {
        let result::Value::WorkflowReply(ref raw) = reply else { panic!("wrong workflow reply") };
        let value: serde_json::Value = serde_json::from_str(&raw.json).unwrap();
        assert_eq!(value["state"]["source_sequences"]["wire:owner"], 42);
        let workspace_id = workspace.as_bytes().to_vec().into();
        let mut plan_get = rpc("plan.get");
        plan_get.payload = Some(request::Payload::PlanGet(pb::PlanGetRequest {
            workspace_id, ..Default::default()
        }));
        let result::Value::Plan(plan) = client.call(plan_get).await.unwrap().value.unwrap() else {
            panic!("wrong Plan reply");
        };
        let view = farcooler_client::plan_json::with_workflow(&plan, workspace, Some(async { Ok(reply) }))
            .await.expect("released app decodes the bounded workflow snapshot");
        let workflow = &view["workflow"];
        assert_eq!(workflow["revision"], state.revision);
        assert_eq!(workflow["epoch"], state.epoch);
        let runs = workflow["runs"].as_array().unwrap();
        assert!(!runs.is_empty());
        assert!(runs.iter().all(|run| run["gate"].is_object()));
        if item_bytes > cap {
            assert!(runs.len() < state.items.len());
        } else {
            assert_eq!(runs.len(), state.items.len());
        }
        let mut full = get.clone();
        full.accepts.push("workflow_full_v1".into());
        oversized(client.call(full).await);
    } else {
        let result::Value::WorkflowReply(reply) = reply else { panic!("wrong reply") };
        let value: serde_json::Value = serde_json::from_str(&reply.json).unwrap();
        assert_eq!(value["state"], serde_json::to_value(&state).unwrap(), "complete workflow state");
        assert!(value["publication_history"].is_array());
        eprintln!("workflow reply: {} bytes", reply.json.len());
    }
    get.accepts.push(farcooler_protocol::accepts::WORKFLOW_PLAN.into());
    let result::Value::WorkflowReply(reply) = client.call(get).await.expect("marked Plan workflow read").value.unwrap() else {
        panic!("wrong Plan workflow reply");
    };
    let value: serde_json::Value = serde_json::from_str(&reply.json).unwrap();
    let plan_state: State = serde_json::from_value(value["state"].clone()).unwrap();
    assert_eq!(plan_state.revision, state.revision);
    assert!(!plan_state.items.is_empty());
    assert!(plan_state.archive.is_empty());
    assert!(plan_state.grouping.is_none());
    assert!(value["context"]["binding"].is_string());

    let mut board = rpc("task.list");
    board.payload = Some(request::Payload::TaskList(pb::TaskListRequest {
        repository_id: repo.id.as_bytes().to_vec().into(), workspace_id: Some(workspace.as_bytes().to_vec().into()),
        ..Default::default()
    }));
    oversized(tokio::time::timeout(Duration::from_secs(5), client.call(board)).await.unwrap());
    // Same link after both reads, not a replacement connection that hides a drop.
    for method in ["worktree.list", "terminal.list", "host.get", "workspace.list", "daemon.version"] {
        tokio::time::timeout(Duration::from_secs(5).saturating_sub(began.elapsed()), client.call(rpc(method)))
            .await.expect("first sync exceeded five seconds")
            .unwrap_or_else(|e| panic!("{method} after first sync: {e}"));
    }
    eprintln!("stdio control connected and answering after large sync: {:?}", began.elapsed());
    assert!(began.elapsed() < Duration::from_secs(5));
    let log = std::fs::read_to_string(dir.path().join("logs/farcoolerd.log")).unwrap();
    let warnings: Vec<_> = log.lines().filter(|line| line.contains("response exceeds frame cap")).collect();
    assert_eq!(warnings.len(), 1 + usize::from(workflow_oversized));
    for warning in warnings { eprintln!("{warning}"); }
}

fn oversized(answer: Result<pb::Result, ClientError>) {
    match answer {
        Err(ClientError::Daemon { code, retryable, what, .. }) => {
            assert_eq!(code, pb::ErrorCode::OperationFailed as i32);
            assert!(!retryable);
            assert_eq!(what, "response_too_large");
        }
        Ok(_) => panic!("oversized sync succeeded with an incomplete reply"),
        Err(error) => panic!("oversized sync must refuse only its read, keeping the link: {error}"),
    }
}

fn large_state(workspace: Uuid, bytes: usize) -> State {
    let mut definition = Definition::coding();
    definition.stages[0].progression = Progression::AwaitApproval { principal: "owner".into() };
    let empty = State::empty(workspace.to_string(), 1);
    let input = Input { schema: 1, id: "seed".into(), workspace: workspace.to_string(), expected_revision: 0,
        epoch: 1, source: "controller".into(), source_sequence: 1, now_ms: 1, grant: Grant::Controller,
        busy_resources: Default::default(), command: Command::Enqueue { item: "seed".into(), definition,
            policy_pin: "policy".into(), resource: "seed".into(), dependencies: Default::default(), initial_candidate: None } };
    let mut state = step(&empty, &input).unwrap().state;
    let template = state.items["seed"].clone();
    state.grouping = Some(Grouping { definition: template.definition.clone(), policy_pin: "policy".into() });
    state.archive.insert("finished".into(), Archived {
        start_load: None, closed: None, phase: Phase::Done, candidate: None, group: None, last_run: None, history: vec![],
    });
    let mut encoded = serde_json::to_vec(&state).unwrap().len();
    while encoded < bytes {
        let name = format!("item-{}", state.items.len());
        let mut item = template.clone();
        item.run = name.clone();
        item.resource = name.clone();
        encoded += serde_json::to_vec(&name).unwrap().len() + serde_json::to_vec(&item).unwrap().len() + 2;
        state.items.insert(name, item);
    }
    assert!(serde_json::to_vec(&state).unwrap().len() >= bytes);
    state
}
