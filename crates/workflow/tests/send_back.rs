//! Send a run back to its build step with a reason, and reopen a closed ticket (ov-968). Private fixtures only.
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
        self.try_run(Command::Start { item: item.into(), policy_pin: "policy-2".into(), resource: format!("run-{item}"),
            dependencies: BTreeSet::new(), initial_candidate: None, definition: d, models: Default::default(),
            tree: tree.map(Box::new), with: vec![] }, Grant::Controller).unwrap();
    }
    /// Answer the effect in flight for `item`.
    fn observe(&mut self, item: &str, outcome: Outcome) {
        let effect = self.effects.iter().find(|e| e.item == item).expect("an effect in flight").clone();
        self.try_run(Command::Observe { item: item.into(), effect: effect.id.clone(), run: effect.run.clone(), fence: effect.fence, outcome },
            Grant::Operation { effect: effect.id, run: effect.run, fence: effect.fence }).unwrap();
    }
    fn send_back(&mut self, item: &str, reason: &str) -> Result<Transition, Refusal> {
        self.try_run(Command::SendBack { item: item.into(), reason: reason.into() }, Grant::Controller)
    }
    fn reopen(&mut self, item: &str, column: &str) -> Result<Transition, Refusal> {
        self.try_run(Command::Reopen { item: item.into(), definition: Definition::ticket_at(column).unwrap(),
            policy_pin: "policy-1".into(), resource: format!("res-{item}") }, Grant::Controller)
    }
    fn step_id(&self, item: &str) -> String { let i = &self.state.items[item]; i.definition.stages[i.stage].id.clone() }
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
        models: Default::default(), send_backs: None,
        stages: vec![stage("build", StageKind::Build), stage("check", StageKind::Check),
            stage("review", StageKind::Review(Reviewer::Orchestrator("orchestrator".into()))), stage("land", StageKind::Land)] }
}
fn tree() -> RunTree {
    RunTree { name: "ov-1".into(), branch: "ov-1-fix".into(), path: "/trees/ov-1".into(), base: "a".repeat(40), adopted: true }
}
/// A ticket started on `d` and built and checked: it waits for the orchestrator's review.
fn at_review(d: Definition) -> Log {
    let mut log = Log::new();
    log.track("ov-1");
    log.start("ov-1", d, Some(tree()));
    log.observe("ov-1", Outcome::Passed(ev("c1")));
    log.observe("ov-1", Outcome::Passed(ev("c1")));
    assert_eq!(log.state.items["ov-1"].phase, Phase::WaitingReview);
    log
}

#[test]
fn a_send_back_builds_again_in_the_same_tree_and_the_build_is_told_why() {
    let mut log = at_review(coding());
    let reason = "The retry loop never ends when the remote refuses: bound it, and test the refusal.";
    let run = log.state.items["ov-1"].run.clone();
    log.send_back("ov-1", reason).unwrap();
    let item = &log.state.items["ov-1"];
    assert_eq!((log.step_id("ov-1"), &item.phase), ("build".into(), &Phase::Running));
    assert_eq!(item.candidate, None, "the build makes a new candidate");
    assert_eq!(item.tree, Some(tree()), "the same tree");
    assert_eq!(item.sent_back, vec![SendBack { id: "command-5".into(), revision: 4, run, from: "review".into(), to: "build".into(), reason: reason.into() }]);
    let build = &log.effects[0];
    assert_eq!((build.kind.clone(), build.brief.as_deref(), build.tree.clone()), (EffectKind::Execute(StageKind::Build), Some(reason), Some(tree())));
    // Once the build passes, it has answered why: nothing after it carries the reason, and the next build won't.
    log.observe("ov-1", Outcome::Passed(ev("c2")));
    assert_eq!(log.state.items["ov-1"].brief, None);
    assert_eq!(log.effects[0].kind, EffectKind::Execute(StageKind::Check));
    assert_eq!(log.effects[0].brief, None);
}

#[test]
fn a_send_back_is_taken_from_a_failed_step_and_a_rework_review_reaches_the_build_too() {
    let mut log = Log::new();
    log.track("ov-1");
    log.start("ov-1", coding(), None);
    log.observe("ov-1", Outcome::Passed(ev("c1")));
    log.observe("ov-1", Outcome::Rejected(ev("c1")));
    assert_eq!(log.state.items["ov-1"].phase, Phase::Failed);
    log.send_back("ov-1", "The check fails on Linux: the path separator.").unwrap();
    assert_eq!(log.step_id("ov-1"), "build");
    assert_eq!(log.effects[0].brief.as_deref(), Some("The check fails on Linux: the path separator."));

    // A review that asks for rework says why, and that reaches the build as well (it never did before ov-968).
    let mut d = coding();
    d.max_reworks = 1; d.rework_target = Some("build".into());
    let mut log = at_review(d);
    let run = log.state.items["ov-1"].run.clone();
    log.try_run(Command::Review { item: "ov-1".into(), run, evidence: ev("c1"), passed: false, reason: Some("Name the error.".into()) },
        Grant::Decision { principal: "orchestrator".into() }).unwrap();
    assert_eq!(log.step_id("ov-1"), "build");
    assert_eq!(log.effects[0].brief.as_deref(), Some("Name the error."));
}

#[test]
fn a_send_back_is_refused_mid_step_on_a_ticket_across_a_landing_past_the_limit_and_from_a_person() {
    let mut log = Log::new();
    log.track("ov-1");
    assert_eq!(log.send_back("ov-1", "why"), Err(Refusal::Phase), "a ticket at rest has no build to go back to");
    log.start("ov-1", coding(), None);
    assert_eq!(log.send_back("ov-1", "why"), Err(Refusal::Phase), "the build is running");
    assert_eq!(log.send_back("ov-9", "why"), Err(Refusal::Missing));

    let mut log = at_review(coding());
    let person = Grant::Decision { principal: "owner".into() };
    assert_eq!(log.try_run(Command::SendBack { item: "ov-1".into(), reason: "why".into() }, person), Err(Refusal::Unauthorized));
    for reason in ["", "   ", "bell\u{7}", &"x".repeat(MAX_REWORK_REASON_BYTES + 1)] {
        assert_eq!(log.send_back("ov-1", reason), Err(Refusal::Invalid), "{reason:?}");
    }

    // The workflow sets the limit.
    let mut d = coding();
    d.send_backs = Some(1);
    let mut log = at_review(d);
    log.send_back("ov-1", "first").unwrap();
    log.observe("ov-1", Outcome::Passed(ev("c2")));
    log.observe("ov-1", Outcome::Passed(ev("c2")));
    assert_eq!(log.send_back("ov-1", "second"), Err(Refusal::Limit));
    let mut d = coding();
    d.send_backs = Some(MAX_SEND_BACKS + 1);
    assert_eq!(d.validate(), Err(Refusal::Definition));
    assert_eq!(coding().send_back_limit(), DEFAULT_SEND_BACKS);

    // What landed is not built and landed again.
    let mut log = at_review(coding());
    let run = log.state.items["ov-1"].run.clone();
    log.try_run(Command::Review { item: "ov-1".into(), run, evidence: ev("c1"), passed: true, reason: None },
        Grant::Decision { principal: "orchestrator".into() }).unwrap();
    log.observe("ov-1", Outcome::Rejected(ev("c1")));
    assert_eq!((log.step_id("ov-1"), &log.state.items["ov-1"].phase), ("land".into(), &Phase::Failed));
    assert_eq!(log.send_back("ov-1", "why"), Err(Refusal::Phase));
}

#[test]
fn a_closed_ticket_reopens_under_its_own_name_and_can_start_again() {
    for closed in ["done", "cancelled"] {
        let mut log = Log::new();
        log.track("ov-1");
        log.try_run(Command::Move { item: "ov-1".into(), to: closed.into() }, Grant::Controller).unwrap();
        // Closed this step and not yet archived: it reopens, and the archive sweep leaves it alone.
        log.reopen("ov-1", "doing").unwrap();
        assert_eq!((log.step_id("ov-1"), &log.state.items["ov-1"].phase), ("doing".into(), &Phase::WaitingApproval));
        log.track("ov-2");
        assert!(log.state.items.contains_key("ov-1") && !log.state.archive.contains_key("ov-1"));

        // Archived: it comes back out.
        log.try_run(Command::Move { item: "ov-1".into(), to: closed.into() }, Grant::Controller).unwrap();
        log.track("ov-3");
        assert!(log.state.archive.contains_key("ov-1"));
        log.reopen("ov-1", "todo").unwrap();
        assert!(!log.state.archive.contains_key("ov-1"));
        assert_eq!((log.step_id("ov-1"), &log.state.items["ov-1"].phase), ("todo".into(), &Phase::WaitingApproval));
        log.start("ov-1", coding(), Some(tree()));
        assert_eq!(log.effects[0].tree, Some(tree()), "a reopened ticket starts on its branch");
    }
}

#[test]
fn a_reopen_is_refused_on_an_open_ticket_an_unknown_one_a_run_definition_and_from_a_person() {
    let mut log = Log::new();
    log.track("ov-1");
    assert_eq!(log.reopen("ov-1", "todo"), Err(Refusal::Phase));
    assert_eq!(log.reopen("ov-9", "todo"), Err(Refusal::Missing));
    log.try_run(Command::Move { item: "ov-1".into(), to: "done".into() }, Grant::Controller).unwrap();
    log.track("ov-2");
    let person = Grant::Decision { principal: "owner".into() };
    let reopen = |d: Definition| Command::Reopen { item: "ov-1".into(), definition: d, policy_pin: "policy-1".into(), resource: "res-ov-1".into() };
    assert_eq!(log.try_run(reopen(Definition::ticket()), person), Err(Refusal::Unauthorized));
    assert_eq!(log.try_run(reopen(coding()), Grant::Controller), Err(Refusal::Invalid));
    assert!(log.state.archive.contains_key("ov-1"), "a refusal changes nothing");
}

#[test]
fn a_routed_workflow_sends_back_and_clears_the_reason_when_its_build_passes() {
    let mut d = coding();
    d.schema = 2;
    d.stages.truncate(2);
    d.validate().unwrap();
    let mut log = Log::new();
    log.track("ov-1");
    log.start("ov-1", d, None);
    log.observe("ov-1", Outcome::Passed(ev("c1")));
    log.observe("ov-1", Outcome::Rejected(ev("c1")));
    assert_eq!(log.state.items["ov-1"].phase, Phase::Failed);
    log.send_back("ov-1", "Again, smaller.").unwrap();
    assert_eq!(log.effects[0].brief.as_deref(), Some("Again, smaller."));
    log.observe("ov-1", Outcome::Passed(ev("c2")));
    assert_eq!((log.step_id("ov-1"), log.state.items["ov-1"].brief.clone()), ("check".into(), None));
}
