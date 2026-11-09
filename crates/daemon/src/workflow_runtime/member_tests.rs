//! ov-1000, ruling R-107: members see engine runs; a workspace's lead acts on its runs, and a Maintainer on any
//! workspace's. Through `workflow.*` dispatch, from a member's own device, with the people gate and admission
//! both in the way.
use super::*;
use super::admission_tests::{factory, held, mutation, ok, receipt, refused, sent, snapshot};
use super::coding_tests::configured;
use std::sync::Arc;
use crate::service::Service;
use farcooler_protocol::{v1::Scope, workflow_commands::Action, workflow_config::AdapterConfiguration};
use farcooler_store::member_roles::Role;
use farcooler_transport::Peer;

/// Alice, a member, on her phone at Control scope.
fn alices_phone(svc: &Arc<Service>) -> Peer {
    svc.store.bind_owner_device("phone-a", None).unwrap();
    svc.store.set_member("alice", "Alice", &["phone-a".into()]).unwrap();
    Peer { client_id: Some("phone-a".into()), scope: Scope::Control }
}

/// The coding workflow published on `workspace`, `ov-7` started under it as the owner's orchestrator would, and
/// its check failed: a run a Maintainer could send back or cancel.
async fn a_failed_run(workspace: Uuid) -> (tempfile::TempDir, Runtime) {
    let w = workspace.to_string();
    let (dir, mut runtime) = configured(false, "check-fail").await;
    let published = runtime.coding.clone().unwrap();
    let definition = published.definition().unwrap();
    let adapter = Some(Box::new(AdapterConfiguration::Coding(published)));
    let start_pin = farcooler_store::workflow_publication::configuration_pin(&definition, adapter.as_deref());
    runtime.wire_command(&w, "owner", Action::Publish { notifications: None, adapter, mutation: mutation("p", 0, 1),
        expected_pin: None, definition }).unwrap();
    assert_eq!(runtime.track_tickets(&[(w.clone(), "ov-7".into(), "todo".into())]).unwrap(), 1);
    let state = runtime.state(&w).unwrap().unwrap(); let item = &state.items["ov-7"];
    runtime.wire_command(&w, "orchestrator", Action::Start { mutation: mutation("s", state.revision, 2), item: "ov-7".into(),
        run: item.run.clone(), pin: item.pin.clone(), start_pin, models: Default::default(), worktree: None, with: vec![] }).unwrap();
    assert_eq!(super::tests::finish(&mut runtime, &w).await.items["ov-7"].phase, Phase::Failed);
    (dir, runtime)
}
fn send_back(runtime: &Runtime, workspace: Uuid, id: &str, sequence: u64) -> Action {
    let state = runtime.state(&workspace.to_string()).unwrap().unwrap(); let item = &state.items["ov-7"];
    Action::SendBack { mutation: mutation(id, state.revision, sequence), item: "ov-7".into(), run: item.run.clone(),
        pin: item.pin.clone(), reason: "result.txt must say what changed".into() }
}
fn cancel(runtime: &Runtime, workspace: Uuid, id: &str, sequence: u64) -> Action {
    let state = runtime.state(&workspace.to_string()).unwrap().unwrap(); let item = &state.items["ov-7"];
    Action::Cancel { mutation: mutation(id, state.revision, sequence), item: "ov-7".into(), run: item.run.clone(), pin: item.pin.clone() }
}

/// A member reads a workspace's runs from their phone, as themself: no controller, and their own principal.
/// Driving the engine is still not theirs.
#[tokio::test]
async fn workflow_a_member_reads_runs_and_drives_nothing() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let phone = alices_phone(&svc);
    let (_run, mut runtime) = a_failed_run(workspace).await;
    let rpc = factory(&svc, &runtime, phone, None);
    let (reply, _) = sent(&mut runtime, &rpc, workspace, &Action::Get).await;
    let got = receipt(&reply);
    assert_eq!(got.pointer("/state/items/ov-7/phase").and_then(|p| p.as_str()), Some("Failed"), "{got}");
    assert_eq!(got.pointer("/context/capabilities/decision_principal").and_then(|p| p.as_str()), Some("member:alice"));
    assert_eq!(got.pointer("/context/capabilities/controller").and_then(|p| p.as_bool()), Some(false));
    let definition = coding_definition(false); let pin = definition.pin();
    for action in [Action::Enqueue { mutation: mutation("e", 0, 1), item: "a".into(), pin },
        Action::Retry { mutation: mutation("r", 0, 1), item: "ov-7".into(), run: "r".into(), pin: "p".into() }]
    {
        let before = snapshot(&runtime, workspace);
        let (reply, reached) = sent(&mut runtime, &rpc, workspace, &action).await;
        assert_eq!(refused(&reply), "not-yours", "{}", action.method());
        assert!(!reached);
        assert_eq!(snapshot(&runtime, workspace), before);
    }
}

/// A plain member may not send a run back or cancel it, and nothing reaches the engine; made a Maintainer, they
/// may, as `member:alice`, on a workspace they don't lead.
#[tokio::test]
async fn workflow_a_maintainer_cancels_and_sends_back_and_a_member_may_not() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let phone = alices_phone(&svc);
    let (_run, mut runtime) = a_failed_run(workspace).await;
    let rpc = factory(&svc, &runtime, phone, None);
    for action in [send_back(&runtime, workspace, "b", 1), cancel(&runtime, workspace, "c", 1)] {
        let before = snapshot(&runtime, workspace);
        let (reply, reached) = sent(&mut runtime, &rpc, workspace, &action).await;
        assert_eq!(refused(&reply), "not-yours", "a plain member's {}", action.method());
        assert!(!reached);
        assert_eq!(snapshot(&runtime, workspace), before);
    }
    svc.store.set_member_role("alice", Role::Maintainer, "user").unwrap();
    let action = send_back(&runtime, workspace, "b", 1);
    let (reply, _) = sent(&mut runtime, &rpc, workspace, &action).await;
    ok(&reply);
    assert_ne!(runtime.state(&workspace.to_string()).unwrap().unwrap().items["ov-7"].phase, Phase::Failed, "sent back to build");
    let action = cancel(&runtime, workspace, "c", 2);
    let (reply, _) = sent(&mut runtime, &rpc, workspace, &action).await;
    ok(&reply);
    let events = runtime.events(&workspace.to_string()).unwrap();
    assert!(events.iter().any(|e| e.source == "wire:member:alice"), "the engine has it as Alice's, not the owner's");
}

/// A Maintainer's cancel held in the engine's inbox while the owner takes the role away is refused when used,
/// and writes nothing; one whose role stands goes through.
#[tokio::test]
async fn workflow_a_maintainers_cancel_is_refused_when_the_role_goes_while_it_waits() {
    for demote in [true, false] {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
        let phone = alices_phone(&svc);
        svc.store.set_member_role("alice", Role::Maintainer, "user").unwrap();
        let (_run, mut runtime) = a_failed_run(workspace).await;
        let rpc = factory(&svc, &runtime, phone, None);
        let action = cancel(&runtime, workspace, "c", 1);
        held(&mut runtime, &rpc, workspace, action, demote, || if demote {
            svc.store.set_member_role("alice", Role::Member, "user").unwrap();
        }).await;
    }
}

/// Ruling R-107: a workspace's lead acts on its runs from their own phone, and on no other workspace's.
#[tokio::test]
async fn workflow_a_lead_acts_on_their_own_workspace_only() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let other = svc.store.create_workspace(repo, "Other", "oth").unwrap().id;
    let phone = alices_phone(&svc);
    svc.store.set_workspace_lead(workspace, Some("alice")).unwrap();
    let (_run, mut runtime) = a_failed_run(workspace).await;
    let rpc = factory(&svc, &runtime, phone, None);
    let (reply, _) = sent(&mut runtime, &rpc, workspace, &Action::Get).await;
    assert_eq!(receipt(&reply).pointer("/context/capabilities/controller").and_then(|p| p.as_bool()), Some(true), "the lead acts");
    let action = send_back(&runtime, workspace, "b", 1);
    let (reply, _) = sent(&mut runtime, &rpc, workspace, &action).await;
    ok(&reply);
    let action = cancel(&runtime, workspace, "c", 2);
    let (reply, _) = sent(&mut runtime, &rpc, workspace, &action).await;
    ok(&reply);
    let pin = coding_definition(false).pin();
    let before = snapshot(&runtime, other);
    let (reply, reached) = sent(&mut runtime, &rpc, other, &Action::Enqueue { mutation: mutation("e", 0, 1), item: "a".into(), pin }).await;
    assert_eq!(refused(&reply), "not-yours", "not her workspace");
    assert!(!reached);
    assert_eq!(snapshot(&runtime, other), before);
}

/// A lead's cancel held in the engine's inbox while the lead passes to someone else is refused when used.
#[tokio::test]
async fn workflow_a_leads_cancel_is_refused_when_the_lead_moves_while_it_waits() {
    for moved in [true, false] {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
        let phone = alices_phone(&svc);
        svc.store.set_member("bob", "Bob", &[]).unwrap();
        svc.store.set_workspace_lead(workspace, Some("alice")).unwrap();
        let (_run, mut runtime) = a_failed_run(workspace).await;
        let rpc = factory(&svc, &runtime, phone, None);
        let action = cancel(&runtime, workspace, "c", 1);
        held(&mut runtime, &rpc, workspace, action, moved, || if moved {
            svc.store.set_workspace_lead(workspace, Some("bob")).unwrap();
        }).await;
    }
}

/// R-107: setting a workflow outright stays the owner's. A Maintainer who leads the workspace, in her own shell,
/// is refused `workspace.set_workflow` at dispatch and nothing is published; she approves a draft instead.
#[tokio::test]
async fn workflow_a_maintainer_does_not_set_a_workflow_outright() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let (_pane, pid) = super::admission_tests::alices_pane(&svc, repo, workspace, false).await;
    svc.store.set_member_role("alice", Role::Maintainer, "user").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
    let rpc = factory(&svc, &runtime, Peer { client_id: None, scope: Scope::Control }, Some(pid));
    let before = snapshot(&runtime, workspace);
    let publish = Action::Publish { notifications: None, adapter: None, mutation: mutation("p", 0, 1), expected_pin: None,
        definition: coding_definition(false) };
    let (reply, reached) = sent(&mut runtime, &rpc, workspace, &publish).await;
    assert_eq!(refused(&reply), "scope-denied");
    assert!(!reached);
    assert_eq!(snapshot(&runtime, workspace), before);
}
