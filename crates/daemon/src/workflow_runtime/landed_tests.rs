//! ov-1231: the orchestrator closes a Failed run as landed with a commit the remote already holds. The remote is
//! read first; a commit it does not hold lands nothing, and no other phase asks the remote at all.
use super::*;
use super::coding_verbs_tests::lander;
use super::tests::finish;
use farcooler_protocol::{workflow_commands::{Action, Mutation}, workflow_config::AdapterConfiguration};

fn m(id: &str, revision: u64, sequence: u64) -> Mutation { Mutation { id: id.into(), revision, epoch: 1, sequence } }
fn ok(value: &serde_json::Value) { assert!(value.pointer("/receipt/result/Ok").is_some(), "{value}"); }

#[tokio::test]
async fn a_failed_run_closes_as_landed_only_with_a_commit_on_the_target() {
    let (_dir, mut runtime) = lander("check-fail").await;
    let published = runtime.coding.clone().unwrap();
    let on_target = published.base.clone();
    let definition = published.definition().unwrap();
    let adapter = Some(Box::new(AdapterConfiguration::Coding(published)));
    let pin = farcooler_store::workflow_publication::configuration_pin(&definition, adapter.as_deref());
    ok(&runtime.wire_command("w", "owner", Action::Publish { reason: None, notifications: None, adapter, mutation: m("publish", 0, 1), expected_pin: None, definition }).unwrap());
    assert_eq!(runtime.track_tickets(&[("w".into(), "ov-7".into(), "todo".into())]).unwrap(), 1);
    let state = runtime.state("w").unwrap().unwrap(); let item = &state.items["ov-7"];
    // Not Failed yet: the remote is not read, and the reducer is left to refuse.
    assert_eq!(runtime.landed_check("w", "ov-7", &item.run, &item.pin, &on_target).await.unwrap(), None);
    ok(&runtime.wire_command("w", "orchestrator", Action::Start { mutation: m("start", state.revision, 2), item: "ov-7".into(),
        run: item.run.clone(), pin: item.pin.clone(), start_pin: pin, models: Default::default(), worktree: None, with: vec![] }).unwrap());
    let state = finish(&mut runtime, "w").await;
    let item = &state.items["ov-7"];
    assert_eq!(item.phase, Phase::Failed, "the check failed");
    let built = item.last_evidence.clone().unwrap().candidate;
    assert_ne!(built, on_target);
    // The run's own commit was never landed: refused, and the run stays Failed.
    let why = runtime.landed_check("w", "ov-7", &item.run, &item.pin, &built).await.unwrap();
    assert_eq!(why.as_deref(), Some("The commit is not on the target branch on the remote."));
    assert_eq!(runtime.state("w").unwrap().unwrap().items["ov-7"].phase, Phase::Failed);
    // A commit on the target is let through, and the run closes as landed with it.
    assert_eq!(runtime.landed_check("w", "ov-7", &item.run, &item.pin, &on_target).await.unwrap(), None);
    let land = Action::Landed { mutation: m("landed", state.revision, 3), item: "ov-7".into(), run: item.run.clone(), pin: item.pin.clone(), sha: on_target.clone() };
    ok(&runtime.wire_command("w", "orchestrator", land).unwrap());
    let item = &runtime.state("w").unwrap().unwrap().items["ov-7"];
    assert_eq!((item.phase.clone(), item.candidate.as_deref()), (Phase::Done, Some(on_target.as_str())));
}

/// A caller who loses their role while the remote is being read closes nothing: the final authority recheck runs
/// after that read, so the run stays Failed and the board and engine are untouched.
#[tokio::test]
async fn a_landed_whose_caller_loses_authority_during_the_remote_read_closes_nothing() {
    use farcooler_transport::Handler;
    use super::admission_tests::{factory, mutation, refused, request, snapshot};
    use super::member_tests::alices_phone;
    use farcooler_store::member_roles::Role;
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let phone = alices_phone(&svc);
    svc.store.set_member_role("alice", Role::Maintainer, "user").unwrap();
    let w = workspace.to_string();
    let (_dir, mut runtime) = lander("check-fail").await;
    let published = runtime.coding.clone().unwrap();
    let definition = published.definition().unwrap();
    let adapter = Some(Box::new(AdapterConfiguration::Coding(published)));
    let pin = farcooler_store::workflow_publication::configuration_pin(&definition, adapter.as_deref());
    ok(&runtime.wire_command(&w, "owner", Action::Publish { reason: None, notifications: None, adapter, mutation: m("publish", 0, 1), expected_pin: None, definition }).unwrap());
    assert_eq!(runtime.track_tickets(&[(w.clone(), "ov-7".into(), "todo".into())]).unwrap(), 1);
    let state = runtime.state(&w).unwrap().unwrap(); let item = &state.items["ov-7"];
    ok(&runtime.wire_command(&w, "orchestrator", Action::Start { mutation: m("start", state.revision, 2), item: "ov-7".into(),
        run: item.run.clone(), pin: item.pin.clone(), start_pin: pin, models: Default::default(), worktree: None, with: vec![] }).unwrap());
    assert_eq!(finish(&mut runtime, &w).await.items["ov-7"].phase, Phase::Failed);
    let state = runtime.state(&workspace.to_string()).unwrap().unwrap(); let item = &state.items["ov-7"];
    let sha = runtime.coding.clone().unwrap().base;
    let action = Action::Landed { mutation: mutation("landed", state.revision, 3), item: "ov-7".into(), run: item.run.clone(), pin: item.pin.clone(), sha };
    let rpc = factory(&svc, &runtime, phone, None);
    let pending = rpc.handle(request(workspace, &action));
    tokio::pin!(pending);
    let message = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::select! { message = runtime.inbox.recv() => message.unwrap(), response = &mut pending => panic!("never reached the inbox: {response:?}") }
    }).await.unwrap();
    let before = snapshot(&runtime, workspace);
    let store = svc.store.clone();
    super::coding_settle::AFTER_LANDED_READ.lock().unwrap().insert(workspace.to_string(), Box::new(move || { store.remove_member("alice").unwrap(); }));
    runtime.receive(message).await;
    let response = pending.await;
    assert!(super::coding_settle::AFTER_LANDED_READ.lock().unwrap().is_empty(), "the remote was read, and the hook ran: {response:?}");
    assert!(!refused(&response).is_empty(), "{response:?}");
    assert_eq!(snapshot(&runtime, workspace), before, "nothing was written");
    assert_eq!(runtime.state(&workspace.to_string()).unwrap().unwrap().items["ov-7"].phase, Phase::Failed);
}
