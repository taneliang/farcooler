use farcooler_workflow::*;
use std::collections::BTreeSet;

fn input(state: &State, command: Command, grant: Grant) -> Input {
    Input { busy_resources: BTreeSet::new(), schema: 1, id: format!("command-{}", state.revision + 1), workspace: state.workspace.clone(),
        expected_revision: state.revision, epoch: state.epoch, source: "fixture".into(),
        source_sequence: state.revision + 1, now_ms: 1000, grant, command }
}
fn enqueue(state: &State, definition: Definition) -> Input {
    let initial_candidate = matches!(definition.stages[0].kind, StageKind::Tracker).then(|| "checklist-v1".into());
    input(state, Command::Enqueue { item: "item".into(), definition, policy_pin: "policy-1".into(),
        resource: "repo/branch".into(), dependencies: BTreeSet::new(), initial_candidate }, Grant::Controller)
}
fn evidence(item: &Item) -> Evidence {
    Evidence { candidate: item.candidate.clone().unwrap_or("candidate-a".into()),
        manifest: "sha256:manifest".into(), receipt: "operation-receipt".into(), policy_pin: item.policy_pin.clone() }
}
fn observe(state: &State, outcome: Outcome) -> Input {
    let item = &state.items["item"];
    let effect = item.pending.clone().unwrap();
    input(state, Command::Observe { item: "item".into(), effect: effect.clone(),
        run: item.run.clone(), fence: item.fence, outcome },
        Grant::Operation { effect, run: item.run.clone(), fence: item.fence })
}
fn approve(state: &State) -> Input {
    let item = &state.items["item"];
    input(state, Command::Approve { item: "item".into(), run: item.run.clone(), pin: item.pin.clone(),
        candidate: item.candidate.clone() }, Grant::Decision { principal: "owner".into() })
}
fn start(definition: Definition) -> State {
    let state = State::empty("workspace".into(), 1);
    step(&state, &enqueue(&state, definition)).unwrap().state
}
#[test]
fn workflow_automatic_coding_replay_with_review_rework() {
    let mut state = State::empty("workspace".into(), 1);
    let mut log = vec![enqueue(&state, Definition::coding())];
    state = step(&state, &log[0]).unwrap().state;
    let mut rejected = false;
    let mut kinds = vec![];
    while state.items["item"].phase != Phase::Done {
        let item = &state.items["item"];
        let kind = item.definition.stages[item.stage].kind.clone();
        kinds.push(kind.clone());
        let outcome = if matches!(kind, StageKind::Review(_)) && !rejected {
            rejected = true; Outcome::Rejected(evidence(item))
        } else { Outcome::Passed(evidence(item)) };
        let event = observe(&state, outcome);
        let transition = step(&state, &event).unwrap();
        assert_eq!(step(&state, &event).unwrap(), transition);
        state = transition.state;
        log.push(event);
    }
    assert_eq!(kinds.len(), 9);
    assert_eq!(state.items["item"].iteration, 1);
    let mut replay = State::empty("workspace".into(), 1);
    for event in log { replay = step(&replay, &event).unwrap().state; }
    assert_eq!(digest(&state), digest(&replay));
}
#[test]
fn workflow_every_stage_approval_and_informational_ack_exclusion() {
    let mut definition = Definition::coding();
    for s in &mut definition.stages {
        s.progression = Progression::AwaitApproval { principal: "owner".into() };
    }
    let mut state = start(definition);
    for _ in 0..6 {
        assert_eq!(state.items["item"].phase, Phase::WaitingApproval);
        assert!(state.items["item"].pending.is_none());
        // There is no ACK command. Even a controller cannot impersonate the configured decision maker.
        let mut bad = approve(&state); bad.grant = Grant::Controller;
        assert_eq!(step(&state, &bad), Err(Refusal::Unauthorized));
        state = step(&state, &approve(&state)).unwrap().state;
        let e = observe(&state, Outcome::Passed(evidence(&state.items["item"])));
        state = step(&state, &e).unwrap().state;
    }
    assert_eq!(state.items["item"].phase, Phase::Done);
}
#[test]
fn workflow_stale_revision_epoch_source_and_duplicate_refused() {
    let state = start(Definition::coding());
    let good = observe(&state, Outcome::Passed(evidence(&state.items["item"])));
    for mutation in 0..4 {
        let mut bad = good.clone();
        match mutation { 0 => bad.expected_revision -= 1, 1 => bad.epoch += 1,
            2 => bad.source_sequence -= 1, _ => bad.workspace = "other".into() }
        assert_eq!(step(&state, &bad), Err(Refusal::Stale));
    }
    let next = step(&state, &good).unwrap().state;
    assert_eq!(step(&next, &good), Err(Refusal::Duplicate));
}
#[test]
fn workflow_uncertainty_cancel_and_late_result_fences() {
    let mut state = start(Definition::coding());
    let late = observe(&state, Outcome::Passed(evidence(&state.items["item"])));
    state = step(&state, &observe(&state, Outcome::Uncertain)).unwrap().state;
    assert_eq!(state.items["item"].phase, Phase::Uncertain);
    assert!(owns_resource(&state.items["item"]));
    assert_eq!(step(&state, &observe(&state, Outcome::SafeFailure)), Err(Refusal::Phase));
    let cancel = input(&state, Command::Cancel { item: "item".into() }, Grant::Controller);
    let transition = step(&state, &cancel).unwrap();
    assert!(matches!(transition.effects[0].kind, EffectKind::Stop { .. }));
    state = transition.state;
    let mut late = late; late.id = "late".into(); late.expected_revision = state.revision;
    late.source_sequence = state.revision + 1;
    assert_eq!(step(&state, &late), Err(Refusal::Stale));
    assert_eq!(step(&state, &observe(&state, Outcome::SafeFailure)), Err(Refusal::Phase));
    state = step(&state, &observe(&state, Outcome::Uncertain)).unwrap().state;
    let mut resolved = observe(&state, stopped(&state));
    resolved.command = match resolved.command {
        Command::Observe { item, effect, run, fence, outcome } => Command::Resolve { item, effect, run, fence, outcome },
        _ => unreachable!(),
    };
    resolved.grant = Grant::Controller;
    state = step(&state, &resolved).unwrap().state;
    assert_eq!(state.items["item"].phase, Phase::Cancelled);
    assert!(!owns_resource(&state.items["item"]));
}
#[test]
fn workflow_no_git_checklist_pin_and_exact_decision() {
    let state = start(Definition::tracker("owner"));
    let pin = state.items["item"].pin.clone();
    let mut modified = Definition::tracker("owner"); modified.source = "new-default".into();
    assert_ne!(pin, modified.pin());
    let mut approval = approve(&state);
    if let Command::Approve { pin, .. } = &mut approval.command { *pin = modified.pin(); }
    assert_eq!(step(&state, &approval), Err(Refusal::Evidence));
    let transition = step(&state, &approve(&state)).unwrap();
    assert_eq!(transition.effects.len(), 1);
    assert_eq!(transition.effects[0].kind, EffectKind::Execute(StageKind::Tracker));
    let state = transition.state;
    let mut wrong = evidence(&state.items["item"]); wrong.candidate = "checklist-v2".into();
    assert_eq!(step(&state, &observe(&state, Outcome::Passed(wrong))), Err(Refusal::Evidence));
    let next = step(&state, &observe(&state, Outcome::Passed(evidence(&state.items["item"])))).unwrap().state;
    assert_eq!(next.items["item"].phase, Phase::Done);
    assert_eq!(next.items["item"].pin, pin);
}
#[test]
fn workflow_definition_bounds_and_stage_order() {
    let mut bad = Definition::coding(); bad.rework_target = Some("missing-stage".into());
    assert_eq!(bad.validate(), Err(Refusal::Definition));
    let mut bad = Definition::coding(); bad.stages[1].id = bad.stages[0].id.clone();
    assert_eq!(bad.validate(), Err(Refusal::Definition));
    let mut bad = Definition::coding(); bad.max_reworks = 3;
    assert_eq!(bad.validate(), Err(Refusal::Definition));
    let mut bad = Definition::coding(); bad.stages[0].max_attempts = 0;
    assert_eq!(bad.validate(), Err(Refusal::Definition));
}
#[test]
fn workflow_reviewer_person_and_rework_exhaustion() {
    let mut definition = Definition::coding();
    definition.stages[3].kind = StageKind::Review(Reviewer::Person("owner".into()));
    let mut state = start(definition);
    for iteration in 0..3 {
        while state.items["item"].phase == Phase::Running {
            state = step(&state, &observe(&state, Outcome::Passed(evidence(&state.items["item"])))).unwrap().state;
        }
        assert_eq!(state.items["item"].phase, Phase::WaitingReview);
        assert_eq!(state.items["item"].iteration, iteration);
        let i = &state.items["item"];
        let event = input(&state, Command::Review { item: "item".into(), run: i.run.clone(),
            evidence: evidence(i), passed: false, reason: None }, Grant::Decision { principal: "owner".into() });
        state = step(&state, &event).unwrap().state;
    }
    assert_eq!(state.items["item"].phase, Phase::Failed);
}
#[test]
fn workflow_retry_limit_and_exact_candidate() {
    let mut state = start(Definition::coding());
    for _ in 0..2 {
        state = step(&state, &observe(&state, Outcome::Passed(evidence(&state.items["item"])))).unwrap().state;
    }
    let mut wrong = evidence(&state.items["item"]); wrong.candidate = "wrong-sha".into();
    assert_eq!(step(&state, &observe(&state, Outcome::Passed(wrong))), Err(Refusal::Evidence));
    state = step(&state, &observe(&state, Outcome::SafeFailure)).unwrap().state;
    assert_eq!(state.items["item"].attempt, 2);
    state = step(&state, &observe(&state, Outcome::SafeFailure)).unwrap().state;
    assert_eq!(state.items["item"].phase, Phase::Failed);
}
#[test]
fn a_failed_run_can_be_cancelled_by_its_controller() {
    let mut state = start(Definition::coding());
    for _ in 0..2 {
        state = step(&state, &observe(&state, Outcome::Passed(evidence(&state.items["item"])))).unwrap().state;
    }
    for _ in 0..2 { state = step(&state, &observe(&state, Outcome::SafeFailure)).unwrap().state; }
    assert_eq!(state.items["item"].phase, Phase::Failed);
    let cancel = input(&state, Command::Cancel { item: "item".into() }, Grant::Controller);
    assert_eq!(step(&state, &cancel).unwrap().state.items["item"].phase, Phase::Cancelled);
    let nobody = input(&state, Command::Cancel { item: "item".into() }, Grant::Decision { principal: "owner".into() });
    assert!(step(&state, &nobody).is_err(), "only a controller may");
}
#[test]
fn workflow_dependencies_and_resource_conflict() {
    let mut state = start(Definition::coding());
    let mut add = enqueue(&state, Definition::tracker("owner"));
    if let Command::Enqueue { item, resource, dependencies, .. } = &mut add.command {
        *item = "dependent".into(); *resource = "other-resource".into(); dependencies.insert("item".into());
    }
    state = step(&state, &add).unwrap().state;
    assert_eq!(state.items["dependent"].phase, Phase::Blocked);
    while state.items["item"].phase != Phase::Done {
        state = step(&state, &observe(&state, Outcome::Passed(evidence(&state.items["item"])))).unwrap().state;
    }
    assert_eq!(state.items["dependent"].phase, Phase::WaitingApproval);
    let mut clash = enqueue(&state, Definition::coding());
    if let Command::Enqueue { item, resource, .. } = &mut clash.command {
        *item = "clash".into(); *resource = "other-resource".into();
    }
    assert_eq!(step(&state, &clash), Err(Refusal::Resource));
}
#[test]
fn workflow_seeded_adversarial_replay_properties() {
    // Reproducible generated traces, not a proof: exercise refusal purity and replay identity.
    let mut accepted = 0;
    let mut refused = 0;
    for seed in 1..=128_u64 {
        let mut rng = seed;
        let initial = State::empty("workspace".into(), 1);
        let mut state = initial.clone();
        let mut log = vec![];
        for n in 0..64 {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            let mut event = if state.items.is_empty() { enqueue(&state, Definition::coding()) }
                else if state.items["item"].phase == Phase::Running {
                    observe(&state, match rng % 7 { 0 => Outcome::SafeFailure, 1 => Outcome::Uncertain,
                        2 => Outcome::Rejected(evidence(&state.items["item"])),
                        _ => Outcome::Passed(evidence(&state.items["item"])) })
                } else { input(&state, Command::Cancel { item: "item".into() }, Grant::Controller) };
            event.id = format!("seed-{seed}-{n}");
            match rng % 11 { 0 => event.epoch += 1, 1 => event.expected_revision += 1,
                2 => event.grant = Grant::Decision { principal: "intruder".into() }, _ => {} }
            let before = digest(&state);
            match step(&state, &event) {
                Ok(next) => {
                    assert_eq!(next, step(&state, &event).unwrap(), "seed {seed} step {n}");
                    assert_eq!(next.state.revision, state.revision + 1);
                    for e in &next.effects {
                        assert_eq!(e.epoch, state.epoch);
                        assert_eq!(e.definition_pin, next.state.items[&e.item].pin);
                    }
                    state = next.state;
                    log.push(event);
                    accepted += 1;
                }
                Err(_) => { assert_eq!(before, digest(&state)); refused += 1; }
            }
        }
        let mut replay = initial;
        for e in log { replay = step(&replay, &e).unwrap().state; }
        assert_eq!(digest(&state), digest(&replay), "seed {seed}");
    }
    assert!(accepted > 500 && refused > 500);
    println!("generated seeds=128 steps=8192 accepted={accepted} refused={refused}");
}
#[test]
fn workflow_canonical_encoding_golden() {
    // Independently computed with Python hashlib over the documented canonical JSON.
    assert_eq!(digest(&State::empty("workspace".into(), 1)),
        "wf1:0701eb6be269f4c0757b23c761c45f6030bc4a5002f23e8dbe739891e20d4696");
}

fn stopped(state: &State) -> Outcome {
    Outcome::Stopped { operation: state.items["item"].stopping.clone().unwrap(), receipt: "drained-reconciled-receipt".into() }
}
#[test]
fn workflow_invalid_snapshot_and_pinned_operation_payload() {
    let state = start(Definition::coding());
    let mut corrupt = state.clone(); corrupt.items.get_mut("item").unwrap().stage = 99;
    assert_eq!(step(&corrupt, &observe(&state, Outcome::Uncertain)), Err(Refusal::Invalid));
    let mut definition = Definition::coding();
    let old_pin = definition.pin();
    definition.stages[2].operation.arguments.push("--release".into());
    assert_ne!(old_pin, definition.pin());
    let state = State::empty("workspace".into(), 1);
    let transition = step(&state, &enqueue(&state, definition.clone())).unwrap();
    assert_eq!(transition.effects[0].operation, definition.stages[0].operation);
}

#[test]
fn workflow_definition_driven_no_review_multiple_checks_and_approvals() {
    for approvals in [vec![], vec![1, 3], vec![0, 1, 2, 3, 4, 5]] {
        let mut definition = Definition::coding();
        definition.stages[3].kind = StageKind::Check;
        definition.stages[3].id = "second-check".into();
        definition.max_reworks = 0;
        definition.rework_target = None;
        for index in &approvals {
            definition.stages[*index].progression = Progression::AwaitApproval { principal: "owner".into() };
        }
        let mut state = start(definition);
        let mut paused = vec![];
        let mut executed = vec![];
        while state.items["item"].phase != Phase::Done {
            let item = &state.items["item"];
            if item.phase == Phase::WaitingApproval {
                paused.push(item.stage);
                // Informational delivery has no authority-bearing command variant.
                let before = digest(&state);
                assert!(serde_json::from_str::<Command>(r#"{"Ack":{"item":"item"}}"#).is_err());
                assert_eq!(before, digest(&state));
                state = step(&state, &approve(&state)).unwrap().state;
            } else {
                assert_eq!(item.phase, Phase::Running);
                executed.push(item.definition.stages[item.stage].kind.clone());
                state = step(&state, &observe(&state, Outcome::Passed(evidence(item)))).unwrap().state;
            }
        }
        assert_eq!(paused, approvals);
        assert_eq!(executed.iter().filter(|k| **k == StageKind::Check).count(), 2);
        assert!(!executed.iter().any(|k| matches!(k, StageKind::Review(_))));
    }
}

#[test]
fn workflow_noncoding_named_actions_and_stable_rework_target() {
    let mut definition = Definition::coding();
    definition.stages.truncate(3);
    definition.stages[0].kind = StageKind::Action("collect-data".into());
    definition.stages[0].id = "collect".into();
    definition.stages[1].kind = StageKind::Action("summarize".into());
    definition.stages[1].id = "summarize".into();
    definition.stages[2].kind = StageKind::Evaluate("fact-check".into());
    definition.rework_target = Some("collect".into());
    definition.max_reworks = 1;
    let pin = definition.pin();
    let mut changed = definition.clone(); changed.rework_target = Some("summarize".into());
    assert_ne!(pin, changed.pin());
    let mut state = start(definition);
    for _ in 0..2 {
        state = step(&state, &observe(&state, Outcome::Passed(evidence(&state.items["item"])))).unwrap().state;
    }
    state = step(&state, &observe(&state, Outcome::Rejected(evidence(&state.items["item"])))).unwrap().state;
    assert_eq!(state.items["item"].stage, 0);
    assert_eq!(state.items["item"].iteration, 1);
    while state.items["item"].phase != Phase::Done {
        state = step(&state, &observe(&state, Outcome::Passed(evidence(&state.items["item"])))).unwrap().state;
    }
    let mut invalid = changed; invalid.rework_target = Some(invalid.stages[2].id.clone());
    assert_eq!(invalid.validate(), Err(Refusal::Definition));
}

#[test]
fn workflow_blocked_shared_resource_does_not_reserve_and_releases_one_waiter() {
    let mut state = start(Definition::coding());
    for id in ["dependent-a", "dependent-b"] {
        let mut add = enqueue(&state, Definition::tracker("owner"));
        if let Command::Enqueue { item, dependencies, .. } = &mut add.command {
            *item = id.into(); dependencies.insert("item".into());
        }
        state = step(&state, &add).unwrap().state;
        assert!(!owns_resource(&state.items[id]));
    }
    while state.items["item"].phase != Phase::Done {
        state = step(&state, &observe(&state, Outcome::Passed(evidence(&state.items["item"])))).unwrap().state;
    }
    assert_eq!(state.items["dependent-a"].phase, Phase::WaitingApproval);
    assert_eq!(state.items["dependent-b"].phase, Phase::Blocked);
    let cancel = input(&state, Command::Cancel { item: "dependent-a".into() }, Grant::Controller);
    state = step(&state, &cancel).unwrap().state;
    assert_eq!(state.items["dependent-b"].phase, Phase::WaitingApproval);
    // Blocked items remain cancellable even though they hold no resource authority.
    let mut add = enqueue(&state, Definition::tracker("owner"));
    if let Command::Enqueue { item, dependencies, .. } = &mut add.command {
        *item = "blocked".into(); dependencies.insert("dependent-b".into());
    }
    state = step(&state, &add).unwrap().state;
    let cancel = input(&state, Command::Cancel { item: "blocked".into() }, Grant::Controller);
    assert_eq!(step(&state, &cancel).unwrap().state.items["blocked"].phase, Phase::Cancelled);
}

#[test]
fn workflow_definition_rejects_missing_candidate_paths() {
    for kind in [StageKind::Check, StageKind::Evaluate("evaluate".into()), StageKind::Review(Reviewer::Dispatch)] {
        let mut definition = Definition::coding();
        definition.stages = vec![definition.stages[0].clone()];
        definition.stages[0].kind = kind;
        definition.max_reworks = 0; definition.rework_target = None;
        assert_eq!(definition.validate(), Err(Refusal::Definition));
    }
    let mut definition = Definition::coding();
    definition.rework_target = Some(definition.stages[2].id.clone());
    assert_eq!(definition.validate(), Err(Refusal::Definition));
}

fn reasoned_review(state: &State, reason: &str) -> Input {
    let i = &state.items["item"];
    input(state, Command::Review { item: "item".into(), run: i.run.clone(),
        evidence: i.last_evidence.clone().unwrap(), passed: false, reason: Some(reason.into()) },
        Grant::Decision { principal: "owner".into() })
}
fn human_review() -> State {
    let mut d = Definition::coding();
    d.stages[3].kind = StageKind::Review(Reviewer::Person("owner".into()));
    let mut s = start(d);
    while s.items["item"].phase == Phase::Running {
        s = step(&s, &observe(&s, Outcome::Passed(evidence(&s.items["item"])))).unwrap().state;
    }
    s
}
#[test]
fn workflow_reasoned_rework_fences_and_exact_retention() {
    let s = human_review();
    let reason = "  请修复 <script> $(touch /never)\n\tretain spaces  ";
    let accepted = reasoned_review(&s, reason);
    let next = step(&s, &accepted).unwrap().state;
    let record = &next.items["item"].rework_history[0];
    assert_eq!(record.reason, reason);
    assert_eq!(record.principal, "owner"); assert_eq!(record.run, s.items["item"].run);
    assert_eq!(record.evidence, s.items["item"].last_evidence.clone().unwrap());
    assert_eq!(record.revision, s.revision); assert_eq!(record.iteration, 0);
    assert_eq!(next.items["item"].iteration, 1);
    assert_eq!(step(&next, &accepted), Err(Refusal::Duplicate));
    for (label, refusal) in [("principal",Refusal::Unauthorized),("revision",Refusal::Stale),
        ("epoch",Refusal::Stale),("sequence",Refusal::Stale),("run",Refusal::Phase),
        ("evidence",Refusal::Evidence),("candidate",Refusal::Evidence),("policy",Refusal::Evidence),
        ("passed",Refusal::Invalid),("budget",Refusal::Limit)] {
        let mut state = s.clone(); let mut bad = accepted.clone();
        match label {
            "principal" => bad.grant = Grant::Controller,
            "revision" => bad.expected_revision += 1,
            "epoch" => bad.epoch += 1,
            "sequence" => bad.source_sequence -= 1,
            "candidate" => state.items.get_mut("item").unwrap().candidate = Some("replacement".into()),
            "policy" => state.items.get_mut("item").unwrap().policy_pin = "replacement".into(),
            "budget" => state.items.get_mut("item").unwrap().iteration = 2,
            _ => if let Command::Review { run, evidence, passed, .. } = &mut bad.command {
                match label { "run" => *run = "old".into(), "evidence" => evidence.receipt = "unseen".into(),
                    "passed" => *passed = true, _ => unreachable!() }
            },
        }
        let bytes = serde_json::to_vec(&state).unwrap();
        assert_eq!(step(&state, &bad), Err(refusal), "{label}");
        assert_eq!(bytes, serde_json::to_vec(&state).unwrap(), "{label} mutated state");
    }
}
#[test]
fn workflow_reasoned_rework_bounds_budget_and_legacy_bytes() {
    let mut s = human_review();
    for reason in ["", " \n\t\u{2003}", "bad\0", "bad\u{7f}", "bad\u{85}", &"界".repeat(1366)] {
        assert_eq!(step(&s, &reasoned_review(&s, reason)), Err(Refusal::Invalid));
    }
    assert!(valid_rework_reason(&"x".repeat(4096)));
    assert!(!valid_rework_reason(&"x".repeat(4097)));
    for n in 0..2 {
        s = step(&s, &reasoned_review(&s, &format!("reason-{n}"))).unwrap().state;
        while s.items["item"].phase == Phase::Running {
            s = step(&s, &observe(&s, Outcome::Passed(evidence(&s.items["item"])))).unwrap().state;
        }
    }
    assert_eq!(s.items["item"].rework_history.len(), 2);
    assert_eq!(step(&s, &reasoned_review(&s, "exhausted")), Err(Refusal::Limit));
    let original = human_review();
    let mut old = reasoned_review(&original, "unused");
    if let Command::Review { reason, .. } = &mut old.command { *reason = None; }
    let raw = serde_json::to_string(&old).unwrap();
    assert!(!raw.contains("reason"));
    assert_eq!(serde_json::to_string(&serde_json::from_str::<Input>(&raw).unwrap()).unwrap(), raw);
    let legacy = step(&original, &old).unwrap().state;
    assert_eq!(legacy.items["item"].iteration, 1);
    assert!(!serde_json::to_string(&legacy).unwrap().contains("rework_history"));
}
