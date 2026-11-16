//! Gate attribution against real Git candidates and retained lane-gate output (ov-1222).
use super::*;
use super::coding_verbs_tests::{lander, batched_push, at_join, command, until, phase_of, outbox};

#[tokio::test]
async fn nov14_gate_output_ejects_the_planted_breaker_and_lands_the_healthy_batch() {
    let (dir, runtime) = lander("named-breaker").await;
    let fixture = dir.path().join("nov14.output");
    std::fs::write(&fixture, include_bytes!("../../../../test/fixtures/landing-gate/nov14-group1781.output")).unwrap();
    let mut config = runtime.coding.clone().unwrap();
    config.checks[1].environment.insert("FAILURE_FIXTURE".into(), fixture.to_string_lossy().into_owned());
    std::fs::write(runtime.root.join("coding.json"), serde_json::to_vec(&config).unwrap()).unwrap();
    let root = runtime.root.clone(); drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    let definition = batched_push(&runtime, "any", 5000);
    for name in ["ahead", "good-a", "breaker", "good-b"] { at_join(&mut runtime, name, &definition).await; }
    for name in ["ahead", "good-a", "breaker", "good-b"] { command(&runtime, Command::Queue { item: name.into() }); }
    let before = runtime.state("w").unwrap().unwrap();
    let group = before.items["breaker"].grouped_in.clone().unwrap();
    let state = until(&mut runtime, |s| s.items["breaker"].brief.is_some()).await;
    let brief = state.items["breaker"].brief.as_ref().unwrap();
    assert!(brief.contains("ratchet: services/web/src/client/saved-copies.ts"), "{brief}");
    assert!(brief.contains("scripts/shared-core-lint.py"), "the brief names the failing check");
    assert!(state.items["breaker"].grouped_in.is_none());
    let stored = runtime.store.workflow_outbox("w").unwrap().into_iter().find(|s| s.effect.item == group
        && s.effect.kind == EffectKind::Execute(StageKind::Run)).unwrap();
    let binding = Binding::new("w", &stored.effect, stored.resource_generation.unwrap(), 0,
        &runtime.item_base("w", &stored.effect).unwrap());
    let recovered = runtime.reconcile_coding(&binding).await.unwrap();
    assert!(matches!(recovered, Outcome::GateRejected { members, brief: recovered_brief, .. }
        if members == ["breaker"] && recovered_brief == *brief), "recovery retains the same attribution and brief");
    assert_eq!(state.items[&group].members.iter().map(|m| m.item.as_str()).collect::<Vec<_>>(), ["good-a", "good-b"]);
    for name in ["good-a", "good-b"] { until(&mut runtime, |s| phase_of(s, name) == Some(Phase::Done)).await; }
    assert_eq!(outbox(&runtime, StageKind::Run), 3, "ahead, failed batch, then the healthy batch together");
    assert_eq!(outbox(&runtime, StageKind::Push), 2);
    assert!(!runtime.state("w").unwrap().unwrap().items["breaker"].back_edges.contains_key("join"));
}

#[test]
fn recorded_gate_diagnostics_exclude_the_changed_file_header_and_match_whole_paths() {
    let raw = include_str!("../../../../test/fixtures/landing-gate/nov14-group1781.output");
    let diagnostics = super::coding_execute::gate_diagnostics(raw);
    assert!(super::coding_execute::diagnostic_names(&diagnostics, "services/web/src/client/saved-copies.ts"));
    assert!(!super::coding_execute::diagnostic_names(&diagnostics, ".github/workflows/ci.yml"));
    assert!(!super::coding_execute::diagnostic_names(&diagnostics, "client/saved-copies.ts"));
    assert!(super::coding_execute::diagnostic_names(&diagnostics, "scripts/lane-gate.py"), "absolute traceback paths match too");
    assert!(super::coding_execute::diagnostic_names(&diagnostics, "scripts/shared-core-lint.py"));
    let brief = super::coding_execute::bounded_gate_brief("check", &"é".repeat(5000));
    assert!(farcooler_workflow::valid_rework_reason(&brief));
}

/// ov-1305: a refold moves the group base while the runner's original binding stays fixed.
#[tokio::test]
async fn group_gate_context_uses_each_fold_base_and_refuses_missing_or_foreign_records() {
    let (_dir, mut runtime) = lander("move-base").await;
    let definition = batched_push(&runtime, "any", 5000);
    at_join(&mut runtime, "one", &definition).await;
    command(&runtime, Command::Queue { item: "one".into() });
    until(&mut runtime, |s| phase_of(s, "one") == Some(Phase::Done)).await;
    let gates: Vec<_> = runtime.store.workflow_outbox("w").unwrap().into_iter()
        .filter(|s| s.effect.kind == EffectKind::Execute(StageKind::Run)).collect();
    assert_eq!(gates.len(), 2);
    let mut bases = Vec::new();
    for stored in gates {
        let binding = Binding::new("w", &stored.effect, stored.resource_generation.unwrap(), 0,
            &runtime.item_base("w", &stored.effect).unwrap());
        let candidate = stored.effect.candidate.as_deref().unwrap();
        let path = runtime.fold_index(&binding, candidate);
        let raw = std::fs::read(&path).unwrap();
        let fold: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        let fact: serde_json::Value = serde_json::from_slice(&std::fs::read(runtime.root.join("receipts")
            .join(format!("{}.process.json", digest(&stored.effect)))).unwrap()).unwrap();
        assert_eq!(fact["environment"]["FARCOOLER_BASE"], fold["base"]);
        assert_eq!(runtime.process_base(&binding, candidate).unwrap(), fold["base"].as_str().unwrap());
        bases.push(fold["base"].clone());
        std::fs::remove_file(&path).unwrap();
        assert!(runtime.process_base(&binding, candidate).is_err(), "no startup-base fallback");
        let mut foreign = fold.clone(); foreign["item"] = serde_json::json!("another-group");
        std::fs::write(&path, serde_json::to_vec(&foreign).unwrap()).unwrap();
        assert!(runtime.process_base(&binding, candidate).is_err());
        std::fs::write(&path, raw).unwrap();
    }
    assert_ne!(bases[0], bases[1], "the refold must change the gate's diff base");
}
