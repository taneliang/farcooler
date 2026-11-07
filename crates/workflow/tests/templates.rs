//! The four landing templates: each validates, has the shape its name promises, and is walked through
//! the reducer end to end with stubbed outcomes. Private fixtures only.
use farcooler_workflow::templates::*;
use farcooler_workflow::*;
use std::collections::{BTreeMap, BTreeSet};

struct Fixture;
impl Pins for Fixture {
    fn operation(&self, version: &str, argument: &str) -> OperationSpec {
        OperationSpec { adapter_version: version.into(), arguments: vec!["pin".into(), argument.into()], environment: BTreeMap::new() }
    }
    fn wait(&self, version: &str, argument: &str, every: u64, patience: u64) -> OperationSpec {
        OperationSpec { adapter_version: version.into(), environment: BTreeMap::new(),
            arguments: vec!["pin".into(), argument.into(), every.to_string(), patience.to_string()] }
    }
    fn landing_resource(&self) -> String { "ref:origin:main".into() }
    fn landing_target(&self) -> String { "refs/heads/main".into() }
    fn ci_subject(&self) -> String { "owner/repo".into() }
}
fn plan(t: Template) -> Plan { Plan::new(t, vec![Gate::Command(0)]) }
fn make(plan: &Plan) -> Definition { build(plan, &Fixture).unwrap() }
fn ids(d: &Definition) -> Vec<&str> { d.stages.iter().map(|s| s.id.as_str()).collect() }
fn kinds(steps: &[Stage]) -> Vec<&StageKind> { steps.iter().map(|s| &s.kind).collect() }

#[test]
fn every_template_validates_and_a_name_finds_it() {
    for t in Template::ALL {
        let d = make(&plan(t));
        assert_eq!(d.validate(), Ok(()), "{}", t.name());
        assert_eq!(Template::parse(t.name()), Some(t));
        assert!(!t.about().is_empty());
    }
    assert_eq!(Template::parse("shipping"), None);
}
#[test]
fn batched_and_per_ticket_push_differ_by_one_number() {
    let (mut batched, mut single) = (make(&plan(Template::BatchedPush)), make(&plan(Template::TicketPush)));
    assert_eq!((batched.groups["landing"].limit, single.groups["landing"].limit), (8, 1));
    batched.groups.get_mut("landing").unwrap().limit = 1;
    batched.source = single.source.clone();
    single.source = batched.source.clone();
    assert_eq!(batched, single);
}
#[test]
fn a_push_template_lands_through_base_fold_gate_push_ci() {
    let d = make(&plan(Template::BatchedPush));
    assert_eq!(ids(&d), ["provision", "build", "join"]);
    let g = &d.groups["landing"];
    assert_eq!(kinds(&g.steps), [&StageKind::AwaitBase, &StageKind::Fold, &StageKind::Run, &StageKind::Push, &StageKind::AwaitCi]);
    assert_eq!(g.exclusive, "ref:origin:main");
    let mut p = plan(Template::BatchedPush); p.ci = false; p.green_base = false;
    let d = make(&p);
    assert_eq!(kinds(&d.groups["landing"].steps), [&StageKind::AwaitBase, &StageKind::Fold, &StageKind::Run, &StageKind::Push]);
    assert_eq!(d.groups["landing"].steps[0].operation.arguments[1], "any");
}
#[test]
fn a_push_with_no_command_gate_is_not_built() {
    for gates in [vec![], vec![Gate::Agent { argument: "reviewer".into() }], vec![Gate::Person { principal: "owner".into() }]] {
        assert_eq!(build(&Plan::new(Template::BatchedPush, gates), &Fixture), Err(Refusal::Definition));
    }
}
#[test]
fn each_gate_can_be_a_command_an_agent_or_a_person_and_keeps_its_order() {
    let gates = vec![Gate::Command(0), Gate::Agent { argument: "reviewer".into() }, Gate::Person { principal: "owner".into() }, Gate::Command(1)];
    let d = make(&Plan::new(Template::BatchedPush, gates.clone()));
    assert_eq!(ids(&d), ["provision", "build", "gate-0", "gate-1", "join"]);
    assert_eq!(d.stages[2].kind, StageKind::Review(Reviewer::Dispatch));
    assert_eq!(d.stages[3].kind, StageKind::Review(Reviewer::Person("owner".into())));
    let group: Vec<_> = d.groups["landing"].steps.iter().filter(|s| s.kind == StageKind::Run).map(|s| s.operation.arguments[1].as_str()).collect();
    assert_eq!(group, ["0", "1"], "command gates run on the folded result, in order");
    // Without a fold the command gate judges the ticket itself.
    let d = make(&Plan::new(Template::TicketPr, gates));
    assert_eq!(ids(&d)[..6], ["provision", "build", "gate-0", "gate-1", "gate-2", "gate-3"]);
    assert_eq!(kinds(&d.stages)[2..6], [&StageKind::Check, &StageKind::Review(Reviewer::Dispatch), &StageKind::Review(Reviewer::Person("owner".into())), &StageKind::Check]);
}

struct Walk { state: State, now: i64, visited: Vec<(String, String)> }
impl Walk {
    fn new() -> Self { Self { state: State::empty("workspace".into(), 1), now: 1000, visited: vec![] } }
    fn input(&self, command: Command, grant: Grant) -> Input {
        let s = &self.state;
        Input { busy_resources: BTreeSet::new(), schema: 1, id: format!("command-{}", s.revision + 1), workspace: s.workspace.clone(),
            expected_revision: s.revision, epoch: s.epoch, source: "fixture".into(), source_sequence: s.revision + 1,
            now_ms: self.now, grant, command }
    }
    fn run(&mut self, command: Command, grant: Grant) -> Result<(), Refusal> {
        let input = self.input(command, grant);
        self.state = step(&self.state, &input)?.state;
        Ok(())
    }
    fn enqueue(&mut self, item: &str, definition: Definition) {
        self.run(Command::Enqueue { item: item.into(), definition, policy_pin: "policy".into(), resource: format!("res-{item}"),
            dependencies: BTreeSet::new(), initial_candidate: None }, Grant::Controller).unwrap();
    }
    fn evidence(&self, item: &str) -> Evidence {
        let i = &self.state.items[item];
        Evidence { candidate: i.candidate.clone().unwrap_or(format!("cand-{item}")), manifest: "m".into(), receipt: "r".into(), policy_pin: i.policy_pin.clone() }
    }
    fn at(&self, item: &str) -> String { let i = &self.state.items[item]; i.definition.stages[i.stage].id.clone() }
    fn outcome(&mut self, item: &str, make: impl Fn(Evidence) -> Outcome) {
        let i = &self.state.items[item];
        self.visited.push((item.into(), self.at(item)));
        let (effect, run, fence) = (i.pending.clone().unwrap(), i.run.clone(), i.fence);
        let outcome = make(self.evidence(item));
        self.run(Command::Observe { item: item.into(), effect: effect.clone(), run: run.clone(), fence, outcome }, Grant::Operation { effect, run, fence }).unwrap();
    }
    fn pass(&mut self, item: &str) { self.outcome(item, Outcome::Passed) }
    fn label(&mut self, item: &str, label: &str) { self.outcome(item, |e| Outcome::Labeled(label.into(), e)) }
    /// Advance every item that can move, answering waits the way their step says: the orchestrator queues,
    /// the named person reviews or approves. Returns false once nothing can move.
    fn tick(&mut self) -> bool {
        let ids: Vec<String> = self.state.items.keys().cloned().collect();
        for id in ids {
            let Some(item) = self.state.items.get(&id) else { continue };
            let stage = &item.definition.stages[item.stage.min(item.definition.stages.len() - 1)];
            match item.phase {
                Phase::WaitingJoin => { self.run(Command::Queue { item: id.clone() }, Grant::Controller).unwrap(); return true; }
                Phase::WaitingReview => {
                    let (run, evidence) = (item.run.clone(), self.evidence(&id));
                    let StageKind::Review(Reviewer::Person(p)) = &stage.kind else { panic!("only a person review waits") };
                    let grant = Grant::Decision { principal: p.clone() };
                    self.visited.push((id.clone(), self.at(&id)));
                    self.run(Command::Review { item: id, run, evidence, passed: true, reason: None }, grant).unwrap();
                    return true;
                }
                Phase::WaitingApproval => {
                    let (run, pin, candidate) = (item.run.clone(), item.pin.clone(), item.candidate.clone());
                    let Progression::AwaitApproval { principal } = &stage.progression else { panic!() };
                    let grant = Grant::Decision { principal: principal.clone() };
                    self.run(Command::Approve { item: id, run, pin, candidate }, grant).unwrap();
                    return true;
                }
                _ if item.pending.is_some() => { self.pass(&id); return true; }
                _ => {}
            }
        }
        false
    }
    fn finish(&mut self) { while self.tick() {} }
}
fn passed_through(w: &Walk, item: &str) -> Vec<String> {
    let mut seen: Vec<String> = vec![];
    for (i, s) in &w.visited { if i == item && seen.last() != Some(s) { seen.push(s.clone()); } }
    seen
}
fn done(w: &Walk, item: &str) -> bool {
    w.state.items.get(item).map(|i| &i.phase).or_else(|| w.state.archive.get(item).map(|a| &a.phase)) == Some(&Phase::Done)
}

#[test]
fn a_batched_push_walks_from_a_ticket_to_a_landed_group_and_every_ticket_is_done() {
    let mut w = Walk::new();
    let d = make(&plan(Template::BatchedPush));
    w.enqueue("one", d.clone());
    w.finish();
    assert!(done(&w, "one"));
    let group = w.visited.iter().find(|(i, _)| i != "one").map(|(i, _)| i.clone()).expect("a group item ran");
    assert_eq!(passed_through(&w, &group), ["base", "fold", "gate-0", "push", "ci"]);
    assert_eq!(passed_through(&w, "one"), ["provision", "build"]);
}
#[test]
fn a_per_ticket_pull_request_walks_branch_pr_merge_and_ci_with_a_person_gate() {
    let mut w = Walk::new();
    let mut p = Plan::new(Template::TicketPr, vec![Gate::Command(0), Gate::Person { principal: "owner".into() }]);
    p.ci = true;
    w.enqueue("one", make(&p));
    w.finish();
    assert!(done(&w, "one"));
    assert_eq!(passed_through(&w, "one"), ["provision", "build", "gate-0", "gate-1", "branch", "pr", "merged", "ci"]);
}
#[test]
fn a_pull_request_that_asks_for_changes_goes_back_to_build_and_a_closed_one_fails() {
    let mut w = Walk::new();
    w.enqueue("one", make(&plan(Template::TicketPr)));
    while w.at("one") != "merged" { w.tick(); }
    w.label("one", "changes_requested");
    assert_eq!(w.at("one"), "build");
    while w.at("one") != "merged" { w.tick(); }
    w.label("one", "changes_requested");
    while w.at("one") != "merged" { w.tick(); }
    // The third request is over the template's limit of two: it fails rather than looping.
    let (effect, run, fence) = { let i = &w.state.items["one"]; (i.pending.clone().unwrap(), i.run.clone(), i.fence) };
    let evidence = w.evidence("one");
    let input = w.input(Command::Observe { item: "one".into(), effect: effect.clone(), run: run.clone(), fence, outcome: Outcome::Labeled("changes_requested".into(), evidence) }, Grant::Operation { effect, run, fence });
    let after = step(&w.state, &input).map(|t| t.state.items["one"].phase.clone());
    assert_eq!(after, Ok(Phase::Failed));
    let mut w = Walk::new();
    w.enqueue("two", make(&plan(Template::TicketPr)));
    while w.at("two") != "merged" { w.tick(); }
    w.label("two", "closed");
    assert_eq!(w.state.items["two"].phase, Phase::Failed);
}
#[test]
fn a_batched_pull_request_rehearses_together_and_a_person_merges_afterwards() {
    let mut w = Walk::new();
    let d = make(&plan(Template::BatchedPr));
    w.enqueue("one", d.clone());
    w.enqueue("two", d);
    w.finish();
    assert!(done(&w, "one") && done(&w, "two"));
    let group = w.visited.iter().find(|(i, _)| i != "one" && i != "two").map(|(i, _)| i.clone()).expect("a rehearsal group ran");
    assert_eq!(passed_through(&w, &group), ["base", "fold", "gate-0"], "a rehearsal never pushes");
    assert_eq!(passed_through(&w, "one"), ["provision", "build", "branch", "pr", "ready", "merged"]);
}
#[test]
fn a_gate_that_rejects_sends_a_ticket_back_to_build_on_every_template() {
    for t in Template::ALL {
        let mut w = Walk::new();
        let gates = vec![Gate::Command(0), Gate::Agent { argument: "reviewer".into() }];
        let d = build(&Plan::new(t, gates), &Fixture).unwrap();
        w.enqueue("one", d);
        while !w.at("one").starts_with("gate-") { w.tick(); }
        let name = w.at("one");
        w.outcome("one", Outcome::Rejected);
        assert_eq!(w.at("one"), "build", "{} {name}", t.name());
    }
}
