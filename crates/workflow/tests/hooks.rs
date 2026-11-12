//! Decision authenticity, current legality and suspended CI routing, independent of any process.
use std::collections::BTreeSet;
use farcooler_workflow::*;
fn input(s: &State, command: Command, grant: Grant) -> Input {
    Input { schema: 1, id: format!("event-{}", s.revision), workspace: s.workspace.clone(), expected_revision: s.revision,
        epoch: s.epoch, source: "test".into(), source_sequence: s.revision + 1, now_ms: 100,
        busy_resources: BTreeSet::new(), command, grant }
}
fn policy(point: HookPoint) -> ScriptPolicy {
    ScriptPolicy { point, script: "/tmp/rule".into(), digest: digest(&vec![1u8]), timeout_ms: 30,
        max_calls: 1, fallback: if point == HookPoint::CiResult { HookMove::Accept } else { HookMove::Leave } }
}
fn observe(s: &State, outcome: Outcome) -> Input {
    let i = &s.items["a"]; let effect = i.pending.clone().unwrap();
    input(s, Command::Observe { item: "a".into(), effect: effect.clone(), run: i.run.clone(), fence: i.fence, outcome },
        Grant::Operation { effect, run: i.run.clone(), fence: i.fence })
}
fn answer(s: &State, choice: serde_json::Value) -> Input {
    let i = &s.items["a"]; let call = &i.deciding.as_ref().unwrap().input.call;
    let mut value = choice; value["schema"] = "hook-decision/v1".into(); value["call"] = call.clone().into();
    input(s, Command::Decide { item: "a".into(), call: call.clone(), fence: i.fence, policy_pin: i.policy_pin.clone(),
        result: HookResult::Receipt { raw: value.to_string(), manifest: "manifest".into() } },
        Grant::Operation { effect: i.pending.clone().unwrap(), run: i.run.clone(), fence: i.fence })
}
fn running_ci() -> State {
    let mut definition = Definition::coding(); definition.stages.retain(|s| matches!(s.kind, StageKind::Provision | StageKind::Build | StageKind::Ci));
    definition.max_reworks = 0; definition.rework_target = None;
    definition.script_policies.insert("ci".into(), policy(HookPoint::CiResult));
    let s = State::empty("w".into(), 1);
    let mut s = step(&s, &input(&s, Command::Enqueue { item: "a".into(), definition, policy_pin: "p".into(), resource: "r".into(),
        dependencies: BTreeSet::new(), initial_candidate: None }, Grant::Controller)).unwrap().state;
    for _ in 0..2 {
        let e = Evidence { candidate: "sha".into(), manifest: "m".into(), receipt: "r".into(), policy_pin: "p".into() };
        s = step(&s, &observe(&s, Outcome::Passed(e))).unwrap().state;
    }
    s
}
#[test]
fn ci_rule_waits_then_routes_the_original_result_and_rejects_stale_or_unpinned_answers() {
    let s = running_ci();
    let e = Evidence { candidate: "sha".into(), manifest: "m".into(), receipt: "r".into(), policy_pin: "p".into() };
    let t = step(&s, &observe(&s, Outcome::Rejected(e))).unwrap();
    assert_eq!(t.state.items["a"].phase, Phase::Running);
    assert_eq!(t.state.items["a"].deciding.as_ref().unwrap().input.label.as_deref(), Some("rejected"));
    assert!(matches!(t.effects[0].kind, EffectKind::Decide { .. }));
    let s = t.state;
    let good = answer(&s, serde_json::json!({"decision":"accept"}));
    let mut stale = good.clone();
    if let Command::Decide { call, .. } = &mut stale.command { *call = "old".into(); }
    assert_eq!(step(&s, &stale), Err(Refusal::Stale));
    let mut pin = good.clone();
    if let Command::Decide { policy_pin, .. } = &mut pin.command { *policy_pin = "new".into(); }
    assert_eq!(step(&s, &pin), Err(Refusal::Evidence));
    let done = step(&s, &good).unwrap();
    assert_eq!(done.state.items["a"].phase, Phase::Failed);
    assert!(done.state.items["a"].hook_refused.is_none());
    // A red result cannot be promoted to green, nor can unknown decision fields smuggle authority.
    for bad in [serde_json::json!({"decision":"relabel","label":"passed"}), serde_json::json!({"decision":"accept","extra":true})] {
        let t = step(&s, &answer(&s, bad)).unwrap();
        assert_eq!(t.state.items["a"].phase, Phase::Failed);
        assert!(t.state.items["a"].hook_refused.is_some());
    }
    let cancel = input(&s, Command::Cancel { item: "a".into(), reason: None }, Grant::Controller);
    let cancelled = step(&s, &cancel).unwrap().state;
    let mut late = good; late.expected_revision = cancelled.revision; late.id = "late".into(); late.source_sequence += 1;
    assert_eq!(step(&cancelled, &late), Err(Refusal::Stale));
}
#[test]
fn policy_validation_is_closed_and_legacy_definitions_keep_their_bytes() {
    let mut d = Definition::coding();
    assert!(serde_json::to_value(&d).unwrap().get("policies").is_none());
    let old_pin = d.pin();
    d.script_policies.insert("rule".into(), policy(HookPoint::RunFails));
    assert!(d.validate().is_ok()); assert_ne!(old_pin, d.pin());
    d.script_policies.get_mut("rule").unwrap().timeout_ms = 600_001;
    assert_eq!(d.validate(), Err(Refusal::Definition));
}
