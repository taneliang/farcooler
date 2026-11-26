//! Dispatch snapshots through the real daemon summary/full negotiation (ov-1159).
use super::*;
use farcooler_daemon::{rpc::RpcFactory, service::Service, watch::Watcher};
use farcooler_protocol::v1::{response, Scope};
use farcooler_transport::{Handler, Peer};
use std::sync::Arc;

struct RpcLink(RpcFactory);
impl DispatchLink for RpcLink {
    fn capabilities(&self) -> Vec<String> { vec![capability::WORKFLOW_COMMANDS.into()] }
    async fn call(&mut self, req: pb::Request) -> Result<pb::Result, ClientError> {
        match self.0.handle(req).await.outcome {
            Some(response::Outcome::Result(reply)) => Ok(reply),
            other => panic!("RPC refused fixture request: {other:?}"),
        }
    }
    async fn pause(&mut self, _: std::time::Duration) { }
}

#[tokio::test]
async fn snapshot_preserves_ticket_hold_and_models_through_rpc() {
    let dir = tempfile::tempdir().unwrap();
    let service = Arc::new(Service::open_in(dir.path().join("home")).await.unwrap());
    let root = service.store.create_repository_root(service.host_id, "/fixture", 0).unwrap();
    let repo = service.store.create_repository(service.host_id, root.id, "fixture", "/fixture/.git", "").unwrap();
    let workspace = service.store.ensure_main_workspace(repo.id).unwrap().id;
    let mut definition = farcooler_daemon::workflow_runtime::coding_definition(false);
    definition.models.insert("stage-1".into(), vec!["claude:opus".into(), "codex:gpt-5.5".into()]);
    // Seed the publication as the board fixtures do: tracking never runs its coding adapter.
    let publication = service.store.workflow_wire_write(&workspace.to_string(), "publish", "publish",
        farcooler_store::workflow_publication::Write::Publish {
            expected_pin: None, definition: &definition, adapter: None, notifications: None, revision: 0,
        }).unwrap();
    assert!(publication.result.is_ok(), "{publication:?}");
    let ticket = service.store.create_task(workspace, "waiting ticket", farcooler_store::models::Actor::User).unwrap();
    let key = &ticket.key;
    service.start_workflows().unwrap();
    service.track_tickets(Some(ticket.id)).await;
    let mut link = RpcLink(RpcFactory::new(service.clone(), Watcher::new(service.clone()),
        Arc::new(tokio::sync::Notify::new()), Peer { client_id: None, scope: Scope::Control }));
    // The public default must still omit definitions: this exercises the projection, not a canned reply.
    let summary = link.call(crate::workflow::request(workspace, &Action::Get).unwrap()).await.unwrap();
    let Some(result::Value::WorkflowReply(reply)) = summary.value else { panic!("workflow reply") };
    let summary: Value = serde_json::from_str(&reply.json).unwrap();
    assert!(summary.pointer("/configuration/definition").is_none());
    assert!(summary["state"]["items"][key].get("definition").is_none());
    let Seen::Held(seen) = snapshot(&mut link, workspace).await else { panic!("held snapshot") };

    // Run both CLI projections through the same reader used by workflow get.
    let mut state = service.store.workflow_state(&workspace.to_string()).unwrap().unwrap();
    state.source_sequences.extend((0u64..30_000)
        .map(|n| (format!("wire:member:former-member-{n:018}"), n + 1)));
    assert!(serde_json::to_vec(&state.source_sequences).unwrap().len()
        > farcooler_protocol::MAX_CONTROL_ENVELOPE_BYTES);
    service.store.workflow_state_for_tests(&state).unwrap();
    for full in [false, true] {
        let reply = crate::workflow::read_snapshot(&mut link, workspace, full).await.unwrap();
        let wire = &reply["state"];
        assert_eq!(wire["source_sequences"], json!(state.source_sequences));
        assert_eq!(wire["revision"], state.revision);
        assert_eq!(wire["epoch"], state.epoch);
        for field in ["run", "pin", "phase"] {
            assert_eq!(wire["items"][key][field], summary["state"]["items"][key][field]);
        }
        assert_eq!(wire["items"][key].get("definition").is_some(), full);
    }

    service.stop_workflows().await.unwrap();
    assert_eq!(summary["state"]["items"][key]["phase"], "WaitingApproval");
    assert_eq!(seen.hold(&key.to_uppercase()), Hold::AtRest {
        run: summary["state"]["items"][key]["run"].as_str().unwrap().into(),
        pin: summary["state"]["items"][key]["pin"].as_str().unwrap().into(),
    });
    assert_eq!(seen.models, vec![("stage-1".into(), vec!["claude:opus".into(), "codex:gpt-5.5".into()])]);
    assert_eq!(picks(&seen, Some("codex:gpt-5.5")).unwrap(),
        BTreeMap::from([("stage-1".into(), "codex:gpt-5.5".into())]));
}
