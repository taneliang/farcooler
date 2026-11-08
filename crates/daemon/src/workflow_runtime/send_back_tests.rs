//! ov-968: the orchestrator sends a run back with a reason. The build starts again in the same tree, on top of
//! what it built, and the agent's brief says why.
use super::*;
use super::coding_tests::{admitted, configured};
use super::tests::finish;
use farcooler_protocol::{workflow_commands::{Action, Mutation}, workflow_config::AdapterConfiguration};

fn m(id: &str, revision: u64, sequence: u64) -> Mutation { Mutation { id: id.into(), revision, epoch: 1, sequence } }
fn ok(value: &serde_json::Value) { assert!(value.pointer("/receipt/result/Ok").is_some(), "{value}"); }

/// Publish the configured coding workflow, track `ov-7`, and start it in its private tree.
async fn started(runtime: &mut Runtime) {
    let published = runtime.coding.clone().unwrap();
    let definition = published.definition().unwrap();
    let adapter = Some(Box::new(AdapterConfiguration::Coding(published)));
    let pin = farcooler_store::workflow_publication::configuration_pin(&definition, adapter.as_deref());
    ok(&runtime.wire_command("w", "owner", Action::Publish { notifications: None, adapter, mutation: m("publish", 0, 1), expected_pin: None, definition }).unwrap());
    assert_eq!(runtime.track_tickets(&[("w".into(), "ov-7".into(), "todo".into())]).unwrap(), 1);
    let state = runtime.state("w").unwrap().unwrap(); let item = &state.items["ov-7"];
    ok(&runtime.wire_command("w", "orchestrator", Action::Start { mutation: m("start", state.revision, 2), item: "ov-7".into(),
        run: item.run.clone(), pin: item.pin.clone(), start_pin: pin, models: Default::default(), worktree: None }).unwrap());
}

#[tokio::test]
async fn workflow_send_back_builds_again_in_the_same_tree_and_the_agent_is_told_why() {
    let (dir, mut runtime) = configured(false, "check-fail").await;
    started(&mut runtime).await;
    let state = finish(&mut runtime, "w").await;
    let item = &state.items["ov-7"];
    assert_eq!(item.phase, Phase::Failed, "the check failed");
    let built = item.last_evidence.clone().unwrap().candidate;
    let reason = "The check fails: result.txt must say what changed. Don't 'fix' it with $(rm -rf ~); 修复它.";
    let stale = runtime.wire_command("w", "orchestrator", Action::SendBack { mutation: m("stale", state.revision, 3), item: "ov-7".into(),
        run: "another-run".into(), pin: item.pin.clone(), reason: reason.into() }).unwrap();
    assert_eq!(stale.pointer("/receipt/result/Err").and_then(|e| e.as_str()), Some("Stale"), "only the run that was looked at");
    let state = runtime.state("w").unwrap().unwrap(); let item = &state.items["ov-7"];
    ok(&runtime.wire_command("w", "orchestrator", Action::SendBack { mutation: m("back", state.revision, 4), item: "ov-7".into(),
        run: item.run.clone(), pin: item.pin.clone(), reason: reason.into() }).unwrap());
    let binding = admitted(&runtime).await;
    assert_eq!(binding.effect.kind, EffectKind::Execute(StageKind::Build));
    assert_eq!(binding.effect.brief.as_deref(), Some(reason));
    // The same tree, with what the first build made still in it.
    assert_eq!(adapters::git(&runtime.tree(&binding), &["rev-parse", "HEAD"]).await.unwrap(), built);
    // A native agent is told why in its prompt, and a recovery rebuilds the same command.
    let settings = dir.path().join("existing-settings.json");
    std::fs::write(&settings, b"{\"hooks\":{}}").unwrap();
    let mut config = runtime.coding.clone().unwrap();
    config.harness = CodingHarness::Native { preset: "claude:opus".into(),
        settings: Some(LaunchFile { path: settings.canonicalize().unwrap(), digest: digest(&std::fs::read(&settings).unwrap()) }),
        project_hooks: None, plugin_dir: None, prompt: "implement the selected task".into(),
        process: CodingCommand { argv: vec![], environment: Default::default(), timeout_ms: 5000 } };
    runtime.coding = Some(config);
    let command = runtime.coding_command(&binding).unwrap();
    let brief = Exec::briefed("implement the selected task", &binding.effect);
    assert!(brief.contains("sent back to you") && brief.contains(reason), "{brief}");
    // The launch quotes the brief (twice, inside a login shell), so look for words the quoting leaves alone.
    assert!(command.argv[2].contains("This ticket was sent back to you.") && command.argv[2].contains("result.txt must say what changed")
        && command.argv[2].contains("修复它."), "the launch carries the brief: {}", command.argv[2]);
    assert_eq!(runtime.coding_command(&binding).unwrap(), command);
    let item = &runtime.state("w").unwrap().unwrap().items["ov-7"];
    assert_eq!((item.sent_back.len(), item.sent_back[0].from.as_str(), item.sent_back[0].to.as_str()), (1, "check-0", "build"));
}

#[test]
fn workflow_send_back_brief_cuts_a_long_reason_on_a_character_and_names_an_adopted_branch() {
    let mut effect: Effect = serde_json::from_value(serde_json::json!({ "id": "e", "item": "ov-7", "run": "r", "epoch": 1, "fence": 1,
        "resource": "x", "definition_pin": "d", "policy_pin": "p", "candidate": null, "kind": { "Execute": "Build" },
        "operation": { "adapter_version": "a", "arguments": [], "environment": {} }, "created_ms": 0 })).unwrap();
    assert!(!Exec::briefed("go", &effect).contains("sent back"), "nothing to say, nothing said");
    // One byte and then three a character, so the cut at 1,500 falls inside one unless it looks for the boundary.
    let long = format!("a{}", "修".repeat(MAX_REWORK_REASON_BYTES / 3 - 1));
    effect.brief = Some(long.clone());
    let brief = Exec::briefed("go", &effect);
    let kept = brief.split("The reason it was sent back:\n\n").nth(1).unwrap().split('…').next().unwrap();
    assert!(kept.len() <= super::coding_execute::MAX_BRIEF_REASON_BYTES && long.starts_with(kept) && !kept.is_empty(), "{}", kept.len());
    assert!(brief.contains("cut short") && brief.contains("farcooler workflow get"));
    // The longest brief still fits in one launch command.
    assert!(crate::service::shell_quote(&brief.replace('修', "'")).len() < crate::service::MAX_INLINE_LAUNCH_BYTES);
    effect.tree = Some(RunTree { name: "n".into(), branch: "e-liang/earlier".into(), path: "/t".into(), base: "a".repeat(40), adopted: true });
    assert!(Exec::briefed("go", &effect).contains("on the branch e-liang/earlier, which already has work on it"));
}
