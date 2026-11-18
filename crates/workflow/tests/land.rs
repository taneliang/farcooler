//! Close a failed run as landed (ov-1231). Private fixtures only.
use farcooler_workflow::*;
use std::collections::BTreeSet;

struct Log { state: State, effects: Vec<Effect> }
impl Log {
    fn new() -> Self { Self { state: State::empty("workspace".into(), 1), effects: Vec::new() } }
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
        self.effects = transition.effects.clone();
        Ok(transition)
    }
    fn track(&mut self, item: &str) {
        let d = Definition::ticket();
        self.try_run(Command::Enqueue { item: item.into(), policy_pin: "policy-1".into(), resource: format!("res-{item}"),
            dependencies: BTreeSet::new(), initial_candidate: Some(d.pin()), definition: d }, Grant::Controller).unwrap();
    }
    fn start(&mut self, item: &str, d: Definition, tree: Option<RunTree>) {
        self.try_run(Command::Start { start_load: None, item: item.into(), policy_pin: "policy-2".into(), resource: format!("run-{item}"),
            dependencies: BTreeSet::new(), initial_candidate: None, definition: d, models: Default::default(),
            tree: tree.map(Box::new), with: vec![] }, Grant::Controller).unwrap();
    }
    /// Answer the effect in flight for `item`.
    fn observe(&mut self, item: &str, outcome: Outcome) {
        let effect = self.effects.iter().find(|e| e.item == item).expect("an effect in flight").clone();
        self.try_run(Command::Observe { item: item.into(), effect: effect.id.clone(), run: effect.run.clone(), fence: effect.fence, outcome },
            Grant::Operation { effect: effect.id, run: effect.run, fence: effect.fence }).unwrap();
    }
}
fn ev(candidate: &str) -> Evidence {
    Evidence { candidate: candidate.into(), manifest: "manifest".into(), receipt: "receipt".into(), policy_pin: "policy-2".into() }
}
/// Build, check, the orchestrator's review, land: the shape a coding workflow has.
fn coding() -> Definition {
    let op = OperationSpec { adapter_version: "coding/v1".into(), arguments: vec![], environment: Default::default() };
    let stage = |id: &str, kind| Stage { id: id.into(), kind, progression: Progression::Automatic, max_attempts: 1,
        operation: op.clone(), routes: Default::default(), limit: 0 };
    Definition { schema: 1, source: "coding/v1".into(), max_reworks: 0, rework_target: None, groups: Default::default(),
        models: Default::default(), send_backs: None, build_folders: None, script_policies: Default::default(), start_policy: None, limits: None, policies: Default::default(),
        stages: vec![stage("build", StageKind::Build), stage("check", StageKind::Check),
            stage("review", StageKind::Review(Reviewer::Orchestrator("orchestrator".into()))), stage("land", StageKind::Land)] }
}

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
impl Log {
    fn land(&mut self, item: &str, commit: &str) -> Result<Transition, Refusal> {
        self.try_run(Command::Land { item: item.into(), commit: commit.into() }, Grant::Controller)
    }
    fn failed() -> Self {
        let mut log = Log::new();
        log.track("ov-1");
        log.start("ov-1", coding(), None);
        log.observe("ov-1", Outcome::Passed(ev("c1")));
        log.observe("ov-1", Outcome::Rejected(ev("c1")));
        assert_eq!(log.state.items["ov-1"].phase, Phase::Failed);
        log
    }
}

#[test]
fn a_failed_run_lands_with_the_commit_and_leaves_live_state_next_step() {
    let mut log = Log::failed();
    let fence = log.state.items["ov-1"].fence;
    let transition = log.land("ov-1", SHA).unwrap();
    assert!(transition.effects.is_empty(), "closing a run starts nothing");
    let item = &log.state.items["ov-1"];
    assert_eq!((item.phase.clone(), item.candidate.as_deref(), item.stage, item.failure.clone()),
        (Phase::Done, Some(SHA), item.definition.stages.len(), None));
    assert!(item.fence > fence, "a late answer from the failed step is stale");
    // The run is archived on the next step, keeping the commit it landed as.
    log.try_run(Command::Advance, Grant::Controller).unwrap();
    assert!(!log.state.items.contains_key("ov-1"));
    assert_eq!((log.state.archive["ov-1"].phase.clone(), log.state.archive["ov-1"].candidate.as_deref()), (Phase::Done, Some(SHA)));
}

#[test]
fn landing_takes_the_controller_and_a_real_commit() {
    let mut log = Log::failed();
    assert_eq!(log.land("ov-1", "abc123"), Err(Refusal::Invalid));
    assert_eq!(log.land("ov-1", &"z".repeat(40)), Err(Refusal::Invalid));
    assert_eq!(log.try_run(Command::Land { item: "ov-1".into(), commit: SHA.into() },
        Grant::Decision { principal: "orchestrator".into() }), Err(Refusal::Unauthorized));
    assert_eq!(log.land("nope", SHA), Err(Refusal::Missing));
    assert_eq!(log.state.items["ov-1"].phase, Phase::Failed, "a refusal changes nothing");
}

/// Every phase a run can be in: only Failed lands, and the rest are refused with the run untouched.
#[test]
fn landing_is_refused_in_every_phase_but_failed() {
    let mut log = Log::new();
    log.track("ov-1");
    log.start("ov-1", coding(), None);
    assert_eq!(log.state.items["ov-1"].phase, Phase::Running);
    assert_eq!(log.land("ov-1", SHA), Err(Refusal::Phase), "Running");
    log.observe("ov-1", Outcome::Passed(ev("c1")));
    log.observe("ov-1", Outcome::Passed(ev("c1")));
    assert_eq!(log.state.items["ov-1"].phase, Phase::WaitingReview);
    assert_eq!(log.land("ov-1", SHA), Err(Refusal::Phase), "WaitingReview");
    // A ticket at rest is WaitingApproval.
    let mut log = Log::new();
    log.track("ov-2");
    assert_eq!(log.state.items["ov-2"].phase, Phase::WaitingApproval);
    assert_eq!(log.land("ov-2", SHA), Err(Refusal::Phase), "WaitingApproval");
    // Uncertain.
    let mut log = Log::new();
    log.track("ov-1");
    log.start("ov-1", coding(), None);
    log.observe("ov-1", Outcome::Uncertain);
    assert_eq!(log.state.items["ov-1"].phase, Phase::Uncertain);
    assert_eq!(log.land("ov-1", SHA), Err(Refusal::Phase), "Uncertain");
    // Cancelled, then Done (landed twice).
    let mut log = Log::failed();
    log.try_run(Command::Cancel { item: "ov-1".into(), reason: None }, Grant::Controller).unwrap();
    assert_eq!(log.state.items["ov-1"].phase, Phase::Cancelled);
    assert_eq!(log.land("ov-1", SHA), Err(Refusal::Phase), "Cancelled");
    let mut log = Log::failed();
    log.land("ov-1", SHA).unwrap();
    assert_eq!(log.land("ov-1", SHA), Err(Refusal::Phase), "Done");
    log.try_run(Command::Advance, Grant::Controller).unwrap();
    assert_eq!(log.land("ov-1", SHA), Err(Refusal::Missing), "archived");
}

/// A Failed run that leads other tickets lands them with it, as a landed run does.
#[test]
fn a_landed_run_takes_the_tickets_that_follow_it() {
    let mut log = Log::new();
    log.track("ov-1");
    log.track("ov-2");
    log.try_run(Command::Start { start_load: None, item: "ov-1".into(), policy_pin: "policy-2".into(), resource: "run-ov-1".into(),
        dependencies: BTreeSet::new(), initial_candidate: None, definition: coding(), models: Default::default(),
        tree: None, with: vec!["ov-2".into()] }, Grant::Controller).unwrap();
    log.observe("ov-1", Outcome::Passed(ev("c1")));
    log.observe("ov-1", Outcome::Rejected(ev("c1")));
    assert_eq!(log.land("ov-2", SHA), Err(Refusal::Phase), "a follower lands with its lead, not alone");
    log.land("ov-1", SHA).unwrap();
    assert_eq!(log.state.items["ov-2"].phase, Phase::Done);
}
