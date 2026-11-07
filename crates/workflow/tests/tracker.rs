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
    Definition { schema: 1, source: "coding/v1".into(), max_reworks: 0, rework_target: None, groups: Default::default(),
        stages: vec![stage("build", StageKind::Build), stage("check", StageKind::Check)] }
}
fn start(item: &str, d: Definition) -> Command {
    Command::Start { item: item.into(), policy_pin: "policy-2".into(), resource: "res-repo".into(),
        dependencies: BTreeSet::new(), initial_candidate: None, definition: d }
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
    assert_eq!(item.resource, "res-repo");
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
    log.try_run(Command::Enqueue { item: "ov-0".into(), policy_pin: "policy-2".into(), resource: "res-repo".into(),
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
        dependencies: BTreeSet::new(), initial_candidate: Some("x".into()), definition: coding() }, Grant::Controller),
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
