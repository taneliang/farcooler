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
    /// Wait for the orchestrator to queue this ticket into the named group, then take the group's
    /// result as a label: `passed`, `rejected`, or `dropped`. `adopt` takes the group's result as
    /// the ticket's candidate.
    Join { group: String, adopt: bool },
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
pub enum Target {
    Step(String), Done, Fail,
    /// Group steps only. With two or more members, this group keeps the first half and goes back
    /// to `step`, and the rest form a new group. With one member (or no room) take `otherwise`.
    Split { step: String, otherwise: Box<Target> },
}
/// A batch of tickets run together by an item of its own. Its steps are a definition of their own.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupDef {
    /// Most members one group holds; a later ticket opens a group behind it.
    pub limit: u8,
    /// The resource one group at a time holds, such as a branch on a remote.
    pub exclusive: String,
    pub steps: Vec<Stage>,
}
/// A finite forward sequence; rejected evaluators may take one bounded, pinned rework edge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Definition {
    pub schema: u32,
    pub source: String,
    pub stages: Vec<Stage>,
    pub max_reworks: u8,
    pub rework_target: Option<String>,
    /// Schema 2 only: batches that tickets reaching a `Join` step are queued into.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub groups: BTreeMap<String, GroupDef>,
}
impl Definition {
    pub fn validate(&self) -> Result<(), Refusal> { self.check(false) }
    /// A group's steps, which may split the group and may not join another.
    pub fn validate_group(&self) -> Result<(), Refusal> { self.check(true) }
    /// The definition a group item runs: the group's steps, as a definition of their own.
    pub fn group_definition(&self, name: &str) -> Option<Definition> {
        let group = self.groups.get(name)?;
        Some(Definition { schema: 2, source: format!("group/{name}"), stages: group.steps.clone(),
            max_reworks: 0, rework_target: None, groups: BTreeMap::new() })
    }
    fn check(&self, in_group: bool) -> Result<(), Refusal> {
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
            if let StageKind::Join { group, .. } = &stage.kind {
                if !valid_id(group) || group.len() > 64 || stage.progression != Progression::Automatic {
                    return Err(Refusal::Definition);
                }
            }
            if let StageKind::Review(Reviewer::Person(p) | Reviewer::Orchestrator(p)) = &stage.kind {
                if !valid_id(p) { return Err(Refusal::Definition); }
            }
        }
        if self.schema == 2 {
            self.validate_routes(in_group)?;
            return if in_group { Ok(()) } else { self.validate_groups() };
        }
        if !self.groups.is_empty() || self.stages.iter().any(|s| !s.routes.is_empty() || s.limit != 0
            || matches!(s.kind, StageKind::Join { .. }))
        { return Err(Refusal::Definition); }
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
    /// Every join names a group, every group has a join, and each group's steps are sound.
    fn validate_groups(&self) -> Result<(), Refusal> {
        let joined: BTreeSet<&str> = self.stages.iter().filter_map(|s| match &s.kind {
            StageKind::Join { group, .. } => Some(group.as_str()), _ => None }).collect();
        // Every join names a group, and every group has a join: nothing is declared and left unused.
        if joined.len() != self.groups.len() || !self.groups.keys().all(|g| joined.contains(g.as_str())) {
            return Err(Refusal::Definition);
        }
        for name in self.groups.keys() {
            let group = &self.groups[name];
            if !(1..=MAX_GROUP_MEMBERS).contains(&group.limit) || !valid_id(&group.exclusive) { return Err(Refusal::Definition); }
            self.group_definition(name).ok_or(Refusal::Definition)?.validate_group()?;
        }
        Ok(())
    }
    /// Schema 2: labels route, back edges are bounded, and every step is reachable and safe.
    fn validate_routes(&self, in_group: bool) -> Result<(), Refusal> {
        if self.max_reworks != 0 || self.rework_target.is_some() { return Err(Refusal::Definition); }
        let n = self.stages.len();
        let index: BTreeMap<&str, usize> = self.stages.iter().enumerate().map(|(i, s)| (s.id.as_str(), i)).collect();
        // Every edge a step can take: (from, label, to), with `None` as the end of the run.
        let mut edges: Vec<(usize, &str, Option<usize>)> = Vec::new();
        let mut done_reachable_from: Vec<usize> = Vec::new();
        for (i, stage) in self.stages.iter().enumerate() {
            if stage.limit > MAX_BACK_EDGE_LIMIT || stage.routes.len() > 8 { return Err(Refusal::Definition); }
            if in_group && matches!(stage.kind, StageKind::Join { .. }) { return Err(Refusal::Definition); }
            let mut back = false;
            let mut labels: Vec<&str> = vec!["passed", "rejected"];
            if matches!(stage.kind, StageKind::Join { .. }) { labels.push("dropped"); }
            for (label, target) in &stage.routes {
                if !valid_id(label) || !can_emit(&stage.kind, label) { return Err(Refusal::Definition); }
                if !labels.contains(&label.as_str()) { labels.push(label); }
                match target {
                    Target::Step(id) => {
                        let &to = index.get(id.as_str()).ok_or(Refusal::Definition)?;
                        back |= to <= i;
                    }
                    Target::Split { step, otherwise } => {
                        // A split restarts the group at an earlier step, so it is a back edge, and
                        // its fallback is a plain place to go: splits do not nest.
                        let &to = index.get(step.as_str()).ok_or(Refusal::Definition)?;
                        if !in_group || to > i { return Err(Refusal::Definition); }
                        // Both halves start over, so the step must discard the candidate the group had.
                        if !makes_candidate_anew(&self.stages[to].kind) { return Err(Refusal::Definition); }
                        // The second half would run again what the first already landed.
                        if self.stages[..=i].iter().any(|s| matches!(s.kind, StageKind::Land)) { return Err(Refusal::Definition); }
                        match &**otherwise {
                            Target::Step(id) if !index.contains_key(id.as_str()) => return Err(Refusal::Definition),
                            Target::Split { .. } => return Err(Refusal::Definition),
                            _ => {}
                        }
                        back = true;
                    }
                    Target::Done | Target::Fail => {}
                }
            }
            if back != (stage.limit > 0) { return Err(Refusal::Definition); }
            for label in labels {
                let target = match stage.routes.get(label) {
                    Some(target) => Some(target.clone()),
                    None if label == "passed" && i + 1 < n => Some(Target::Step(self.stages[i + 1].id.clone())),
                    None if label == "passed" => Some(Target::Done),
                    None => None,
                };
                let ends: Vec<Target> = match target {
                    Some(Target::Split { step, otherwise }) => vec![Target::Step(step), *otherwise],
                    Some(other) => vec![other],
                    None => vec![],
                };
                if ends.is_empty() { edges.push((i, label, None)); }
                for end in ends {
                    match end {
                        Target::Step(id) => edges.push((i, label, Some(index[id.as_str()]))),
                        Target::Done => { done_reachable_from.push(i); edges.push((i, label, None)); }
                        _ => edges.push((i, label, None)),
                    }
                }
            }
        }
        // A landing moves the base, so no edge may go back to or across one.
        for &(from, _, to) in &edges {
            let Some(to) = to else { continue };
            if to <= from && (to..=from).any(|l| matches!(self.stages[l].kind, StageKind::Land) && to <= l && l <= from) {
                return Err(Refusal::Definition);
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
        Self { schema: 1, source: "coding/v1".into(), max_reworks: 2, rework_target: Some("stage-1".into()), groups: BTreeMap::new(),
            stages: kinds.into_iter().enumerate().map(|(i, kind)| Stage {
                id: format!("stage-{i}"), kind, progression: Progression::Automatic,
                max_attempts: 2, routes: BTreeMap::new(), limit: 0,
                operation: OperationSpec { adapter_version: "fixture/v1".into(),
                    arguments: if i == 2 { vec!["cargo".into(), "test".into(), "--locked".into()] }
                        else { vec![format!("coding-operation-{i}")] }, environment: BTreeMap::new() },
            }).collect() }
    }
    pub fn tracker(principal: &str) -> Self {
        Self { schema: 1, source: "tracker/v1".into(), max_reworks: 0, rework_target: None, groups: BTreeMap::new(),
            stages: vec![Stage { id: "checklist".into(), kind: StageKind::Tracker,
                progression: Progression::AwaitApproval { principal: principal.into() },
                max_attempts: 1, routes: BTreeMap::new(), limit: 0,
                operation: OperationSpec { adapter_version: "tracker/v1".into(),
                    arguments: vec![], environment: BTreeMap::new() } }] }
    }
}
/// Back edges per step are capped so a loop cannot be a polite spelling of forever.
pub const MAX_BACK_EDGE_LIMIT: u8 = 3;
/// A group holds at most this many tickets.
pub const MAX_GROUP_MEMBERS: u8 = 32;
/// Labels a step can produce. `passed` and `rejected` come from anything; a label of a
/// definition's own is only for steps whose binding or command the definition chose.
pub fn can_emit(kind: &StageKind, label: &str) -> bool {
    matches!(label, "passed" | "rejected")
        || (label == "dropped" && matches!(kind, StageKind::Join { .. }))
        || matches!(kind, StageKind::Check | StageKind::Action(_) | StageKind::Evaluate(_))
}
fn needs_candidate(kind: &StageKind) -> bool {
    matches!(kind, StageKind::Check | StageKind::Review(_) | StageKind::Land | StageKind::Ci
        | StageKind::Tracker | StageKind::Evaluate(_) | StageKind::Join { .. })
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
