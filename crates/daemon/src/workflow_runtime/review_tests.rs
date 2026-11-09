//! Ruling R-111: the fixes and the untested refusals from the ov-999..1001 security review
//! (`.claude/agent/reports/ov-999/independent-review.md`). Each test here was seen red with its check removed.
use super::*;
use super::admission_tests::{alices_pane, factory, held, mutation, ok, receipt, refused, sent, snapshot};
use super::member_tests::{a_failed_run, alices_phone, send_back};
use std::sync::{Arc, OnceLock};
use farcooler_protocol::{v1::{self as pb, Scope}, workflow_commands::Action};
use farcooler_store::member_roles::Role;
use farcooler_transport::{Handler, Peer};

fn local() -> Peer { Peer { client_id: None, scope: Scope::Control } }

/// Every value in the configuration's adapter environments, in order.
fn environment_values(reply: &pb::Response) -> Vec<String> {
    fn walk(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(fields) => for (name, field) in fields {
                match (name.as_str(), field) {
                    ("environment", serde_json::Value::Object(environment)) =>
                        out.extend(environment.values().map(|v| v.as_str().unwrap_or_default().to_string())),
                    (_, field) => walk(field, out),
                }
            },
            serde_json::Value::Array(items) => items.iter().for_each(|i| walk(i, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(receipt(reply).pointer("/configuration/adapter").expect("a configuration with an adapter"), &mut out);
    out
}

/// M1: a gate's environment values are the owner's to read. A member's `workflow.get` names each setting and
/// hides its value; the owner's shows them.
#[tokio::test]
async fn workflow_get_hides_gate_environment_values_from_all_but_the_owner() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let phone = alices_phone(&svc);
    let (_run, mut runtime) = a_failed_run(workspace).await;
    let owners = factory(&svc, &runtime, local(), None);
    let (reply, _) = sent(&mut runtime, &owners, workspace, &Action::Get).await;
    let shown = environment_values(&reply);
    assert!(!shown.is_empty() && shown.iter().all(|v| v != wire_commands::HIDDEN), "the owner reads them: {shown:?}");
    let alices = factory(&svc, &runtime, phone, None);
    let (reply, _) = sent(&mut runtime, &alices, workspace, &Action::Get).await;
    let hidden = environment_values(&reply);
    assert_eq!(hidden.len(), shown.len(), "every setting is still named");
    assert!(hidden.iter().all(|v| v == wire_commands::HIDDEN), "a member reads none: {hidden:?}");
}

/// L2: watching is a person's. Alice's own shell may ask (nothing runs, so it is not found); the orchestrator
/// she leads, an agent, is refused.
#[tokio::test]
async fn workflow_watch_is_a_persons_not_an_agents() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
    for (orchestrator, word) in [(true, "scope-denied"), (false, "not-found")] {
        let (_pane, pid) = alices_pane(&svc, repo, workspace, orchestrator).await;
        let mut watch = farcooler_transport::request("workflow.watch");
        watch.target_resource_id = Some(bytes::Bytes::copy_from_slice(workspace.as_bytes()));
        watch.payload = Some(pb::request::Payload::WorkflowCommand(pb::WorkflowCommand { schema: 1, json: r#"{"item":"ov-9"}"#.into() }));
        let reply = factory(&svc, &runtime, local(), Some(pid)).handle(watch).await;
        assert_eq!(refused(&reply), word, "orchestrator: {orchestrator}");
    }
    // L10: a caller whose process cannot be read is never taken for a person either, so a seat check that only
    // turns agents away (or none) lets it watch.
    let mut watch = farcooler_transport::request("workflow.watch");
    watch.target_resource_id = Some(bytes::Bytes::copy_from_slice(workspace.as_bytes()));
    watch.payload = Some(pb::request::Payload::WorkflowCommand(pb::WorkflowCommand { schema: 1, json: r#"{"item":"ov-9"}"#.into() }));
    let reply = factory(&svc, &runtime, local(), Some(crate::caller_seat::UNKNOWN_PID)).handle(watch).await;
    assert_eq!(refused(&reply), "scope-denied", "an unreadable caller");
}

/// L3: a receipt answered again to someone other than its sender names who sent it, never their device or pane.
/// The sender's own retry gets it whole.
#[tokio::test]
async fn workflow_a_replayed_receipt_keeps_its_senders_device_to_them() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let phone = alices_phone(&svc);
    svc.store.bind_owner_device("phone-a2", None).unwrap();
    svc.store.set_member("alice", "Alice", &["phone-a".into(), "phone-a2".into()]).unwrap();
    svc.store.set_member_role("alice", Role::Maintainer, "user").unwrap();
    let (_run, mut runtime) = a_failed_run(workspace).await;
    let action = send_back(&runtime, workspace, "b", 1);
    let rpc = factory(&svc, &runtime, phone, None);
    let (reply, _) = sent(&mut runtime, &rpc, workspace, &action).await;
    ok(&reply);
    let actor = |reply: &pb::Response| receipt(reply).pointer("/receipt/actor").cloned();
    assert_eq!(actor(&reply), Some(serde_json::json!({"person": "member:alice", "device": "phone-a"})));
    let (again, _) = sent(&mut runtime, &rpc, workspace, &action).await;
    assert_eq!(actor(&again), actor(&reply), "the sender's own retry");
    let other = factory(&svc, &runtime, Peer { client_id: Some("phone-a2".into()), scope: Scope::Control }, None);
    let (replayed, _) = sent(&mut runtime, &other, workspace, &action).await;
    assert_eq!(actor(&replayed), Some(serde_json::json!({"person": "member:alice"})), "another device learns no device");
}

/// L5: a member-led workspace's orchestrator is seated by a person. An agent pane on the lead's own login asking
/// to become it is refused; the lead, in her own shell, may seat it.
#[tokio::test]
async fn an_agent_cannot_seat_a_member_led_orchestrator() {
    use farcooler_store::models::TerminalRole;
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let (agent, agents_pid) = alices_pane(&svc, repo, workspace, true).await;
    svc.store.set_terminal_role(agent, TerminalRole::Agent).unwrap();
    let (_shell, shells_pid) = alices_pane(&svc, repo, workspace, false).await;
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
    let seat = |pid| {
        let mut req = farcooler_transport::request("terminal.set_role");
        req.target_resource_id = Some(bytes::Bytes::copy_from_slice(agent.as_bytes()));
        req.payload = Some(pb::request::Payload::TerminalSetRole(pb::TerminalSetRole { role: pb::TerminalRole::Orchestrator as i32 }));
        (factory(&svc, &runtime, local(), Some(pid)), req)
    };
    let (rpc, req) = seat(agents_pid);
    assert_eq!(refused(&rpc.handle(req).await), "scope-denied", "the agent seats itself");
    assert_eq!(svc.store.get_terminal(agent).unwrap().role, TerminalRole::Agent);
    let (rpc, req) = seat(shells_pid);
    let reply = rpc.handle(req).await;
    assert!(matches!(reply.outcome, Some(pb::response::Outcome::Result(_))), "the lead seats it: {reply:?}");
    assert_eq!(svc.store.get_terminal(agent).unwrap().role, TerminalRole::Orchestrator);
}

fn settle(runtime: &Runtime, workspace: Uuid) -> Action {
    let state = runtime.state(&workspace.to_string()).unwrap().unwrap(); let item = &state.items["ov-7"];
    Action::Settle { mutation: mutation("settle", state.revision, 1), item: "ov-7".into(), run: item.run.clone(),
        pin: item.pin.clone(), landed: true }
}

/// Untested refusal 1: settling a push is the owner's. A Maintainer who leads the workspace, in her own shell,
/// is refused at dispatch, and nothing reaches the engine.
#[tokio::test]
async fn workflow_a_member_never_settles_a_push() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let (_pane, pid) = alices_pane(&svc, repo, workspace, false).await;
    svc.store.set_member_role("alice", Role::Maintainer, "user").unwrap();
    let (_run, mut runtime) = a_failed_run(workspace).await;
    let rpc = factory(&svc, &runtime, local(), Some(pid));
    let before = snapshot(&runtime, workspace);
    let action = settle(&runtime, workspace);
    let (reply, reached) = sent(&mut runtime, &rpc, workspace, &action).await;
    assert_eq!(refused(&reply), "scope-denied");
    assert!(!reached);
    assert_eq!(snapshot(&runtime, workspace), before);
}

/// Untested refusal 3: what stays the owner's (setting a workflow, settling) is refused a member's device at the
/// people gate, a Maintainer's included, before dispatch is asked. (A cutover needs a scope no Control device has,
/// so scope refuses it before the gate; `rpc.rs` asks for the owner as well.)
#[tokio::test]
async fn workflow_owners_methods_are_refused_a_members_device_at_the_gate() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let phone = alices_phone(&svc);
    svc.store.set_member_role("alice", Role::Maintainer, "user").unwrap();
    let (_run, mut runtime) = a_failed_run(workspace).await;
    let rpc = factory(&svc, &runtime, phone, None);
    let publish = Action::Publish { notifications: None, adapter: None, mutation: mutation("p", 0, 1), expected_pin: None,
        definition: coding_definition(false) };
    for action in [publish, settle(&runtime, workspace)] {
        let before = snapshot(&runtime, workspace);
        let (reply, reached) = sent(&mut runtime, &rpc, workspace, &action).await;
        assert_eq!(refused(&reply), "not-yours", "{}", action.method());
        assert!(!reached);
        assert_eq!(snapshot(&runtime, workspace), before);
    }
}

/// Untested refusal 2, and 1 again, at the layer the engine asks when it takes a command (`Admission::permits`),
/// which dispatch otherwise masks: a member never settles, and publishes only as a Maintainer (through a draft).
#[tokio::test]
async fn admission_permits_no_member_settle_and_only_a_maintainers_publish() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let (_pane, pid) = alices_pane(&svc, repo, workspace, false).await;
    let (_run, runtime) = a_failed_run(workspace).await;
    let admission = Admission::capture(svc.clone(), local(), svc.sessions().open(None), Some(pid), Arc::new(OnceLock::new()), workspace)
        .await.expect("Alice, in her own shell, where she leads");
    let publish = Action::Publish { notifications: None, adapter: None, mutation: mutation("p", 0, 1), expected_pin: None,
        definition: coding_definition(false) };
    let not_yours = |r: farcooler_core::Result<()>| matches!(r, Err(farcooler_core::DomainError::NotYours { .. }));
    assert!(not_yours(admission.permits(&settle(&runtime, workspace))), "a lead settles");
    assert!(not_yours(admission.permits(&publish)), "a lead who isn't a Maintainer publishes");
    svc.store.set_member_role("alice", Role::Maintainer, "user").unwrap();
    assert!(not_yours(admission.permits(&settle(&runtime, workspace))), "a Maintainer settles");
    assert!(admission.permits(&publish).is_ok(), "a Maintainer publishes a draft she approved");
}

/// Untested refusal 4: the person behind a command changing while it waits. A pane's login is written once
/// (`set_terminal_login` refuses a second), so this is a device given to someone else: Alice's phone, now Bob's.
/// Bob may do all Alice may (both are Maintainers), so only the person having changed refuses it.
#[tokio::test]
async fn workflow_a_command_from_a_device_given_to_someone_else_while_held_is_refused() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let phone = alices_phone(&svc);
    svc.store.set_member_role("alice", Role::Maintainer, "user").unwrap();
    svc.store.set_member("bob", "Bob", &[]).unwrap();
    svc.store.set_member_role("bob", Role::Maintainer, "user").unwrap();
    let (_run, mut runtime) = a_failed_run(workspace).await;
    let rpc = factory(&svc, &runtime, phone, None);
    let action = send_back(&runtime, workspace, "b", 1);
    held(&mut runtime, &rpc, workspace, action, true, || {
        svc.store.set_member("alice", "Alice", &[]).unwrap();
        svc.store.set_member("bob", "Bob", &["phone-a".into()]).unwrap();
    }).await;
}

/// Untested refusal 5: the orchestrator a member leads, its command held, when the lead moves to someone else:
/// refused, nothing written. (Its login can't move: a pane's login is written once.)
#[tokio::test]
async fn workflow_a_member_led_orchestrators_command_held_is_refused_when_its_lead_changes() {
    {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
        let (_pane, pid) = alices_pane(&svc, repo, workspace, true).await;
        svc.store.set_member("bob", "Bob", &[]).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
        let definition = coding_definition(false); let pin = definition.pin();
        runtime.wire_command(&workspace.to_string(), "owner", Action::Publish { notifications: None, adapter: None,
            mutation: mutation("p", 0, 1), expected_pin: None, definition }).unwrap();
        let rpc = factory(&svc, &runtime, local(), Some(pid));
        held(&mut runtime, &rpc, workspace, Action::Enqueue { mutation: mutation("e", 0, 1), item: "a".into(), pin }, true, || {
            svc.store.set_workspace_lead(workspace, Some("bob")).unwrap();
        }).await;
    }
}

/// Untested refusal 6: a person removed while their command waits, from their own shell or through the
/// orchestrator they led: refused, nothing written.
#[tokio::test]
async fn workflow_a_removed_persons_command_held_is_refused() {
    for orchestrator in [false, true] {
        let (_dir, svc, repo) = crate::test_support::fixture().await;
        let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
        let (_pane, pid) = alices_pane(&svc, repo, workspace, orchestrator).await;
        svc.store.set_member_role("alice", Role::Maintainer, "user").unwrap();
        let (_run, mut runtime) = a_failed_run(workspace).await;
        let rpc = factory(&svc, &runtime, local(), Some(pid));
        let action = send_back(&runtime, workspace, "b", 1);
        eprintln!("orchestrator: {orchestrator}");
        held(&mut runtime, &rpc, workspace, action, true, || { svc.store.remove_member_with_enrolled("alice").unwrap(); }).await;
    }
}

/// Untested refusal 7: a Maintainer acts on any workspace in person, but the orchestrator pane on her login, in a
/// workspace Bob leads, is not that workspace's orchestrator: refused, and nothing reaches the engine.
#[tokio::test]
async fn workflow_a_maintainers_orchestrator_pane_in_someone_elses_workspace_is_refused() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let (_pane, pid) = alices_pane(&svc, repo, workspace, true).await;
    svc.store.set_member_role("alice", Role::Maintainer, "user").unwrap();
    svc.store.set_member("bob", "Bob", &[]).unwrap();
    svc.store.set_workspace_lead(workspace, Some("bob")).unwrap();
    let (_run, mut runtime) = a_failed_run(workspace).await;
    let rpc = factory(&svc, &runtime, local(), Some(pid));
    let before = snapshot(&runtime, workspace);
    let action = send_back(&runtime, workspace, "b", 1);
    let (reply, reached) = sent(&mut runtime, &rpc, workspace, &action).await;
    let word = refused(&reply);
    assert!(!reached, "refused before the engine ({word})");
    assert_eq!(snapshot(&runtime, workspace), before);
}

