//! Fixture adapters that exist only in tests (test/mail, test/calendar, test/files, test/cms) and a driver
//! that feeds the pipeline reducer inputs and keeps the log, so a test can replay it.
#![allow(dead_code)]
use std::collections::{BTreeMap, BTreeSet};
use farcooler_workflow::pipeline::*;
use serde_json::{json, Value};

/// One fixture adapter: a static catalog; outward operations listed in `fresh` refuse a start whose evidence
/// is the artifact of the last crossing of their boundary (a stale draft sent again).
pub struct Fixture { pub version: &'static str, pub ops: Vec<OpSpec> }
fn op(name: &str, kind: Kind, labels: &[&str]) -> OpSpec {
    OpSpec { name: name.into(), kinds: vec![kind], labels: labels.iter().map(|l| l.to_string()).collect(), outward: None, resources: Vec::new() }
}
fn outward(mut spec: OpSpec, boundary: &str, repeat: Repeat) -> OpSpec {
    spec.outward = Some(Outward { boundary: boundary.into(), repeat, dedupe: Dedupe::None, crossing: vec!["passed".into()] });
    spec
}
impl AdapterRules for Fixture {
    fn version(&self) -> &'static str { self.version }
    fn catalog(&self, _: &Value) -> Vec<OpSpec> { self.ops.clone() }
    fn validate(&self, _: &Layout, _: &Value) -> Result<(), Vec<Problem>> { Ok(()) }
    fn start(&self, op: &str, subject: &View, _: &Value) -> Result<StartPlan, Refusal> {
        let spec = self.ops.iter().find(|o| o.name == op).ok_or(Refusal::Definition)?;
        if let Some(o) = spec.outward.as_ref().filter(|o| o.repeat == Repeat::Fresh) {
            let last = subject.crossings.iter().rev().find(|c| c.boundary == o.boundary);
            if let (Some(last), Some(e)) = (last, subject.evidence) {
                if last.artifact == e.artifact { return Err(Refusal::Evidence); }
            }
        }
        Ok(StartPlan { resources: Vec::new(), data: None, spec: format!("{}:{op}", self.version) })
    }
    fn observe(&self, op: &str, _: &View, outcome: &Outcome) -> Result<Applied, Refusal> {
        let Outcome::Labeled { label, data, .. } = outcome else { return Err(Refusal::Invalid) };
        let spec = self.ops.iter().find(|o| o.name == op).ok_or(Refusal::Definition)?;
        if !spec.labels.contains(label) { return Err(Refusal::Invalid); }
        Ok(Applied { label: label.clone(), data: data.clone(), again: None })
    }
    fn suspects(&self, _: &View, outcome: &Outcome) -> Vec<String> {
        match outcome {
            Outcome::Labeled { data: Some(d), .. } => d.value["suspects"].as_array().into_iter().flatten()
                .filter_map(|v| v.as_str().map(str::to_string)).collect(),
            _ => Vec::new(),
        }
    }
    fn project(&self, data: &Opaque) -> Vec<(String, String)> { vec![("fixture".into(), data.value.to_string())] }
}
pub fn mail() -> Fixture {
    Fixture { version: "test/mail", ops: vec![
        op("triage", Kind::Agent, &["reply", "schedule", "fyi", "ignore"]),
        op("draft", Kind::Agent, &["passed"]),
        op("draft_follow_up", Kind::Agent, &["passed", "not_needed"]),
        outward(op("send", Kind::Command, &["passed", "refused"]), "mail:send", Repeat::Fresh),
        op("reply", Kind::Wait, &["replied", "timeout"]),
        op("archive", Kind::Command, &["passed"]),
        op("summarize", Kind::Agent, &["passed"]),
        outward(op("send_digest", Kind::Command, &["passed", "refused"]), "mail:digest", Repeat::Once),
    ] }
}
pub fn calendar() -> Fixture { Fixture { version: "test/calendar", ops: vec![op("free_slots", Kind::Command, &["passed"])] } }
pub fn files() -> Fixture {
    Fixture { version: "test/files", ops: vec![
        op("research", Kind::Agent, &["passed"]), op("outline", Kind::Agent, &["passed"]),
        op("write", Kind::Agent, &["passed"]), op("fact_check", Kind::Agent, &["passed", "rejected"]),
        op("archive", Kind::Command, &["passed"]),
    ] }
}
pub fn cms() -> Fixture {
    Fixture { version: "test/cms", ops: vec![
        op("assemble", Kind::Command, &["passed"]), op("edit_issue", Kind::Agent, &["passed"]),
        outward(op("publish", Kind::Command, &["passed", "refused"]), "cms:issue", Repeat::Once),
        outward(op("publish_mirror", Kind::Command, &["passed", "refused"]), "cms:issue", Repeat::Once),
        op("stats", Kind::Wait, &["measured", "timeout"]),
    ] }
}
pub fn adapters() -> Adapters<'static> {
    let all: Vec<&'static dyn AdapterRules> = vec![Box::leak(Box::new(mail())), Box::leak(Box::new(calendar())),
        Box::leak(Box::new(files())), Box::leak(Box::new(cms()))];
    Adapters::new(&all)
}
pub fn layout(value: Value) -> Layout { serde_json::from_value(value).expect("fixture layouts parse") }

pub const HOUR: i64 = 3_600_000;
pub const DAY: i64 = 24 * HOUR;

/// Feeds inputs to the reducer one at a time, keeps every accepted input, and answers effects.
pub struct Run {
    pub p: Pipeline,
    pub start: Pipeline,
    pub adapters: Adapters<'static>,
    pub log: Vec<Input>,
    pub effects: Vec<Effect>,
    pub now: i64,
    pub host: BTreeMap<String, HostPool>,
    n: u64,
    answers: u64,
}
pub fn tok(k: &str) -> Subject { Subject::Token(k.into()) }
impl Run {
    pub fn new(pools: &[(&str, u16)], workflow: Value) -> Self {
        let pools: BTreeMap<String, u16> = pools.iter().map(|(k, n)| (k.to_string(), *n)).collect();
        let p = Pipeline::new("workspace", 1, &pools);
        let mut run = Run { start: p.clone(), p, adapters: adapters(), log: Vec::new(), effects: Vec::new(),
            now: 1_000, host: BTreeMap::new(), n: 0, answers: 0 };
        run.orch(Command::Publish { layout: layout(workflow), moves: BTreeMap::new(), after: BTreeMap::new(),
            reason: "first layout".into() }).expect("the fixture layout publishes");
        run
    }
    pub fn input(&mut self, grant: Grant, command: Command) -> Input {
        self.n += 1;
        Input { schema: 3, id: format!("input-{}", self.n), workspace: "workspace".into(), expected_revision: self.p.revision,
            epoch: 1, source: "fixture".into(), source_sequence: self.n, now_ms: self.now, grant, command,
            busy_resources: BTreeSet::new(), host: self.host.clone() }
    }
    pub fn send(&mut self, grant: Grant, command: Command) -> Result<Vec<Effect>, Refusal> {
        let input = self.input(grant, command);
        let t = step(&self.p, &input, &self.adapters)?;
        self.p = t.state;
        self.log.push(input);
        self.effects.extend(t.effects.iter().cloned());
        Ok(t.effects)
    }
    pub fn orch(&mut self, command: Command) -> Result<Vec<Effect>, Refusal> { self.send(Grant::Orchestrator, command) }
    pub fn person(&mut self, name: &str, command: Command) -> Result<Vec<Effect>, Refusal> {
        self.send(Grant::Person { name: name.into() }, command)
    }
    pub fn advance(&mut self) { self.send(Grant::Runtime, Command::Advance).unwrap(); }
    pub fn intake(&mut self, key: &str, to: Option<&str>, priority: i32) {
        self.send(Grant::Runtime, Command::Intake { token: key.into(), to: to.map(str::to_string), priority,
            data: BTreeMap::new(), dependencies: BTreeSet::new(), models: BTreeMap::new(), brief: None }).unwrap();
    }
    pub fn location(&self, s: &Subject) -> Location {
        match s { Subject::Token(k) => self.p.tokens.get(k).map(|t| t.at.clone()).unwrap_or(Location::Closed { crossed: false }),
            Subject::Batch(k) => self.p.batches[k].at.clone() }
    }
    pub fn at(&self, key: &str) -> Location { self.location(&tok(key)) }
    pub fn station(&self, key: &str) -> Option<String> { self.at(key).station().map(str::to_string) }
    pub fn work(&self, s: &Subject) -> Work {
        match self.location(s) {
            Location::Working { work, .. } | Location::Stopping { work, .. } | Location::Settling { work, .. } => work,
            other => panic!("{s:?} is not working: {other:?}"),
        }
    }
    pub fn fence(&self, s: &Subject) -> u64 {
        match s { Subject::Token(k) => self.p.tokens[k].fence, Subject::Batch(k) => self.p.batches[k].fence }
    }
    pub fn effect_for(&self, s: &Subject) -> Effect {
        let id = self.work(s).pending.expect("a pending effect");
        self.effects.iter().find(|e| e.id == id).cloned().expect("the effect was emitted")
    }
    pub fn observe(&mut self, s: &Subject, outcome: Outcome) -> Result<Vec<Effect>, Refusal> {
        let effect = self.work(s).pending.expect("a pending effect");
        let fence = self.fence(s);
        self.send(Grant::Operation { effect: effect.clone(), fence },
            Command::Observe { subject: s.clone(), effect, fence, outcome })
    }
    pub fn evidence(&mut self, s: &Subject) -> Evidence {
        self.answers += 1;
        Evidence { artifact: format!("{}-artifact-{}", s.key(), self.answers), manifest: "manifest".into(),
            receipt: format!("receipt-{}", self.answers), spec: "spec".into() }
    }
    pub fn answer(&mut self, s: &Subject, label: &str) {
        let evidence = self.evidence(s);
        self.observe(s, Outcome::Labeled { label: label.into(), evidence, data: None })
            .unwrap_or_else(|r| panic!("{s:?} answering {label}: {r:?}"));
    }
    pub fn answer_suspects(&mut self, s: &Subject, label: &str, suspects: &[&str]) {
        let evidence = self.evidence(s);
        let data = Opaque { version: "test/cms".into(), value: json!({ "suspects": suspects }), digest: "d".into() };
        self.observe(s, Outcome::Labeled { label: label.into(), evidence, data: Some(data) }).unwrap();
    }
    pub fn stopped(&mut self, s: &Subject) {
        let operation = self.work(s).stopping.expect("stopping");
        self.observe(s, Outcome::Stopped { operation, receipt: "quiet".into() }).unwrap();
    }
    pub fn batch_of(&self, key: &str) -> String {
        match self.at(key) { Location::InBatch { batch } => batch, other => panic!("{key} is not in a batch: {other:?}") }
    }
    pub fn queue(&self, station: &str) -> Vec<String> { self.p.stations[station].queue.clone() }
    pub fn closed(&self, key: &str) -> &Archived { self.p.archive.get(key).unwrap_or_else(|| panic!("{key} is not closed")) }
    /// Replay the log from the starting state: the same inputs must reach the same state and effects.
    pub fn replay(&self) -> (Pipeline, Vec<Effect>) {
        let mut p = self.start.clone();
        let mut effects = Vec::new();
        for input in &self.log {
            let t = step(&p, input, &self.adapters).expect("a logged input replays");
            effects.extend(t.effects);
            p = t.state;
        }
        (p, effects)
    }
    pub fn assert_replays(&self) {
        let (p, effects) = self.replay();
        assert_eq!(p, self.p, "replay reaches the same state");
        assert_eq!(effects, self.effects, "replay emits the same effects");
        assert_eq!(farcooler_workflow::digest(&p), farcooler_workflow::digest(&self.p));
    }
}
