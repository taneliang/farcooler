//! The differential test's worlds, drivers and scenarios (ov-1273), shared with the migration tests (ov-1281):
//! a migration can be cut in at any point of an old run, and the pipeline driver carries on from there.
#![allow(dead_code)]
#[path = "../pipeline_support/mod.rs"]
pub mod support;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use farcooler_workflow as old;
use farcooler_workflow::adapters::coding::{data_of, opaque, Conflict, Facts, Op};
use farcooler_workflow::adapters::{coding_templates, linked};
use farcooler_workflow::pipeline as new;
use farcooler_workflow::templates::{build, Gate, PathRoutes, Pins, Plan, Template};
use serde_json::json;

// ---- the world ----

#[derive(Clone, Default)]
pub struct World {
    /// A ticket's own steps, by (station, ticket): answers in order, then `passed`.
    pub steps: BTreeMap<(String, String), VecDeque<&'static str>>,
    /// A ticket's first `n` builds fail the landing gate.
    pub gate_bad: BTreeMap<String, u32>,
    /// A ticket's first `n` builds conflict at the fold, which names it (or names no one, if `blind`).
    pub conflict: BTreeMap<String, u32>,
    pub blind: bool,
    /// The remote refuses this many pushes, then takes them.
    pub refused: u32,
    /// The landing base's answers, then `passed`.
    pub base: VecDeque<&'static str>,
    /// CI's answers (`red:flaky` is red on a job the policy reruns), then `passed`.
    pub ci: VecDeque<&'static str>,
    /// The orchestrator's interventions: (ticket, at station, what), each once.
    pub act: Vec<(&'static str, &'static str, Act)>,
    /// A landing step fails this many times, saying nothing went out (a safe failure), then answers as usual.
    pub fails: BTreeMap<&'static str, u32>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act { Cancel, SendBack, Retry }
impl World {
    pub fn step(mut self, station: &str, ticket: &str, answers: &[&'static str]) -> Self {
        self.steps.insert((station.into(), ticket.into()), answers.iter().copied().collect()); self
    }
    /// What `station` answers for `members`, given how many times each has been built.
    pub fn answer(&mut self, station: &str, members: &[String], builds: &BTreeMap<String, u32>) -> (&'static str, Vec<String>) {
        let built = |m: &String| builds.get(m).copied().unwrap_or(0);
        let local = station.rsplit('.').next().unwrap();
        // A step inside a landing batch is named `<batch>.<step>`; a ticket's own step is not.
        if let Some(n) = self.fails.get_mut(local).filter(|n| **n > 0 && station.contains('.')) { *n -= 1; return ("fail", vec![]); }
        match (station.contains('.'), local) {
            (true, "base") => (self.base.pop_front().unwrap_or("passed"), vec![]),
            (true, "fold") => {
                let bad: Vec<String> = members.iter().filter(|m| built(m) <= self.conflict.get(*m).copied().unwrap_or(0)).cloned().collect();
                if bad.is_empty() { ("passed", vec![]) } else { ("conflict", if self.blind { vec![] } else { bad }) }
            }
            (true, g) if g.starts_with("gate-") => {
                let bad = members.iter().any(|m| built(m) <= self.gate_bad.get(m).copied().unwrap_or(0));
                (if bad { "red" } else { "passed" }, vec![])
            }
            (true, "push") => if self.refused > 0 { self.refused -= 1; ("refused", vec![]) } else { ("passed", vec![]) },
            (_, "ci") => (self.ci.pop_front().unwrap_or("passed"), vec![]),
            _ => {
                let key = (local.to_string(), members[0].clone());
                (self.steps.get_mut(&key).and_then(VecDeque::pop_front).unwrap_or("passed"), vec![])
            }
        }
    }
}

// ---- outcomes ----

#[derive(Debug, Default, PartialEq)]
pub struct Outcomes { pub tickets: BTreeMap<String, String>, pub landings: Vec<Vec<String>>, pub builds: BTreeMap<String, u32> }
/// `inner` names a station inside a landing batch: its outcome is the batch station's.
pub fn top(station: &str) -> String { station.split('.').next().unwrap().to_string() }
pub fn diff(old: &Outcomes, new: &Outcomes) -> Vec<String> {
    let mut out = Vec::new();
    for (t, o) in &old.tickets {
        let n = new.tickets.get(t).cloned().unwrap_or_default();
        if *o != n { out.push(format!("{t}: old {o}, new {n}")); }
    }
    if old.landings != new.landings { out.push(format!("landings: old {:?}, new {:?}", old.landings, new.landings)); }
    if old.builds != new.builds { out.push(format!("builds: old {:?}, new {:?}", old.builds, new.builds)); }
    out
}

// ---- the old reducer ----

pub struct Fixture;
impl Pins for Fixture {
    fn operation(&self, version: &str, argument: &str) -> old::OperationSpec {
        old::OperationSpec { adapter_version: version.into(), arguments: vec!["pin".into(), argument.into()], environment: BTreeMap::new() }
    }
    fn wait(&self, version: &str, argument: &str, every: u64, patience: u64) -> old::OperationSpec {
        old::OperationSpec { adapter_version: version.into(), environment: BTreeMap::new(),
            arguments: vec!["pin".into(), argument.into(), every.to_string(), patience.to_string()] }
    }
    fn landing_resource(&self) -> String { "ref:origin:main".into() }
    fn landing_target(&self) -> String { "refs/heads/main".into() }
    fn ci_subject(&self) -> String { "owner/repo".into() }
}
#[derive(Clone)]
pub struct Old { pub state: old::State, pub log: Vec<old::Input>, pub world: World, pub builds: BTreeMap<String, u32>, pub landings: Vec<Vec<String>>,
    pub tickets: Vec<String>, pub done: BTreeSet<usize>, pub n: u64, pub ci_attempt: u32 }
impl Old {
    pub fn send(&mut self, command: old::Command, grant: old::Grant) {
        let s = &self.state;
        let input = old::Input { busy_resources: BTreeSet::new(), schema: 1, id: format!("command-{}", s.revision + 1),
            workspace: s.workspace.clone(), expected_revision: s.revision, epoch: s.epoch, source: "fixture".into(),
            source_sequence: s.revision + 1, now_ms: 1000, grant, command };
        let t = old::step(&self.state, &input).unwrap_or_else(|e| panic!("old reducer refused {:?}: {e:?}", input.command));
        self.state = t.state;
        self.log.push(input);
    }
    pub fn fresh(&mut self, what: &str) -> String { self.n += 1; format!("{what}-{}", self.n) }
    pub fn ev(candidate: &str) -> old::Evidence {
        old::Evidence { candidate: candidate.into(), manifest: "m".into(), receipt: "r".into(), policy_pin: "policy".into() }
    }
    pub fn stage(&self, id: &str) -> Option<(String, old::Phase)> {
        let i = self.state.items.get(id)?;
        Some((i.definition.stages.get(i.stage).map(|s| s.id.clone()).unwrap_or_default(), i.phase.clone()))
    }
    pub fn intervene(&mut self) -> bool {
        for (k, (t, at, what)) in self.world.act.clone().into_iter().enumerate() {
            if self.done.contains(&k) { continue; }
            let Some((stage, phase)) = self.stage(t) else { continue };
            let ready = stage == at && match what {
                Act::Cancel => matches!(phase, old::Phase::Running),
                Act::SendBack => matches!(phase, old::Phase::WaitingReview),
                Act::Retry => matches!(phase, old::Phase::Failed),
            };
            if !ready { continue; }
            self.done.insert(k);
            let item = t.to_string();
            self.send(match what {
                Act::Cancel => old::Command::Cancel { item, reason: None },
                Act::SendBack => old::Command::SendBack { item, reason: "send it back".into() },
                Act::Retry => old::Command::Retry { item },
            }, old::Grant::Controller);
            return true;
        }
        false
    }
    pub fn act(&mut self) -> bool {
        if self.intervene() { return true; }
        for t in self.tickets.clone() {
            let Some(item) = self.state.items.get(&t) else { continue };
            if item.pending.is_some() { self.answer(&t); return true; }
            if item.phase == old::Phase::WaitingReview { self.review(&t); return true; }
        }
        for t in self.tickets.clone() {
            if self.state.items.get(&t).is_some_and(|i| i.phase == old::Phase::WaitingJoin) {
                self.send(old::Command::Queue { item: t }, old::Grant::Controller);
                return true;
            }
        }
        let groups: Vec<String> = self.state.items.iter().filter(|(_, i)| i.group_of.is_some() && i.pending.is_some())
            .map(|(id, _)| id.clone()).collect();
        if let Some(g) = groups.first() { self.answer(g); return true; }
        false
    }
    pub fn review(&mut self, t: &str) {
        let item = &self.state.items[t];
        let stage = &item.definition.stages[item.stage];
        let old::StageKind::Review(old::Reviewer::Person(p) | old::Reviewer::Orchestrator(p)) = &stage.kind else { unreachable!() };
        let (station, run, principal) = (stage.id.clone(), item.run.clone(), p.clone());
        let evidence = item.last_evidence.clone().unwrap();
        let (label, _) = self.world.answer(&station, &[t.to_string()], &self.builds);
        let passed = label == "passed";
        self.send(old::Command::Review { item: t.into(), run, evidence, passed,
            reason: (!passed).then(|| "not yet".to_string()) }, old::Grant::Decision { principal });
    }
    pub fn answer(&mut self, id: &str) {
        let item = self.state.items[id].clone();
        let (effect, run, fence) = (item.pending.clone().unwrap(), item.run.clone(), item.fence);
        let stage = item.definition.stages[item.stage].clone();
        let members: Vec<String> = if item.members.is_empty() { vec![id.to_string()] } else { item.members.iter().map(|m| m.item.clone()).collect() };
        let cand = item.candidate.clone().unwrap_or_else(|| "none".into());
        let outcome = if run.starts_with("cancel:") {
            old::Outcome::Stopped { operation: item.stopping.clone().unwrap(), receipt: "quiet".into() }
        } else if item.ci_retry.as_ref().is_some_and(|r| r.pending) {
            old::Outcome::Passed(Self::ev(&cand))
        } else {
            let station = if item.group_of.is_some() { format!("join.{}", stage.id) } else { stage.id.clone() };
            let (label, named) = self.world.answer(&station, &members, &self.builds);
            use old::StageKind as K;
            match (&stage.kind, label) {
                (_, "fail") => old::Outcome::SafeFailure,
                (K::Build, "passed") => {
                    *self.builds.entry(id.to_string()).or_default() += 1;
                    let c = self.fresh(&format!("{id}-c"));
                    old::Outcome::Passed(Self::ev(&c))
                }
                (K::AwaitBase | K::Fold | K::AwaitPr, "passed") => { let c = self.fresh("made"); old::Outcome::Passed(Self::ev(&c)) }
                (K::Push, "passed") => { self.landings.push(members.clone()); old::Outcome::Passed(Self::ev(&cand)) }
                (_, "passed") => old::Outcome::Passed(Self::ev(&cand)),
                (K::Check | K::Run | K::Review(_), "red" | "rejected") => old::Outcome::Rejected(Self::ev(&cand)),
                (K::Fold, "conflict") => old::Outcome::FoldConflict { evidence: Self::ev(&cand), files: vec!["src/lib.rs".into()],
                    members: (!named.is_empty()).then_some(named) },
                (K::Push, "refused") => old::Outcome::Labeled("base_moved".into(), Self::ev(&cand)),
                (K::Route, "needs_review") => old::Outcome::Labeled("ask".into(), Self::ev(&cand)),
                (K::AwaitCi, l) => {
                    self.ci_attempt += 1;
                    let (label, jobs) = match l { "red:flaky" => ("rejected", vec!["flaky"]), "red" => ("rejected", vec!["unit"]), other => (other, vec![]) };
                    old::Outcome::CiResult { label: label.into(), evidence: Self::ev(&cand), failure: None,
                        run: Some(old::CiRun { id: 1, attempt: self.ci_attempt, failed_jobs: jobs.into_iter().map(String::from).collect() }) }
                }
                (_, l) => old::Outcome::Labeled(l.into(), Self::ev(&cand)),
            }
        };
        self.send(old::Command::Observe { item: id.into(), effect: effect.clone(), run: run.clone(), fence, outcome },
            old::Grant::Operation { effect, run, fence });
    }
    pub fn outcomes(&self) -> Outcomes {
        let tickets = self.tickets.iter().map(|t| {
            let o = match (self.state.items.get(t), self.state.archive.get(t)) {
                (Some(i), _) => match &i.phase {
                    old::Phase::Done => "done".into(),
                    old::Phase::Cancelled => "cancelled".into(),
                    old::Phase::Failed => format!("held:{}", i.definition.stages[i.stage].id),
                    old::Phase::Joined => {
                        let g = &self.state.items[i.grouped_in.as_ref().unwrap()];
                        let stage = &i.definition.stages[i.stage].id;
                        if g.phase == old::Phase::Failed { format!("held:{stage}") } else { format!("at:{stage}") }
                    }
                    _ => format!("at:{}", i.definition.stages[i.stage].id),
                },
                (None, Some(a)) if a.phase == old::Phase::Done => "done".into(),
                (None, Some(_)) => "cancelled".into(),
                (None, None) => "missing".into(),
            };
            (t.clone(), o)
        }).collect();
        Outcomes { tickets, landings: self.landings.clone(), builds: self.builds.clone() }
    }
}

// ---- the pipeline ----

pub struct New { pub r: support::Run, pub world: World, pub builds: BTreeMap<String, u32>, pub landings: Vec<Vec<String>>, pub tickets: Vec<String>,
    pub done: BTreeSet<usize>, pub n: u64, pub ci_attempt: u32 }
impl New {
    pub fn decider(&self, s: &new::Subject) -> new::Grant {
        let new::Location::Awaiting { decision, .. } = self.r.location(s) else { unreachable!() };
        match decision.deciders[0].strip_prefix("person:") {
            Some(name) => new::Grant::Person { name: name.into() },
            None => new::Grant::Orchestrator,
        }
    }
    pub fn intervene(&mut self) -> bool {
        for (k, (t, at, what)) in self.world.act.clone().into_iter().enumerate() {
            if self.done.contains(&k) || !self.r.p.tokens.contains_key(t) { continue; }
            let loc = self.r.at(t);
            let ready = loc.station() == Some(at) && match what {
                Act::Cancel => matches!(loc, new::Location::Working { .. }),
                Act::SendBack => matches!(loc, new::Location::Awaiting { .. }),
                Act::Retry => matches!(loc, new::Location::Held { .. }),
            };
            if !ready { continue; }
            self.done.insert(k);
            let s = support::tok(t);
            match what {
                Act::Cancel => self.r.orch(new::Command::Cancel { token: t.into(), reason: "cancelled".into() }),
                Act::SendBack => self.r.orch(new::Command::Send { token: t.into(), station: at.into(), to: "build".into(), reason: "send it back".into() }),
                Act::Retry => self.r.orch(new::Command::Release { subject: s, station: at.into(), to: None, reason: Some("retry".into()) }),
            }.unwrap_or_else(|e| panic!("{what:?} {t}: {e:?}"));
            return true;
        }
        false
    }
    pub fn act(&mut self) -> bool {
        if self.intervene() { return true; }
        for t in self.tickets.clone() {
            let s = support::tok(&t);
            if !self.r.p.tokens.contains_key(&t) { continue; }
            match self.r.at(&t) {
                new::Location::Working { work, .. } | new::Location::Stopping { work, .. } if work.pending.is_some() => { self.answer(&s); return true; }
                new::Location::Awaiting { station, .. } => {
                    let (label, _) = self.world.answer(&station, std::slice::from_ref(&t), &self.builds);
                    let verdict = if label == "passed" { "approved" } else { label };
                    let grant = self.decider(&s);
                    self.r.send(grant, new::Command::Decide { subject: s, station, verdict: verdict.into(), suspects: vec![],
                        reason: Some("not yet".into()) }).unwrap();
                    return true;
                }
                _ => {}
            }
        }
        // Oldest first: a group migrated from the old engine keeps its id, and is older than any batch formed since.
        let mut batches: Vec<(u64, String)> = self.r.p.batches.keys()
            .map(|k| (k.split_once('#').map_or(0, |(_, n)| n.parse().unwrap()), k.clone())).collect();
        batches.sort();
        for (_, b) in batches {
            let s = new::Subject::Batch(b);
            if matches!(self.r.location(&s), new::Location::Working { ref work, .. } | new::Location::Stopping { ref work, .. } if work.pending.is_some()) {
                self.answer(&s);
                return true;
            }
        }
        false
    }
    pub fn answer(&mut self, s: &new::Subject) {
        if matches!(self.r.location(s), new::Location::Stopping { .. }) { return self.r.stopped(s); }
        let work = self.r.work(s);
        // A migrated step's effect was the old engine's, so its station and members are read from where it is.
        let pending = work.pending.clone().expect("a pending effect");
        let (station, members) = match self.r.effects.iter().find(|e| e.id == pending) {
            Some(e) => (e.station.clone(), e.members.clone()),
            None => (self.r.location(s).station().unwrap().to_string(), match s {
                new::Subject::Batch(k) => { let b = &self.r.p.batches[k];
                    match &b.bisect { Some(x) => x.trying.clone(), None => b.members.iter().map(|m| m.token.clone()).collect() } }
                new::Subject::Token(_) => Vec::new(),
            }),
        };
        let members = if members.is_empty() { vec![s.key().to_string()] } else { members };
        let (label, named) = self.world.answer(&station, &members, &self.builds);
        let (op, _) = Op::parse(&work.op.op).unwrap();
        let data = match s { new::Subject::Token(k) => &self.r.p.tokens[k].data, new::Subject::Batch(k) => &self.r.p.batches[k].data };
        let cand = data_of(data).unwrap().candidate.unwrap_or_else(|| "none".into());
        self.n += 1;
        let outcome = |artifact: String, label: &str, facts: Option<Facts>| new::Outcome::Labeled { label: label.into(),
            evidence: new::Evidence { artifact, manifest: "m".into(), receipt: "r".into(), spec: "s".into() }, data: facts.map(|f| opaque(&f)) };
        let outcome = match (op, label) {
            (_, "fail") => new::Outcome::Failed { reason: None },
            (Op::Build, "passed") => {
                *self.builds.entry(s.key().to_string()).or_default() += 1;
                outcome(format!("{}-c-{}", s.key(), self.n), "passed", None)
            }
            (Op::Base | Op::Fold | Op::Merged, "passed") => outcome(format!("made-{}", self.n), "passed", None),
            (Op::Push, "passed") => { self.landings.push(members.clone()); outcome(cand, "passed", None) }
            (Op::Fold, "conflict") => outcome(cand, "conflict", Some(Facts { conflict: Some(Conflict { files: vec!["src/lib.rs".into()],
                members: (!named.is_empty()).then_some(named) }), ..Facts::default() })),
            (Op::Ci, l) => {
                self.ci_attempt += 1;
                let (label, jobs) = match l { "red:flaky" => ("red", vec!["flaky"]), "red" => ("red", vec!["unit"]), other => (other, vec![]) };
                let run = old::CiRun { id: 1, attempt: self.ci_attempt, failed_jobs: jobs.into_iter().map(String::from).collect() };
                outcome(cand, label, Some(Facts { run: Some(run), ..Facts::default() }))
            }
            (_, l) => outcome(cand, l, None),
        };
        self.r.observe(s, outcome).unwrap_or_else(|e| panic!("{s:?} at {station} answering {label}: {e:?}"));
    }
    pub fn outcomes(&self) -> Outcomes {
        let tickets = self.tickets.iter().map(|t| {
            let o = match (self.r.p.tokens.get(t), self.r.p.archive.get(t)) {
                (Some(tok), _) => match &tok.at {
                    new::Location::Held { station, .. } => format!("held:{}", top(station)),
                    new::Location::InBatch { batch } => {
                        let b = &self.r.p.batches[batch];
                        if matches!(b.at, new::Location::Held { .. }) { format!("held:{}", b.station) } else { format!("at:{}", b.station) }
                    }
                    at => format!("at:{}", top(at.station().unwrap_or("?"))),
                },
                (None, Some(a)) if a.closed == new::Closure::Done => "done".into(),
                (None, Some(_)) => "cancelled".into(),
                (None, None) => "missing".into(),
            };
            (t.clone(), o)
        }).collect();
        Outcomes { tickets, landings: self.landings.clone(), builds: self.builds.clone() }
    }
}

// ---- scenarios ----

pub struct Scenario { pub name: &'static str, pub plan: Plan, pub tickets: &'static [&'static str], pub world: World, pub rerun: bool,
    /// Differences the design intends; each is explained in the report.
    pub intended: &'static [&'static str] }
pub fn sc(name: &'static str, plan: Plan, tickets: &'static [&'static str], world: World) -> Scenario {
    Scenario { name, plan, tickets, world, rerun: false, intended: &[] }
}
pub fn plan(t: Template, gates: Vec<Gate>) -> Plan { Plan::new(t, gates) }
pub fn cmd() -> Vec<Gate> { vec![Gate::Command(0)] }
pub fn world() -> World { World::default() }
pub fn gate_bad(t: &str, n: u32) -> World { let mut w = world(); w.gate_bad.insert(t.into(), n); w }

/// The adapter config both engines' scenarios use.
pub fn config(s: &Scenario) -> serde_json::Value {
    json!({ "repository": ".", "target": "main", "ci": "ci.yml", "checks": ["c0", "c1"],
        "ci_rerun": { "jobs": if s.rerun { vec!["flaky"] } else { vec![] } } })
}
pub const POOLS: [(&str, u16); 3] = [("agents", 100), ("work", 100), ("landing", 1)];
/// The old reducer with the scenario's tickets enqueued on the old template, before anything answers.
pub fn old_start(s: &Scenario) -> Old {
    let mut definition = build(&s.plan, &Fixture).unwrap();
    if s.rerun {
        definition.policies.insert("flaky".into(), old::CiPolicy { point: old::CiPoint::Result, step: "ci".into(), fallback: None,
            rule: old::CiRule { rerun_failed: old::RerunFailed { jobs: vec!["flaky".into()], times: 1 } } });
        definition.validate().unwrap();
    }
    let tickets: Vec<String> = s.tickets.iter().map(|t| t.to_string()).collect();
    let mut o = Old { state: old::State::empty("workspace".into(), 1), log: vec![], world: s.world.clone(), builds: BTreeMap::new(),
        landings: vec![], tickets: tickets.clone(), done: BTreeSet::new(), n: 0, ci_attempt: 0 };
    for t in &tickets {
        o.send(old::Command::Enqueue { item: t.clone(), definition: definition.clone(), policy_pin: "policy".into(),
            resource: format!("res-{t}"), dependencies: BTreeSet::new(), initial_candidate: None }, old::Grant::Controller);
    }
    o
}
/// The pipeline driver on `r`, carrying on from where `from` (an old run cut short) had got to, or from nothing.
pub fn new_driver(s: &Scenario, r: support::Run, from: Option<&Old>) -> New {
    let tickets: Vec<String> = s.tickets.iter().map(|t| t.to_string()).collect();
    match from {
        Some(o) => New { r, world: o.world.clone(), builds: o.builds.clone(), landings: o.landings.clone(), tickets,
            done: o.done.clone(), n: o.n, ci_attempt: o.ci_attempt },
        None => New { r, world: s.world.clone(), builds: BTreeMap::new(), landings: vec![], tickets, done: BTreeSet::new(), n: 0, ci_attempt: 0 },
    }
}
pub fn settle_new(s: &Scenario, n: &mut New) {
    let mut turns = 0;
    while n.act() { turns += 1; assert!(turns < 1000, "{}: the pipeline run never settles", s.name); }
}

pub fn run(s: &Scenario) -> (Outcomes, Outcomes) {
    // The old reducer, on the old template.
    let mut o = old_start(s);
    let mut turns = 0;
    while o.act() { turns += 1; assert!(turns < 1000, "{}: the old run never settles", s.name); }
    // The recorded log replays to the same state.
    let mut replay = old::State::empty("workspace".into(), 1);
    for input in &o.log { replay = old::step(&replay, input).unwrap().state; }
    assert_eq!(old::digest(&replay), old::digest(&o.state), "{}: the old log replays", s.name);

    // The pipeline, on the same plan as a layout.
    let layout = coding_templates::layout(&s.plan, &config(s)).unwrap();
    let mut n = new_driver(s, support::Run::with(&POOLS, serde_json::to_value(&layout).unwrap(), linked()), None);
    for t in s.tickets {
        n.r.send(new::Grant::Runtime, new::Command::Intake { token: t.to_string(), to: Some("provision".into()), priority: 0,
            data: BTreeMap::new(), dependencies: BTreeSet::new(), models: BTreeMap::new(), brief: None }).unwrap();
    }
    settle_new(s, &mut n);
    n.r.assert_replays();
    (o.outcomes(), n.outcomes())
}

pub fn scenarios() -> Vec<Scenario> {
    use Template::*;
    let mut v = vec![
        // templates.rs: each template walked end to end.
        sc("batched push lands one ticket (templates.rs)", plan(BatchedPush, cmd()), &["a"], world()),
        sc("per-ticket push lands one ticket (templates.rs)", plan(TicketPush, cmd()), &["a"], world()),
        sc("per-ticket pull request merges and CI passes (templates.rs)", plan(TicketPr, cmd()), &["a"], world()),
        sc("batched pull requests rehearse, then merge (templates.rs)", plan(BatchedPr, cmd()), &["a"], world()),
        // gates.rs: review gates send back, within their limit.
        sc("an agent's review rejects once, then passes (gates.rs)", plan(BatchedPush, vec![Gate::Command(0), Gate::Agent]), &["a"],
            world().step("gate-0", "a", &["rejected"])),
        sc("a review rejects three times and the ticket stops at the gate (gates.rs)",
            plan(TicketPush, vec![Gate::Command(0), Gate::Reviewer { argument: "r".into() }]), &["a"],
            world().step("gate-0", "a", &["rejected", "rejected", "rejected"])),
        sc("a ticket's own command gate rejects once on a pull request (gates.rs)", plan(TicketPr, cmd()), &["a"],
            world().step("gate-0", "a", &["red"])),
        // groups.rs: tickets queued while a group lands accumulate behind it.
        sc("tickets queued while a group lands go together (groups.rs)", plan(BatchedPush, cmd()), &["a", "b", "c"], world()),
        { let mut p = plan(BatchedPush, cmd()); p.batch = Some(2);
          sc("a group holds up to its limit and the next forms behind (groups.rs)", p, &["a", "b", "c", "d"], world()) },
        sc("a failed group rejects its single member back to build (groups.rs)", plan(BatchedPush, cmd()), &["a"], gate_bad("a", 1)),
        sc("a rejected gate splits a pair and the good half lands (groups.rs)", plan(BatchedPush, cmd()), &["a", "b", "c"], gate_bad("c", 1)),
        Scenario { intended: &["builds: old {\"a\": 1, \"b\": 2, \"c\": 2}, new {\"a\": 1, \"b\": 1, \"c\": 2}"],
            ..sc("a diagnosed fold conflict returns only the bad member (groups.rs)", plan(BatchedPush, cmd()), &["a", "b", "c"],
                { let mut w = world(); w.conflict.insert("c".into(), 1); w }) },
        sc("a conflict that names no one returns the member to rebase (groups.rs)", plan(BatchedPush, cmd()), &["a"],
            { let mut w = world(); w.conflict.insert("a".into(), 1); w.blind = true; w }),
        // verbs.rs, land.rs: the push and the base.
        sc("a refused push refolds on the new base and lands (verbs.rs)", plan(BatchedPush, cmd()), &["a"], { let mut w = world(); w.refused = 1; w }),
        Scenario { intended: &["a: old done, new held:join", "landings: old [[\"a\"]], new []", "builds: old {\"a\": 2}, new {\"a\": 1}"],
            ..sc("a push refused past its limit (verbs.rs)", plan(BatchedPush, cmd()), &["a"], { let mut w = world(); w.refused = 3; w }) },
        Scenario { intended: &["a: old done, new held:join", "landings: old [[\"a\"]], new []", "builds: old {\"a\": 2}, new {\"a\": 1}"],
            ..sc("a base that never comes ends the group (groups.rs)", plan(BatchedPush, cmd()), &["a"],
                { let mut w = world(); w.base = VecDeque::from(["timeout"]); w }) },
        sc("landing safe failures retry twice without splitting (groups.rs)", plan(BatchedPush, cmd()), &["a", "b"],
            { let mut w = world(); w.fails.insert("fold", 2); w.fails.insert("gate-0", 2); w }),
        Scenario { intended: &["a: old done, new held:join", "landings: old [[\"a\"]], new []", "builds: old {\"a\": 2}, new {\"a\": 1}"],
            ..sc("a third safe failure stops the landing (groups.rs)", plan(BatchedPush, cmd()), &["a"],
                { let mut w = world(); w.fails.insert("fold", 3); w }) },
        // groups.rs: the CI rerun policy.
        Scenario { rerun: true, ..sc("a listed CI failure reruns once, then green lands (groups.rs)", plan(BatchedPush, cmd()), &["a"],
            { let mut w = world(); w.ci = VecDeque::from(["red:flaky"]); w }) },
        Scenario { rerun: true, ..sc("a second red after the rerun stops the landed group (groups.rs)", plan(BatchedPush, cmd()), &["a"],
            { let mut w = world(); w.ci = VecDeque::from(["red:flaky", "red:flaky"]); w }) },
        sc("an unlisted CI failure stops the landed group without a rerun (groups.rs)", plan(BatchedPush, cmd()), &["a"],
            { let mut w = world(); w.ci = VecDeque::from(["red"]); w }),
        // pr_verbs.rs: a pull request's own answers.
        sc("changes requested on a pull request go back to build (pr_verbs.rs)", plan(TicketPr, cmd()), &["a"],
            world().step("merged", "a", &["changes_requested"])),
        sc("a pull request closed unmerged stops the run (pr_verbs.rs)", plan(TicketPr, cmd()), &["a"],
            world().step("merged", "a", &["closed"])),
        sc("a rehearsal's gate rejects a ticket, which rebuilds and merges (groups.rs)", plan(BatchedPr, cmd()), &["a", "b"], gate_bad("b", 1)),
        // workflow.rs, send_back.rs: the orchestrator's hand.
        sc("cancelling a run while it builds (workflow.rs)", plan(BatchedPush, cmd()), &["a", "b"],
            { let mut w = world(); w.act.push(("a", "build", Act::Cancel)); w }),
        sc("the orchestrator sends a ticket back from a person's gate (send_back.rs)",
            plan(BatchedPush, vec![Gate::Command(0), Gate::Person { principal: "owner".into() }]), &["a"],
            { let mut w = world(); w.act.push(("a", "gate-0", Act::SendBack)); w }),
        sc("a build that fails twice stops, and a retry lands it (workflow.rs)", plan(BatchedPush, cmd()), &["a"],
            { let mut w = world().step("build", "a", &["fail", "fail"]); w.act.push(("a", "build", Act::Retry)); w }),
    ];
    // routes.rs: path routes send a ticket through a person's approval first.
    let mut p = plan(BatchedPush, cmd());
    p.paths = Some(PathRoutes { argument: "rules".into(), approver: Some("owner".into()) });
    v.push(sc("a path rule asks a person before the ticket lands (routes.rs)", p, &["a", "b"], world().step("route", "a", &["needs_review"])));
    v
}

