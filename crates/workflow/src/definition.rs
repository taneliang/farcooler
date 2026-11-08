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
    /// Group steps. Wait for the target's head (green, if the step says so) and take it as the base.
    AwaitBase,
    /// Group steps. Apply each member's commits, in order, onto the base in a fresh tree.
    Fold,
    /// Run one pinned command on the candidate and judge it by its exit.
    Run,
    /// Group steps. Push the candidate to the target once, only while the base it was gated on is current.
    Push,
    /// Wait for CI on the candidate: one effect, polled inside.
    AwaitCi,
    /// Read the paths the ticket changed and answer with the label its pinned rules give them. It only
    /// reads: the definition routes each label, and a label it doesn't route ends the run.
    Route,
    /// Pull request steps (ov-957). Push the candidate to the head branch the engine names for this item.
    PushBranch,
    /// Open the one pull request for the head branch, or find the one already open.
    OpenPr,
    /// Take a draft pull request out of draft.
    PrReady,
    /// Wait for the pull request to be merged. The merge commit becomes the item's candidate.
    AwaitPr,
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
    /// The models a build step may run with, by step id (ov-966): `claude:opus`, `codex:gpt-5.5`. The first is
    /// the step's default, and the orchestrator picks among them at Start. A step not listed runs the harness
    /// the runtime is configured with, as before; an empty map leaves the pin as it was.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, Vec<String>>,
}
impl Definition {
    pub fn validate(&self) -> Result<(), Refusal> { self.check(false) }
    /// A group's steps, which may split the group and may not join another.
    pub fn validate_group(&self) -> Result<(), Refusal> { self.check(true) }
    /// The definition a group item runs: the group's steps, as a definition of their own.
    pub fn group_definition(&self, name: &str) -> Option<Definition> {
        let group = self.groups.get(name)?;
        Some(Definition { schema: 2, source: format!("group/{name}"), stages: group.steps.clone(),
            max_reworks: 0, rework_target: None, groups: BTreeMap::new(), models: BTreeMap::new() })
    }
    fn check(&self, in_group: bool) -> Result<(), Refusal> {
        let mut ids = BTreeSet::new();
        if !matches!(self.schema, 1 | 2) || self.source.is_empty() || self.source.len() > 256
            || self.stages.is_empty() || self.stages.len() > 16 || self.max_reworks > 2
        { return Err(Refusal::Definition); }
        self.validate_models()?;
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
            // Landing verbs are routed (schema 2); the ones that act on a group's members are for groups.
            if !routed && is_verb(&stage.kind) { return Err(Refusal::Definition); }
            if !in_group && matches!(stage.kind, StageKind::Fold | StageKind::AwaitBase | StageKind::Push) {
                return Err(Refusal::Definition);
            }
            // A route reads a ticket's own change, which a group has no single answer for.
            if in_group && matches!(stage.kind, StageKind::Route) { return Err(Refusal::Definition); }
            // A pull request belongs to one ticket's branch, so these are not steps of a batch.
            if in_group && is_pr_verb(&stage.kind) { return Err(Refusal::Definition); }
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
    /// Each list names a build step, holds 1 to 8 distinct models, and each is a plain `harness[:model]`.
    fn validate_models(&self) -> Result<(), Refusal> {
        for (step, models) in &self.models {
            let build = self.stages.iter().any(|s| &s.id == step && s.kind == StageKind::Build);
            let distinct = models.iter().collect::<BTreeSet<_>>().len() == models.len();
            if !build || models.is_empty() || models.len() > MAX_MODELS || !distinct || !models.iter().all(|m| valid_model(m)) {
                return Err(Refusal::Definition);
            }
        }
        Ok(())
    }
    /// The model each listed step runs with: the one picked at Start, or the step's default. `None` when a pick
    /// names a step that lists no models, or a model the step doesn't list.
    pub fn picked_models(&self, picks: &BTreeMap<String, String>) -> Option<BTreeMap<String, String>> {
        if picks.iter().any(|(step, model)| !self.models.get(step).is_some_and(|list| list.contains(model))) { return None; }
        Some(self.models.iter().map(|(step, list)| (step.clone(), picks.get(step).unwrap_or(&list[0]).clone())).collect())
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
            if matches!(stage.kind, StageKind::Join { .. }) { labels.extend(["dropped", "landed"]); }
            for (label, target) in &stage.routes {
                if !valid_id(label) || !can_emit(&stage.kind, label) { return Err(Refusal::Definition); }
                // A rejected gate must stop, retry or split: falling forward or finishing would push or adopt what failed.
                if label == "rejected" && matches!(stage.kind, StageKind::Run | StageKind::Check)
                    && self.stages.iter().any(|s| matches!(s.kind, StageKind::Push))
                    && finishes_or_goes_on(target, &index, i)
                { return Err(Refusal::Definition); }
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
                        if self.stages[..=i].iter().any(|s| lands(&s.kind)) { return Err(Refusal::Definition); }
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
        // A landing moves the base, so no edge may go back to or across one. The one exception is a
        // push the remote refused (`base_moved`): nothing landed, so it goes back to start from the new base.
        for &(from, label, to) in &edges {
            let Some(to) = to else { continue };
            // A push the remote refused landed nothing; a change requested on a pull request means it is not merged.
            let unlanded = (label == "base_moved" && matches!(self.stages[from].kind, StageKind::Push))
                || (label == "changes_requested" && matches!(self.stages[from].kind, StageKind::AwaitPr));
            if to <= from && (to..=from).any(|l| lands(&self.stages[l].kind) && !(unlanded && l == from)) {
                return Err(Refusal::Definition);
            }
        }
        // Every path into a push crosses a gate that passed after the last step that changed what the
        // push sends. Walk the edges, not the list: a route can skip a gate that sits just above.
        let mut judged = vec![true; n];
        judged[0] = false;
        loop {
            let mut changed = false;
            for &(from, label, to) in &edges {
                let Some(to) = to else { continue };
                let kind = &self.stages[from].kind;
                let gate = matches!(kind, StageKind::Run | StageKind::Check | StageKind::Evaluate(_));
                let out = if gate { label == "passed" } else if makes_candidate(kind) { false } else { judged[from] };
                if judged[to] && !out { judged[to] = false; changed = true; }
            }
            if !changed { break; }
        }
        for (p, stage) in self.stages.iter().enumerate() {
            if matches!(stage.kind, StageKind::Push) && !judged[p] { return Err(Refusal::Definition); }
        }
        // A pull request needs its branch pushed, and waiting on one or readying it needs it opened.
        for (p, stage) in self.stages.iter().enumerate() {
            let needs = match stage.kind {
                StageKind::OpenPr => StageKind::PushBranch,
                StageKind::PrReady | StageKind::AwaitPr => StageKind::OpenPr,
                _ => continue,
            };
            if !self.stages[..p].iter().any(|s| s.kind == needs) { return Err(Refusal::Definition); }
        }
        // A requested change is answered by making a new candidate, not by pushing the old one again.
        for stage in self.stages.iter().filter(|s| matches!(s.kind, StageKind::AwaitPr)) {
            if let Some(Target::Step(to)) = stage.routes.get("changes_requested") {
                if !makes_candidate_anew(&self.stages[index[to.as_str()]].kind) { return Err(Refusal::Definition); }
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
        Self { schema: 1, source: "coding/v1".into(), max_reworks: 2, rework_target: Some("stage-1".into()), groups: BTreeMap::new(), models: BTreeMap::new(),
            stages: kinds.into_iter().enumerate().map(|(i, kind)| Stage {
                id: format!("stage-{i}"), kind, progression: Progression::Automatic,
                max_attempts: 2, routes: BTreeMap::new(), limit: 0,
                operation: OperationSpec { adapter_version: "fixture/v1".into(),
                    arguments: if i == 2 { vec!["cargo".into(), "test".into(), "--locked".into()] }
                        else { vec![format!("coding-operation-{i}")] }, environment: BTreeMap::new() },
            }).collect() }
    }
    /// The tracker-only template for a ticket with no run: one step per live column. The orchestrator
    /// moves it with `Move`; `done` and `cancelled` close it. It starts a run with `Start`.
    pub fn ticket() -> Self { Self::ticket_at("backlog").expect("backlog is a column") }
    /// The same template, starting in `column` (an existing card keeps the column it is already in).
    pub fn ticket_at(column: &str) -> Option<Self> {
        let start = TICKET_COLUMNS.iter().position(|c| *c == column)?;
        let stage = |id: &str| Stage { id: id.into(), kind: StageKind::Tracker,
            progression: Progression::AwaitApproval { principal: "orchestrator".into() },
            max_attempts: 1, routes: BTreeMap::new(), limit: 0,
            operation: OperationSpec { adapter_version: "tracker/v1".into(), arguments: vec![], environment: BTreeMap::new() } };
        Some(Self { schema: 1, source: TICKET_SOURCE.into(), max_reworks: 0, rework_target: None, groups: BTreeMap::new(), models: BTreeMap::new(),
            stages: TICKET_COLUMNS.iter().cycle().skip(start).take(TICKET_COLUMNS.len()).map(|id| stage(id)).collect() })
    }
    /// Whether this is the tracker-only template: a ticket with no run, moved by column.
    pub fn is_ticket(&self) -> bool {
        self.source == TICKET_SOURCE && self.stages.iter().all(|s| s.kind == StageKind::Tracker)
    }
    pub fn tracker(principal: &str) -> Self {
        Self { schema: 1, source: "tracker/v1".into(), max_reworks: 0, rework_target: None, groups: BTreeMap::new(), models: BTreeMap::new(),
            stages: vec![Stage { id: "checklist".into(), kind: StageKind::Tracker,
                progression: Progression::AwaitApproval { principal: principal.into() },
                max_attempts: 1, routes: BTreeMap::new(), limit: 0,
                operation: OperationSpec { adapter_version: "tracker/v1".into(),
                    arguments: vec![], environment: BTreeMap::new() } }] }
    }
}
/// Whether a route ends the run or moves on: a split's fallback counts, because a split with one member takes it.
fn finishes_or_goes_on(target: &Target, index: &BTreeMap<&str, usize>, from: usize) -> bool {
    match target {
        Target::Done => true,
        Target::Step(id) => index.get(id.as_str()).is_some_and(|&to| to > from),
        Target::Split { otherwise, .. } => finishes_or_goes_on(otherwise, index, from),
        Target::Fail => false,
    }
}
/// The source of the tracker-only template, and its columns in order.
pub const TICKET_SOURCE: &str = "ticket/v1";
pub const TICKET_COLUMNS: [&str; 5] = ["backlog", "todo", "doing", "review", "decision"];
/// Back edges per step are capped so a loop cannot be a polite spelling of forever.
pub const MAX_BACK_EDGE_LIMIT: u8 = 3;
/// A group holds at most this many tickets.
pub const MAX_GROUP_MEMBERS: u8 = 32;
/// Labels a step can produce. `passed` and `rejected` come from anything; a label of a
/// definition's own is only for steps whose binding or command the definition chose.
pub fn can_emit(kind: &StageKind, label: &str) -> bool {
    emits_unrouted(kind, label) || matches!(kind, StageKind::Check | StageKind::Action(_) | StageKind::Evaluate(_) | StageKind::Route)
}
/// Labels a step's own kind produces, which need no route to be a valid answer: with none, they fail
/// the item. A label of a definition's own (a check's or an action's) must be declared by a route.
pub fn emits_unrouted(kind: &StageKind, label: &str) -> bool {
    matches!(label, "passed" | "rejected")
        || (matches!(label, "dropped" | "landed") && matches!(kind, StageKind::Join { .. }))
        || (label == "conflict" && matches!(kind, StageKind::Fold))
        || (label == "base_moved" && matches!(kind, StageKind::Push))
        || (label == "timeout" && matches!(kind, StageKind::AwaitBase | StageKind::AwaitCi))
        || (label == "cancelled" && matches!(kind, StageKind::AwaitCi))
        || (label == "timeout" && matches!(kind, StageKind::AwaitPr))
        || (label == "changes_requested" && matches!(kind, StageKind::AwaitPr))
        || (label == "closed" && matches!(kind, StageKind::OpenPr | StageKind::PrReady | StageKind::AwaitPr))
}
/// The landing verbs: all of them need routes, so they are for schema 2.
fn is_verb(kind: &StageKind) -> bool {
    matches!(kind, StageKind::AwaitBase | StageKind::Fold | StageKind::Run | StageKind::Push | StageKind::AwaitCi | StageKind::Route) || is_pr_verb(kind)
}
/// The waits: a restart only makes them look again, and they keep their first deadline.
pub fn is_wait(kind: &StageKind) -> bool { matches!(kind, StageKind::AwaitBase | StageKind::AwaitCi | StageKind::AwaitPr) }
/// The pull request verbs: they act on one ticket's head branch.
fn is_pr_verb(kind: &StageKind) -> bool {
    matches!(kind, StageKind::PushBranch | StageKind::OpenPr | StageKind::PrReady | StageKind::AwaitPr)
}
/// Steps that move the base. Nothing after one may run its group over again. A merge moves the
/// base too, and it is a person's act on GitHub, so what waits for one is a landing.
pub fn lands(kind: &StageKind) -> bool { matches!(kind, StageKind::Land | StageKind::Push | StageKind::AwaitPr) }
fn needs_candidate(kind: &StageKind) -> bool {
    matches!(kind, StageKind::Check | StageKind::Review(_) | StageKind::Land | StageKind::Ci
        | StageKind::Tracker | StageKind::Evaluate(_) | StageKind::Join { .. }
        | StageKind::Fold | StageKind::Run | StageKind::Push | StageKind::AwaitCi | StageKind::Route
        | StageKind::PushBranch | StageKind::OpenPr | StageKind::PrReady | StageKind::AwaitPr)
}
/// What a step's result becomes the item's candidate. Waiting for the base takes the base as the
/// candidate, which the fold then builds on and replaces.
pub fn makes_candidate(kind: &StageKind) -> bool {
    matches!(kind, StageKind::Build | StageKind::Action(_) | StageKind::Tracker | StageKind::AwaitBase | StageKind::Fold
        | StageKind::AwaitPr)
}
/// Entering these discards the old candidate, because they are about to make a new one.
pub fn makes_candidate_anew(kind: &StageKind) -> bool { matches!(kind, StageKind::Build | StageKind::Action(_) | StageKind::AwaitBase) }
/// Most models one step may list.
pub const MAX_MODELS: usize = 8;
/// `harness` or `harness:model`, each part plain: it ends up as `--model` in a shell command.
pub fn valid_model(model: &str) -> bool {
    let plain = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    model.len() <= 64 && match model.split_once(':') { Some((harness, name)) => plain(harness) && plain(name), None => plain(model) }
}
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_:./".contains(&b))
}
