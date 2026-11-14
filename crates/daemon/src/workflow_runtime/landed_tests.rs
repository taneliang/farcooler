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
    ok(&runtime.wire_command("w", "owner", Action::Publish { notifications: None, adapter, mutation: m("publish", 0, 1), expected_pin: None, definition }).unwrap());
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
