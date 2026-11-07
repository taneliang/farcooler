//! Groups: tickets queued by the orchestrator run as one item, fan out, and split. Private fixtures only.
use farcooler_workflow::*;
use std::collections::{BTreeMap, BTreeSet};

struct Log { state: State, inputs: Vec<Input> }
impl Log {
    fn new() -> Self { Self { state: State::empty("workspace".into(), 1), inputs: vec![] } }
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
        self.state = transition.state.clone(); self.inputs.push(input);
        Ok(transition)
    }
    fn run(&mut self, command: Command, grant: Grant) -> Transition { self.try_run(command, grant).unwrap() }
    fn enqueue(&mut self, item: &str, definition: Definition) {
        self.run(Command::Enqueue { item: item.into(), definition, policy_pin: "policy-1".into(),
            resource: format!("res-{item}"), dependencies: BTreeSet::new(), initial_candidate: None }, Grant::Controller);
    }
    fn evidence(&self, item: &str) -> Evidence {
        let i = &self.state.items[item];
        Evidence { candidate: i.candidate.clone().unwrap_or(format!("cand-{item}")), manifest: "m".into(),
            receipt: "r".into(), policy_pin: i.policy_pin.clone() }
    }
    fn outcome(&mut self, item: &str, outcome: Outcome) -> Transition {
        let i = &self.state.items[item];
        let (effect, run, fence) = (i.pending.clone().unwrap(), i.run.clone(), i.fence);
        self.run(Command::Observe { item: item.into(), effect: effect.clone(), run: run.clone(), fence, outcome },
            Grant::Operation { effect, run, fence })
    }
    fn pass(&mut self, item: &str) -> Transition { let e = self.evidence(item); self.outcome(item, Outcome::Passed(e)) }
    fn reject(&mut self, item: &str) -> Transition { let e = self.evidence(item); self.outcome(item, Outcome::Rejected(e)) }
    fn label(&mut self, item: &str, label: &str) -> Transition {
        let e = self.evidence(item); self.outcome(item, Outcome::Labeled(label.into(), e))
    }
    fn queue(&mut self, item: &str) -> Transition { self.run(Command::Queue { item: item.into() }, Grant::Controller) }
    fn stage(&self, item: &str) -> String {
        let i = &self.state.items[item]; i.definition.stages[i.stage].id.clone()
    }
    /// Run a ticket until it waits at its join step.
    fn to_join(&mut self, item: &str) {
        self.enqueue(item, ticket());
        while self.state.items[item].phase != Phase::WaitingJoin { self.pass(item); }
    }
    fn groups(&self) -> Vec<(&String, &Item)> {
        self.state.items.iter().filter(|(_, i)| i.group_of.is_some()).collect()
    }
    fn group_of(&self, member: &str) -> String { self.state.items[member].grouped_in.clone().unwrap() }
    fn replay_digest(&self) -> String {
        let mut state = State::empty("workspace".into(), 1);
        for input in &self.inputs { state = step(&state, input).unwrap().state; }
        digest(&state)
    }
}
fn action(id: &str) -> Stage {
    Stage { id: id.into(), kind: StageKind::Action(id.into()), progression: Progression::Automatic, max_attempts: 1,
        operation: OperationSpec { adapter_version: "fixture/v1".into(), arguments: vec![id.into()], environment: BTreeMap::new() },
        routes: BTreeMap::new(), limit: 0 }
}
fn split_to_base(mut stage: Stage) -> Stage {
    stage.limit = 3; stage
}
/// provision, build, check, join(landing), tidy. The landing group folds and gates what it holds.
fn ticket() -> Definition {
    let mut d = Definition::coding();
    d.schema = 2; d.max_reworks = 0; d.rework_target = None;
    d.stages.truncate(3);
    d.stages[2].routes.insert("rejected".into(), Target::Step("stage-1".into()));
    d.stages[2].limit = 2;
    let mut join = action("join"); join.kind = StageKind::Join { group: "landing".into(), adopt: true };
    join.routes.insert("rejected".into(), Target::Step("stage-1".into())); join.limit = 1;
    let mut tidy = action("tidy"); tidy.kind = StageKind::Ci;
    d.stages.push(join); d.stages.push(tidy);
    let base = action("base");
    let mut fold = split_to_base(action("fold"));
    fold.routes.insert("conflict".into(), Target::Split { step: "base".into(), otherwise: Box::new(Target::Fail) });
    let mut gate = split_to_base(action("gate")); gate.kind = StageKind::Check;
    gate.routes.insert("rejected".into(), Target::Split { step: "base".into(), otherwise: Box::new(Target::Fail) });
    d.groups.insert("landing".into(), GroupDef { limit: 2, exclusive: "ref:origin:main".into(), steps: vec![base, fold, gate] });
    d
}
/// Base passes, the fold makes `result`, the gate passes it: the group is done.
fn finish_with(log: &mut Log, group: &str, result: &str) -> Transition {
    log.pass(group);
    let e = Evidence { candidate: result.into(), ..log.evidence(group) };
    log.outcome(group, Outcome::Passed(e));
    log.pass(group)
}
fn pass_group(log: &mut Log, group: &str, steps: usize) { for _ in 0..steps { log.pass(group); } }

#[test]
fn a_ticket_at_its_join_step_waits_for_the_orchestrator_and_frees_its_resource() {
    let mut log = Log::new();
    log.to_join("a");
    let item = &log.state.items["a"];
    assert_eq!((item.phase.clone(), item.pending.clone()), (Phase::WaitingJoin, None));
    assert!(!owns_resource(item));
    assert!(log.groups().is_empty(), "nothing joins on its own");
    // Another ticket may take the freed resource.
    let definition = ticket();
    log.run(Command::Enqueue { item: "other".into(), definition, policy_pin: "policy-1".into(),
        resource: "res-a".into(), dependencies: BTreeSet::new(), initial_candidate: None }, Grant::Controller);
    assert_eq!(log.state.items["other"].phase, Phase::Running);
}
#[test]
fn only_the_controller_queues_and_only_a_ticket_at_a_join_step() {
    let mut log = Log::new();
    log.enqueue("early", ticket());
    assert_eq!(log.try_run(Command::Queue { item: "early".into() }, Grant::Controller).unwrap_err(), Refusal::Phase);
    log.to_join("a");
    let owner = Grant::Decision { principal: "owner".into() };
    assert_eq!(log.try_run(Command::Queue { item: "a".into() }, owner).unwrap_err(), Refusal::Unauthorized);
    log.queue("a");
    assert_eq!(log.try_run(Command::Queue { item: "a".into() }, Grant::Controller).unwrap_err(), Refusal::Phase);
}
#[test]
fn queueing_opens_a_group_that_leaves_at_once_when_nothing_is_ahead() {
    let mut log = Log::new();
    log.to_join("a");
    let transition = log.queue("a");
    assert_eq!(transition.effects.len(), 1);
    let effect = &transition.effects[0];
    assert_eq!(effect.members, vec![Member { item: "a".into(), candidate: log.state.items["a"].candidate.clone().unwrap() }]);
    assert_eq!(effect.resource, "ref:origin:main");
    assert_eq!(log.state.items["a"].phase, Phase::Joined);
    let (id, group) = log.groups()[0];
    assert_eq!((group.phase.clone(), group.members.len()), (Phase::Running, 1));
    assert_eq!(log.group_of("a"), *id);
    assert_eq!(group.group_of.as_ref().unwrap().name, "landing");
}
#[test]
fn tickets_queued_during_a_wait_accumulate_up_to_the_limit_then_a_new_group_opens() {
    let mut log = Log::new();
    for t in ["a", "b", "c", "d"] { log.to_join(t); }
    log.queue("a");
    let first = log.group_of("a");
    for t in ["b", "c", "d"] { log.queue(t); }
    assert_eq!(log.group_of("b"), log.group_of("c"), "two fit in one group");
    assert_ne!(log.group_of("d"), log.group_of("b"), "the limit is two");
    assert_ne!(log.group_of("b"), first, "a group that has left is closed");
    let behind = &log.state.items[&log.group_of("b")];
    assert_eq!((behind.phase.clone(), behind.members.len()), (Phase::Blocked, 2));
    assert_eq!(log.state.items[&log.group_of("d")].phase, Phase::Blocked);
    // First in, first out: the ids sort in the order the groups opened.
    assert!(log.group_of("b") < log.group_of("d"));
}
#[test]
fn a_ticket_without_a_candidate_cannot_be_queued() {
    let mut log = Log::new();
    log.to_join("a");
    log.state.items.get_mut("a").unwrap().candidate = None;
    assert_eq!(log.try_run(Command::Queue { item: "a".into() }, Grant::Controller).unwrap_err(), Refusal::Invalid);
}
#[test]
fn a_finished_group_fans_out_and_a_member_adopts_the_result() {
    let mut log = Log::new();
    log.to_join("a"); log.queue("a");
    let group = log.group_of("a");
    let last = finish_with(&mut log, &group, "folded-result");
    assert_eq!(log.state.items["a"].candidate.as_deref(), Some("folded-result"), "adopted");
    assert_eq!(log.stage("a"), "tidy");
    assert_eq!(log.state.items["a"].phase, Phase::Running, "its resource was free, so it went on");
    assert!(last.effects.iter().any(|e| e.item == "a"));
    assert_eq!(log.state.items[&group].members, vec![]);
}
#[test]
fn a_member_whose_resource_was_taken_while_it_waited_queues_for_it() {
    let mut log = Log::new();
    log.to_join("a"); log.queue("a");
    // While `a` is in the group its resource is free; another ticket takes it.
    log.run(Command::Enqueue { item: "other".into(), definition: ticket(), policy_pin: "policy-1".into(),
        resource: "res-a".into(), dependencies: BTreeSet::new(), initial_candidate: None }, Grant::Controller);
    let group = log.group_of("a");
    finish_with(&mut log, &group, "folded");
    assert_eq!(log.state.items["a"].phase, Phase::Blocked);
    assert_eq!(log.state.items["a"].pending, None);
    assert_eq!(log.stage("a"), "tidy");
    // It goes when the resource frees: drive `other` to its end of life.
    log.run(Command::Cancel { item: "other".into() }, Grant::Controller);
    let stop = log.state.items["other"].stopping.clone().unwrap();
    log.outcome("other", Outcome::Stopped { operation: stop, receipt: "quiet".into() });
    assert_eq!(log.state.items["a"].phase, Phase::Running);
}
#[test]
fn a_member_that_does_not_adopt_keeps_its_own_candidate() {
    let mut log = Log::new();
    let mut d = ticket(); d.stages[3].kind = StageKind::Join { group: "landing".into(), adopt: false };
    log.enqueue("a", d);
    while log.state.items["a"].phase != Phase::WaitingJoin { log.pass("a"); }
    let own = log.state.items["a"].candidate.clone();
    log.queue("a");
    let group = log.group_of("a");
    finish_with(&mut log, &group, "folded-result");
    assert_eq!(log.state.items["a"].candidate, own);
}
#[test]
fn a_failed_group_rejects_its_members_through_their_own_routes() {
    let mut log = Log::new();
    log.to_join("a"); log.queue("a");
    let group = log.group_of("a");
    pass_group(&mut log, &group, 2);
    // One member, so the gate's split falls to its `else`: the group fails.
    log.reject(&group);
    assert_eq!(log.state.items[&group].phase, Phase::Failed);
    assert_eq!(log.stage("a"), "stage-1", "the join step routes rejected back to the build");
    assert_eq!(log.state.items["a"].back_edges["join"], 1);
    assert_eq!(log.state.items["a"].grouped_in, None);
}
#[test]
fn a_cancelled_group_drops_its_members_and_an_unrouted_drop_fails_them() {
    let mut log = Log::new();
    log.to_join("a"); log.queue("a");
    let group = log.group_of("a");
    log.run(Command::Cancel { item: group.clone() }, Grant::Controller);
    assert_eq!(log.state.items[&group].phase, Phase::Cancelling);
    assert_eq!(log.state.items["a"].phase, Phase::Joined, "not until it has really stopped");
    let stop = log.state.items[&group].stopping.clone().unwrap();
    log.outcome(&group, Outcome::Stopped { operation: stop, receipt: "quiet".into() });
    assert_eq!(log.state.items["a"].phase, Phase::Failed);
    assert_eq!(log.stage("a"), "join");
}
#[test]
fn a_rejected_gate_splits_a_group_of_two_and_the_halves_run_in_order() {
    let mut log = Log::new();
    for t in ["x", "a", "b"] { log.to_join(t); }
    log.queue("x");
    let ahead = log.group_of("x");
    log.queue("a"); log.queue("b");
    let pair = log.group_of("a");
    assert_eq!(pair, log.group_of("b"));
    // The group ahead finishes; the pair leaves in the same step.
    pass_group(&mut log, &ahead, 3);
    assert_eq!(log.state.items[&pair].phase, Phase::Running);
    pass_group(&mut log, &pair, 2);
    let before = log.state.items[&pair].clone();
    let transition = log.reject(&pair);
    let kept = &log.state.items[&pair];
    assert_eq!(kept.members.iter().map(|m| m.item.as_str()).collect::<Vec<_>>(), ["a"], "keeps the first half");
    assert_eq!(log.stage(&pair), "base", "and starts again at the split step");
    assert!(kept.fence > before.fence);
    assert_eq!(kept.back_edges["gate"], 1);
    assert_eq!(transition.effects.iter().find(|e| e.item == pair).unwrap().members.len(), 1);
    let second = log.state.items["b"].grouped_in.clone().unwrap();
    assert_ne!(second, pair);
    let rest = &log.state.items[&second];
    assert_eq!((rest.phase.clone(), rest.stage, rest.members.len()), (Phase::Blocked, 0, 1), "waits for the resource");
    assert_eq!(rest.pin, kept.pin);
    // When the first half is done the second half goes.
    pass_group(&mut log, &pair, 3);
    assert_eq!(log.state.items[&second].phase, Phase::Running);
    assert_eq!(log.state.items["a"].phase, Phase::Running);
    assert_eq!(log.replay_digest(), digest(&log.state));
}
#[test]
fn a_split_of_a_single_member_takes_its_else_route() {
    let mut log = Log::new();
    log.to_join("a"); log.queue("a");
    let group = log.group_of("a");
    pass_group(&mut log, &group, 1);
    log.label(&group, "conflict");
    assert_eq!(log.state.items[&group].phase, Phase::Failed);
}
#[test]
fn a_member_can_leave_a_group_that_has_not_left_and_an_empty_group_is_cancelled() {
    let mut log = Log::new();
    for t in ["x", "a", "b"] { log.to_join(t); }
    log.queue("x"); log.queue("a"); log.queue("b");
    let pair = log.group_of("a");
    log.run(Command::Leave { item: "a".into() }, Grant::Controller);
    assert_eq!(log.state.items["a"].phase, Phase::WaitingJoin);
    assert_eq!(log.state.items["a"].grouped_in, None);
    assert_eq!(log.state.items[&pair].members.len(), 1);
    log.run(Command::Leave { item: "b".into() }, Grant::Controller);
    assert_eq!(log.state.items[&pair].phase, Phase::Cancelled);
    // It can be queued again.
    log.queue("a");
    assert_eq!(log.state.items["a"].phase, Phase::Joined);
}
#[test]
fn leaving_needs_the_controller_and_a_group_still_waiting() {
    let mut log = Log::new();
    log.to_join("a"); log.queue("a");
    assert_eq!(log.try_run(Command::Leave { item: "a".into() }, Grant::Controller).unwrap_err(), Refusal::Phase);
    log.to_join("b"); log.queue("b");
    assert_eq!(log.try_run(Command::Leave { item: "b".into() },
        Grant::Decision { principal: "owner".into() }).unwrap_err(), Refusal::Unauthorized);
    log.to_join("c");
    assert_eq!(log.try_run(Command::Leave { item: "c".into() }, Grant::Controller).unwrap_err(), Refusal::Phase);
}
#[test]
fn cancelling_a_waiting_member_removes_it_and_cancelling_a_departed_one_is_refused() {
    let mut log = Log::new();
    for t in ["x", "a"] { log.to_join(t); }
    log.queue("x"); log.queue("a");
    let waiting = log.group_of("a");
    log.run(Command::Cancel { item: "a".into() }, Grant::Controller);
    assert_eq!(log.state.items["a"].phase, Phase::Cancelled);
    assert_eq!(log.state.items[&waiting].phase, Phase::Cancelled, "no members left");
    assert_eq!(log.try_run(Command::Cancel { item: "x".into() }, Grant::Controller).unwrap_err(), Refusal::Phase);
}
#[test]
fn a_member_of_a_group_in_flight_cannot_leave_by_any_route() {
    let mut log = Log::new();
    log.to_join("a"); log.queue("a");
    assert!(log.try_run(Command::Cancel { item: "a".into() }, Grant::Controller).is_err());
    assert!(log.try_run(Command::Retry { item: "a".into() }, Grant::Controller).is_err());
}
#[test]
fn group_runs_replay_to_the_same_state() {
    let mut log = Log::new();
    for t in ["x", "a", "b"] { log.to_join(t); }
    log.queue("x"); log.queue("a"); log.queue("b");
    let ahead = log.group_of("x"); let pair = log.group_of("a");
    pass_group(&mut log, &ahead, 3);
    pass_group(&mut log, &pair, 2);
    log.reject(&pair);
    pass_group(&mut log, &pair, 3);
    let second = log.state.items["b"].grouped_in.clone().unwrap();
    pass_group(&mut log, &second, 3);
    assert_eq!(log.stage("b"), "tidy");
    assert_eq!(log.replay_digest(), digest(&log.state));
}
#[test]
fn schema_1_effects_and_items_serialize_exactly_as_before() {
    let mut log = Log::new();
    log.run(Command::Enqueue { item: "old".into(), definition: Definition::coding(), policy_pin: "p".into(),
        resource: "r".into(), dependencies: BTreeSet::new(), initial_candidate: None }, Grant::Controller);
    let text = serde_json::to_string(&log.state).unwrap();
    for key in ["members", "group_of", "grouped_in", "groups"] { assert!(!text.contains(key), "{key}"); }
}

fn invalid(edit: impl FnOnce(&mut Definition)) -> bool {
    let mut d = ticket(); edit(&mut d); d.validate() == Err(Refusal::Definition)
}
#[test]
fn the_validator_accepts_the_grouped_fixture() { assert_eq!(ticket().validate(), Ok(())); }
#[test]
fn the_validator_rejects_bad_groups() {
    let cases: Vec<(&str, Box<dyn FnOnce(&mut Definition)>)> = vec![
        ("join names no group", Box::new(|d| { d.groups.clear(); })),
        ("group nobody joins", Box::new(|d| { d.groups.insert("spare".into(), d.groups["landing"].clone()); })),
        ("limit zero", Box::new(|d| { d.groups.get_mut("landing").unwrap().limit = 0; })),
        ("limit above 32", Box::new(|d| { d.groups.get_mut("landing").unwrap().limit = 33; })),
        ("bad resource", Box::new(|d| { d.groups.get_mut("landing").unwrap().exclusive = "has space".into(); })),
        ("no group steps", Box::new(|d| { d.groups.get_mut("landing").unwrap().steps.clear(); })),
        ("join inside a group", Box::new(|d| {
            d.groups.get_mut("landing").unwrap().steps[1].kind = StageKind::Join { group: "landing".into(), adopt: false }; })),
        ("split to a missing step", Box::new(|d| {
            d.groups.get_mut("landing").unwrap().steps[2].routes.insert("rejected".into(),
                Target::Split { step: "nowhere".into(), otherwise: Box::new(Target::Fail) }); })),
        ("split without a limit", Box::new(|d| { d.groups.get_mut("landing").unwrap().steps[2].limit = 0; })),
        ("split inside a ticket", Box::new(|d| {
            d.stages[2].routes.insert("rejected".into(), Target::Split { step: "stage-1".into(), otherwise: Box::new(Target::Fail) }); })),
        ("split that never ends", Box::new(|d| {
            d.groups.get_mut("landing").unwrap().steps[2].routes.insert("rejected".into(),
                Target::Split { step: "base".into(), otherwise: Box::new(Target::Split { step: "base".into(), otherwise: Box::new(Target::Fail) }) }); })),
        ("join waits for approval", Box::new(|d| {
            d.stages[3].progression = Progression::AwaitApproval { principal: "owner".into() }; })),
        ("custom label on a join", Box::new(|d| { d.stages[3].routes.insert("odd".into(), Target::Done); })),
        ("dropped label on a build", Box::new(|d| { d.stages[1].routes.insert("dropped".into(), Target::Fail); })),
        ("groups on schema 1", Box::new(|d| { d.schema = 1; d.stages[3].routes.clear(); d.stages[3].limit = 0; d.stages[2].routes.clear(); d.stages[2].limit = 0; })),
        ("join before a candidate", Box::new(|d| { d.stages.swap(1, 3); })),
    ];
    for (name, edit) in cases { assert!(invalid(edit), "{name}"); }
}
#[test]
fn the_state_validator_refuses_forged_membership() {
    let mut log = Log::new();
    log.to_join("a"); log.queue("a");
    let group = log.group_of("a");
    let probe = |state: &State| step(state, &Input { id: "probe".into(), expected_revision: state.revision,
        source_sequence: state.revision + 1, ..log.input(Command::Advance, Grant::Controller) });
    let mut forged = log.state.clone();
    forged.items.get_mut(&group).unwrap().members.clear();
    assert_eq!(probe(&forged).unwrap_err(), Refusal::Invalid, "a joined ticket its group does not list");
    let mut forged = log.state.clone();
    forged.items.get_mut("a").unwrap().grouped_in = Some("nowhere".into());
    assert_eq!(probe(&forged).unwrap_err(), Refusal::Invalid, "a group that does not exist");
    let mut forged = log.state.clone();
    forged.items.get_mut(&group).unwrap().members.push(Member { item: "ghost".into(), candidate: "c".into() });
    assert_eq!(probe(&forged).unwrap_err(), Refusal::Invalid, "a member that does not exist");
}
