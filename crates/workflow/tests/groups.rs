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
    fn join_group(&mut self, item: &str) {
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
    let mut fold = action("fold");
    fold.routes.insert("conflict".into(), Target::Split { step: "base".into(), otherwise: Box::new(Target::Fail) });
    let mut gate = action("gate"); gate.kind = StageKind::Check;
    gate.routes.insert("rejected".into(), Target::Split { step: "base".into(), otherwise: Box::new(Target::Fail) });
    d.groups.insert("landing".into(), GroupDef { limit: Some(2), exclusive: "ref:origin:main".into(), steps: vec![base, fold, gate] });
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
    log.join_group("a");
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
    log.join_group("a");
    let owner = Grant::Decision { principal: "owner".into() };
    assert_eq!(log.try_run(Command::Queue { item: "a".into() }, owner).unwrap_err(), Refusal::Unauthorized);
    log.queue("a");
    assert_eq!(log.try_run(Command::Queue { item: "a".into() }, Grant::Controller).unwrap_err(), Refusal::Phase);
}
#[test]
fn queueing_opens_a_group_that_leaves_at_once_when_nothing_is_ahead() {
    let mut log = Log::new();
    log.join_group("a");
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
    for t in ["a", "b", "c", "d"] { log.join_group(t); }
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
fn an_uncapped_group_takes_every_ticket_waiting_and_splits_down_to_the_one_that_fails() {
    // ov-1071: a group with no limit has no cap, in the definition or the engine. Everything ready lands together.
    let mut uncapped = ticket(); uncapped.groups.get_mut("landing").unwrap().limit = None;
    assert_eq!(uncapped.validate(), Ok(()));
    let text = serde_json::to_string(&uncapped.groups["landing"]).unwrap();
    assert!(!text.contains("limit"), "absent, not zero: {text}");
    let mut log = Log::new();
    // More than the 32 the engine once allowed, and more than five back edges' worth of halving.
    let tickets: Vec<String> = (0..41).map(|i| format!("t{i:02}")).collect();
    for t in &tickets {
        log.enqueue(t, uncapped.clone());
        while log.state.items[t].phase != Phase::WaitingJoin { log.pass(t); }
    }
    log.queue(&tickets[0]);
    for t in &tickets[1..] { log.queue(t); }
    let behind = log.group_of(&tickets[1]);
    assert!(tickets[1..].iter().all(|t| log.group_of(t) == behind), "every waiting ticket joins the one group");
    assert_eq!(log.state.items[&behind].members.len(), 40);
    // The group ahead lands; then every gate run of the big group fails, so it halves until one ticket is left.
    let ahead = log.group_of(&tickets[0]);
    pass_group(&mut log, &ahead, 3);
    let mut splits = 0;
    while log.state.items[&behind].members.len() >= 2 {
        pass_group(&mut log, &behind, 2);
        log.reject(&behind);
        splits += 1;
        let kept = &log.state.items[&behind];
        assert_eq!((log.stage(&behind), kept.iteration, kept.back_edges.len()), ("base".to_string(), 0, 0), "a split takes no back edge");
    }
    // 40, 20, 10, 5, 3, 2, 1: six splits, one more than any step's back edges may be taken.
    assert!(splits > usize::from(MAX_BACK_EDGE_LIMIT));
    assert_eq!(log.state.items[&behind].members.iter().map(|m| m.item.as_str()).collect::<Vec<_>>(), [tickets[1].as_str()]);
    pass_group(&mut log, &behind, 2);
    log.reject(&behind);
    assert_eq!(log.state.items[&behind].phase, Phase::Failed, "a single ticket takes the split's fallback");
    assert_eq!(log.replay_digest(), digest(&log.state));
}
#[test]
fn a_ticket_without_a_candidate_cannot_be_queued() {
    let mut log = Log::new();
    log.join_group("a");
    log.state.items.get_mut("a").unwrap().candidate = None;
    assert_eq!(log.try_run(Command::Queue { item: "a".into() }, Grant::Controller).unwrap_err(), Refusal::Invalid);
}
#[test]
fn a_finished_group_fans_out_and_a_member_adopts_the_result() {
    let mut log = Log::new();
    log.join_group("a"); log.queue("a");
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
    log.join_group("a"); log.queue("a");
    // While `a` is in the group its resource is free; another ticket takes it.
    log.run(Command::Enqueue { item: "other".into(), definition: ticket(), policy_pin: "policy-1".into(),
        resource: "res-a".into(), dependencies: BTreeSet::new(), initial_candidate: None }, Grant::Controller);
    let group = log.group_of("a");
    finish_with(&mut log, &group, "folded");
    assert_eq!(log.state.items["a"].phase, Phase::Blocked);
    assert_eq!(log.state.items["a"].pending, None);
    assert_eq!(log.stage("a"), "tidy");
    // It goes when the resource frees: drive `other` to its end of life.
    log.run(Command::Cancel { item: "other".into() , reason: None }, Grant::Controller);
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
    log.join_group("a"); log.queue("a");
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
fn a_cancelled_group_waits_until_stopped_before_returning_members() {
    let mut log = Log::new();
    log.join_group("a"); log.queue("a");
    let group = log.group_of("a");
    log.run(Command::Cancel { item: group.clone() , reason: None }, Grant::Controller);
    assert_eq!(log.state.items[&group].phase, Phase::Cancelling);
    assert_eq!(log.state.items["a"].phase, Phase::Joined, "not until it has really stopped");
    let stop = log.state.items[&group].stopping.clone().unwrap();
    log.outcome(&group, Outcome::Stopped { operation: stop, receipt: "quiet".into() });
    assert_eq!(log.state.items["a"].phase, Phase::WaitingJoin);
    assert_eq!(log.stage("a"), "join");
}
#[test]
fn a_rejected_gate_splits_a_group_of_two_and_the_halves_run_in_order() {
    let mut log = Log::new();
    for t in ["x", "a", "b"] { log.join_group(t); }
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
    assert_eq!((kept.back_edges.get("gate"), kept.iteration), (None, 0), "halving bounds a split, not the step's limit");
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
    log.join_group("a"); log.queue("a");
    let group = log.group_of("a");
    pass_group(&mut log, &group, 1);
    log.label(&group, "conflict");
    assert_eq!(log.state.items[&group].phase, Phase::Failed);
}
#[test]
fn a_member_can_leave_a_group_that_has_not_left_and_an_empty_group_is_cancelled() {
    let mut log = Log::new();
    for t in ["x", "a", "b"] { log.join_group(t); }
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
    log.join_group("a"); log.queue("a");
    assert_eq!(log.try_run(Command::Leave { item: "a".into() }, Grant::Controller).unwrap_err(), Refusal::Phase);
    log.join_group("b"); log.queue("b");
    assert_eq!(log.try_run(Command::Leave { item: "b".into() },
        Grant::Decision { principal: "owner".into() }).unwrap_err(), Refusal::Unauthorized);
    log.join_group("c");
    assert_eq!(log.try_run(Command::Leave { item: "c".into() }, Grant::Controller).unwrap_err(), Refusal::Phase);
}
#[test]
fn cancelling_a_waiting_member_removes_it_and_cancelling_a_departed_one_is_refused() {
    let mut log = Log::new();
    for t in ["x", "a"] { log.join_group(t); }
    log.queue("x"); log.queue("a");
    let waiting = log.group_of("a");
    log.run(Command::Cancel { item: "a".into() , reason: None }, Grant::Controller);
    assert_eq!(log.state.items["a"].phase, Phase::Cancelled);
    assert_eq!(log.state.items[&waiting].phase, Phase::Cancelled, "no members left");
    assert_eq!(log.try_run(Command::Cancel { item: "x".into() , reason: None }, Grant::Controller).unwrap_err(), Refusal::Phase);
}
#[test]
fn a_member_of_a_group_in_flight_cannot_leave_by_any_route() {
    let mut log = Log::new();
    log.join_group("a"); log.queue("a");
    assert!(log.try_run(Command::Cancel { item: "a".into() , reason: None }, Grant::Controller).is_err());
    assert!(log.try_run(Command::Retry { item: "a".into() }, Grant::Controller).is_err());
}
#[test]
fn a_failed_group_cannot_be_retried() {
    // Its members already took their rejected routes; a revived group would land them anyway.
    let mut log = Log::new();
    log.join_group("a"); log.queue("a");
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
    log.join_group("a");
    fill(&mut log, 1024);
    assert_eq!(log.try_run(Command::Queue { item: "a".into() }, Grant::Controller).unwrap_err(), Refusal::Limit);
    // The workspace still takes commands.
    log.run(Command::Cancel { item: "filler-0".into() , reason: None }, Grant::Controller);
}
#[test]
fn a_split_with_no_room_for_a_new_group_takes_its_fallback() {
    let mut log = Log::new();
    for t in ["x", "a", "b"] { log.join_group(t); }
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
    for t in ["x", "a", "b"] { log.join_group(t); }
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
fn a_split_needs_no_limit_and_one_published_with_a_limit_still_reads() {
    // ov-1071: halving bounds a split. A group step published before then kept a limit of three for its splits.
    let mut d = ticket();
    assert_eq!(d.groups["landing"].steps[2].limit, 0);
    for step in &mut d.groups.get_mut("landing").unwrap().steps[1..] { step.limit = 3; }
    assert_eq!(d.validate(), Ok(()));
}
#[test]
fn the_validator_rejects_bad_groups() {
    type Break = Box<dyn FnOnce(&mut Definition)>;
    let cases: Vec<(&str, Break)> = vec![
        ("join names no group", Box::new(|d| { d.groups.clear(); })),
        ("group nobody joins", Box::new(|d| { d.groups.insert("spare".into(), d.groups["landing"].clone()); })),
        ("a limit of zero", Box::new(|d| { d.groups.get_mut("landing").unwrap().limit = Some(0); })),
        ("bad resource", Box::new(|d| { d.groups.get_mut("landing").unwrap().exclusive = "has space".into(); })),
        ("no group steps", Box::new(|d| { d.groups.get_mut("landing").unwrap().steps.clear(); })),
        ("join inside a group", Box::new(|d| {
            let fold = &mut d.groups.get_mut("landing").unwrap().steps[1];
            fold.kind = StageKind::Join { group: "landing".into(), adopt: false };
            fold.routes.clear(); fold.limit = 0; })),
        ("split to a missing step", Box::new(|d| {
            d.groups.get_mut("landing").unwrap().steps[2].routes.insert("rejected".into(),
                Target::Split { step: "nowhere".into(), otherwise: Box::new(Target::Fail) }); })),
        ("split falling back to a back edge with no limit", Box::new(|d| {
            d.groups.get_mut("landing").unwrap().steps[2].routes.insert("rejected".into(),
                Target::Split { step: "base".into(), otherwise: Box::new(Target::Step("base".into())) }); })),
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
    log.join_group("a"); log.queue("a");
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
    let cancel = Command::Cancel { item: group.clone() , reason: None };
    assert_eq!(log.try_run(cancel.clone(), Grant::Controller).unwrap_err(), Refusal::Phase, "the push may be in flight");
    log.pass(&group);
    assert_eq!(log.stage(&group), "ci");
    assert_eq!(log.try_run(cancel, Grant::Controller).unwrap_err(), Refusal::Phase, "its commits are on the target");
    // Before the push it is still allowed.
    let mut early = Log::new();
    let group = verb_group(&mut early, verb_ticket());
    walk_to(&mut early, &group, "gate");
    early.run(Command::Cancel { item: group , reason: None }, Grant::Controller);
}
#[test]
fn a_landing_wait_interrupted_by_restart_keeps_its_patience_and_candidate() {
    for step in ["base", "push", "ci"] {
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
        // Neither the deadline nor the candidate changes when the read resumes.
        log.now = 5000;
        let second = log.outcome(&group, Outcome::Interrupted);
        let later: Vec<_> = second.effects.iter().filter(|e| e.item == group).collect();
        assert_eq!(later[0].created_ms, again[0].created_ms, "{step}: original stage deadline");
        assert_eq!(later[0].candidate, again[0].candidate);
    }
    // A fold is made again and a restart there is a failure, unlike a proven pre-send push wait.
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
        let budget = 3;
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
    assert!(log.state.items.get(&group).is_none_or(|g| g.phase == Phase::Failed), "the pushed group has an exit");
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

fn flaky_definition() -> Definition {
    let mut definition = verb_ticket();
    definition.policies.insert("flaky-ci".into(), CiPolicy { point: CiPoint::Result, step: "ci".into(),
        rule: CiRule { rerun_failed: RerunFailed { jobs: vec!["iOS UI tests".into()], times: 1 } },
        fallback: None });
    definition
}
fn ci_answer(log: &mut Log, group: &str, label: &str, attempt: u32, jobs: &[&str]) -> Transition {
    let evidence = log.evidence(group);
    log.outcome(group, Outcome::CiResult { label: label.into(), evidence,
        run: Some(CiRun { id: 73, attempt, failed_jobs: jobs.iter().map(|j| (*j).into()).collect(), failed_steps: vec![] }), failure: None })
}
#[test]
fn a_listed_ci_failure_reruns_once_then_green_lands_or_second_red_routes_rejected() {
    for green in [true, false] {
        let mut log = Log::new();
        let group = verb_group(&mut log, flaky_definition());
        walk_to(&mut log, &group, "ci");
        let candidate = log.state.items[&group].candidate.clone();
        let first = ci_answer(&mut log, &group, "rejected", 1, &["iOS UI tests"]);
        assert_eq!(first.effects.len(), 1);
        assert_eq!(first.effects[0].kind, EffectKind::Execute(StageKind::RerunCi));
        assert_eq!(log.state.items[&group].ci_reruns["ci"], 1);
        assert_eq!(log.state.items[&group].phase, Phase::Running, "red did not become passed");
        assert!(log.state.items[&group].ci_retry.as_ref().unwrap().pending);
        // Restart serializes and restores both the count and the exact run before it dispatches.
        log.state = serde_json::from_str(&serde_json::to_string(&log.state).unwrap()).unwrap();
        let resumed = log.pass(&group);
        assert_eq!(resumed.effects[0].kind, EffectKind::Execute(StageKind::AwaitCi));
        assert_eq!(resumed.effects[0].candidate, candidate);
        assert_eq!(resumed.effects[0].ci_retry.as_ref().unwrap().run.id, 73);
        assert_eq!(log.state.items[&group].phase, Phase::Running, "rerun acceptance is not green CI");
        // A stale red/green from attempt 1 cannot finish the look at attempt 2.
        let evidence = log.evidence(&group);
        assert_eq!(log.try_outcome(&group, Outcome::CiResult { label: "passed".into(), evidence,
            run: Some(CiRun { id: 73, attempt: 1, failed_jobs: vec![], failed_steps: vec![] }), failure: None }), Err(Refusal::Evidence));
        let result = ci_answer(&mut log, &group, if green { "passed" } else { "rejected" }, 2,
            if green { &[] } else { &["iOS UI tests"] });
        assert_eq!(result.state.items[&group].phase, if green { Phase::Done } else { Phase::Failed });
        assert!(result.effects.iter().all(|e| e.kind != EffectKind::Execute(StageKind::RerunCi)));
        assert_eq!(result.state.items[&group].ci_reruns["ci"], 1);
        assert_eq!(log.replay_digest(), digest(&log.state));
    }
}
/// ov-1280: run 43745907131's two failed jobs, as the CI look reports them; `own` puts the iOS failure in the build.
fn outage(attempt: u32, own: bool) -> CiRun {
    let step = |job: &str, step: &str, action: &str| FailedStep { job: job.into(), step: step.into(), action: Some(action.into()) };
    CiRun { id: 73, attempt, failed_jobs: vec!["iOS".into(), "iOS UI (smoke)".into()], failed_steps: vec![
        step("iOS", "Run ./.github/actions/ios-ui-build", if own { "__self.__run_2" } else { "__self.__dtolnay_rust-toolchain.__run_4" }),
        step("iOS UI (smoke)", "Run ./.github/actions/ios-ui-shard", "__self.__actions_download-artifact")] }
}
fn ci_run(log: &mut Log, group: &str, label: &str, run: CiRun) -> Transition {
    let evidence = log.evidence(group);
    log.outcome(group, Outcome::CiResult { label: label.into(), evidence, run: Some(run), failure: None })
}
#[test]
fn a_setup_only_ci_failure_reruns_once_with_no_policy_then_its_verdict_decides() {
    for green in [true, false] {
        let mut log = Log::new();
        let group = verb_group(&mut log, verb_ticket());
        assert!(log.state.items[&group].definition.policies.is_empty(), "no published rerun rule");
        walk_to(&mut log, &group, "ci");
        let first = ci_run(&mut log, &group, "rejected", outage(1, false));
        assert_eq!(first.effects.iter().map(|e| e.kind.clone()).collect::<Vec<_>>(), [EffectKind::Execute(StageKind::RerunCi)]);
        let item = &log.state.items[&group];
        assert_eq!((item.phase.clone(), item.ci_reruns["ci"]), (Phase::Running, 1), "red is not the verdict yet");
        assert_eq!(item.ci_retry.as_ref().map(|r| (r.policy.as_str(), r.run.id)), Some((SETUP_RERUN, 73)));
        assert_eq!(serde_json::from_str::<State>(&serde_json::to_string(&log.state).unwrap()).unwrap(), log.state);
        assert_eq!(log.pass(&group).effects[0].kind, EffectKind::Execute(StageKind::AwaitCi), "the look follows the rerun");
        let second = if green { ci_run(&mut log, &group, "passed", CiRun { failed_jobs: vec![], failed_steps: vec![], ..outage(2, false) }) }
            else { ci_run(&mut log, &group, "rejected", outage(2, false)) };
        assert_eq!(second.state.items[&group].phase, if green { Phase::Done } else { Phase::Failed }, "one rerun, then its verdict");
        assert!(second.effects.iter().all(|e| e.kind != EffectKind::Execute(StageKind::RerunCi)));
        assert_eq!(log.replay_digest(), digest(&log.state));
    }
}
#[test]
fn a_real_failure_beside_a_setup_one_fails_the_landing_without_a_rerun() {
    let mut log = Log::new();
    let group = verb_group(&mut log, verb_ticket());
    walk_to(&mut log, &group, "ci");
    let result = ci_run(&mut log, &group, "rejected", outage(1, true));
    assert_eq!(result.state.items[&group].phase, Phase::Failed, "the red goes where red always went");
    assert!(result.effects.iter().all(|e| e.kind != EffectKind::Execute(StageKind::RerunCi)));
    assert!(result.state.items[&group].ci_reruns.is_empty());
    let mut taken = verb_ticket();
    taken.policies.insert(SETUP_RERUN.into(), CiPolicy { point: CiPoint::Result, step: "ci".into(),
        rule: CiRule { rerun_failed: RerunFailed { jobs: vec!["iOS".into()], times: 1 } }, fallback: None });
    assert_eq!(taken.validate(), Err(Refusal::Definition), "no published policy takes the engine's name");
}
#[test]
fn unlisted_empty_or_missing_ci_jobs_route_rejected_without_a_rerun() {
    for jobs in [vec!["Rust"], vec!["iOS UI tests", "Rust"], vec![]] {
        let mut log = Log::new();
        let group = verb_group(&mut log, flaky_definition());
        walk_to(&mut log, &group, "ci");
        let result = ci_answer(&mut log, &group, "rejected", 1, &jobs);
        assert_eq!(result.state.items[&group].phase, Phase::Failed);
        assert!(result.effects.iter().all(|e| e.kind != EffectKind::Execute(StageKind::RerunCi)));
    }
}
#[test]
fn an_ambiguous_rerun_stays_uncertain_without_refunding_the_count() {
    let mut log = Log::new();
    let group = verb_group(&mut log, flaky_definition());
    walk_to(&mut log, &group, "ci");
    ci_answer(&mut log, &group, "rejected", 1, &["iOS UI tests"]);
    let result = log.outcome(&group, Outcome::Interrupted);
    assert!(result.effects.is_empty());
    assert_eq!(log.state.items[&group].phase, Phase::Uncertain);
    assert_eq!(log.state.items[&group].ci_reruns["ci"], 1);
    let result = log.run(Command::Advance, Grant::Controller);
    assert!(result.effects.is_empty());
    assert_eq!(log.replay_digest(), digest(&log.state));
}
#[test]
fn ci_rules_are_checked_at_publication_and_empty_policies_preserve_legacy_bytes() {
    let definition = flaky_definition();
    assert_eq!(definition.validate(), Ok(()));
    for (times, jobs, step) in [(0, vec!["iOS UI tests"], "ci"), (2, vec!["iOS UI tests"], "ci"),
        (1, vec![], "ci"), (1, vec![""], "ci"), (1, vec!["a", "a"], "ci"), (1, vec!["a"], "gate")] {
        let mut bad = definition.clone();
        let rule = bad.policies.get_mut("flaky-ci").unwrap();
        rule.step = step.into(); rule.rule.rerun_failed.times = times;
        rule.rule.rerun_failed.jobs = jobs.into_iter().map(String::from).collect();
        assert_eq!(bad.validate(), Err(Refusal::Definition));
    }
    let mut bad = definition.clone();
    bad.stages[0].kind = StageKind::RerunCi;
    assert_eq!(bad.validate(), Err(Refusal::Definition), "only the rule may emit a rerun");
    let legacy = verb_ticket();
    assert!(!serde_json::to_string(&legacy).unwrap().contains("policies"));
    let raw = serde_json::to_value(&definition).unwrap();
    assert_eq!(raw["policies"]["flaky-ci"]["point"], "ci.result");
    assert_eq!(raw["policies"]["flaky-ci"]["rule"]["rerun_failed"]["times"], 1);
    let mut bad = raw;
    bad["policies"]["flaky-ci"]["rule"]["pass_anyway"] = true.into();
    assert!(serde_json::from_value::<Definition>(bad).is_err());
}

#[test]
fn a_ci_receipt_can_resolve_uncertainty_without_firing_a_rerun_policy() {
    for label in ["passed", "rejected"] {
        let mut log = Log::new();
        let group = verb_group(&mut log, flaky_definition());
        walk_to(&mut log, &group, "ci");
        log.outcome(&group, Outcome::Uncertain);
        let item = &log.state.items[&group];
        let command = Command::Resolve { item: group.clone(), effect: item.pending.clone().unwrap(),
            run: item.run.clone(), fence: item.fence,
            outcome: Outcome::CiResult { label: label.into(), evidence: log.evidence(&group),
                run: Some(CiRun { id: 73, attempt: 1, failed_jobs: vec!["iOS UI tests".into()], failed_steps: vec![] }), failure: None } };
        let result = log.run(command, Grant::Controller);
        assert_eq!(result.state.items[&group].phase, if label == "passed" { Phase::Done } else { Phase::Failed });
        assert!(result.effects.iter().all(|e| e.kind != EffectKind::Execute(StageKind::RerunCi)));
        assert_eq!(log.replay_digest(), digest(&log.state));
    }
}

#[test]
fn repeated_fold_conflicts_return_to_build_without_spending_join_rework() {
    let mut log = Log::new();
    let group = verb_group(&mut log, verb_ticket());
    walk_to(&mut log, &group, "gate");
    log.reject(&group);
    while log.state.items["a"].phase != Phase::WaitingJoin { log.pass("a"); }
    log.queue("a");
    for round in 0..3 {
        let group = log.group_of("a");
        walk_to(&mut log, &group, "fold");
        let e = log.evidence(&group);
        let next = log.outcome(&group, Outcome::FoldConflict { members: None, evidence: e,
            files: vec!["coding_native.rs".into(), "state.rs".into()] });
        let ticket = &log.state.items["a"];
        let brief = next.effects.iter().find(|e| e.item == "a").unwrap().brief.as_ref().unwrap();
        assert!(brief.contains("coding_native.rs") && brief.contains("state.rs"), "{brief}");
        assert!(brief.contains(&"f".repeat(40)), "the landing base is named: {brief}");
        assert_eq!(ticket.phase, Phase::Running, "conflict round {round}");
        assert_eq!(log.stage("a"), "stage-1");
        assert_eq!(ticket.iteration, 1);
        assert_eq!(ticket.back_edges.get("join"), Some(&1));
        log.pass("a");
        assert_eq!(log.stage("a"), "stage-2", "checks run on the rebased candidate");
        assert!(log.state.items["a"].brief.is_none());
        while log.state.items["a"].phase != Phase::WaitingJoin { log.pass("a"); }
        log.queue("a");
    }
    assert_eq!(digest(&log.state), log.replay_digest());
}

#[test]
fn a_conflict_return_waits_for_its_build_resource_and_keeps_a_bounded_brief() {
    let mut log = Log::new();
    let group = verb_group(&mut log, verb_ticket());
    walk_to(&mut log, &group, "fold");
    log.run(Command::Enqueue { item: "occupant".into(), definition: ticket(), policy_pin: "policy-1".into(),
        resource: "res-a".into(), dependencies: BTreeSet::new(), initial_candidate: None }, Grant::Controller);
    let evidence = log.evidence(&group);
    let files = vec!["a\nstrange-path.rs".into(), "界.rs".repeat(2000)];
    let next = log.outcome(&group, Outcome::FoldConflict { members: None, evidence, files });
    assert_eq!(log.state.items["a"].phase, Phase::Blocked);
    assert!(next.effects.iter().all(|e| e.item != "a"));
    let brief = log.state.items["a"].brief.clone().unwrap();
    assert!(valid_rework_reason(&brief));
    assert!(brief.contains("a\\nstrange-path.rs") && brief.contains("full conflicting file list"));
    log.run(Command::Cancel { item: "occupant".into(), reason: None }, Grant::Controller);
    assert_eq!(log.state.items["a"].phase, Phase::Blocked);
    let operation = log.state.items["occupant"].stopping.clone().unwrap();
    let next = log.outcome("occupant", Outcome::Stopped { operation, receipt: "stop-receipt".into() });
    assert_eq!(next.effects.iter().find(|e| e.item == "a").unwrap().brief.as_ref(), Some(&brief));
    assert_eq!(log.state.items["a"].phase, Phase::Running);
    assert_eq!(log.state.items["a"].brief.as_ref(), Some(&brief));
    assert!(log.state.items["a"].back_edges.is_empty());
    assert_eq!(digest(&log.state), log.replay_digest());
}

#[test]
fn a_script_decides_the_split_half_before_it_can_acquire_the_resource() {
    let mut log = Log::new();
    let mut definition = ticket();
    definition.script_policies.insert("split".into(), ScriptPolicy { point: HookPoint::GroupSplits,
        script: "/tmp/split".into(), digest: digest(&vec![0u8]), timeout_ms: 30, max_calls: 1, fallback: HookMove::OwnGroup });
    for name in ["x", "a", "b"] {
        log.enqueue(name, definition.clone());
        while log.state.items[name].phase != Phase::WaitingJoin { log.pass(name); }
    }
    log.queue("x"); let ahead = log.group_of("x");
    log.queue("a"); log.queue("b"); let pair = log.group_of("a");
    pass_group(&mut log, &ahead, 3);
    pass_group(&mut log, &pair, 2);
    let t = log.reject(&pair);
    let rest = log.group_of("b");
    assert!(matches!(t.effects.iter().find(|e| e.item == rest).unwrap().kind, EffectKind::Decide { .. }));
    let open = log.state.items[&rest].deciding.as_ref().unwrap().clone();
    pass_group(&mut log, &pair, 3);
    assert_eq!(log.state.items[&rest].phase, Phase::Blocked, "a free resource must not bypass the rule");
    let i = &log.state.items[&rest];
    let grant = Grant::Operation { effect: i.pending.clone().unwrap(), run: i.run.clone(), fence: i.fence };
    let command = Command::Decide { item: rest.clone(), call: open.input.call.clone(), fence: i.fence, policy_pin: i.policy_pin.clone(),
        result: HookResult::Receipt { raw: serde_json::json!({"schema":"hook-decision/v1","call":open.input.call,"decision":"own_group"}).to_string(), manifest: "m".into() } };
    log.run(command, grant);
    assert_eq!(log.state.items[&rest].phase, Phase::Running);
    assert!(log.state.items[&rest].hook_refused.is_none());
    assert_eq!(log.replay_digest(), digest(&log.state));
}

#[test]
fn a_split_rule_rejoins_only_a_waiting_group_with_the_same_pins_and_room() {
    for (limit, published_limit) in [(Some(3), None), (None, None), (Some(1), Some(None)), (Some(1), Some(Some(3)))] {
        let mut log = Log::new();
        let mut definition = ticket();
        definition.groups.get_mut("landing").unwrap().limit = limit;
        definition.script_policies.insert("split".into(), ScriptPolicy { point: HookPoint::GroupSplits,
            script: "/tmp/split".into(), digest: digest(&vec![0u8]), timeout_ms: 30, max_calls: 1, fallback: HookMove::OwnGroup });
        for name in ["x", "a", "b", "c"] {
            log.enqueue(name, definition.clone());
            while log.state.items[name].phase != Phase::WaitingJoin { log.pass(name); }
        }
        if let Some(limit) = published_limit {
            let mut published = definition.clone();
            published.groups.get_mut("landing").unwrap().limit = limit;
            log.run(Command::RefreshGroups { definition: published, policy_pin: "policy-1".into() }, Grant::Controller);
        }
        log.queue("x"); let ahead = log.group_of("x");
        log.queue("a"); log.queue("b"); let pair = log.group_of("a");
        pass_group(&mut log, &ahead, 3);
        if published_limit.is_some() {
            // Create the destination under the old limit, then upgrade it while the pair runs.
            log.run(Command::RefreshGroups { definition: definition.clone(), policy_pin: "policy-1".into() }, Grant::Controller);
        }
        log.queue("c"); let waiting = log.group_of("c");
        if let Some(limit) = published_limit {
            assert_eq!(log.state.items[&waiting].group_of.as_ref().unwrap().pin, definition.pin());
            let mut published = definition.clone();
            published.groups.get_mut("landing").unwrap().limit = limit;
            log.run(Command::RefreshGroups { definition: published.clone(), policy_pin: "policy-1".into() }, Grant::Controller);
            assert_eq!(log.state.items[&waiting].group_of.as_ref().unwrap().pin, published.pin());
            assert_eq!(log.state.items["c"].definition, definition, "build evidence retains the old pin");
        }
        pass_group(&mut log, &pair, 2);
        log.reject(&pair); let rest = log.group_of("b");
        let i = &log.state.items[&rest]; let call = i.deciding.as_ref().unwrap().input.call.clone();
        assert!(i.deciding.as_ref().unwrap().input.groups.iter().any(|g| g.item == waiting));
        let grant = Grant::Operation { effect: i.pending.clone().unwrap(), run: i.run.clone(), fence: i.fence };
        let command = |group: &str| Command::Decide { item: rest.clone(), call: call.clone(), fence: i.fence, policy_pin: i.policy_pin.clone(),
            result: HookResult::Receipt { raw: serde_json::json!({"schema":"hook-decision/v1","call":call,"decision":"rejoin","group":group}).to_string(), manifest: "m".into() } };
        let invalid = step(&log.state, &log.input(command(&pair), grant.clone())).unwrap();
        assert!(invalid.state.items[&rest].hook_refused.is_some(), "a running group cannot take more members");
        let valid = command(&waiting);
        log.run(valid, grant);
        assert_eq!(log.group_of("b"), waiting);
        assert_eq!(log.state.items[&waiting].members.len(), 2);
        assert_eq!(log.state.archive[&rest].phase, Phase::Cancelled);
        assert_eq!(log.replay_digest(), digest(&log.state));
    }
}

#[test]
fn a_script_observes_the_new_ci_result_before_the_builtin_rerun() {
    let mut definition = flaky_definition();
    definition.script_policies.insert("ci-script".into(), ScriptPolicy { point: HookPoint::CiResult,
        script: "/tmp/ci-rule".into(), digest: digest(&vec![0u8]), timeout_ms: 30,
        max_calls: 1, fallback: HookMove::Accept });
    let mut log = Log::new();
    let group = verb_group(&mut log, definition);
    walk_to(&mut log, &group, "ci");
    let result = ci_answer(&mut log, &group, "rejected", 1, &["iOS UI tests"]);
    assert!(matches!(result.effects[0].kind, EffectKind::Decide { .. }));
    let item = &log.state.items[&group];
    let call = item.deciding.as_ref().unwrap().input.call.clone();
    assert_eq!(item.deciding.as_ref().unwrap().input.label.as_deref(), Some("rejected"));
    let grant = Grant::Operation { effect: item.pending.clone().unwrap(), run: item.run.clone(), fence: item.fence };
    let command = Command::Decide { item: group.clone(), call: call.clone(), fence: item.fence, policy_pin: item.policy_pin.clone(),
        result: HookResult::Receipt { raw: serde_json::json!({"schema":"hook-decision/v1","call":call,"decision":"accept"}).to_string(), manifest: "m".into() } };
    let result = log.run(command, grant);
    assert_eq!(result.effects[0].kind, EffectKind::Execute(StageKind::RerunCi));
    assert_eq!(log.replay_digest(), digest(&log.state));
}

#[test]
fn unlimited_waiting_group_lands_three_later_tickets_in_one_pass() {
    let mut log = Log::new();
    let mut d = ticket(); d.groups.get_mut("landing").unwrap().limit = None;
    let mut watch = action("watch"); watch.kind = StageKind::Ci;
    d.groups.get_mut("landing").unwrap().steps.push(watch);
    assert_eq!(d.validate(), Ok(()));
    for name in ["a", "b", "c", "d"] {
        log.enqueue(name, d.clone());
        while log.state.items[name].phase != Phase::WaitingJoin { log.pass(name); }
        log.queue(name);
        if name == "a" { let group = log.group_of(name); pass_group(&mut log, &group, 2); }
        log.now += 100;
    }
    let a = log.group_of("a");
    let b = log.group_of("b");
    assert_eq!(b, log.group_of("c")); assert_eq!(b, log.group_of("d"));
    assert_ne!(a, b);
    pass_group(&mut log, &a, 2);
    assert_eq!(log.state.items[&b].members.len(), 3);
    for stage in ["base", "fold", "gate", "watch"] { assert_eq!(log.stage(&b), stage); log.pass(&b); }
    assert_eq!(log.state.items[&b].phase, Phase::Done);
    assert_eq!(log.state.items[&b].handed, ["b", "c", "d"]);
    assert_eq!(log.replay_digest(), digest(&log.state));
}

#[test]
fn publication_merges_existing_waiters_and_new_arrivals_without_repinning_active_work() {
    let mut log = Log::new();
    for name in ["a", "b", "c", "d", "e"] { log.join_group(name); log.queue(name); }
    let active = log.group_of("a");
    let active_before = log.state.items[&active].clone();
    let oldest = log.group_of("b");
    let redundant = log.group_of("d");
    assert_ne!(oldest, redundant);
    let old_ticket_pin = log.state.items["b"].pin.clone();
    let mut published = ticket(); published.groups.get_mut("landing").unwrap().limit = None;
    let command = Command::RefreshGroups { definition: published.clone(), policy_pin: "policy-2".into() };
    assert_eq!(log.try_run(command.clone(), Grant::Decision { principal: "owner".into() }).unwrap_err(), Refusal::Unauthorized);
    log.run(command, Grant::Controller);
    for name in ["b", "c", "d", "e"] { assert_eq!(log.group_of(name), oldest); }
    assert_eq!(log.state.items[&redundant].phase, Phase::Cancelled);
    assert_eq!(log.state.items[&active], active_before);
    assert_eq!(log.state.items[&oldest].group_of.as_ref().unwrap().pin, published.pin());
    assert_eq!(log.state.items[&oldest].policy_pin, "policy-2");
    assert_eq!(log.state.items["b"].pin, old_ticket_pin, "build evidence retains its provenance");
    log.join_group("later"); log.queue("later");
    assert_eq!(log.group_of("later"), oldest, "old pinned tickets use the published rules at join");
    assert_eq!(log.replay_digest(), digest(&log.state));
}

#[test]
fn a_split_sibling_rejoins_the_waiter_while_the_suspect_half_retries_alone() {
    let mut log = Log::new();
    let mut d = ticket(); d.groups.get_mut("landing").unwrap().limit = None;
    for name in ["x", "a", "b"] {
        log.enqueue(name, d.clone());
        while log.state.items[name].phase != Phase::WaitingJoin { log.pass(name); }
        log.queue(name);
    }
    let ahead = log.group_of("x"); let pair = log.group_of("a");
    pass_group(&mut log, &ahead, 3);
    log.enqueue("c", d);
    while log.state.items["c"].phase != Phase::WaitingJoin { log.pass("c"); }
    log.queue("c"); let waiting = log.group_of("c");
    pass_group(&mut log, &pair, 2); log.reject(&pair);
    assert_eq!(log.group_of("a"), pair);
    assert_eq!(log.state.items[&pair].members.len(), 1);
    assert_eq!(log.group_of("b"), waiting);
    assert_eq!(log.state.items[&waiting].members.iter().map(|m| m.item.as_str()).collect::<Vec<_>>(), ["c", "b"]);
    assert_eq!(log.replay_digest(), digest(&log.state));
}

#[test]
fn unlimited_and_published_finite_limits_can_exceed_the_old_thirty_two_ticket_cap() {
    let mut log = Log::new();
    let mut d = ticket(); d.groups.get_mut("landing").unwrap().limit = Some(40);
    for n in 0..36 {
        let name = format!("ticket-{n}");
        log.enqueue(&name, d.clone());
        while log.state.items[&name].phase != Phase::WaitingJoin { log.pass(&name); }
        log.queue(&name);
    }
    let waiting = log.group_of("ticket-1");
    assert_eq!(log.state.items[&waiting].members.len(), 35);
    log.run(Command::Advance, Grant::Controller);
    assert_eq!(log.group_of("ticket-35"), waiting);
}

#[test]
fn a_diagnosed_fold_conflict_returns_only_bad_members_and_never_bisects_the_healthy_batch() {
    let mut log = Log::new();
    let mut d = verb_ticket();
    let g = d.groups.get_mut("landing").unwrap(); g.limit = None;
    g.steps[1].routes.insert("conflict".into(), Target::Split { step: "base".into(), otherwise: Box::new(Target::Fail) });
    g.steps[1].limit = 3;
    for name in ["ahead", "a", "bad-one", "b", "bad-two", "c"] {
        log.enqueue(name, d.clone());
        while log.state.items[name].phase != Phase::WaitingJoin { log.pass(name); }
        log.queue(name);
    }
    let ahead = log.group_of("ahead");
    let group = log.group_of("a");
    pass_group(&mut log, &ahead, 5);
    walk_to(&mut log, &group, "fold");
    let evidence = log.evidence(&group);
    for members in [vec![], vec!["foreign".into()], vec!["bad-one".into(), "bad-one".into()]] {
        assert_eq!(log.try_outcome(&group, Outcome::FoldConflict { members: Some(members), files: vec![], evidence: evidence.clone() }).unwrap_err(), Refusal::Evidence);
    }
    let transition = log.outcome(&group, Outcome::FoldConflict {
        members: Some(vec!["bad-one".into(), "bad-two".into()]), files: vec!["conflicting.rs".into()], evidence });
    assert_eq!(log.stage(&group), "base");
    assert_eq!(log.state.items[&group].members.iter().map(|m| m.item.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
    for name in ["bad-one", "bad-two"] {
        assert_eq!(log.stage(name), "stage-1");
        assert_eq!(log.state.items[name].phase, Phase::Running);
        assert!(log.state.items[name].brief.as_ref().unwrap().contains("conflict"));
        assert!(log.state.items[name].brief.as_ref().unwrap().contains("conflicting.rs"));
        assert_eq!(log.state.items[name].iteration, 0, "integration conflicts spend no review iteration");
        assert!(log.state.items[name].back_edges.is_empty());
        assert_eq!(log.state.items[name].grouped_in, None);
    }
    assert_eq!(transition.effects.iter().filter(|e| e.item == group).count(), 1);
    assert_eq!(log.groups().len(), 1, "the ahead group is archived; no split sibling was created");
    pass_group(&mut log, &group, 5);
    assert_eq!(log.state.items[&group].handed, ["a", "b", "c"]);
    assert_eq!(log.replay_digest(), digest(&log.state));
}

#[test]
fn a_diagnosed_conflict_keeps_the_published_failure_policy_and_releases_an_empty_group() {
    for split in [false, true] {
        let mut log = Log::new();
        let mut d = verb_ticket();
        if split {
            let fold = &mut d.groups.get_mut("landing").unwrap().steps[1];
            fold.routes.insert("conflict".into(), Target::Split { step: "base".into(), otherwise: Box::new(Target::Fail) });
            fold.limit = 3;
        }
        let group = verb_group(&mut log, d);
        walk_to(&mut log, &group, "fold");
        let evidence = log.evidence(&group);
        log.outcome(&group, Outcome::FoldConflict { members: Some(vec!["a".into()]), files: vec![], evidence });
        assert_eq!(log.state.items[&group].phase, Phase::Failed);
        assert!(!owns_resource(&log.state.items[&group]));
        assert!(log.state.items[&group].members.is_empty());
        assert_eq!(log.stage("a"), "stage-1");
        assert_eq!(log.state.items["a"].grouped_in, None);
        assert_eq!(log.replay_digest(), digest(&log.state));
    }
}

#[test]
fn a_split_scripts_own_group_decision_survives_builtin_consolidation() {
    let mut log = Log::new();
    let mut definition = ticket();
    definition.groups.get_mut("landing").unwrap().limit = None;
    definition.script_policies.insert("split".into(), ScriptPolicy { point: HookPoint::GroupSplits,
        script: "/tmp/split".into(), digest: digest(&vec![0u8]), timeout_ms: 30,
        max_calls: 1, fallback: HookMove::OwnGroup });
    for name in ["x", "a", "b", "c"] {
        log.enqueue(name, definition.clone());
        while log.state.items[name].phase != Phase::WaitingJoin { log.pass(name); }
    }
    log.queue("x"); let ahead = log.group_of("x");
    log.queue("a"); log.queue("b"); let pair = log.group_of("a");
    pass_group(&mut log, &ahead, 3);
    log.queue("c"); let waiting = log.group_of("c");
    pass_group(&mut log, &pair, 2);
    log.reject(&pair); let rest = log.group_of("b");
    let item = &log.state.items[&rest];
    let call = item.deciding.as_ref().unwrap().input.call.clone();
    let grant = Grant::Operation { effect: item.pending.clone().unwrap(), run: item.run.clone(), fence: item.fence };
    let command = Command::Decide { item: rest.clone(), call: call.clone(), fence: item.fence,
        policy_pin: item.policy_pin.clone(), result: HookResult::Receipt { manifest: "m".into(),
            raw: serde_json::json!({"schema":"hook-decision/v1","call":call,"decision":"own_group"}).to_string() } };
    log.run(command, grant);
    log.run(Command::Advance, Grant::Controller);
    assert_eq!(log.group_of("b"), rest, "consolidation must preserve the script's choice");
    assert_ne!(rest, waiting);
    assert_eq!(log.state.items[&waiting].members.len(), 1);
    assert_eq!(log.replay_digest(), digest(&log.state));
}

#[test]
fn every_new_split_group_gets_its_own_script_call_budget() {
    let mut log = Log::new();
    let mut definition = ticket();
    definition.groups.get_mut("landing").unwrap().limit = None;
    definition.script_policies.insert("split".into(), ScriptPolicy { point: HookPoint::GroupSplits,
        script: "/tmp/split".into(), digest: digest(&vec![0u8]), timeout_ms: 30,
        max_calls: 1, fallback: HookMove::OwnGroup });
    for name in ["x", "a", "b", "c", "d"] {
        log.enqueue(name, definition.clone());
        while log.state.items[name].phase != Phase::WaitingJoin { log.pass(name); }
    }
    log.queue("x"); let ahead = log.group_of("x");
    for name in ["a", "b", "c", "d"] { log.queue(name); }
    let parent = log.group_of("a");
    pass_group(&mut log, &ahead, 3);
    pass_group(&mut log, &parent, 2);
    log.reject(&parent);
    let child = log.group_of("c");
    let item = &log.state.items[&child];
    let call = item.deciding.as_ref().unwrap().input.call.clone();
    let grant = Grant::Operation { effect: item.pending.clone().unwrap(), run: item.run.clone(), fence: item.fence };
    let command = Command::Decide { item: child.clone(), call: call.clone(), fence: item.fence,
        policy_pin: item.policy_pin.clone(), result: HookResult::Receipt { manifest: "m".into(),
            raw: serde_json::json!({"schema":"hook-decision/v1","call":call,"decision":"own_group"}).to_string() } };
    log.run(command, grant);
    pass_group(&mut log, &parent, 3);
    pass_group(&mut log, &child, 2);
    let split = log.reject(&child);
    let grandchild = log.group_of("d");
    let item = &log.state.items[&grandchild];
    let open = item.deciding.as_ref().expect("a new split must not inherit the parent's exhausted budget");
    assert_ne!(open.input.call, call);
    assert_eq!(item.hook_calls["split"], 1);
    assert_eq!(log.state.items[&child].hook_calls["split"], 1);
    assert_eq!(item.phase, Phase::Blocked);
    assert!(split.effects.iter().any(|e| e.item == grandchild && matches!(e.kind, EffectKind::Decide { .. })));
    assert_eq!(log.replay_digest(), digest(&log.state));
}

#[test]
fn gate_diagnostics_survive_both_halves_and_replay() {
    let mut log = Log::new();
    for t in ["x", "a", "b"] { log.join_group(t); }
    log.queue("x");
    let ahead = log.group_of("x");
    log.queue("a"); log.queue("b");
    let pair = log.group_of("a");
    pass_group(&mut log, &ahead, 3);
    pass_group(&mut log, &pair, 2);
    let evidence = log.evidence(&pair);
    log.outcome(&pair, Outcome::RejectedWithDetails(evidence, GroupFailure {
        job: "cargo test".into(), first_line: "error: gate assertion failed".into(), split: false,
    }));
    let second = log.group_of("b");
    for id in [&pair, &second] {
        let failure = log.state.items[id].group_failure.as_ref().unwrap();
        assert_eq!(failure.job, "cargo test");
        assert_eq!(failure.first_line, "error: gate assertion failed");
        assert!(failure.split);
    }
    log.pass(&pair);
    assert!(log.state.items[&pair].group_failure.is_some(), "a later step must not erase why it split");
    assert_eq!(log.replay_digest(), digest(&log.state));
    // Missing optional state fields retain the old serialized identity.
    let mut old = serde_json::to_value(&log.state).unwrap();
    for item in old["items"].as_object_mut().unwrap().values_mut() {
        item.as_object_mut().unwrap().remove("group_failure");
    }
    let decoded: State = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), old);
}

#[test]
fn ci_diagnostics_survive_a_failed_rerun() {
    let mut log = Log::new();
    let group = verb_group(&mut log, flaky_definition());
    walk_to(&mut log, &group, "ci");
    ci_answer(&mut log, &group, "rejected", 1, &["iOS UI tests"]);
    log.pass(&group);
    let evidence = log.evidence(&group);
    let result = log.outcome(&group, Outcome::CiResult {
        label: "rejected".into(), evidence,
        run: Some(CiRun { id: 73, attempt: 2, failed_jobs: vec!["iOS UI tests".into()], failed_steps: vec![] }),
        failure: Some(GroupFailure { job: "iOS UI tests".into(), first_line: "error: failed".into(), split: false }),
    });
    assert_eq!(result.state.items[&group].group_failure.as_ref().unwrap().first_line, "error: failed");
    assert_eq!(log.replay_digest(), digest(&log.state));
}

#[test]
fn split_halves_queued_past_patience_start_a_full_base_wait() {
    let mut log = Log::new();
    let mut d = verb_ticket();
    d.groups.get_mut("landing").unwrap().steps[2].routes.insert("rejected".into(),
        Target::Split { step: "base".into(), otherwise: Box::new(Target::Fail) });
    d.groups.get_mut("landing").unwrap().steps[2].limit = 3;
    for id in ["a", "b", "c"] {
        log.enqueue(id, d.clone());
        while log.state.items[id].phase != Phase::WaitingJoin { log.pass(id); }
    }
    log.queue("a"); log.queue("b"); log.queue("c");
    let ahead = log.group_of("a");
    walk_to(&mut log, &ahead, "ci"); log.pass(&ahead);
    let group = log.group_of("b");
    walk_to(&mut log, &group, "gate");
    log.reject(&group);
    let half = log.group_of("c");
    assert_eq!(log.state.items[&half].wait_since, None);
    log.now = 20_000_000;
    // Gate-during-CI (ov-1209) can launch the waiting half as soon as the preceding push settles.
    let mut created = None;
    while log.stage(&group) != "ci" {
        let next = log.pass(&group);
        if let Some(effect) = next.effects.iter().find(|e| e.item == half) { created = Some(effect.created_ms); }
    }
    let next = log.pass(&group);
    if let Some(effect) = next.effects.iter().find(|e| e.item == half) { created = Some(effect.created_ms); }
    assert_eq!(created, Some(log.now), "the half starts its own full patience window, even if it overlaps CI");
}

#[test]
fn landing_safe_failures_retry_twice_without_splitting() {
    let mut log = Log::new();
    let group = verb_group(&mut log, verb_ticket());
    walk_to(&mut log, &group, "gate");
    for _ in 0..2 {
        let next = log.outcome(&group, Outcome::SafeFailure);
        assert_eq!(next.state.items[&group].phase, Phase::Running);
        assert_eq!(log.stage(&group), "gate");
        assert_eq!(log.groups().len(), 1);
    }
    log.outcome(&group, Outcome::SafeFailure);
    assert_eq!(log.state.items[&group].phase, Phase::Failed);
}

#[test]
fn a_cancelled_group_returns_its_members_to_waiting_at_the_join() {
    let mut log = Log::new();
    log.join_group("a"); log.queue("a");
    let group = log.group_of("a");
    log.run(Command::Cancel { item: group.clone(), reason: None }, Grant::Controller);
    if log.state.items[&group].pending.is_some() {
        let operation = log.state.items[&group].stopping.clone().unwrap();
        log.outcome(&group, Outcome::Stopped { operation, receipt: "stop-receipt".into() });
    }
    assert_eq!(log.state.items["a"].phase, Phase::WaitingJoin);
    assert_eq!(log.stage("a"), "join");
    assert!(log.state.items["a"].grouped_in.is_none());
    log.queue("a");
    assert!(log.state.items["a"].grouped_in.is_some());
    assert_eq!(log.replay_digest(), digest(&log.state));
}

#[test]
fn a_named_gate_breaker_returns_with_output_and_the_rest_retry_together() {
    let mut log = Log::new();
    for t in ["x", "a", "b", "c"] {
        let mut d = ticket(); d.groups.get_mut("landing").unwrap().limit = Some(4);
        log.enqueue(t, d);
        while log.state.items[t].phase != Phase::WaitingJoin { log.pass(t); }
    }
    for t in ["x", "a", "b", "c"] { log.queue(t); }
    let ahead = log.group_of("x"); let group = log.group_of("a");
    pass_group(&mut log, &ahead, 3); pass_group(&mut log, &group, 2);
    let evidence = log.evidence(&group);
    let brief = "Landing gate failed: copy lint\nratchet: broken.rs has extra strings";
    let transition = log.outcome(&group, Outcome::GateRejected { evidence, members: vec!["b".into()], brief: brief.into(),
        failure: GroupFailure { job: "copy lint".into(), first_line: "ratchet: broken.rs".into(), split: false } });
    assert_eq!(log.state.items["b"].brief.as_deref(), Some(brief));
    assert_eq!(log.stage("b"), "stage-1");
    assert!(log.state.items["b"].grouped_in.is_none());
    assert_eq!(log.group_of("a"), log.group_of("c"));
    assert_eq!(log.state.items[&group].members.iter().map(|m| m.item.as_str()).collect::<Vec<_>>(), ["a", "c"]);
    assert_eq!(log.stage(&group), "base");
    assert!(transition.effects.iter().any(|e| e.item == "b" && e.brief.as_deref() == Some(brief)));
    assert_eq!(log.replay_digest(), digest(&log.state));
}

#[test]
fn an_unnamed_gate_failure_still_halves_the_batch_and_briefs_the_singleton() {
    let mut log = Log::new();
    for t in ["x", "a", "b"] { log.join_group(t); log.queue(t); }
    let ahead = log.group_of("x"); let group = log.group_of("a");
    pass_group(&mut log, &ahead, 3); pass_group(&mut log, &group, 2);
    let reject = |log: &mut Log, id: &str| {
        let evidence = log.evidence(id);
        log.outcome(id, Outcome::GateRejected { evidence, members: vec![], brief: "error: unnamed failure".into(),
            failure: GroupFailure { job: "gate".into(), first_line: "error: unnamed failure".into(), split: false } });
    };
    reject(&mut log, &group);
    assert_ne!(log.group_of("a"), log.group_of("b"));
    assert_eq!(log.state.items[&group].members.len(), 1);
    pass_group(&mut log, &group, 2); reject(&mut log, &group);
    assert_eq!(log.state.items["a"].brief.as_deref(), Some("error: unnamed failure"));
    assert_eq!(log.stage("a"), "stage-1");
    assert_eq!(log.replay_digest(), digest(&log.state));
}

#[test]
fn gate_attribution_refuses_foreign_members_and_releases_an_empty_group() {
    let mut log = Log::new();
    log.join_group("a"); log.queue("a");
    let group = log.group_of("a"); pass_group(&mut log, &group, 2);
    let answer = |members| Outcome::GateRejected { evidence: log.evidence(&group), members,
        brief: "error: broken.rs".into(), failure: GroupFailure { job: "gate".into(), first_line: "error".into(), split: false } };
    let foreign = answer(vec!["foreign".into()]); let duplicate = answer(vec!["a".into(), "a".into()]);
    let valid = answer(vec!["a".into()]);
    assert_eq!(log.try_outcome(&group, foreign).unwrap_err(), Refusal::Evidence);
    assert_eq!(log.try_outcome(&group, duplicate).unwrap_err(), Refusal::Evidence);
    log.outcome(&group, valid);
    assert_eq!(log.state.items[&group].phase, Phase::Failed);
    assert!(log.state.items[&group].members.is_empty());
    assert!(!owns_resource(&log.state.items[&group]));
    assert_eq!(log.stage("a"), "stage-1");
    assert_eq!(log.state.items["a"].brief.as_deref(), Some("error: broken.rs"));
    assert_eq!(log.replay_digest(), digest(&log.state));
}
