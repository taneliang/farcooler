//! The reducer: `step(&Pipeline, &Input, &Adapters) -> Result<Transition, Refusal>`. Pure: no clock, no I/O;
//! the input carries the time, the grant and the host's occupancy, so replay reaches the same state.
use std::collections::{BTreeMap, BTreeSet};
use serde::{Deserialize, Serialize};
use super::adapter::Adapters;
use super::flow::Ctx;
use super::layout::{Graph, Layout, Move};
use super::types::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal {
    Stale, Duplicate, Invalid, Missing, Evidence, Definition, Unauthorized,
    /// Rule 1 of 5.3: the work crossed a `repeat: once` boundary that this would lead back to.
    Crossed,
    /// The work's running step has no stop receipt yet (5.2).
    Stopping,
    /// Only a person may do this here (a station's `skip: person`).
    PersonOnly,
    /// A publish would leave work at no station (7.3).
    Stranded,
}
/// Who is asking. The runtime authenticates it; an untrusted payload never constructs one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Grant { Runtime, Orchestrator, Person { name: String }, Operation { effect: String, fence: u64 } }
impl Grant {
    pub fn principal(&self) -> String {
        match self { Grant::Runtime => "runtime".into(), Grant::Orchestrator => "orchestrator".into(),
            Grant::Person { name } => format!("person:{name}"), Grant::Operation { effect, .. } => format!("effect:{effect}") }
    }
}
/// The host-wide limit for one pool name and what other workspaces hold of it, stamped by the store (6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostPool { pub limit: u16, pub busy: u16 }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Input {
    pub schema: u32,
    pub id: String,
    pub workspace: String,
    pub expected_revision: u64,
    pub epoch: u64,
    pub source: String,
    pub source_sequence: u64,
    pub now_ms: i64,
    pub grant: Grant,
    pub command: Command,
    #[serde(default)]
    pub busy_resources: BTreeSet<String>,
    #[serde(default)]
    pub host: BTreeMap<String, HostPool>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope { Station(StationId), Pool(String) }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    Publish { layout: Layout, moves: BTreeMap<StationId, Move>, after: BTreeMap<String, After>, reason: String },
    /// An adapter source (or the orchestrator) creates a token at a station, the first intake's by default.
    Intake { token: String, to: Option<StationId>, priority: i32, data: BTreeMap<String, Opaque>,
        dependencies: BTreeSet<String>, models: BTreeMap<StationId, String>, brief: Option<String> },
    Track { token: String, column: String },
    Start { token: String, station: StationId },
    Advance,
    Observe { subject: Subject, effect: String, fence: u64, outcome: Outcome },
    Decide { subject: Subject, station: StationId, verdict: String, suspects: Vec<String>, reason: Option<String> },
    Reorder { station: StationId, token: String, position: usize },
    Prioritize { token: String, priority: i32 },
    Hold { subject: Subject, station: StationId, reason: String },
    Release { subject: Subject, station: StationId, to: Option<StationId>, reason: Option<String> },
    Pull { token: String, reason: String },
    Send { token: String, station: StationId, to: StationId, reason: String },
    Skip { subject: Subject, station: StationId, reason: String },
    Stop { subject: Subject, station: StationId, reason: String },
    Cancel { token: String, reason: String },
    Reopen { token: String, station: StationId },
    SetCapacity { scope: Scope, n: u16, reason: String },
    HoldStation { station: StationId, reason: String },
    ResumeStation { station: StationId },
    Drain { station: StationId },
    Settle { subject: Subject, station: StationId, crossed: bool, evidence: Evidence },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition { pub state: Pipeline, pub effects: Vec<Effect> }

pub(super) fn valid_key(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_:.".contains(&b))
}

pub fn step(state: &Pipeline, input: &Input, adapters: &Adapters) -> Result<Transition, Refusal> {
    if input.schema != 3 || state.schema != 3 || !valid_key(&input.id) || !valid_key(&input.source)
        || input.source_sequence == 0 || input.workspace.is_empty() { return Err(Refusal::Invalid); }
    if state.seen.contains(&input.id) { return Err(Refusal::Duplicate); }
    if input.workspace != state.workspace || input.epoch != state.epoch || input.expected_revision != state.revision
        || input.source_sequence <= *state.source_sequences.get(&input.source).unwrap_or(&0) { return Err(Refusal::Stale); }
    let mut p = state.clone();
    p.revision += 1;
    p.seen.insert(input.id.clone());
    p.source_sequences.insert(input.source.clone(), input.source_sequence);
    let graph = match &p.workflow {
        Some(w) => Graph::new(&w.layout, adapters),
        None if matches!(input.command, Command::Publish { .. }) => Graph { nodes: BTreeMap::new(), order: Vec::new() },
        None => return Err(Refusal::Definition),
    };
    let mut ctx = Ctx { p, graph, adapters, input, effects: Vec::new() };
    ctx.command(&input.command)?;
    ctx.schedule();
    Ok(Transition { state: ctx.p, effects: ctx.effects })
}

impl<'a, 'r> Ctx<'a, 'r> {
    fn by(&self) -> String { self.input.grant.principal() }
    fn controller(&self) -> Result<(), Refusal> {
        match self.input.grant { Grant::Orchestrator | Grant::Person { .. } => Ok(()), _ => Err(Refusal::Unauthorized) }
    }
    fn runtime(&self) -> Result<(), Refusal> {
        match self.input.grant { Grant::Runtime | Grant::Orchestrator | Grant::Person { .. } => Ok(()), _ => Err(Refusal::Unauthorized) }
    }
    pub(super) fn top_station(&self, id: &str) -> Result<(), Refusal> {
        match self.graph.node(id) { Some(n) if n.parent.is_none() && self.p.stations.contains_key(id) => Ok(()), _ => Err(Refusal::Missing) }
    }
    fn command(&mut self, command: &Command) -> Result<(), Refusal> {
        match command {
            Command::Publish { layout, moves, after, reason } => { self.controller()?; self.publish(layout, moves, after, reason) }
            Command::Intake { token, to, priority, data, dependencies, models, brief } => {
                self.runtime()?;
                if !valid_key(token) || token.contains('#') || self.p.tokens.contains_key(token) { return Err(Refusal::Invalid); }
                if self.p.archive.contains_key(token) { return Err(Refusal::Duplicate); }
                if dependencies.iter().any(|d| !self.p.tokens.contains_key(d) && !self.p.archive.contains_key(d)) { return Err(Refusal::Missing); }
                let to = match to { Some(t) => t.clone(),
                    None => self.p.workflow.as_ref().unwrap().layout.intake.first().map(|i| i.to.clone()).ok_or(Refusal::Invalid)? };
                self.top_station(&to)?;
                self.p.tokens.insert(token.clone(), new_token(*priority, data.clone(), dependencies.clone(), models.clone(), brief.clone()));
                let by = self.by();
                self.arrive(&Subject::Token(token.clone()), &to, &by, Some("intake".into()));
                Ok(())
            }
            Command::Track { token, column } => {
                self.runtime()?;
                if !valid_key(token) || token.contains('#') || self.p.tokens.contains_key(token) || self.p.archive.contains_key(token) { return Err(Refusal::Invalid); }
                let mut t = new_token(0, BTreeMap::new(), BTreeSet::new(), BTreeMap::new(), None);
                t.at = Location::Resting { column: column.clone() };
                self.p.tokens.insert(token.clone(), t);
                Ok(())
            }
            Command::Start { token, station } => {
                self.controller()?;
                if !matches!(self.p.tokens.get(token).map(|t| &t.at), Some(Location::Resting { .. })) { return Err(Refusal::Stale); }
                self.top_station(station)?;
                let by = self.by();
                self.arrive(&Subject::Token(token.clone()), station, &by, None);
                Ok(())
            }
            Command::Advance => self.runtime(),
            Command::Observe { subject, effect, fence, outcome } => self.observe(subject, effect, *fence, outcome),
            Command::Decide { subject, station, verdict, suspects, reason } => self.decide(subject, station, verdict, suspects, reason),
            Command::Settle { subject, station, crossed, evidence } => self.settle(subject, station, *crossed, evidence),
            other => self.intervene(other),
        }
    }
    fn observe(&mut self, s: &Subject, effect: &str, fence: u64, outcome: &Outcome) -> Result<(), Refusal> {
        match &self.input.grant {
            Grant::Operation { effect: e, fence: f } if e == effect && *f == fence => {}
            _ => return Err(Refusal::Unauthorized),
        }
        if !self.exists(s) { return Err(Refusal::Stale); }
        let at = self.at(s).clone();
        let (station, work, then, settling) = match at {
            Location::Working { station, work } => (station, work, None, false),
            Location::Stopping { station, work, then } => (station, work, Some(then), false),
            Location::Settling { station, work, then } => (station, work, Some(then), true),
            _ => return Err(Refusal::Stale),
        };
        let current = work.pending.as_deref() == Some(effect) || (work.uncertain && work.run == effect);
        if !current || (fence != self.fence(s) && !work.uncertain) { return Err(Refusal::Stale); }
        let Some(then) = then else { return self.answer(s, &station, work, outcome) };
        match (outcome, settling) {
            (Outcome::Stopped { operation, receipt }, false) => {
                if Some(operation) != work.stopping.as_ref() || receipt.is_empty() { return Err(Refusal::Evidence); }
                self.apply_then(s, &station, *then, None);
                Ok(())
            }
            (Outcome::Settled { operation, original, receipt }, true) => {
                if Some(operation) != work.stopping.as_ref() || receipt.is_empty() { return Err(Refusal::Evidence); }
                let mut original_work = work.clone();
                original_work.pending = work.stopping.clone();
                let crossed = match &**original {
                    Outcome::Labeled { label, .. } => self.graph.node(&station).and_then(|n| n.spec.as_ref())
                        .and_then(|sp| sp.outward.as_ref()).is_some_and(|o| o.crossing.contains(label)),
                    _ => false,
                };
                if crossed {
                    // It went out: the crossing is recorded first and the request is overruled.
                    if let Some(v) = self.visits_mut(s).iter_mut().rev().find(|v| v.left.is_none()) {
                        v.reason = Some("a request to move it was overruled: it had crossed".into());
                    }
                    self.answer(s, &station, original_work, original)
                } else if matches!(**original, Outcome::Uncertain) {
                    original_work.uncertain = true;
                    original_work.pending = None;
                    self.set_at(s, Location::Working { station, work: original_work });
                    Ok(())
                } else {
                    self.apply_then(s, &station, *then, None);
                    Ok(())
                }
            }
            _ => Err(Refusal::Evidence),
        }
    }
    fn decide(&mut self, s: &Subject, station: &str, verdict: &str, suspects: &[String], reason: &Option<String>) -> Result<(), Refusal> {
        if !self.exists(s) { return Err(Refusal::Stale); }
        let Location::Awaiting { station: at, mut decision } = self.at(s).clone() else { return Err(Refusal::Stale) };
        if at != station || !valid_key(verdict) { return Err(Refusal::Stale); }
        let by = self.by();
        if !decision.deciders.contains(&by) { return Err(Refusal::Unauthorized); }
        decision.given.push(Verdict { by: by.clone(), label: verdict.into() });
        let approvals = decision.given.iter().filter(|v| v.label == "approved").count();
        let needs = decision.needs as usize;
        self.set_at(s, Location::Awaiting { station: station.into(), decision });
        if verdict == "approved" && approvals < needs { return Ok(()); }
        if let Subject::Batch(b) = s { self.p.batches.get_mut(b).unwrap().suspects = suspects.to_vec(); }
        if let Subject::Token(k) = s { if reason.is_some() { self.p.tokens.get_mut(k).unwrap().brief = reason.clone(); } }
        self.leave(s, station, verdict, false);
        Ok(())
    }
    fn settle(&mut self, s: &Subject, station: &str, crossed: bool, evidence: &Evidence) -> Result<(), Refusal> {
        self.controller()?;
        if !self.exists(s) { return Err(Refusal::Stale); }
        let Location::Working { station: at, work } = self.at(s).clone() else { return Err(Refusal::Stale) };
        if at != station || !work.uncertain { return Err(Refusal::Stale); }
        if evidence.receipt.is_empty() || evidence.artifact.is_empty() { return Err(Refusal::Evidence); }
        let outward = self.graph.node(station).and_then(|n| n.spec.clone()).and_then(|sp| sp.outward);
        match (crossed, outward) {
            (true, Some(o)) => {
                self.cross(s, station, &o.boundary, work.round.unwrap_or(1), &work.run, evidence);
                let label = o.crossing.first().cloned().unwrap_or_else(|| "passed".into());
                self.leave(s, station, &label, false);
            }
            (true, None) => return Err(Refusal::Invalid),
            (false, _) => {
                let by = self.by();
                self.close_visit(s, "settled", Some("settled: it did not go out".into()), None);
                self.free_place(s.key(), station);
                self.arrive(s, station, &by, Some("settled: run it again".into()));
            }
        }
        Ok(())
    }
}
pub(super) fn new_token(priority: i32, data: BTreeMap<String, Opaque>, dependencies: BTreeSet<String>,
    models: BTreeMap<StationId, String>, brief: Option<String>) -> Token {
    Token { at: Location::Resting { column: String::new() }, priority, data, evidence: None, crossings: Vec::new(),
        fresh_from: 0, brief, failure: None, models, dependencies, visits: Vec::new(), fence: 0 }
}
