//! ov-962: a board tracks its cards once it has a published workflow, and not before. A real `farcoolerd`:
//! a card made before any publication is a plain card; publishing brings it under the engine; a card made
//! afterwards is tracked when it is made; a card made while the daemon was stopped is tracked when it
//! starts again; and Start (what `agent dispatch` sends) takes a tracked card to a run.
//!
//! Every daemon here carries the integration tests' stub-agents switch, as it must, and that switch has no
//! say in tracking: removing the call in `main.rs` or in `task.create` fails a step below.

use farcooler_protocol::{capability, v1::{self as pb, request, result}, workflow_commands::{Action, Mutation}};
use farcooler_workflow::Definition;

mod common;
#[path = "support/gh_pr.rs"]
mod gh_pr;
use gh_pr::*;

fn mutation(id: &str, revision: u64, sequence: u64) -> Mutation { Mutation { id: id.into(), revision, epoch: 1, sequence } }

async fn workflow(client: &mut SocketClient, workspace: &pb::Workspace, action: Action) -> serde_json::Value {
    let mut req = farcooler_transport::request(action.method());
    req.target_resource_id = Some(workspace.id.clone());
    req.required_capabilities = vec![capability::WORKFLOW_COMMANDS.into()];
    req.payload = Some(request::Payload::WorkflowCommand(pb::WorkflowCommand { schema: 1, json: serde_json::to_string(&action).unwrap() }));
    let result::Value::WorkflowReply(reply) = client.call(req).await.expect("a workflow reply").value.expect("a value") else { panic!("wrong result") };
    serde_json::from_str(&reply.json).unwrap()
}

async fn held(client: &mut SocketClient, workspace: &pb::Workspace, key: &str) -> bool {
    let state = workflow(client, workspace, Action::Get).await;
    state.pointer(&format!("/state/items/{key}")).is_some()
}

/// A hand move to Todo: refused on a card the engine holds.
async fn hand_moves(client: &mut SocketClient, card: &pb::Task) -> bool {
    let mut req = farcooler_transport::request("task.set_status");
    req.required_capabilities = vec![WORKSTREAMS_CAPABILITY.into()];
    req.payload = Some(request::Payload::TaskSetStatus(pb::TaskSetStatus {
        task_id: card.id.clone(), status: pb::TaskStatus::Todo as i32, actor: "manager".into() }));
    client.call(req).await.is_ok()
}
const WORKSTREAMS_CAPABILITY: &str = farcooler_protocol::capability::WORKSTREAMS;

#[tokio::test]
async fn a_board_tracks_its_cards_once_it_has_a_published_workflow() {
    let (dir, mut daemon, mut client, workspace) = a_runner().await;
    let home = dir.path().join("home");

    // No publication: a plain card, moved by hand as always.
    let before = a_card(&mut client, &workspace, "Before").await;
    assert!(!held(&mut client, &workspace, &before.key).await, "nothing is published, so nothing is tracked");
    assert!(hand_moves(&mut client, &before).await);

    // Publishing brings the board's open cards under the engine.
    let definition = Definition::tracker("owner");
    let publish = Action::Publish { notifications: None, adapter: None, mutation: mutation("publish", 0, 1), expected_pin: None, definition };
    let receipt = workflow(&mut client, &workspace, publish).await;
    assert!(receipt.pointer("/receipt/result/Ok").is_some(), "{receipt}");
    assert!(held(&mut client, &workspace, &before.key).await, "publishing tracks the cards already there");
    assert!(!hand_moves(&mut client, &before).await, "a tracked card is not moved by hand");

    // A card made afterwards is tracked when it is made.
    let after = a_card(&mut client, &workspace, "After").await;
    assert!(held(&mut client, &workspace, &after.key).await, "task.create tracks the new card");
    assert!(!hand_moves(&mut client, &after).await);

    // A card made while the daemon was stopped is tracked when it starts again.
    drop(client);
    daemon.child.kill().await.unwrap();
    let stopped = {
        let service = farcooler_daemon::service::Service::open_in(home.clone()).await.expect("the stopped daemon's home opens");
        let card = service.store.create_task(uuid::Uuid::from_slice(&workspace.id).unwrap(), "While stopped", farcooler_store::models::Actor::User).unwrap();
        card.key
    };
    let _daemon = common::listening_daemon_with_env(&home, &[]).await;
    let mut client = SocketClient::connect(&home.join("farcoolerd.sock"), "test-client", "0.0.0").await.expect("connect");
    assert!(held(&mut client, &workspace, &stopped).await, "starting the daemon tracks the open cards");

    // Start, which is what `agent dispatch` sends for a tracked card, takes it to a run.
    let state = workflow(&mut client, &workspace, Action::Get).await;
    let item = &state["state"]["items"][&after.key];
    let started = Action::Start {
        mutation: mutation("start", state["state"]["revision"].as_u64().unwrap(), 2), item: after.key.clone(),
        run: item["run"].as_str().unwrap().into(), pin: item["pin"].as_str().unwrap().into(),
        start_pin: state["configuration"]["pin"].as_str().unwrap().into(), models: Default::default(), worktree: None,
    };
    let receipt = workflow(&mut client, &workspace, started).await;
    assert!(receipt.pointer("/receipt/result/Ok").is_some(), "{receipt}");
    let state = workflow(&mut client, &workspace, Action::Get).await;
    assert_eq!(state["state"]["items"][&after.key]["definition"]["source"], "tracker/v1", "the card now has a run");
}
