//! One run on several tickets (ov-970): a lead runs, the rest follow it and close with it. Private fixtures only.
use farcooler_workflow::*;
use std::collections::BTreeSet;

struct Log { state: State, effects: Vec<Effect> }
impl Log {
    fn new() -> Self { Self { state: State::empty("workspace".into(), 1), effects: Vec::new() } }
    fn try_run(&mut self, command: Command, grant: Grant) -> Result<Transition, Refusal> {
        let s = &self.state;
        let input = Input { busy_resources: BTreeSet::new(), schema: 1, id: format!("command-{}", s.revision + 1),
            workspace: s.workspace.clone(), expected_revision: s.revision, epoch: s.epoch,
            source: "fixture".into(), source_sequence: s.revision + 1, now_ms: 1000, grant, command };
        let transition = step(&self.state, &input)?;
        self.state = transition.state.clone();
        self.effects = transition.effects.clone();
        Ok(transition)
    }
    fn track(&mut self, item: &str) {
        let d = Definition::ticket();
        self.try_run(Command::Enqueue { item: item.into(), policy_pin: "policy-1".into(), resource: format!("res-{item}"),
            dependencies: BTreeSet::new(), initial_candidate: Some(d.pin()), definition: d }, Grant::Controller).unwrap();
    }
    fn start(&mut self, item: &str, with: &[&str]) -> Result<Transition, Refusal> {
        self.try_run(Command::Start { item: item.into(), policy_pin: "policy-2".into(), resource: format!("run-{item}"),
            dependencies: BTreeSet::new(), initial_candidate: None, definition: coding(), models: Default::default(), tree: None,
            with: with.iter().map(|w| w.to_string()).collect() }, Grant::Controller)
    }
    fn observe(&mut self, item: &str, outcome: Outcome) {
        let effect = self.effects.iter().find(|e| e.item == item).expect("an effect in flight").clone();
        self.try_run(Command::Observe { item: item.into(), effect: effect.id.clone(), run: effect.run.clone(), fence: effect.fence, outcome },
            Grant::Operation { effect: effect.id, run: effect.run, fence: effect.fence }).unwrap();
    }
    fn phase(&self, item: &str) -> Phase { self.state.items[item].phase.clone() }
}
fn ev(candidate: &str) -> Evidence {
    Evidence { candidate: candidate.into(), manifest: "manifest".into(), receipt: "receipt".into(), policy_pin: "policy-2".into() }
}
fn coding() -> Definition {
    let op = OperationSpec { adapter_version: "coding/v1".into(), arguments: vec![], environment: Default::default() };
    let stage = |id: &str, kind| Stage { id: id.into(), kind, progression: Progression::Automatic, max_attempts: 1,
        operation: op.clone(), routes: Default::default(), limit: 0 };
    Definition { schema: 1, source: "coding/v1".into(), max_reworks: 0, rework_target: None, groups: Default::default(),
        models: Default::default(), send_backs: None,
        stages: vec![stage("build", StageKind::Build), stage("check", StageKind::Check), stage("land", StageKind::Land)] }
}
fn three() -> Log {
    let mut log = Log::new();
    for t in ["ov-1", "ov-2", "ov-3"] { log.track(t); }
    log
}

#[test]
fn one_run_carries_three_tickets_and_they_land_with_it() {
    let mut log = three();
    log.start("ov-1", &["ov-2", "ov-3"]).unwrap();
    assert_eq!(log.state.items["ov-1"].followers, ["ov-2", "ov-3"]);
    for f in ["ov-2", "ov-3"] {
        assert_eq!(log.state.items[f].follows.as_deref(), Some("ov-1"));
        assert_eq!(log.phase(f), Phase::WaitingApproval, "it has no run of its own");
    }
    assert_eq!(log.effects.len(), 1, "one run, one agent");
    assert_eq!(log.effects[0].with, ["ov-2", "ov-3"], "the build knows every ticket");
    // Its followers go where it goes: moving or starting one alone is refused.
    assert_eq!(log.try_run(Command::Move { item: "ov-2".into(), to: "done".into() }, Grant::Controller), Err(Refusal::Phase));
    assert_eq!(log.try_run(Command::Cancel { item: "ov-2".into() }, Grant::Controller), Err(Refusal::Phase));
    assert_eq!(log.start("ov-2", &[]), Err(Refusal::Phase));
    for outcome in [ev("c1"), ev("c1"), ev("c1")] { log.observe("ov-1", Outcome::Passed(outcome)); }
    for t in ["ov-1", "ov-2", "ov-3"] { assert_eq!(log.phase(t), Phase::Done, "{t}"); }
    assert!(log.state.items["ov-2"].follows.is_none());
    log.track("ov-4");
    assert!(["ov-1", "ov-2", "ov-3"].iter().all(|t| log.state.archive[*t].phase == Phase::Done), "all three leave live state");
    // Landed, a follower is a ticket of its own again: it reopens alone (ov-968).
    log.try_run(Command::Reopen { item: "ov-2".into(), definition: Definition::ticket_at("todo").unwrap(), policy_pin: "policy-1".into(),
        resource: "res-ov-2".into() }, Grant::Controller).unwrap();
    assert_eq!((log.phase("ov-2"), log.state.items["ov-2"].follows.clone()), (Phase::WaitingApproval, None));
}

#[test]
fn a_cancelled_run_hands_its_followers_back_and_a_failed_one_keeps_them() {
    let mut log = three();
    log.start("ov-1", &["ov-2"]).unwrap();
    log.observe("ov-1", Outcome::Passed(ev("c1")));
    log.observe("ov-1", Outcome::Rejected(ev("c1")));
    assert_eq!((log.phase("ov-1"), log.phase("ov-2")), (Phase::Failed, Phase::WaitingApproval));
    assert_eq!(log.state.items["ov-2"].follows.as_deref(), Some("ov-1"), "a failed run still has its tickets");
    let fence = log.state.items["ov-2"].fence;
    log.try_run(Command::Cancel { item: "ov-1".into() }, Grant::Controller).unwrap();
    // Cancelling the run drops its own ticket. The one it carried goes back to rest, where it was before.
    assert_eq!((log.phase("ov-1"), log.phase("ov-2")), (Phase::Cancelled, Phase::WaitingApproval));
    let ov2 = &log.state.items["ov-2"];
    assert_eq!((ov2.follows.clone(), ov2.stage, ov2.fence), (None, 0, fence + 1), "at rest, its old stage, fenced past the run");
    // A ticket of its own again: it moves and starts alone.
    log.start("ov-2", &[]).unwrap();
    assert_eq!(log.effects[0].item, "ov-2");
}

#[test]
fn followers_must_be_distinct_tickets_at_rest() {
    let mut log = three();
    assert_eq!(log.start("ov-1", &["ov-1"]), Err(Refusal::Invalid), "not itself");
    assert_eq!(log.start("ov-1", &["ov-2", "ov-2"]), Err(Refusal::Invalid), "once each");
    assert_eq!(log.start("ov-1", &["ov-9"]), Err(Refusal::Missing));
    let many: Vec<String> = (0..=MAX_GROUP_MEMBERS).map(|n| format!("x-{n}")).collect();
    assert_eq!(log.start("ov-1", &many.iter().map(String::as_str).collect::<Vec<_>>()), Err(Refusal::Invalid));
    log.start("ov-1", &["ov-2"]).unwrap();
    assert_eq!(log.start("ov-3", &["ov-2"]), Err(Refusal::Phase), "already following");
    assert_eq!(log.start("ov-3", &["ov-1"]), Err(Refusal::Phase), "a run, not a ticket at rest");
    assert!(log.state.items["ov-3"].follows.is_none() && log.phase("ov-3") == Phase::WaitingApproval, "a refusal changes nothing");
}
