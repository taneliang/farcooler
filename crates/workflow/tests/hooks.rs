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
fn ci_rule_waits_then_routes_the_original_result_and_drops_stale_answers() {
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
    let mut stale_fence = good.clone();
    if let Command::Decide { fence, .. } = &mut stale_fence.command { *fence += 1; }
    assert_eq!(step(&s, &stale_fence), Err(Refusal::Stale));
    let mut pin = good.clone();
    if let Command::Decide { policy_pin, .. } = &mut pin.command { *policy_pin = "new".into(); }
    let mut unauthorized = pin.clone(); unauthorized.grant = Grant::Controller;
    assert_eq!(step(&s, &unauthorized), Err(Refusal::Unauthorized));
    let refused = step(&s, &pin).unwrap().state;
    assert_eq!(refused.items["a"].phase, Phase::Failed);
    assert!(refused.items["a"].deciding.is_none());
    assert!(refused.items["a"].hook_refused.is_some());
    // Even a missing answer cannot spend a retry under a different policy pin.
    if let Command::Decide { result, .. } = &mut pin.command { *result = HookResult::Missing; }
    let refused = step(&s, &pin).unwrap();
    assert!(refused.state.items["a"].deciding.is_none());
    assert!(refused.state.items["a"].hook_refused.is_some());
    assert!(!refused.effects.iter().any(|e| matches!(e.kind, EffectKind::Decide { .. })));
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

#[test]
fn ambiguous_receipts_apply_the_fallback_instead_of_choosing_the_last_field() {
    let s = running_ci();
    let e = Evidence { candidate: "sha".into(), manifest: "m".into(), receipt: "r".into(), policy_pin: "p".into() };
    let s = step(&s, &observe(&s, Outcome::Rejected(e))).unwrap().state;
    for duplicate in [
        "\"schema\":\"unknown\",", "\"call\":\"another-call\",",
        "\"decision\":\"retry\",", "\"decision\":\"accept\",",
    ] {
        let mut receipt = answer(&s, serde_json::json!({"decision":"accept"}));
        if let Command::Decide { result: HookResult::Receipt { raw, .. }, .. } = &mut receipt.command {
            raw.insert_str(1, duplicate);
        }
        let done = step(&s, &receipt).unwrap().state;
        assert_eq!(done.items["a"].phase, Phase::Failed);
        assert!(done.items["a"].hook_refused.is_some(), "accepted duplicate: {duplicate}");
    }
}

#[test]
fn ci_rules_never_suspend_observations_that_need_controller_resolution() {
    let s = running_ci();
    let uncertain = step(&s, &observe(&s, Outcome::Uncertain)).unwrap().state;
    let e = Evidence { candidate: "sha".into(), manifest: "m".into(), receipt: "r".into(), policy_pin: "p".into() };
    assert_eq!(step(&uncertain, &observe(&uncertain, Outcome::Passed(e))), Err(Refusal::Phase));
    assert!(uncertain.items["a"].deciding.is_none());
    assert!(uncertain.items["a"].hook_calls.is_empty());
}

#[test]
fn ci_rules_validate_result_labels_before_spending_a_call() {
    let s = running_ci();
    let e = Evidence { candidate: "sha".into(), manifest: "m".into(), receipt: "r".into(), policy_pin: "p".into() };
    assert_eq!(step(&s, &observe(&s, Outcome::Labeled("unknown".into(), e.clone()))), Err(Refusal::Invalid));
    let mut routed = s.clone();
    routed.items.get_mut("a").unwrap().definition.schema = 2;
    let item = routed.items.get_mut("a").unwrap();
    item.pin = item.definition.pin();
    assert_eq!(step(&routed, &observe(&routed, Outcome::Labeled("unknown".into(), e.clone()))), Err(Refusal::Evidence));
    // A real result still fires and settles; refused observations did not consume the budget.
    let s = step(&s, &observe(&s, Outcome::Passed(e.clone()))).unwrap().state;
    let done = step(&s, &answer(&s, serde_json::json!({"decision":"accept"}))).unwrap().state;
    assert_eq!(done.items["a"].phase, Phase::Done);
    // A completed run's stage is past the end of its definition. Late observations must not index it.
    let mut late = observe(&s, Outcome::Passed(e));
    late.expected_revision = done.revision;
    late.id = "late-ci-result".into(); late.source_sequence = done.revision + 1;
    assert_eq!(step(&done, &late), Err(Refusal::Phase));
}

#[test]
fn failure_rule_advertises_the_published_retry_allowance() {
    for allowance in [0, 5] {
        let mut definition = Definition::coding();
        definition.stages.truncate(1);
        definition.max_reworks = 0; definition.rework_target = None;
        definition.limits = Some(scheduling::Limits { retries: allowance, ..Default::default() });
        definition.script_policies.insert("failure".into(), policy(HookPoint::RunFails));
        let state = State::empty("w".into(), 1);
        let state = step(&state, &input(&state, Command::Enqueue { item: "a".into(), definition,
            policy_pin: "p".into(), resource: "r".into(), dependencies: BTreeSet::new(), initial_candidate: None },
            Grant::Controller)).unwrap().state;
        let mut state = state;
        // A run already retried three times can still use a larger published allowance.
        state.items.get_mut("a").unwrap().retries = allowance.min(3);
        let evidence = Evidence { candidate: "sha".into(), manifest: "m".into(), receipt: "r".into(), policy_pin: "p".into() };
        let failed = step(&state, &observe(&state, Outcome::Rejected(evidence))).unwrap().state;
        let call = failed.items["a"].deciding.as_ref().unwrap();
        assert_eq!(call.input.allowed.iter().any(|m| m == "retry"), allowance > 3);
    }
}

#[test]
fn an_exhausted_review_requires_a_controller_to_rebuild() {
    let mut definition = Definition::coding();
    definition.max_reworks = 1;
    definition.script_policies.insert("failure".into(), policy(HookPoint::RunFails));
    let state = State::empty("w".into(), 1);
    let mut state = step(&state, &input(&state, Command::Enqueue { item: "a".into(), definition,
        policy_pin: "p".into(), resource: "r".into(), dependencies: BTreeSet::new(),
        initial_candidate: None }, Grant::Controller)).unwrap().state;
    let evidence = Evidence { candidate: "sha".into(), manifest: "m".into(), receipt: "r".into(), policy_pin: "p".into() };
    for _ in 0..3 {
        state = step(&state, &observe(&state, Outcome::Passed(evidence.clone()))).unwrap().state;
    }
    for round in 0..2 {
        state = step(&state, &observe(&state, Outcome::ReviewRejected {
            evidence: evidence.clone(), summary: format!("Finding {round}"), receipt: "review.json".into(),
        })).unwrap().state;
        if round == 0 {
            for _ in 0..2 {
                state = step(&state, &observe(&state, Outcome::Passed(evidence.clone()))).unwrap().state;
            }
        }
    }

    let failed = &state.items["a"];
    assert_eq!(failed.phase, Phase::Failed);
    let failure = failed.failure.clone().unwrap();
    assert!(failure.contains("send-back limit of 1"), "{failure}");
    let allowed = &failed.deciding.as_ref().unwrap().input.allowed;
    assert!(allowed.iter().all(|move_| move_ != "retry" && move_ != "send_back"));
    for choice in [serde_json::json!({"decision":"retry"}), serde_json::json!({"decision":"send_back","reason":"Rebuild"})] {
        let declined = step(&state, &answer(&state, choice)).unwrap();
        assert!(declined.effects.is_empty());
        let still_failed = &declined.state.items["a"];
        assert_eq!(still_failed.phase, Phase::Failed);
        assert_eq!(still_failed.failure.as_deref(), Some(failure.as_str()));
    }
}
