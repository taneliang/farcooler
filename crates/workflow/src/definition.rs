use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, BTreeMap};
use crate::{digest, Refusal, CiPolicy};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Progression { Automatic, AwaitApproval { principal: String } }
/// Who decides a review gate (ov-969). Each one blocks the run until that decider gives a verdict.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reviewer {
    /// The configured review command, which writes the verdict.
    Dispatch,
    /// The named principal, through a `Review` it sends: the orchestrator reviews however it chooses.
    Orchestrator(String),
    /// The named person, through a `Review` they send.
    Person(String),
    /// An agent the engine starts in the run's tree, with a model from `models` like a build step's. Its
    /// verdict is the receipt it writes, never its prose; a turn that ends without one is no verdict.
    Agent,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StageKind {
    Provision, Build, Check, Review(Reviewer), Land, Ci, Tracker,
    Action(String), Evaluate(String),
    /// Wait for the orchestrator to queue this ticket into the named group, then take the group's
    /// result as a label: `passed`, `rejected`, or `dropped`. `adopt` takes the group's result as
    /// the ticket's candidate.
    Join { group: String, adopt: bool },
    /// Group steps. Wait for the target's head and take it as the base. A `green` step also waits for
    /// CI on it to finish: a red base is taken too, so a fix for red main can land (ov-1076).
    AwaitBase,
    /// Group steps. Apply each member's commits, in order, onto the base in a fresh tree.
    Fold,
    /// Run one pinned command on the candidate and judge it by its exit.
    Run,
    /// Group steps. Push the candidate to the target once, only while the base it was gated on is current.
    Push,
    /// Wait for CI on the candidate: one effect, polled inside.
    AwaitCi,
    /// Internal CI rerun effect. A definition cannot schedule this verb directly.
    RerunCi,
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
    /// Group steps (ov-1269). Put the folded candidate on the group's own branch of the landing remote, so CI runs
    /// on it while the gate does. A CI wait after it judges that exact commit before the push moves the target.
    PushCandidate,
}
impl StageKind {
    /// An interrupted builder or dispatched reviewer resumes the same attempt.
    pub fn resumes_after_runner_restart(&self) -> bool {
        matches!(self, Self::Build | Self::Review(Reviewer::Agent | Reviewer::Dispatch))
    }
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
    /// Most members one group holds; a later ticket opens a group behind it. Absent is no cap: every ticket waiting
    /// joins (ov-1071). A failing one is still found, as a split halves the group however big it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// The resource one group at a time holds, such as a branch on a remote.
    pub exclusive: String,
    pub steps: Vec<Stage>,
}
impl GroupDef {
    /// Whether a group holding `members` tickets has room for one more.
    pub fn has_room(&self, members: usize) -> bool { self.limit.is_none_or(|l| members < l as usize) }
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
    /// The models a build step or an agent's review gate may run with, by step id (ov-966, ov-969): `claude:opus`,
    /// `codex:gpt-5.5`. The first is the step's default, and the orchestrator picks among them at Start. A step not
    /// listed runs the harness the runtime is configured with, as before; an empty map leaves the pin as it was.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, Vec<String>>,
    /// How many times the orchestrator may send one run back to its build step (ov-968). Left out, it is
    /// `DEFAULT_SEND_BACKS`; bounded by the published execution limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_backs: Option<u8>,
    /// Built-in engine rules, pinned with the definition (ov-1113).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub policies: BTreeMap<String, CiPolicy>,
    /// Pinned, read-only decision scripts at rare engine points (ov-1114).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub script_policies: BTreeMap<String, crate::ScriptPolicy>,
    /// Owner-published admission policy. Omission keeps manual starts and legacy pin bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_policy: Option<crate::scheduling::StartPolicy>,
    /// Execution limits apply to new operations, including those of an in-place publication update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<crate::scheduling::Limits>,
    /// Build directories this run may release when it stops building (ov-1126). Omitted on older
    /// definitions: use the engine defaults without changing their canonical bytes or pins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_folders: Option<Vec<String>>,
}
impl Definition {
    pub fn execution_limits(&self) -> crate::scheduling::Limits { self.limits.clone().unwrap_or_default() }
    pub fn validate(&self) -> Result<(), Refusal> {
        self.validate_detailed().map_err(|_| Refusal::Definition)
    }
    /// The same definition rules, with their location for a caller editing a draft.
    pub fn validate_detailed(&self) -> Result<(), String> { self.check(false) }
    /// A group's steps, which may split the group and may not join another.
    pub fn validate_group(&self) -> Result<(), Refusal> {
        self.check(true).map_err(|_| Refusal::Definition)
    }
    /// The definition a group item runs: the group's steps, as a definition of their own.
    pub fn group_definition(&self, name: &str) -> Option<Definition> {
        let group = self.groups.get(name)?;
        Some(Definition { schema: 2, source: format!("group/{name}"), stages: group.steps.clone(),
            max_reworks: 0, rework_target: None, groups: BTreeMap::new(), models: BTreeMap::new(), send_backs: None, build_folders: self.build_folders.clone(), policies: self.policies.iter().filter(|(_, p)| group.steps.iter().any(|s| s.id == p.step)).map(|(n,p)| (n.clone(),p.clone())).collect() , script_policies: self.script_policies.clone(), start_policy: None, limits: self.limits.clone() })
    }
    /// Every principal a step waits on, a group's steps included, in step order, each once (ov-1002): an
    /// approval step's, and a person's or the orchestrator's review gate's.
    pub fn deciders(&self) -> Vec<&str> {
        let mut names: Vec<&str> = Vec::new();
        for stage in self.stages.iter().chain(self.groups.values().flat_map(|g| g.steps.iter())) {
            let approval = match &stage.progression { Progression::AwaitApproval { principal } => Some(principal.as_str()), _ => None };
            let review = match &stage.kind {
                StageKind::Review(Reviewer::Person(p) | Reviewer::Orchestrator(p)) => Some(p.as_str()),
                _ => None,
            };
            for name in approval.into_iter().chain(review) {
                if !names.contains(&name) { names.push(name); }
            }
        }
        names
    }
    fn check(&self, in_group: bool) -> Result<(), String> {
        let limits = self.execution_limits();
        if self.start_policy.as_ref().is_some_and(|p| !p.valid()) {
            return Err("start_policy must name a valid automatic start policy".into());
        }
        if !matches!(self.schema, 1 | 2) { return Err("schema must be 1 or 2".into()); }
        if self.source.is_empty() || self.source.len() > 256 {
            return Err("source must contain 1 to 256 bytes".into());
        }
        if self.stages.is_empty() || self.stages.len() > limits.stages {
            return Err(format!("stages must contain 1 to {} entries", limits.stages));
        }
        if self.max_reworks > limits.reworks { return Err("max_reworks exceeds the execution limit".into()); }
        if self.send_backs.is_some_and(|n| n > limits.send_backs) {
            return Err("send_backs exceeds the execution limit".into());
        }
        crate::hooks::validate_policies(&self.script_policies).map_err(|_| "script_policies contains an invalid or duplicate hook")?;
        self.validate_models()?;
        self.validate_ci_policies().map_err(|_| "policies must contain valid, distinct rerun policies for CI wait stages")?;
        if self.build_folders.as_ref().is_some_and(|paths| paths.len() > 32
            || paths.iter().any(|p| !crate::cleanup::valid_folder(p))) {
            return Err("build_folders must contain at most 32 safe relative paths".into());
        }
        let mut ids = BTreeSet::new();
        let mut candidate_available = matches!(self.stages[0].kind, StageKind::Tracker);
        let routed = self.schema == 2;
        for (index, stage) in self.stages.iter().enumerate() {
            if !valid_id(&stage.id) { return Err(format!("stages[{index}].id is not a valid stage identifier")); }
            let problem = |field, rule| stage_problem(stage, field, rule);
            if !ids.insert(&stage.id) { return Err(problem("id", "stage IDs must be unique")); }
            if stage.max_attempts == 0 || stage.max_attempts > limits.attempts {
                return Err(problem("max_attempts", "attempts must be positive and within the execution limit"));
            }
            if !valid_id(&stage.operation.adapter_version) {
                return Err(problem("operation.adapter_version", "adapter version must be a valid identifier"));
            }
            if stage.operation.arguments.len() > 32 || stage.operation.arguments.iter().any(|a| a.len() > 4096 || a.contains('\0')) {
                return Err(problem("operation.arguments", "arguments are limited to 32 entries of 4096 bytes without NUL"));
            }
            if stage.operation.environment.len() > 32
                || stage.operation.environment.iter().any(|(k, v)| !valid_id(k) || v.len() > 4096 || v.contains('\0')) {
                return Err(problem("operation.environment", "environment entries need valid names and bounded values without NUL"));
            }
            if stage.kind == StageKind::RerunCi { return Err(problem("kind", "CI reruns must be defined as a policy")); }
            if !routed && is_verb(&stage.kind) { return Err(problem("kind", "landing operations require schema 2")); }
            if !in_group && matches!(stage.kind, StageKind::Fold | StageKind::AwaitBase | StageKind::Push | StageKind::PushCandidate) {
                return Err(problem("kind", "this operation requires a group"));
            }
            if in_group && (matches!(stage.kind, StageKind::Route) || is_pr_verb(&stage.kind)) {
                return Err(problem("kind", "this operation requires an individual ticket"));
            }
            match stage.kind {
                // Routed definitions check candidate availability along their edges instead.
                _ if routed => {}
                StageKind::Build | StageKind::Action(_) => candidate_available = true,
                StageKind::Check | StageKind::Review(_) | StageKind::Land | StageKind::Ci
                    | StageKind::Tracker | StageKind::Evaluate(_) if !candidate_available => {
                    return Err(problem("kind", "this operation requires a candidate"));
                }
                _ => {}
            }
            if let Progression::AwaitApproval { principal } = &stage.progression {
                if !valid_id(principal) { return Err(problem("progression.AwaitApproval.principal", "invalid approval principal")); }
            }
            if let StageKind::Action(binding) | StageKind::Evaluate(binding) = &stage.kind {
                if !valid_id(binding) { return Err(problem("kind", "invalid operation binding")); }
            }
            if let StageKind::Join { group, .. } = &stage.kind {
                if !valid_id(group) || group.len() > 64 {
                    return Err(problem("kind.Join.group", "invalid group identifier"));
                }
                if stage.progression != Progression::Automatic {
                    return Err(problem("progression", "a join must progress automatically"));
                }
            }
            if let StageKind::Review(Reviewer::Person(p) | Reviewer::Orchestrator(p)) = &stage.kind {
                if !valid_id(p) { return Err(problem("kind.Review", "invalid review principal")); }
            }
        }
        if routed {
            self.validate_routes(in_group)?;
            return if in_group { Ok(()) } else { self.validate_groups() };
        }
        if !self.groups.is_empty() { return Err("groups requires schema 2".into()); }
        for stage in &self.stages {
            if !stage.routes.is_empty() { return Err(stage_problem(stage, "routes", "routes require schema 2")); }
            if stage.limit != 0 { return Err(stage_problem(stage, "limit", "back edge limits require schema 2")); }
            if matches!(stage.kind, StageKind::Join { .. }) {
                return Err(stage_problem(stage, "kind", "a join requires schema 2"));
            }
        }
        // Sequence order is definition policy, not a global coding/reviewer policy.
        let target = self.rework_target.as_ref().map(|id| self.stages.iter().position(|s| &s.id == id));
        if self.max_reworks == 0 {
            if target.is_some() { return Err("rework_target requires a nonzero max_reworks".into()); }
        } else {
            let Some(Some(target)) = target else { return Err("rework_target must name a stage".into()); };
            let valid_target = matches!(self.stages[target].kind, StageKind::Build | StageKind::Action(_));
            let evaluators: Vec<_> = self.stages.iter().enumerate().filter(|(_, s)|
                matches!(s.kind, StageKind::Review(_) | StageKind::Evaluate(_))).collect();
            if !valid_target || evaluators.is_empty() || evaluators.iter().any(|(index, _)| target >= *index) {
                return Err("rework_target must be a build or action before all review and evaluation stages".into());
            }
        }
        Ok(())
    }
    /// Each list names a step that starts an agent (a build, or an agent's review gate), holds distinct
    /// models, and each is a plain `harness[:model]`.
    fn validate_models(&self) -> Result<(), String> {
        for (ordinal, (step, models)) in self.models.iter().enumerate() {
            let Some(stage) = self.stages.iter().find(|s| &s.id == step && valid_id(step)) else {
                return Err(format!("models entry {} must name a stage", ordinal + 1));
            };
            let distinct = models.iter().collect::<BTreeSet<_>>().len() == models.len();
            if !starts_agent(&stage.kind) || models.is_empty() || models.len() > self.execution_limits().models
                || !distinct || !models.iter().all(|m| valid_model(m)) {
                return Err(stage_problem(stage, "models", "an agent stage must list a bounded set of distinct harness or harness:model names"));
            }
        }
        Ok(())
    }
    /// The model each listed step runs with: the one picked at Start, or the step's default.
    /// A refusal identifies the model and step when the step doesn't list that pick.
    pub fn picked_models(&self, picks: &BTreeMap<String, String>) -> Result<BTreeMap<String, String>, Refusal> {
        if let Some((step, model)) = picks.iter().find(|(step, model)| !self.models.get(*step).is_some_and(|list| list.contains(model))) {
            return Err(Refusal::ModelRefused(format!("Model {model} is not listed for step {step}.")));
        }
        Ok(self.models.iter().map(|(step, list)| (step.clone(), picks.get(step).unwrap_or(&list[0]).clone())).collect())
    }
    /// Every join names a group, every group has a join, and each group's steps are sound.
    fn validate_groups(&self) -> Result<(), String> {
        let mut joined = BTreeSet::new();
        for stage in &self.stages {
            if let StageKind::Join { group, .. } = &stage.kind {
                if !self.groups.contains_key(group) {
                    return Err(stage_problem(stage, "kind.Join.group", "the group is not defined"));
                }
                joined.insert(group);
            }
        }
        for (index, (name, group)) in self.groups.iter().enumerate() {
            if !joined.contains(name) { return Err(format!("groups entry {} must be used by a join stage", index + 1)); }
            if group.limit == Some(0) { return Err(format!("group {name:?} field limit must be positive")); }
            if !valid_id(&group.exclusive) { return Err(format!("group {name:?} field exclusive must be a valid identifier")); }
            let definition = self.group_definition(name).expect("group exists");
            definition.check(true).map_err(|reason| format!("group {name:?}, {reason}"))?;
        }
        Ok(())
    }
    /// Schema 2: labels route, back edges are bounded, and every step is reachable and safe.
    fn validate_routes(&self, in_group: bool) -> Result<(), String> {
        if self.max_reworks != 0 { return Err("max_reworks must be zero in schema 2; use a stage limit".into()); }
        if self.rework_target.is_some() { return Err("rework_target must be absent in schema 2; use a stage route".into()); }
        let n = self.stages.len();
        let index: BTreeMap<&str, usize> = self.stages.iter().enumerate().map(|(i, s)| (s.id.as_str(), i)).collect();
        // Every edge a step can take: (from, label, to), with `None` as the end of the run.
        let mut edges: Vec<(usize, &str, Option<usize>)> = Vec::new();
        let mut done_reachable_from: Vec<usize> = Vec::new();
        for (i, stage) in self.stages.iter().enumerate() {
            let problem = |field, rule| stage_problem(stage, field, rule);
            if stage.limit > self.execution_limits().back_edges {
                return Err(problem("limit", "back edge limit exceeds the execution limit"));
            }
            if stage.routes.len() > self.execution_limits().routes {
                return Err(problem("routes", "the number of routes exceeds the execution limit"));
            }
            if in_group && matches!(stage.kind, StageKind::Join { .. }) {
                return Err(problem("kind", "a group cannot join another group"));
            }
            let (mut back, mut splits) = (false, false);
            let mut labels: Vec<&str> = vec!["passed", "rejected"];
            if matches!(stage.kind, StageKind::Join { .. }) { labels.extend(["dropped", "landed"]); }
            for (label, target) in &stage.routes {
                if !valid_id(label) { return Err(problem("routes", "invalid route label")); }
                let field = format!("routes.{label}");
                let route_problem = |rule| stage_problem(stage, &field, rule);
                if !can_emit(&stage.kind, label) { return Err(route_problem("the stage cannot emit this label")); }
                // A rejected gate must stop, retry or split: falling forward or finishing would push or adopt what failed.
                if label == "rejected" && matches!(stage.kind, StageKind::Run | StageKind::Check)
                    && self.stages.iter().any(|s| matches!(s.kind, StageKind::Push))
                    && finishes_or_goes_on(target, &index, i)
                { return Err(route_problem("a rejected gate must fail, split, or retry before a push")); }
                if !labels.contains(&label.as_str()) { labels.push(label); }
                match target {
                    Target::Step(id) => {
                        let &to = index.get(id.as_str()).ok_or_else(|| route_problem("target stage does not exist"))?;
                        back |= to <= i;
                    }
                    Target::Split { step, otherwise } => {
                        // A split restarts the group at an earlier step, so it is a back edge, and
                        // its fallback is a plain place to go: splits do not nest.
                        let &to = index.get(step.as_str()).ok_or_else(|| route_problem("split target stage does not exist"))?;
                        if !in_group || to > i { return Err(route_problem("a split must restart at an earlier group stage")); }
                        // Both halves start over, so the step must discard the candidate the group had.
                        if !makes_candidate_anew(&self.stages[to].kind) { return Err(route_problem("a split must restart at a stage that makes a fresh candidate")); }
                        // The second half would run again what the first already landed.
                        if self.stages[..=i].iter().any(|s| lands(&s.kind)) { return Err(route_problem("a split cannot follow a landing stage")); }
                        match &**otherwise {
                            Target::Step(id) => back |= *index.get(id.as_str()).ok_or_else(|| route_problem("split fallback stage does not exist"))? <= i,
                            Target::Split { .. } => return Err(route_problem("splits cannot be nested")),
                            _ => {}
                        }
                        splits = true;
                    }
                    Target::Done | Target::Fail => {}
                }
            }
            // A back edge needs a limit. A split needs none, as halving bounds it, but may keep one it was published
            // with before ov-1071; a limit on a step with neither bounds nothing.
            if back && stage.limit == 0 { return Err(problem("limit", "a back edge requires a positive limit")); }
            if !back && !splits && stage.limit > 0 { return Err(problem("limit", "a limit requires a back edge or split")); }
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
                return Err(stage_problem(&self.stages[from], &format!("routes.{label}"), "a retry cannot cross a landing stage"));
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
            if matches!(stage.kind, StageKind::Push) && !judged[p] { return Err(stage_problem(stage, "kind", "every path into a push must pass a gate for its candidate")); }
        }
        // A group that puts its candidate on a branch for CI (ov-1269) pushes only what that CI passed: every path into
        // a push crosses a CI wait that passed after the branch push, with no new candidate made since. A CI wait
        // before any push has nothing else to look at, so it needs the branch push behind it.
        if in_group {
            let walk = |gives: &dyn Fn(usize, &str) -> Option<bool>| -> Vec<bool> {
                let mut holds = vec![true; n];
                holds[0] = false;
                loop {
                    let mut changed = false;
                    for &(from, label, to) in &edges {
                        let Some(to) = to else { continue };
                        let out = gives(from, label).unwrap_or_else(|| !makes_candidate(&self.stages[from].kind) && holds[from]);
                        if holds[to] && !out { holds[to] = false; changed = true; }
                    }
                    if !changed { return holds; }
                }
            };
            let branched = walk(&|from, label| (self.stages[from].kind == StageKind::PushCandidate).then_some(label == "passed"));
            let green = walk(&|from, label| (self.stages[from].kind == StageKind::AwaitCi).then(|| label == "passed" && branched[from]));
            let branches = self.stages.iter().any(|s| s.kind == StageKind::PushCandidate);
            for (p, stage) in self.stages.iter().enumerate() {
                let pushed_before = self.stages[..p].iter().any(|s| s.kind == StageKind::Push);
                if branches && stage.kind == StageKind::Push && !green[p] {
                    return Err(stage_problem(stage, "kind", "every path into a push must pass CI for its candidate branch"));
                }
                if stage.kind == StageKind::AwaitCi && !pushed_before && !branched[p] {
                    return Err(stage_problem(stage, "kind", "waiting for CI requires a pushed branch"));
                }
            }
        }
        // A pull request needs its branch pushed, and waiting on one or readying it needs it opened.
        for (p, stage) in self.stages.iter().enumerate() {
            let needs = match stage.kind {
                StageKind::OpenPr => StageKind::PushBranch,
                StageKind::PrReady | StageKind::AwaitPr => StageKind::OpenPr,
                _ => continue,
            };
            if !self.stages[..p].iter().any(|s| s.kind == needs) { return Err(stage_problem(stage, "kind", "the pull request prerequisite stage must come first")); }
        }
        // A requested change is answered by making a new candidate, not by pushing the old one again.
        for stage in self.stages.iter().filter(|s| matches!(s.kind, StageKind::AwaitPr)) {
            if let Some(Target::Step(to)) = stage.routes.get("changes_requested") {
                if !makes_candidate_anew(&self.stages[index[to.as_str()]].kind) { return Err(stage_problem(stage, "routes.changes_requested", "rework must make a fresh candidate")); }
            }
        }
        // Reachability from the first step, and a way to Done.
        let mut seen = vec![false; n];
        let mut todo = vec![0usize];
        while let Some(i) = todo.pop() {
            if std::mem::replace(&mut seen[i], true) { continue; }
            todo.extend(edges.iter().filter(|e| e.0 == i).filter_map(|e| e.2));
        }
        if let Some(i) = seen.iter().position(|s| !s) {
            return Err(stage_problem(&self.stages[i], "routes", "the stage cannot be reached from the first stage"));
        }
        if !done_reachable_from.iter().any(|&i| seen[i]) { return Err("stages must have a route to Done".into()); }
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
            if needs_candidate(&stage.kind) && !has[i] { return Err(stage_problem(stage, "kind", "every path into this stage must supply a candidate")); }
        }
        Ok(())
    }
    pub fn pin(&self) -> String { digest(self) }
    /// How many send-backs one run may take.
    pub fn send_back_limit(&self) -> u8 { self.send_backs.unwrap_or(DEFAULT_SEND_BACKS.min(self.execution_limits().send_backs)) }
    pub fn coding() -> Self {
        let kinds = [StageKind::Provision, StageKind::Build, StageKind::Check,
            StageKind::Review(Reviewer::Dispatch), StageKind::Land, StageKind::Ci];
        Self { schema: 1, source: "coding/v1".into(), max_reworks: 2, rework_target: Some("stage-1".into()), groups: BTreeMap::new(), models: BTreeMap::new(), send_backs: None, build_folders: None, script_policies: Default::default(), start_policy: None, limits: None, policies: Default::default(),
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
        Some(Self { schema: 1, source: TICKET_SOURCE.into(), max_reworks: 0, rework_target: None, groups: BTreeMap::new(), models: BTreeMap::new(), send_backs: None, build_folders: None, script_policies: Default::default(), start_policy: None, limits: None, policies: Default::default(),
            stages: TICKET_COLUMNS.iter().cycle().skip(start).take(TICKET_COLUMNS.len()).map(|id| stage(id)).collect() })
    }
    /// Whether this is the tracker-only template: a ticket with no run, moved by column.
    pub fn is_ticket(&self) -> bool {
        self.source == TICKET_SOURCE && self.stages.iter().all(|s| s.kind == StageKind::Tracker)
    }
    pub fn tracker(principal: &str) -> Self {
        Self { schema: 1, source: "tracker/v1".into(), max_reworks: 0, rework_target: None, groups: BTreeMap::new(), models: BTreeMap::new(), send_backs: None, build_folders: None, script_policies: Default::default(), start_policy: None, limits: None, policies: Default::default(),
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
/// Back edges per step are capped so a loop cannot be a polite spelling of forever. Five lets a review turn work back
/// four times (ov-1071). A split isn't counted here: each one halves the group, so the group's size bounds it.
pub const MAX_BACK_EDGE_LIMIT: u8 = 5;
/// A ticket can carry at most this many followers when it starts.
pub const MAX_FOLLOWERS: usize = 32;
/// Default follower allowance; published execution limits can override it.
pub const MAX_GROUP_MEMBERS: u8 = MAX_FOLLOWERS as u8;
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
    matches!(kind, StageKind::AwaitBase | StageKind::Fold | StageKind::Run | StageKind::Push | StageKind::AwaitCi | StageKind::Route
        | StageKind::PushCandidate) || is_pr_verb(kind)
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
        | StageKind::PushBranch | StageKind::OpenPr | StageKind::PrReady | StageKind::AwaitPr | StageKind::PushCandidate)
}
/// What a step's result becomes the item's candidate. Waiting for the base takes the base as the
/// candidate, which the fold then builds on and replaces.
pub fn makes_candidate(kind: &StageKind) -> bool {
    matches!(kind, StageKind::Build | StageKind::Action(_) | StageKind::Tracker | StageKind::AwaitBase | StageKind::Fold
        | StageKind::AwaitPr)
}
/// Entering these discards the old candidate, because they are about to make a new one.
pub fn makes_candidate_anew(kind: &StageKind) -> bool { matches!(kind, StageKind::Build | StageKind::Action(_) | StageKind::AwaitBase) }
/// The steps the engine starts an agent for, which may list the models it runs with.
pub fn starts_agent(kind: &StageKind) -> bool { matches!(kind, StageKind::Build | StageKind::Review(Reviewer::Agent)) }
/// Send-backs a run may take when its workflow doesn't say, and the most it may say (ov-968).
pub const DEFAULT_SEND_BACKS: u8 = 3;
pub const MAX_SEND_BACKS: u8 = 5;
/// Default model-list allowance; a publication can override it in its execution limits.
pub const MAX_MODELS: usize = 8;
/// `harness` or `harness:model`, each part plain: it ends up as `--model` in a shell command.
pub fn valid_model(model: &str) -> bool {
    let plain = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    model.len() <= 64 && match model.split_once(':') { Some((harness, name)) => plain(harness) && plain(name), None => plain(model) }
}
fn stage_problem(stage: &Stage, field: &str, rule: &str) -> String {
    format!("stage {:?} field {field}: {rule}", stage.id)
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_:./".contains(&b))
}
