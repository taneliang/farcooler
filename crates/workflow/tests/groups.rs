//! Groups: tickets queued by the orchestrator run as one item, fan out, and split. Private fixtures only.
use farcooler_workflow::*;
use std::collections::{BTreeMap, BTreeSet};

struct Log { state: State, inputs: Vec<Input>, now: i64 }
impl Log {
    fn new() -> Self { Self { state: State::empty("workspace".into(), 1), inputs: vec![], now: 1000 } }
    fn input(&self, command: Command, grant: Grant) -> Input {
        let s = &self.state;
        Input { busy_resources: BTreeSet::new(), schema: 1, id: format!("command-{}", s.revision + 1),
            workspace: s.workspace.clone(), expected_revision: s.revision, epoch: s.epoch,
            source: "fixture".into(), source_sequence: s.revision + 1, now_ms: self.now, grant, command }
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
    fn outcome(&mut self, item: &str, outcome: Outcome) -> Transition { self.try_outcome(item, outcome).unwrap() }
    fn try_outcome(&mut self, item: &str, outcome: Outcome) -> Result<Transition, Refusal> {
        let i = &self.state.items[item];
        let (effect, run, fence) = (i.pending.clone().unwrap(), i.run.clone(), i.fence);
        self.try_run(Command::Observe { item: item.into(), effect: effect.clone(), run: run.clone(), fence, outcome },
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
    assert!(log.state.items[&group].members.is_empty());
    assert_eq!(log.state.items[&group].handed, vec!["a".to_string()], "a finished group remembers whom it ran");
    assert_eq!(log.state.items[&group].since_ms, 1000, "and when it reached the phase it ended in");
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
fn a_failed_group_cannot_be_retried() {
    // Its members already took their rejected routes; a revived group would land them anyway.
    let mut log = Log::new();
    log.to_join("a"); log.queue("a");
    let group = log.group_of("a");
    pass_group(&mut log, &group, 2);
    log.reject(&group);
    assert_eq!(log.state.items[&group].phase, Phase::Failed);
    assert!(log.try_run(Command::Retry { item: group.clone() }, Grant::Controller).is_err());
    assert_eq!(log.state.items[&group].phase, Phase::Failed);
}
/// Fill the workspace with plain tickets until it holds exactly `total` items.
fn fill(log: &mut Log, total: usize) {
    // Cloning one real ticket keeps the fixture honest and a thousand protocol steps out of the test.
    log.enqueue("filler-0", ticket());
    let template = log.state.items["filler-0"].clone();
    let mut n = 1;
    while log.state.items.len() < total {
        let mut item = template.clone(); item.resource = format!("res-filler-{n}");
        log.state.items.insert(format!("filler-{n}"), item); n += 1;
    }
}
#[test]
fn queueing_that_would_open_a_group_past_the_item_cap_is_refused_and_nothing_is_bricked() {
    let mut log = Log::new();
    log.to_join("a");
    fill(&mut log, 1024);
    assert_eq!(log.try_run(Command::Queue { item: "a".into() }, Grant::Controller).unwrap_err(), Refusal::Limit);
    // The workspace still takes commands.
    log.run(Command::Cancel { item: "filler-0".into() }, Grant::Controller);
}
#[test]
fn a_split_with_no_room_for_a_new_group_takes_its_fallback() {
    let mut log = Log::new();
    for t in ["x", "a", "b"] { log.to_join(t); }
    log.queue("x");
    let ahead = log.group_of("x");
    log.queue("a"); log.queue("b");
    let pair = log.group_of("a");
    pass_group(&mut log, &ahead, 3);
    pass_group(&mut log, &pair, 2);
    fill(&mut log, 1024);
    let e = log.evidence(&pair);
    let (effect, run, fence) = {
        let i = &log.state.items[&pair]; (i.pending.clone().unwrap(), i.run.clone(), i.fence)
    };
    // The cap never refuses an observation: with no room to split, the fallback route decides.
    log.try_run(Command::Observe { item: pair.clone(), effect: effect.clone(), run: run.clone(), fence,
        outcome: Outcome::Rejected(e) }, Grant::Operation { effect, run, fence }).unwrap();
    assert!(log.state.items.len() <= 1024);
    assert_eq!(log.state.items[&pair].phase, Phase::Failed, "the group took its fallback");
    // Each member got the group's result through its own join route: back to rework, not half a split.
    for t in ["a", "b"] { assert_eq!((log.stage(t), log.state.items[t].grouped_in.clone()), ("stage-1".to_string(), None)); }
}
#[test]
fn a_caller_cannot_enqueue_an_id_a_group_will_use() {
    let mut log = Log::new();
    let result = log.try_run(Command::Enqueue { item: format!("group-{:020}-landing", 1), definition: ticket(),
        policy_pin: "policy-1".into(), resource: "res".into(), dependencies: BTreeSet::new(), initial_candidate: None },
        Grant::Controller);
    assert_eq!(result.unwrap_err(), Refusal::Invalid);
}
#[test]
fn a_split_cannot_follow_a_landing_in_a_group() {
    // The first half would land, then the second half would re-run what already landed.
    let land_first = |d: &mut Definition| {
        let steps = &mut d.groups.get_mut("landing").unwrap().steps;
        steps[1].kind = StageKind::Land; steps[1].routes.clear(); steps[1].limit = 0;
    };
    // Landing with the gate's split pointing back across it is refused; the same group without the split is fine.
    assert!(invalid(land_first));
    let mut d = ticket(); land_first(&mut d);
    d.groups.get_mut("landing").unwrap().steps[2].routes.clear();
    d.groups.get_mut("landing").unwrap().steps[2].limit = 0;
    assert_eq!(d.validate(), Ok(()));
}
#[test]
fn a_split_cannot_target_a_step_at_or_after_a_landing() {
    // base, fold, land, ci: a red ci that splits back onto ci would land the second half twice.
    let mut d = ticket();
    let g = &mut d.groups.get_mut("landing").unwrap().steps;
    g[1].routes.clear(); g[1].limit = 0;
    g[2].kind = StageKind::Land; g[2].routes.clear(); g[2].limit = 0;
    let mut ci = action("ci"); ci.kind = StageKind::Ci;
    ci.routes.insert("rejected".into(), Target::Split { step: "ci".into(), otherwise: Box::new(Target::Fail) });
    ci.limit = 1;
    g.push(ci);
    assert_eq!(d.validate(), Err(Refusal::Definition));
    d.groups.get_mut("landing").unwrap().steps[3].routes.clear();
    d.groups.get_mut("landing").unwrap().steps[3].limit = 0;
    assert_eq!(d.validate(), Ok(()));
}
#[test]
fn a_split_after_a_landing_is_refused_by_the_landing_rule_alone() {
    // base, fold, land, rerun, ci. The split target `rerun` makes a candidate anew and sits after the
    // landing, so no other rule refuses it: only "a split never follows a landing" can.
    let mut d = ticket();
    let g = &mut d.groups.get_mut("landing").unwrap().steps;
    g[1].routes.clear(); g[1].limit = 0;
    g[2].kind = StageKind::Land; g[2].routes.clear(); g[2].limit = 0;
    g.push(action("rerun"));
    let mut ci = action("ci"); ci.kind = StageKind::Ci;
    ci.routes.insert("rejected".into(), Target::Split { step: "rerun".into(), otherwise: Box::new(Target::Fail) });
    ci.limit = 1;
    g.push(ci);
    assert_eq!(d.validate(), Err(Refusal::Definition));
    // The same shape with a plain back edge to `rerun` is sound, so the split is what was refused.
    let g = &mut d.groups.get_mut("landing").unwrap().steps;
    g[4].routes.insert("rejected".into(), Target::Step("rerun".into()));
    assert_eq!(d.validate(), Ok(()));
}
#[test]
fn a_split_must_restart_at_a_step_that_makes_its_candidate_anew() {
    // A Tracker only adopts a candidate, so a half restarting there would keep the one it had.
    let mut d = ticket();
    let g = &mut d.groups.get_mut("landing").unwrap().steps;
    g[0].kind = StageKind::Tracker;
    assert_eq!(d.validate(), Err(Refusal::Definition));
}
#[test]
fn only_the_exact_shape_of_a_group_id_is_reserved() {
    let mut log = Log::new();
    log.enqueue("group-12", ticket());
    log.enqueue("group-", ticket());
    let result = log.try_run(Command::Enqueue { item: format!("group-{:020}-x", 7), definition: ticket(),
        policy_pin: "policy-1".into(), resource: "r".into(), dependencies: BTreeSet::new(), initial_candidate: None },
        Grant::Controller);
    assert_eq!(result.unwrap_err(), Refusal::Invalid);
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
            let fold = &mut d.groups.get_mut("landing").unwrap().steps[1];
            fold.kind = StageKind::Join { group: "landing".into(), adopt: false };
            fold.routes.clear(); fold.limit = 0; })),
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

// ov-956 review blockers: a verb's own label, a pushed group, and what a restart may spend.

fn verb(id: &str, kind: StageKind) -> Stage { let mut s = action(id); s.kind = kind; s }
/// The batched push as a group of verbs with no routes at all: every label but `passed` falls to `Fail`.
fn verb_ticket() -> Definition {
    let mut d = ticket();
    let g = d.groups.get_mut("landing").unwrap();
    g.steps = vec![verb("base", StageKind::AwaitBase), verb("fold", StageKind::Fold), verb("gate", StageKind::Run),
        verb("push", StageKind::Push), verb("ci", StageKind::AwaitCi)];
    d
}
fn verb_group(log: &mut Log, definition: Definition) -> String {
    log.enqueue("a", definition);
    while log.state.items["a"].phase != Phase::WaitingJoin { log.pass("a"); }
    log.queue("a");
    log.group_of("a")
}
/// Walk the group to the named step, passing every step before it.
fn walk_to(log: &mut Log, group: &str, step: &str) {
    while log.stage(group) != step {
        let e = Evidence { candidate: "f".repeat(40), ..log.evidence(group) };
        let kind = log.state.items[group].definition.stages[log.state.items[group].stage].kind.clone();
        let e = if matches!(kind, StageKind::AwaitBase | StageKind::Fold) { e } else { log.evidence(group) };
        log.outcome(group, Outcome::Passed(e));
    }
}

#[test]
fn a_verbs_own_label_with_no_route_fails_the_group_and_is_never_refused() {
    for (step, label) in [("base", "timeout"), ("fold", "conflict"), ("push", "base_moved"), ("ci", "timeout"), ("ci", "cancelled")] {
        let mut log = Log::new();
        let group = verb_group(&mut log, verb_ticket());
        walk_to(&mut log, &group, step);
        let e = log.evidence(&group);
        let (i, effect) = (&log.state.items[&group], log.state.items[&group].pending.clone().unwrap());
        let input = log.input(Command::Observe { item: group.clone(), effect: effect.clone(), run: i.run.clone(), fence: i.fence,
            outcome: Outcome::Labeled(label.into(), e) }, Grant::Operation { effect, run: i.run.clone(), fence: i.fence });
        let next = step_of(&log, &input).unwrap_or_else(|r| panic!("{step}/{label} refused: {r:?}"));
        assert_eq!(next.state.items[&group].phase, Phase::Failed, "{step}/{label}");
    }
}
fn step_of(log: &Log, input: &Input) -> Result<Transition, Refusal> { step(&log.state, input) }

#[test]
fn a_group_that_pushed_hands_its_members_the_landed_commit_and_never_says_rejected() {
    let mut definition = verb_ticket();
    // The member routes `landed` forward, past the join, where `rejected` would send it back to rework.
    definition.stages[3].routes.insert("landed".into(), Target::Step("tidy".into()));
    let mut log = Log::new();
    let group = verb_group(&mut log, definition);
    walk_to(&mut log, &group, "ci");
    let pushed = log.state.items[&group].pushed.clone();
    assert_eq!(pushed.as_deref(), Some("f".repeat(40).as_str()), "recorded when the push passed");
    // CI is red.
    log.reject(&group);
    assert_eq!(log.state.items[&group].phase, Phase::Failed);
    assert_eq!(log.stage("a"), "tidy", "landed, not rejected");
    assert_eq!(log.state.items["a"].candidate, pushed, "it carries what landed");
    assert_eq!(log.state.items["a"].back_edges.get("join"), None);
}
#[test]
fn an_unrouted_landed_label_fails_the_member_with_the_landed_commit() {
    let mut log = Log::new();
    let group = verb_group(&mut log, verb_ticket());
    walk_to(&mut log, &group, "ci");
    let pushed = log.state.items[&group].pushed.clone();
    log.reject(&group);
    assert_eq!((log.state.items["a"].phase.clone(), log.stage("a")), (Phase::Failed, "join".to_string()));
    assert_eq!(log.state.items["a"].candidate, pushed);
}
#[test]
fn a_group_cannot_be_cancelled_while_it_pushes_or_after_it_has() {
    let mut log = Log::new();
    let group = verb_group(&mut log, verb_ticket());
    walk_to(&mut log, &group, "push");
    let cancel = Command::Cancel { item: group.clone() };
    assert_eq!(log.try_run(cancel.clone(), Grant::Controller).unwrap_err(), Refusal::Phase, "the push may be in flight");
    log.pass(&group);
    assert_eq!(log.stage(&group), "ci");
    assert_eq!(log.try_run(cancel, Grant::Controller).unwrap_err(), Refusal::Phase, "its commits are on the target");
    // Before the push it is still allowed.
    let mut early = Log::new();
    let group = verb_group(&mut early, verb_ticket());
    walk_to(&mut early, &group, "gate");
    early.run(Command::Cancel { item: group }, Grant::Controller);
}
#[test]
fn a_wait_that_a_restart_interrupted_looks_again_spends_no_attempt_and_keeps_its_patience() {
    for step in ["base", "ci"] {
        let mut log = Log::new();
        let group = verb_group(&mut log, verb_ticket());
        walk_to(&mut log, &group, step);
        let before = log.state.items[&group].clone();
        let first = log.outcome(&group, Outcome::Interrupted);
        let after = &log.state.items[&group];
        assert_eq!((after.phase.clone(), log.stage(&group), after.attempt), (Phase::Running, step.to_string(), before.attempt), "{step}");
        assert!(after.fence > before.fence);
        let again: Vec<_> = first.effects.iter().filter(|e| e.item == group).collect();
        assert_eq!(again.len(), 1, "{step}: read again");
        // The clock moves on; a second interruption must still carry the first deadline.
        log.now = 5000;
        let second = log.outcome(&group, Outcome::Interrupted);
        let later: Vec<_> = second.effects.iter().filter(|e| e.item == group).collect();
        assert_eq!(later[0].created_ms, again[0].created_ms, "{step}: patience survives a restart");
    }
    // Only a wait can be interrupted; a fold is made again and a restart there is a failure.
    let mut log = Log::new();
    let group = verb_group(&mut log, verb_ticket());
    walk_to(&mut log, &group, "fold");
    assert_eq!(log.try_outcome(&group, Outcome::Interrupted).unwrap_err(), Refusal::Phase);
}
#[test]
fn a_wait_that_fails_for_a_real_reason_spends_attempts_and_ends() {
    for step in ["base", "ci"] {
        let mut log = Log::new();
        let group = verb_group(&mut log, verb_ticket());
        walk_to(&mut log, &group, step);
        let budget = log.state.items[&group].definition.stages[log.state.items[&group].stage].max_attempts;
        for round in 0..budget {
            assert_eq!(log.state.items[&group].phase, Phase::Running, "{step} round {round}");
            log.outcome(&group, Outcome::SafeFailure);
        }
        assert_eq!(log.state.items[&group].phase, Phase::Failed, "{step}: a wait that keeps failing ends");
    }
    // A group that already pushed ends as landed, with the pushed commit, not stuck.
    let mut log = Log::new();
    let group = verb_group(&mut log, verb_ticket());
    walk_to(&mut log, &group, "ci");
    assert!(log.state.items[&group].pushed.is_some());
    for _ in 0..8 { if log.state.items.get(&group).is_some_and(|g| g.phase == Phase::Running) { log.outcome(&group, Outcome::SafeFailure); } }
    assert!(log.state.items.get(&group).map_or(true, |g| g.phase == Phase::Failed), "the pushed group has an exit");
}

// ov-957 review blocker B5: the wait for a pull request is a wait like the others.

#[test]
fn a_pull_request_wait_a_restart_interrupted_looks_again_and_keeps_its_patience() {
    let mut definition = ticket();
    definition.groups.clear();
    let mut wait = verb("merged", StageKind::AwaitPr);
    wait.routes.insert("closed".into(), Target::Fail);
    definition.stages = vec![action("build"), verb("branch", StageKind::PushBranch), verb("pr", StageKind::OpenPr), wait];
    definition.schema = 2;
    definition.validate().unwrap();
    let mut log = Log::new();
    log.enqueue("a", definition);
    while log.stage("a") != "merged" { log.pass("a"); }
    let before = log.state.items["a"].clone();
    let first = log.outcome("a", Outcome::Interrupted);
    let after = &log.state.items["a"];
    assert_eq!((after.phase.clone(), log.stage("a"), after.attempt), (Phase::Running, "merged".to_string(), before.attempt));
    let again: Vec<_> = first.effects.iter().filter(|e| e.item == "a").collect();
    assert_eq!(again.len(), 1);
    log.now = 9000;
    let second = log.outcome("a", Outcome::Interrupted);
    assert_eq!(second.effects.iter().find(|e| e.item == "a").unwrap().created_ms, again[0].created_ms, "patience survives a restart");
    // The steps before it are not waits.
    let mut early = Log::new();
    let mut d = ticket(); d.groups.clear();
    d.stages = vec![action("build"), verb("branch", StageKind::PushBranch)]; d.schema = 2;
    early.enqueue("a", d);
    while early.stage("a") != "branch" { early.pass("a"); }
    assert_eq!(early.try_outcome("a", Outcome::Interrupted).unwrap_err(), Refusal::Phase);
}
