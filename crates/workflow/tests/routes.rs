//! Routes, labeled results and retry. Every fixture is private to this file.
use farcooler_workflow::*;
use std::collections::BTreeSet;

fn input(state: &State, command: Command, grant: Grant) -> Input {
    Input { busy_resources: BTreeSet::new(), schema: 1, id: format!("command-{}", state.revision + 1),
        workspace: state.workspace.clone(), expected_revision: state.revision, epoch: state.epoch,
        source: "fixture".into(), source_sequence: state.revision + 1, now_ms: 1000, grant, command }
}
fn enqueue(state: &State, definition: Definition, item: &str, resource: &str) -> Input {
    input(state, Command::Enqueue { item: item.into(), definition, policy_pin: "policy-1".into(),
        resource: resource.into(), dependencies: BTreeSet::new(), initial_candidate: None }, Grant::Controller)
}
fn evidence(item: &Item) -> Evidence {
    Evidence { candidate: item.candidate.clone().unwrap_or("candidate-a".into()),
        manifest: "sha256:manifest".into(), receipt: "operation-receipt".into(), policy_pin: item.policy_pin.clone() }
}
fn observe(state: &State, outcome: Outcome) -> Input {
    let item = &state.items["item"];
    let effect = item.pending.clone().unwrap();
    input(state, Command::Observe { item: "item".into(), effect: effect.clone(), run: item.run.clone(),
        fence: item.fence, outcome }, Grant::Operation { effect, run: item.run.clone(), fence: item.fence })
}
fn pass(state: &State) -> Input { observe(state, Outcome::Passed(evidence(&state.items["item"]))) }
fn reject(state: &State) -> Input { observe(state, Outcome::Rejected(evidence(&state.items["item"]))) }
fn label(state: &State, name: &str) -> Input {
    observe(state, Outcome::Labeled(name.into(), evidence(&state.items["item"])))
}
fn start(definition: Definition) -> State {
    let state = State::empty("workspace".into(), 1);
    step(&state, &enqueue(&state, definition, "item", "repo/branch")).unwrap().state
}
fn go(state: State, command: Input) -> State { step(&state, &command).unwrap().state }
fn stage(state: &State) -> String {
    let item = &state.items["item"];
    item.definition.stages[item.stage].id.clone()
}
/// provision, build, check, review, land, ci: the coding sequence on schema 2.
fn routed() -> Definition {
    let mut d = Definition::coding();
    d.schema = 2; d.max_reworks = 0; d.rework_target = None;
    d.stages[2].routes.insert("rejected".into(), Target::Step("stage-1".into()));
    d.stages[2].routes.insert("skip-review".into(), Target::Step("stage-4".into()));
    d.stages[2].limit = 2;
    d.stages[3].routes.insert("rejected".into(), Target::Step("stage-1".into()));
    d.stages[3].limit = 2;
    d
}
fn bad(edit: impl FnOnce(&mut Definition)) -> Result<(), Refusal> {
    let mut d = routed(); edit(&mut d); d.validate()
}
/// Drive provision and build to the check.
fn to_check(mut state: State) -> State {
    while stage(&state) != "stage-2" { state = go(state.clone(), pass(&state)); }
    state
}

#[test]
fn a_rejected_check_routes_back_to_the_build_and_forgets_the_candidate() {
    let state = to_check(start(routed()));
    assert!(state.items["item"].candidate.is_some());
    let state = go(state.clone(), reject(&state));
    assert_eq!(stage(&state), "stage-1");
    assert_eq!(state.items["item"].candidate, None);
    assert_eq!(state.items["item"].phase, Phase::Running);
    assert_eq!(state.items["item"].back_edges["stage-2"], 1);
}
#[test]
fn a_back_edge_stops_at_its_limit_and_the_run_fails() {
    let mut state = to_check(start(routed()));
    for round in 1..=2 {
        state = go(state.clone(), reject(&state));
        assert_eq!(stage(&state), "stage-1", "round {round}");
        state = to_check(state);
    }
    state = go(state.clone(), reject(&state));
    assert_eq!(state.items["item"].phase, Phase::Failed);
    assert_eq!(stage(&state), "stage-2");
}
#[test]
fn a_labeled_result_follows_its_route_and_skips_steps() {
    let state = to_check(start(routed()));
    let state = go(state.clone(), label(&state, "skip-review"));
    assert_eq!(stage(&state), "stage-4");
    assert_eq!(state.items["item"].phase, Phase::Running);
}
#[test]
fn a_label_the_step_never_declared_is_refused() {
    let state = to_check(start(routed()));
    assert_eq!(step(&state, &label(&state, "mystery")), Err(Refusal::Evidence));
}
#[test]
fn a_label_is_refused_on_a_schema_1_definition() {
    let mut state = start(Definition::coding());
    state = go(state.clone(), pass(&state));
    assert_eq!(step(&state, &label(&state, "passed")), Err(Refusal::Invalid));
}
#[test]
fn a_done_route_finishes_the_run_early() {
    let mut d = routed();
    d.stages[2].routes.insert("shipped".into(), Target::Done);
    let state = to_check(start(d));
    let state = go(state.clone(), label(&state, "shipped"));
    assert_eq!(state.items["item"].phase, Phase::Done);
}
#[test]
fn an_unrouted_rejection_fails_and_an_unrouted_pass_falls_through() {
    let mut d = routed();
    d.stages[3].routes.clear(); d.stages[3].limit = 0;
    let state = to_check(start(d));
    let passed = go(state.clone(), pass(&state));
    assert_eq!(stage(&passed), "stage-3");
    let failed = go(passed.clone(), reject(&passed));
    assert_eq!(failed.items["item"].phase, Phase::Failed);
}
#[test]
fn a_review_reason_follows_a_back_edge_and_is_kept() {
    let mut d = routed();
    d.stages[3].kind = StageKind::Review(Reviewer::Person("owner".into()));
    let mut state = to_check(start(d));
    state = go(state.clone(), pass(&state));
    assert_eq!(state.items["item"].phase, Phase::WaitingReview);
    let item = &state.items["item"];
    let review = input(&state, Command::Review { item: "item".into(), run: item.run.clone(),
        evidence: evidence(item), passed: false, reason: Some("Handle the empty case.".into()) },
        Grant::Decision { principal: "owner".into() });
    let state = go(state.clone(), review);
    assert_eq!(stage(&state), "stage-1");
    assert_eq!(state.items["item"].rework_history.len(), 1);
    assert_eq!(state.items["item"].iteration, 1);
}
#[test]
fn routed_runs_replay_to_the_same_state() {
    let mut state = State::empty("workspace".into(), 1);
    let mut log = vec![enqueue(&state, routed(), "item", "repo/branch")];
    state = step(&state, &log[0]).unwrap().state;
    let mut rejected = 0;
    while state.items["item"].phase != Phase::Done {
        let event = if stage(&state) == "stage-2" && rejected < 2 { rejected += 1; reject(&state) }
            else if stage(&state) == "stage-2" { label(&state, "skip-review") } else { pass(&state) };
        let transition = step(&state, &event).unwrap();
        assert_eq!(step(&state, &event).unwrap(), transition);
        state = transition.state; log.push(event);
    }
    let mut replay = State::empty("workspace".into(), 1);
    for event in log { replay = step(&replay, &event).unwrap().state; }
    assert_eq!(digest(&state), digest(&replay));
}
#[test]
fn schema_1_definitions_serialize_and_pin_exactly_as_before() {
    let text = serde_json::to_string(&Definition::coding()).unwrap();
    for key in ["routes", "limit", "back_edges", "retries"] { assert!(!text.contains(key), "{key}"); }
    assert_eq!(Definition::coding().pin(), "wf1:25734e5f86665b36583002c68160dd4969a40cf29a6d5aa3fbc6f08feba2ffa4");
}

#[test]
fn the_validator_accepts_the_routed_fixture() { assert_eq!(bad(|_| {}), Ok(())); }
#[test]
fn the_validator_rejects_bad_routes() {
    let cases: Vec<(&str, Box<dyn FnOnce(&mut Definition)>)> = vec![
        ("route to a missing step", Box::new(|d| { d.stages[2].routes.insert("skip-review".into(), Target::Step("nope".into())); })),
        ("back edge with no limit", Box::new(|d| { d.stages[2].limit = 0; })),
        ("limit with no back edge", Box::new(|d| { d.stages[0].limit = 1; })),
        ("limit above three", Box::new(|d| { d.stages[2].limit = 4; })),
        ("custom label on a build", Box::new(|d| { d.stages[1].routes.insert("odd".into(), Target::Done); })),
        ("bad label text", Box::new(|d| { d.stages[2].routes.insert("has space".into(), Target::Done); })),
        ("unreachable steps", Box::new(|d| { d.stages[0].routes.insert("passed".into(), Target::Done); })),
        ("done is unreachable", Box::new(|d| { d.stages[5].routes.insert("passed".into(), Target::Fail); })),
        ("routes with a rework target", Box::new(|d| { d.max_reworks = 1; d.rework_target = Some("stage-1".into()); })),
        ("rework target without schema 1", Box::new(|d| { d.rework_target = Some("stage-1".into()); })),
        ("routes on schema 1", Box::new(|d| { d.schema = 1; d.max_reworks = 2; d.rework_target = Some("stage-1".into()); })),
        ("unknown schema", Box::new(|d| { d.schema = 3; })),
    ];
    for (name, edit) in cases { assert_eq!(bad(edit), Err(Refusal::Definition), "{name}"); }
}
#[test]
fn a_route_cannot_reach_a_step_that_needs_a_candidate_before_one_exists() {
    // The provision step jumps straight to the check: no build has made a candidate.
    // The build stays reachable through the check's own back edge, so only the candidate rule objects.
    let result = bad(|d| { d.stages[0].routes.insert("passed".into(), Target::Step("stage-2".into())); });
    assert_eq!(result, Err(Refusal::Definition));
}
#[test]
fn a_back_edge_to_a_build_needs_no_candidate_but_one_to_a_check_keeps_it() {
    let mut d = routed();
    d.stages[3].routes.insert("rejected".into(), Target::Step("stage-2".into()));
    assert_eq!(d.validate(), Ok(()));
    let mut state = to_check(start(d));
    state = go(state.clone(), pass(&state));
    let candidate = state.items["item"].candidate.clone();
    state = go(state.clone(), reject(&state));
    assert_eq!(stage(&state), "stage-2");
    assert_eq!(state.items["item"].candidate, candidate);
}
#[test]
fn the_state_validator_refuses_a_forged_back_edge_count() {
    let state = to_check(start(routed()));
    let mut forged = state.clone();
    forged.items.get_mut("item").unwrap().back_edges.insert("stage-2".into(), 3);
    let probe = pass(&forged);
    assert_eq!(step(&forged, &probe), Err(Refusal::Invalid));
}

fn failed() -> State {
    let mut state = start(Definition::coding());
    // Two attempts at the provision step, both unsafe: the run fails there.
    for _ in 0..2 { state = go(state.clone(), observe(&state, Outcome::SafeFailure)); }
    assert_eq!(state.items["item"].phase, Phase::Failed);
    state
}
fn retry(state: &State) -> Input { input(state, Command::Retry { item: "item".into() }, Grant::Controller) }
#[test]
fn a_controller_can_retry_a_failed_run_and_it_runs_the_same_step_again() {
    let state = failed();
    let old_run = state.items["item"].run.clone();
    let transition = step(&state, &retry(&state)).unwrap();
    assert_eq!(transition.effects.len(), 1);
    let item = &transition.state.items["item"];
    assert_eq!((item.phase.clone(), item.stage, item.attempt, item.retries), (Phase::Running, 0, 1, 1));
    assert_ne!(item.run, old_run);
    assert_eq!(step(&state, &retry(&state)).unwrap(), transition);
}
#[test]
fn only_a_controller_can_retry_and_only_a_failed_run() {
    let state = failed();
    let mut other = retry(&state); other.grant = Grant::Decision { principal: "owner".into() };
    assert_eq!(step(&state, &other), Err(Refusal::Unauthorized));
    let running = start(Definition::coding());
    assert_eq!(step(&running, &retry(&running)), Err(Refusal::Phase));
    let cancelled = go(state.clone(), input(&state, Command::Cancel { item: "item".into() }, Grant::Controller));
    assert_eq!(step(&cancelled, &retry(&cancelled)), Err(Refusal::Phase));
}
#[test]
fn retries_are_bounded() {
    let mut state = failed();
    for _ in 0..MAX_RETRIES {
        state = go(state.clone(), retry(&state));
        for _ in 0..2 { state = go(state.clone(), observe(&state, Outcome::SafeFailure)); }
        assert_eq!(state.items["item"].phase, Phase::Failed);
    }
    assert_eq!(step(&state, &retry(&state)), Err(Refusal::Limit));
}
#[test]
fn a_retry_waits_when_another_run_holds_the_resource() {
    let mut state = failed();
    state = go(state.clone(), enqueue(&state, Definition::coding(), "other", "repo/branch"));
    assert_eq!(state.items["other"].phase, Phase::Running);
    let state = go(state.clone(), retry(&state));
    assert_eq!(state.items["item"].phase, Phase::Blocked);
}
