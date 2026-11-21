//! ov-1162: a board read must not turn a working remote runner into a reconnect loop.
use std::time::{Duration, Instant};
use farcooler_daemon::service::Service;
use farcooler_protocol::{v1::{self as pb, request, result, Scope}, workflow_commands::Action};
use farcooler_store::models::Actor;
use farcooler_transport::{ClientError, request as rpc};
use farcooler_workflow::{Command, Definition, Grant, Input, Progression, State, step};
use uuid::Uuid;

mod common;

/// A real daemon and its real stdio relay, with no sshd/account files involved.
/// The 750 KiB state is larger than the reported 695 KB workflow reply. A board
/// with 1,200 descriptions independently crosses the 1 MiB envelope ceiling.
#[tokio::test]
async fn a_large_first_sync_does_not_disconnect_the_control_session() {
    exercise(750 * 1024).await;
}

/// Older Plan clients use the unmarked workflow read, even with many live items.
#[tokio::test]
async fn an_oversized_live_workflow_read_loads_on_the_control_session() {
    exercise(1200 * 1024).await;
}

async fn exercise(state_bytes: usize) {
    let dir = tempfile::Builder::new().prefix("fc1162").tempdir_in("/tmp").unwrap();
    let service = Service::open_in(dir.path().to_path_buf()).await.unwrap();
    let root = service.store.create_repository_root(service.host_id, "/fixture", 0).unwrap();
    let repo = service.store.create_repository(service.host_id, root.id, "fixture", "/fixture/.git", "").unwrap();
    let workspace = service.store.ensure_main_workspace(repo.id).unwrap().id;
    let state = large_state(workspace, state_bytes);
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
    get.payload = Some(request::Payload::WorkflowCommand(pb::WorkflowCommand {
        schema: 1, json: serde_json::to_string(&Action::Get).unwrap(),
    }));
    let read = tokio::time::timeout(Duration::from_secs(5), client.call(get)).await.unwrap();
    let result::Value::WorkflowReply(reply) = read.expect("legacy workflow sync").value.unwrap() else { panic!("wrong reply") };
    let value: serde_json::Value = serde_json::from_str(&reply.json).unwrap();
    if state_bytes < farcooler_protocol::MAX_CONTROL_ENVELOPE_BYTES {
        assert_eq!(value["state"], serde_json::to_value(&state).unwrap(), "no truncated state");
        eprintln!("workflow reply: {} bytes", reply.json.len());
    } else {
        let projected: State = serde_json::from_value(value["state"].clone()).unwrap();
        assert_eq!(projected.revision, state.revision);
        assert!(projected.items.len() < state.items.len());
        let auth = serde_json::from_value(value["context"]["capabilities"].clone()).unwrap();
        let view = farcooler_client::workflow_presentation::project(
            &projected, &auth, farcooler_client::workflow_presentation::Style::Apple,
        ).unwrap();
        assert!(view.runs.len() > 100, "the legacy app still has live runs to render");
    }
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
    assert_eq!(warnings.len(), 1);
    for warning in warnings { eprintln!("{warning}"); }
}

fn oversized(answer: Result<pb::Result, ClientError>) {
    match answer {
        Err(ClientError::Daemon { code, retryable, what, .. }) => {
            assert_eq!(code, pb::ErrorCode::OperationFailed as i32);
            assert!(!retryable);
            assert_eq!(what, "response_too_large");
        }
        other => panic!("oversized sync must refuse only its read, keeping the link: {other:?}"),
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
