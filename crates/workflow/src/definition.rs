use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, BTreeMap};
use crate::{digest, Refusal};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Progression { Automatic, AwaitApproval { principal: String } }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reviewer { Dispatch, Orchestrator(String), Person(String) }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StageKind {
    Provision, Build, Check, Review(Reviewer), Land, Ci, Tracker,
    Action(String), Evaluate(String),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationSpec {
    pub adapter_version: String,
    pub arguments: Vec<String>,
    pub environment: BTreeMap<String, String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stage {
    pub id: String,
    pub kind: StageKind,
    pub progression: Progression,
    pub max_attempts: u8,
    pub operation: OperationSpec,
    /// Schema 2 only: where each result label goes. A label with no route falls forward on
    /// `passed` and to `Fail` on anything else.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub routes: BTreeMap<String, Target>,
    /// Schema 2 only: how many times this step's back edges may be taken in one run.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub limit: u8,
}
fn is_zero(n: &u8) -> bool { *n == 0 }
/// Where a labeled result goes next.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target { Step(String), Done, Fail }
/// A finite forward sequence; rejected evaluators may take one bounded, pinned rework edge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Definition {
    pub schema: u32,
    pub source: String,
    pub stages: Vec<Stage>,
    pub max_reworks: u8,
    pub rework_target: Option<String>,
}
impl Definition {
    pub fn validate(&self) -> Result<(), Refusal> {
        let mut ids = BTreeSet::new();
        if !matches!(self.schema, 1 | 2) || self.source.is_empty() || self.source.len() > 256
            || self.stages.is_empty() || self.stages.len() > 16 || self.max_reworks > 2
        { return Err(Refusal::Definition); }
        let mut candidate_available = matches!(self.stages[0].kind, StageKind::Tracker);
        let routed = self.schema == 2;
        for stage in &self.stages {
            if !valid_id(&stage.id) || !ids.insert(&stage.id)
                || stage.max_attempts == 0 || stage.max_attempts > 5
            { return Err(Refusal::Definition); }
            if !valid_id(&stage.operation.adapter_version) || stage.operation.arguments.len() > 32
                || stage.operation.arguments.iter().any(|a| a.len() > 4096 || a.contains('\0'))
                || stage.operation.environment.len() > 32
                || stage.operation.environment.iter().any(|(k, v)| !valid_id(k) || v.len() > 4096 || v.contains('\0'))
            { return Err(Refusal::Definition); }
            match stage.kind {
                // Routed definitions check candidate availability along their edges instead.
                _ if routed => {}
                StageKind::Build | StageKind::Action(_) => candidate_available = true,
                StageKind::Check | StageKind::Review(_) | StageKind::Land | StageKind::Ci
                    | StageKind::Tracker | StageKind::Evaluate(_) if !candidate_available => {
                    return Err(Refusal::Definition);
                }
                _ => {}
            }
            if let Progression::AwaitApproval { principal } = &stage.progression {
                if !valid_id(principal) { return Err(Refusal::Definition); }
            }
            if let StageKind::Action(binding) | StageKind::Evaluate(binding) = &stage.kind {
                if !valid_id(binding) { return Err(Refusal::Definition); }
            }
            if let StageKind::Review(Reviewer::Person(p) | Reviewer::Orchestrator(p)) = &stage.kind {
                if !valid_id(p) { return Err(Refusal::Definition); }
            }
        }
        if self.schema == 2 { return self.validate_routes(); }
        if self.stages.iter().any(|s| !s.routes.is_empty() || s.limit != 0) { return Err(Refusal::Definition); }
        // Sequence order is definition policy, not a global coding/reviewer policy.
        let target = self.rework_target.as_ref().map(|id| self.stages.iter().position(|s| &s.id == id));
        if self.max_reworks == 0 {
            if target.is_some() { return Err(Refusal::Definition); }
        } else {
            let Some(Some(target)) = target else { return Err(Refusal::Definition); };
            if !matches!(self.stages[target].kind, StageKind::Build | StageKind::Action(_)) {
                return Err(Refusal::Definition);
            }
            let evaluators: Vec<_> = self.stages.iter().enumerate().filter(|(_, s)|
                matches!(s.kind, StageKind::Review(_) | StageKind::Evaluate(_))).collect();
            if evaluators.is_empty() || evaluators.iter().any(|(index, _)| target >= *index) {
                return Err(Refusal::Definition);
            }
        }
        Ok(())
    }
    /// Schema 2: labels route, back edges are bounded, and every step is reachable and safe.
    fn validate_routes(&self) -> Result<(), Refusal> {
        if self.max_reworks != 0 || self.rework_target.is_some() { return Err(Refusal::Definition); }
        let n = self.stages.len();
        let index: BTreeMap<&str, usize> = self.stages.iter().enumerate().map(|(i, s)| (s.id.as_str(), i)).collect();
        // Every edge a step can take: (from, label, to), with `None` as the end of the run.
        let mut edges: Vec<(usize, &str, Option<usize>)> = Vec::new();
        let mut done_reachable_from: Vec<usize> = Vec::new();
        for (i, stage) in self.stages.iter().enumerate() {
            if stage.limit > MAX_BACK_EDGE_LIMIT || stage.routes.len() > 8 { return Err(Refusal::Definition); }
            let mut back = false;
            let mut labels: Vec<&str> = vec!["passed", "rejected"];
            for (label, target) in &stage.routes {
                if !valid_id(label) || !can_emit(&stage.kind, label) { return Err(Refusal::Definition); }
                if !labels.contains(&label.as_str()) { labels.push(label); }
                if let Target::Step(id) = target {
                    let &to = index.get(id.as_str()).ok_or(Refusal::Definition)?;
                    back |= to <= i;
                }
            }
            if back != (stage.limit > 0) { return Err(Refusal::Definition); }
            for label in labels {
                let to = match stage.routes.get(label) {
                    Some(Target::Step(id)) => Some(index[id.as_str()]),
                    Some(Target::Done) => { done_reachable_from.push(i); None }
                    Some(Target::Fail) => None,
                    None if label == "passed" && i + 1 < n => Some(i + 1),
                    None if label == "passed" => { done_reachable_from.push(i); None }
                    None => None,
                };
                edges.push((i, label, to));
            }
        }
        // Reachability from the first step, and a way to Done.
        let mut seen = vec![false; n];
        let mut todo = vec![0usize];
        while let Some(i) = todo.pop() {
            if std::mem::replace(&mut seen[i], true) { continue; }
            todo.extend(edges.iter().filter(|e| e.0 == i).filter_map(|e| e.2));
        }
        if seen.iter().any(|s| !s) || !done_reachable_from.iter().any(|&i| seen[i]) { return Err(Refusal::Definition); }
        // A step that needs a candidate must have one on every path into it.
        let mut has = vec![true; n];
        has[0] = matches!(self.stages[0].kind, StageKind::Tracker);
        loop {
            let mut changed = false;
            for &(from, label, to) in &edges {
                let Some(to) = to else { continue };
                if makes_candidate_anew(&self.stages[to].kind) { continue; }
                let out = has[from] || (label != "rejected" && makes_candidate(&self.stages[from].kind));
                if has[to] && !out { has[to] = false; changed = true; }
            }
            if !changed { break; }
        }
        for (i, stage) in self.stages.iter().enumerate() {
            if needs_candidate(&stage.kind) && !has[i] { return Err(Refusal::Definition); }
        }
        Ok(())
    }
    pub fn pin(&self) -> String { digest(self) }
    pub fn coding() -> Self {
        let kinds = [StageKind::Provision, StageKind::Build, StageKind::Check,
            StageKind::Review(Reviewer::Dispatch), StageKind::Land, StageKind::Ci];
        Self { schema: 1, source: "coding/v1".into(), max_reworks: 2, rework_target: Some("stage-1".into()),
            stages: kinds.into_iter().enumerate().map(|(i, kind)| Stage {
                id: format!("stage-{i}"), kind, progression: Progression::Automatic,
                max_attempts: 2, routes: BTreeMap::new(), limit: 0,
                operation: OperationSpec { adapter_version: "fixture/v1".into(),
                    arguments: if i == 2 { vec!["cargo".into(), "test".into(), "--locked".into()] }
                        else { vec![format!("coding-operation-{i}")] }, environment: BTreeMap::new() },
            }).collect() }
    }
    pub fn tracker(principal: &str) -> Self {
        Self { schema: 1, source: "tracker/v1".into(), max_reworks: 0, rework_target: None,
            stages: vec![Stage { id: "checklist".into(), kind: StageKind::Tracker,
                progression: Progression::AwaitApproval { principal: principal.into() },
                max_attempts: 1, routes: BTreeMap::new(), limit: 0,
                operation: OperationSpec { adapter_version: "tracker/v1".into(),
                    arguments: vec![], environment: BTreeMap::new() } }] }
    }
}
/// Back edges per step are capped so a loop cannot be a polite spelling of forever.
pub const MAX_BACK_EDGE_LIMIT: u8 = 3;
/// Labels a step can produce. `passed` and `rejected` come from anything; a label of a
/// definition's own is only for steps whose binding or command the definition chose.
pub fn can_emit(kind: &StageKind, label: &str) -> bool {
    matches!(label, "passed" | "rejected")
        || matches!(kind, StageKind::Check | StageKind::Action(_) | StageKind::Evaluate(_))
}
fn needs_candidate(kind: &StageKind) -> bool {
    matches!(kind, StageKind::Check | StageKind::Review(_) | StageKind::Land | StageKind::Ci
        | StageKind::Tracker | StageKind::Evaluate(_))
}
pub fn makes_candidate(kind: &StageKind) -> bool {
    matches!(kind, StageKind::Build | StageKind::Action(_) | StageKind::Tracker)
}
/// Entering these discards the old candidate, because they are about to make a new one.
pub fn makes_candidate_anew(kind: &StageKind) -> bool { matches!(kind, StageKind::Build | StageKind::Action(_)) }
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_:./".contains(&b))
}
