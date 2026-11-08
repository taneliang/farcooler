//! ov-969: each review gate names who decides it, and none passes without that decider's verdict.
use farcooler_workflow::*;
use std::collections::{BTreeMap, BTreeSet};

fn input(state: &State, command: Command, grant: Grant) -> Input {
    Input { busy_resources: BTreeSet::new(), schema: 1, id: format!("command-{}", state.revision + 1), workspace: state.workspace.clone(),
        expected_revision: state.revision, epoch: state.epoch, source: "fixture".into(),
        source_sequence: state.revision + 1, now_ms: 1000, grant, command }
}
fn stage(id: &str, kind: StageKind) -> Stage {
    Stage { id: id.into(), kind, progression: Progression::Automatic, max_attempts: 2, routes: BTreeMap::new(), limit: 0,
        operation: OperationSpec { adapter_version: "fixture/v1".into(), arguments: vec![id.into()], environment: BTreeMap::new() } }
}
/// One workflow with all three deciders, in the order a ticket meets them, then landing.
fn three_gates() -> Definition {
    Definition { schema: 1, source: "gates/v1".into(), max_reworks: 0, rework_target: None, groups: BTreeMap::new(),
        models: BTreeMap::from([("agent-review".into(), vec!["claude:opus".into(), "codex:gpt-5.5".into()])]),
        stages: vec![stage("provision", StageKind::Provision), stage("build", StageKind::Build),
            stage("agent-review", StageKind::Review(Reviewer::Agent)),
            stage("orchestrator-review", StageKind::Review(Reviewer::Orchestrator("orchestrator".into()))),
            stage("person-review", StageKind::Review(Reviewer::Person("owner".into()))),
            stage("land", StageKind::Land)] }
}
fn evidence(item: &Item) -> Evidence {
    Evidence { candidate: item.candidate.clone().unwrap_or("candidate-a".into()),
        manifest: "sha256:manifest".into(), receipt: "operation-receipt".into(), policy_pin: item.policy_pin.clone() }
}
fn observe(state: &State, outcome: Outcome) -> Input {
    let item = &state.items["item"];
    let effect = item.pending.clone().unwrap();
    input(state, Command::Observe { item: "item".into(), effect: effect.clone(), run: item.run.clone(), fence: item.fence, outcome },
        Grant::Operation { effect, run: item.run.clone(), fence: item.fence })
}
fn review(state: &State, grant: Grant) -> Input {
    let item = &state.items["item"];
    input(state, Command::Review { item: "item".into(), run: item.run.clone(),
        evidence: item.last_evidence.clone().unwrap(), passed: true, reason: None }, grant)
}
fn decision(principal: &str) -> Grant { Grant::Decision { principal: principal.into() } }
fn at(state: &State) -> &str { let item = &state.items["item"]; &item.definition.stages[item.stage].id }
/// Every way a run could try to move on without the gate's own decider: none moves it.
fn nobody_else_moves_it(state: &State, decider: &str) {
    let item = &state.items["item"];
    let others = ["owner", "orchestrator", "reviewer"].into_iter().filter(|p| *p != decider);
    for grant in others.map(decision).chain([Grant::Controller]) {
        assert_eq!(step(state, &review(state, grant.clone())), Err(Refusal::Unauthorized), "{grant:?} at {}", at(state));
    }
    let approve = input(state, Command::Approve { item: "item".into(), run: item.run.clone(), pin: item.pin.clone(),
        candidate: item.candidate.clone() }, decision(decider));
    assert_eq!(step(state, &approve), Err(Refusal::Phase));
    let advanced = step(state, &input(state, Command::Advance, Grant::Controller)).unwrap().state;
    assert_eq!((advanced.items["item"].phase.clone(), at(&advanced)), (Phase::WaitingReview, at(state)));
}

#[test]
fn an_agent_an_orchestrator_and_a_person_gate_each_wait_for_their_own_verdict() {
    let definition = three_gates();
    definition.validate().unwrap();
    let state = State::empty("workspace".into(), 1);
    let enqueue = input(&state, Command::Enqueue { item: "item".into(), definition, policy_pin: "policy-1".into(),
        resource: "repo/branch".into(), dependencies: BTreeSet::new(), initial_candidate: None }, Grant::Controller);
    let mut state = step(&state, &enqueue).unwrap().state;
    for _ in 0..2 { state = step(&state, &observe(&state, Outcome::Passed(evidence(&state.items["item"])))).unwrap().state; }

    // The agent gate: the engine starts the agent, with the step's default model, and nobody can answer for it.
    assert_eq!(at(&state), "agent-review");
    assert_eq!(state.items["item"].phase, Phase::Running);
    assert_eq!(state.items["item"].models.get("agent-review").map(String::as_str), Some("claude:opus"));
    for principal in ["owner", "orchestrator"] {
        assert_eq!(step(&state, &review(&state, decision(principal))), Err(Refusal::Phase));
    }
    // An agent that ends with no verdict is a safe failure: it runs again, and never moves on.
    let retried = step(&state, &observe(&state, Outcome::SafeFailure)).unwrap();
    assert_eq!((at(&retried.state), retried.state.items["item"].phase.clone()), ("agent-review", Phase::Running));
    assert!(matches!(&retried.effects[..], [e] if e.kind == EffectKind::Execute(StageKind::Review(Reviewer::Agent))
        && e.model.as_deref() == Some("claude:opus")));
    state = retried.state;
    state = step(&state, &observe(&state, Outcome::Passed(evidence(&state.items["item"])))).unwrap().state;

    // The orchestrator gate, then the person gate: each waits, holding no effect, for its own decider.
    for (gate, decider) in [("orchestrator-review", "orchestrator"), ("person-review", "owner")] {
        assert_eq!((at(&state), state.items["item"].phase.clone()), (gate, Phase::WaitingReview));
        assert!(state.items["item"].pending.is_none());
        nobody_else_moves_it(&state, decider);
        state = step(&state, &review(&state, decision(decider))).unwrap().state;
    }
    // Landing is a step of its own, which the engine runs.
    assert_eq!((at(&state), state.items["item"].phase.clone()), ("land", Phase::Running));
}

#[test]
fn an_agent_gate_with_no_verdict_after_its_last_attempt_fails_the_run() {
    let state = State::empty("workspace".into(), 1);
    let enqueue = input(&state, Command::Enqueue { item: "item".into(), definition: three_gates(), policy_pin: "policy-1".into(),
        resource: "repo/branch".into(), dependencies: BTreeSet::new(), initial_candidate: None }, Grant::Controller);
    let mut state = step(&state, &enqueue).unwrap().state;
    for _ in 0..2 { state = step(&state, &observe(&state, Outcome::Passed(evidence(&state.items["item"])))).unwrap().state; }
    for _ in 0..2 { state = step(&state, &observe(&state, Outcome::SafeFailure)).unwrap().state; }
    assert_eq!((at(&state), state.items["item"].phase.clone()), ("agent-review", Phase::Failed));
}

#[test]
fn the_orchestrator_picks_an_agent_gates_model_from_the_ones_it_lists() {
    let definition = three_gates();
    let pick = |step: &str, model: &str| definition.picked_models(&BTreeMap::from([(step.into(), model.into())]));
    assert_eq!(pick("agent-review", "codex:gpt-5.5").unwrap()["agent-review"], "codex:gpt-5.5");
    assert_eq!(pick("agent-review", "codex:gpt-4"), None);
    // Only a step that starts an agent takes a model: a person or the orchestrator is no model.
    for kind in [StageKind::Review(Reviewer::Orchestrator("orchestrator".into())),
        StageKind::Review(Reviewer::Person("owner".into())), StageKind::Review(Reviewer::Dispatch)] {
        let mut d = three_gates();
        d.stages[2].kind = kind.clone();
        assert_eq!(d.validate(), Err(Refusal::Definition), "{kind:?}");
    }
}

#[test]
fn an_agent_gate_needs_a_candidate_to_judge() {
    let mut d = three_gates();
    d.stages.swap(1, 2);
    assert_eq!(d.validate(), Err(Refusal::Definition));
}
