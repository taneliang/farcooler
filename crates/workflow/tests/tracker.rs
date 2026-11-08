//! Tracker-only tickets (ov-951): a ticket with no run is an engine item. The orchestrator moves it
//! between columns with `Move` and starts a run on it with `Start`. Private fixtures only.
use farcooler_workflow::*;
use std::collections::BTreeSet;

struct Log { state: State }
impl Log {
    fn new() -> Self { Self { state: State::empty("workspace".into(), 1) } }
    fn input(&self, command: Command, grant: Grant) -> Input {
        let s = &self.state;
        Input { busy_resources: BTreeSet::new(), schema: 1, id: format!("command-{}", s.revision + 1),
            workspace: s.workspace.clone(), expected_revision: s.revision, epoch: s.epoch,
            source: "fixture".into(), source_sequence: s.revision + 1, now_ms: 1000, grant, command }
    }
    fn try_run(&mut self, command: Command, grant: Grant) -> Result<Transition, Refusal> {
        let input = self.input(command, grant);
        let transition = step(&self.state, &input)?;
        assert_eq!(step(&self.state, &input).unwrap(), transition, "a step is a pure function");
        self.state = transition.state.clone();
        Ok(transition)
    }
    fn track(&mut self, item: &str) {
        let d = Definition::ticket();
        self.try_run(Command::Enqueue { item: item.into(), policy_pin: "policy-1".into(), resource: format!("res-{item}"),
            dependencies: BTreeSet::new(), initial_candidate: Some(d.pin()), definition: d }, Grant::Controller).unwrap();
    }
    fn mv(&mut self, item: &str, to: &str) -> Result<Transition, Refusal> {
        self.try_run(Command::Move { item: item.into(), to: to.into() }, Grant::Controller)
    }
    fn step_id(&self, item: &str) -> String {
        let i = &self.state.items[item]; i.definition.stages[i.stage].id.clone()
    }
}
fn coding() -> Definition {
    let op = |a: &str| OperationSpec { adapter_version: a.into(), arguments: vec![], environment: Default::default() };
    let stage = |id: &str, kind| Stage { id: id.into(), kind, progression: Progression::Automatic, max_attempts: 1,
        operation: op("coding/v1"), routes: Default::default(), limit: 0 };
    Definition { schema: 1, source: "coding/v1".into(), max_reworks: 0, rework_target: None, groups: Default::default(), models: Default::default(),
        stages: vec![stage("build", StageKind::Build), stage("check", StageKind::Check)] }
}
fn start(item: &str, d: Definition) -> Command { start_with(item, d, &[]) }
/// A Start that names a model for some of the steps (ov-966).
fn start_with(item: &str, d: Definition, picks: &[(&str, &str)]) -> Command {
    Command::Start { item: item.into(), policy_pin: "policy-2".into(), resource: format!("run-{item}"),
        dependencies: BTreeSet::new(), initial_candidate: None, definition: d,
        models: picks.iter().map(|(s, m)| (s.to_string(), m.to_string())).collect() }
}
/// The coding definition, with its build step listing the models it may run with: the first is the default.
fn coding_with_models(models: &[&str]) -> Definition {
    let mut d = coding();
    d.models.insert("build".into(), models.iter().map(|m| m.to_string()).collect());
    d
}

#[test]
fn the_template_is_valid_and_tracker_only() {
    let d = Definition::ticket();
    d.validate().unwrap();
    assert!(d.stages.iter().all(|s| s.kind == StageKind::Tracker));
    let ids: Vec<_> = d.stages.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["backlog", "todo", "doing", "review", "decision"]);
}

#[test]
fn a_tracked_ticket_waits_in_its_first_column() {
    let mut log = Log::new();
    log.track("ov-1");
    let item = &log.state.items["ov-1"];
    assert_eq!(item.phase, Phase::WaitingApproval);
    assert_eq!(log.step_id("ov-1"), "backlog");
}

#[test]
fn the_orchestrator_moves_a_ticket_to_any_column_and_back() {
    let mut log = Log::new();
    log.track("ov-1");
    let run = log.state.items["ov-1"].run.clone();
    for to in ["doing", "review", "todo", "decision", "backlog"] {
        log.mv("ov-1", to).unwrap();
        assert_eq!(log.step_id("ov-1"), to);
        assert_eq!(log.state.items["ov-1"].phase, Phase::WaitingApproval);
    }
    assert_ne!(log.state.items["ov-1"].run, run, "each move is a new run, so an old approval cannot be replayed");
}

#[test]
fn done_and_cancelled_close_the_ticket_for_good() {
    for to in ["done", "cancelled"] {
        let mut log = Log::new();
        log.track("ov-1");
        log.mv("ov-1", to).unwrap();
        let want = if to == "done" { Phase::Done } else { Phase::Cancelled };
        assert_eq!(log.state.items["ov-1"].phase, want);
        // It leaves live state on the next step, and a name stays taken.
        log.track("ov-2");
        assert!(!log.state.items.contains_key("ov-1"));
        assert_eq!(log.state.archive["ov-1"].phase, want);
        assert_eq!(log.mv("ov-1", "todo"), Err(Refusal::Missing));
    }
}

#[test]
fn a_move_needs_the_controller_a_real_column_and_a_different_one() {
    let mut log = Log::new();
    log.track("ov-1");
    let person = Grant::Decision { principal: "owner".into() };
    assert_eq!(log.try_run(Command::Move { item: "ov-1".into(), to: "doing".into() }, person), Err(Refusal::Unauthorized));
    assert_eq!(log.mv("ov-1", "nowhere"), Err(Refusal::Invalid));
    assert_eq!(log.mv("ov-1", "backlog"), Err(Refusal::Invalid), "already there");
    assert_eq!(log.mv("ov-9", "doing"), Err(Refusal::Missing));
}

#[test]
fn a_run_item_cannot_be_moved_as_a_column() {
    let mut log = Log::new();
    let d = coding();
    log.try_run(Command::Enqueue { item: "ov-2".into(), policy_pin: "policy-1".into(), resource: "res-2".into(),
        dependencies: BTreeSet::new(), initial_candidate: None, definition: d }, Grant::Controller).unwrap();
    assert_eq!(log.mv("ov-2", "done"), Err(Refusal::Phase));
}

#[test]
fn starting_a_run_gives_the_same_item_a_real_definition() {
    let mut log = Log::new();
    log.track("ov-1");
    log.mv("ov-1", "doing").unwrap();
    let d = coding();
    log.try_run(start("ov-1", d.clone()), Grant::Controller).unwrap();
    let item = &log.state.items["ov-1"];
    assert_eq!(item.pin, d.pin());
    assert_eq!(item.policy_pin, "policy-2");
    assert_eq!(item.resource, "run-ov-1");
    assert_eq!(item.stage, 0);
    assert_eq!(item.phase, Phase::Running, "it is ready at once, so it begins");
    assert!(item.pending.is_some());
    assert_eq!(item.iteration, 0);
    assert!(item.candidate.is_none());
}

#[test]
fn a_start_waits_for_a_busy_resource_like_any_enqueue() {
    let mut log = Log::new();
    log.track("ov-1");
    log.try_run(Command::Enqueue { item: "ov-0".into(), policy_pin: "policy-2".into(), resource: "run-ov-1".into(),
        dependencies: BTreeSet::new(), initial_candidate: None, definition: coding() }, Grant::Controller).unwrap();
    assert_eq!(log.try_run(start("ov-1", coding()), Grant::Controller), Err(Refusal::Resource));
}

#[test]
fn only_a_tracker_only_ticket_can_start_and_only_with_a_real_definition() {
    let mut log = Log::new();
    log.track("ov-1");
    let person = Grant::Decision { principal: "owner".into() };
    assert_eq!(log.try_run(start("ov-1", coding()), person), Err(Refusal::Unauthorized));
    assert_eq!(log.try_run(start("ov-1", Definition::ticket()), Grant::Controller), Err(Refusal::Invalid), "a tracker is not a run");
    assert_eq!(log.try_run(Command::Start { item: "ov-1".into(), policy_pin: "p".into(), resource: "r".into(),
        dependencies: BTreeSet::new(), initial_candidate: Some("x".into()), definition: coding(), models: Default::default() }, Grant::Controller),
        Err(Refusal::Evidence), "a build makes its own candidate");
    log.try_run(start("ov-1", coding()), Grant::Controller).unwrap();
    assert_eq!(log.try_run(start("ov-1", coding()), Grant::Controller), Err(Refusal::Phase), "a run cannot start twice");
    assert_eq!(log.try_run(start("ov-7", coding()), Grant::Controller), Err(Refusal::Missing));
}

#[test]
fn a_ticket_that_started_cannot_be_moved_by_column() {
    let mut log = Log::new();
    log.track("ov-1");
    log.try_run(start("ov-1", coding()), Grant::Controller).unwrap();
    assert_eq!(log.mv("ov-1", "todo"), Err(Refusal::Phase));
}

#[test]
fn a_ticket_can_begin_in_the_column_its_card_is_already_in() {
    let mut log = Log::new();
    let d = Definition::ticket_at("doing").unwrap();
    d.validate().unwrap();
    assert!(d.is_ticket());
    assert_ne!(d.pin(), Definition::ticket().pin());
    assert!(Definition::ticket_at("nowhere").is_none());
    log.try_run(Command::Enqueue { item: "ov-1".into(), policy_pin: "policy-1".into(), resource: "res".into(),
        dependencies: BTreeSet::new(), initial_candidate: Some(d.pin()), definition: d }, Grant::Controller).unwrap();
    assert_eq!(log.step_id("ov-1"), "doing");
    log.mv("ov-1", "backlog").unwrap();
    log.mv("ov-1", "decision").unwrap();
    assert_eq!(log.step_id("ov-1"), "decision");
}

/// ov-966: the orchestrator picks each ticket's model at Start, from the ones the workflow lists, and the run
/// records it. The build's effect carries it, so the launch reads it from the effect alone.
#[test]
fn two_tickets_on_one_workflow_start_with_different_models_and_each_records_its_own() {
    let mut log = Log::new();
    let d = coding_with_models(&["claude:opus", "codex:gpt-5.5"]);
    d.validate().unwrap();
    log.track("ov-1");
    log.track("ov-2");
    let first = log.try_run(start_with("ov-1", d.clone(), &[("build", "codex:gpt-5.5")]), Grant::Controller).unwrap();
    let second = log.try_run(start_with("ov-2", d.clone(), &[]), Grant::Controller).unwrap();
    assert_eq!(log.state.items["ov-1"].models["build"], "codex:gpt-5.5");
    assert_eq!(log.state.items["ov-2"].models["build"], "claude:opus", "a step not named runs with its default");
    assert_eq!(first.effects[0].model.as_deref(), Some("codex:gpt-5.5"));
    assert_eq!(second.effects[0].model.as_deref(), Some("claude:opus"));
}

#[test]
fn a_start_refuses_a_model_the_step_does_not_list_or_a_step_that_lists_none() {
    let mut log = Log::new();
    log.track("ov-1");
    let d = coding_with_models(&["claude:opus"]);
    for picks in [[("build", "claude:sonnet")], [("check", "claude:opus")], [("nowhere", "claude:opus")]] {
        assert_eq!(log.try_run(start_with("ov-1", d.clone(), &picks), Grant::Controller), Err(Refusal::Invalid), "{picks:?}");
    }
    assert_eq!(log.try_run(start_with("ov-1", coding(), &[("build", "claude:opus")]), Grant::Controller), Err(Refusal::Invalid),
        "a workflow that lists no models takes no pick");
    log.try_run(start_with("ov-1", coding(), &[]), Grant::Controller).unwrap();
    assert!(log.state.items["ov-1"].models.is_empty(), "nothing listed, nothing recorded: the configured harness runs");
}

#[test]
fn a_workflow_lists_models_only_on_a_build_step_and_only_plain_names() {
    let with = |step: &str, models: &[&str]| {
        let mut d = coding();
        d.models.insert(step.into(), models.iter().map(|m| m.to_string()).collect());
        d.validate()
    };
    assert_eq!(with("build", &["claude", "claude:opus", "codex:gpt-5.5", "cursor:auto"]), Ok(()));
    for (step, models) in [("check", vec!["claude:opus"]), ("nowhere", vec!["claude:opus"]), ("build", vec![]),
        ("build", vec!["claude:opus", "claude:opus"]), ("build", vec!["claude opus"]), ("build", vec![""]),
        ("build", vec!["claude:op;us"]), ("build", vec!["a:b:c"]), ("build", vec![":opus"]), ("build", vec!["claude:"])]
    {
        assert_eq!(with(step, &models), Err(Refusal::Definition), "{step} {models:?}");
    }
    let many: Vec<String> = (0..9).map(|i| format!("claude:m{i}")).collect();
    assert_eq!(with("build", &many.iter().map(String::as_str).collect::<Vec<_>>()), Err(Refusal::Definition), "at most 8");
    assert_eq!(coding().pin(), { let mut d = coding(); d.models.clear(); d.pin() }, "no list, the same pin as before");
}
