//! `coding/v1`, the pure half of the coding adapter (docs/engine-pipeline.md 3.3, 4.2), on the pipeline core.
//!
//! These are today's coding rules, moved out of the old reducer rather than rewritten:
//! - the operations and what each answers, from `StageKind` and `emits_unrouted` (definition.rs);
//! - the order rules a layout must keep, from `Definition::validate_routes` (definition.rs): landing verbs only
//!   inside a batch and ticket verbs only outside one, a pull request opened before it is readied or awaited,
//!   a candidate on every path into a step that needs one, and a gate passed after the last change on every
//!   path into a push;
//! - what a push may send, from the same rule at run time: only the candidate a gate passed, unless the
//!   orchestrator skipped the gate (5.1);
//! - candidates and evidence, from `evidence_matches` and `take_route` (reducer.rs): a judging step answers for
//!   the candidate it was given, and a build, base, fold or merge makes the next one;
//! - fold conflicts and gate failures, from `Outcome::FoldConflict` and `GroupFailure`;
//! - the CI rerun rule, from ci.rs: a red run whose failed jobs are all listed runs again once, and the look
//!   after it must name the same run at a newer attempt.
//!
//! What the core keeps for every adapter (routing, rework and attempts, batches and bisecting, crossings and
//! settling) is not repeated here. The token's coding facts (tree, candidate, base, what a gate passed, what
//! was pushed) are this adapter's data, which the core stores and never reads.
use std::collections::{BTreeMap, BTreeSet};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::pipeline::{AdapterRules, Adapters, Applied, Dedupe, Graph, Kind, Layout, OpSpec, Opaque, Outcome, Outward,
    Problem, Refusal, Repeat, StartPlan, Target, View};
use crate::{CiRun, GroupFailure, RunTree};

pub const VERSION: &str = "coding/v1";

/// The coding adapter's rules. Linked into every build, so a log that names `coding/v1` replays.
pub struct CodingV1;

/// The operations, by name. `check:<n>` and `gate:<n>` run one configured check; `check` and `gate` run all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op { Provision, Build, Check, Review, Route, Base, Fold, Candidate, Gate, GroupReview, Push, Ci, PushBranch, OpenPr, PrReady, Merged, Land }
impl Op {
    const ALL: [(Op, &'static str); 17] = [(Op::Provision, "provision"), (Op::Build, "build"), (Op::Check, "check"),
        (Op::Review, "review"), (Op::Route, "route"), (Op::Base, "base"), (Op::Fold, "fold"), (Op::Candidate, "candidate"), (Op::Gate, "gate"),
        (Op::GroupReview, "group_review"), (Op::Push, "push"), (Op::Ci, "ci"), (Op::PushBranch, "push_branch"),
        (Op::OpenPr, "open_pr"), (Op::PrReady, "pr_ready"), (Op::Merged, "merged"), (Op::Land, "land")];
    /// An op name and, for `check:<n>` and `gate:<n>`, the check it runs.
    pub fn parse(name: &str) -> Option<(Op, Option<usize>)> {
        let (base, index) = match name.split_once(':') {
            Some((b, n)) => (b, Some(n.parse::<usize>().ok().filter(|_| !n.starts_with('+'))?)),
            None => (name, None),
        };
        let op = Self::ALL.iter().find(|(_, n)| *n == base)?.0;
        if index.is_some() && !matches!(op, Op::Check | Op::Gate) { return None; }
        Some((op, index))
    }
    fn name(self) -> &'static str { Self::ALL.iter().find(|(o, _)| *o == self).unwrap().1 }
    /// The station kinds it may run under.
    fn kinds(self) -> Vec<Kind> {
        match self {
            Op::Build | Op::GroupReview => vec![Kind::Agent],
            Op::Review => vec![Kind::Agent, Kind::Command],
            Op::Base | Op::Ci | Op::Merged => vec![Kind::Wait],
            _ => vec![Kind::Command],
        }
    }
    /// What it answers (definition.rs `emits_unrouted`, in the layout's words: a failed check is `red`).
    fn labels(self, config: &Value) -> Vec<String> {
        let fixed: &[&str] = match self {
            Op::Provision | Op::Build | Op::PushBranch | Op::Candidate => &["passed"],
            Op::Check | Op::Gate => &["passed", "red"],
            Op::Review | Op::GroupReview => &["passed", "rejected"],
            Op::Route => return route_labels(config),
            Op::Base => &["passed", "timeout"],
            Op::Fold => &["passed", "conflict"],
            Op::Push | Op::Land => &["passed", "refused"],
            Op::Ci => &["passed", "red", "cancelled", "timeout"],
            Op::OpenPr | Op::PrReady => &["passed", "closed"],
            Op::Merged => &["passed", "closed", "changes_requested", "timeout"],
        };
        fixed.iter().map(|l| l.to_string()).collect()
    }
    /// Steps of a landing batch: they act on the batch's folded result, never on one ticket.
    fn in_batch(self) -> bool { matches!(self, Op::Base | Op::Fold | Op::Candidate | Op::Gate | Op::GroupReview | Op::Push) }
    /// Steps of one ticket: they read its own change or its own pull request.
    fn on_ticket(self) -> bool {
        matches!(self, Op::Provision | Op::Build | Op::Route | Op::PushBranch | Op::OpenPr | Op::PrReady | Op::Merged | Op::Land)
    }
    /// Outward: a push, a landing and a merge change the target, and can't be taken back (5.3).
    fn outward(self) -> bool { matches!(self, Op::Push | Op::Land | Op::Merged) }
    /// What it runs on, held while it runs: the landing target, so two batches never fold or push at once.
    fn holds_target(self) -> bool { matches!(self, Op::Fold | Op::Gate | Op::GroupReview | Op::Push | Op::Land) }
    /// Its answer is the next candidate (definition.rs `makes_candidate`).
    fn makes(self) -> bool { matches!(self, Op::Build | Op::Base | Op::Fold | Op::Merged) }
    /// Entering it discards the candidate, as it makes a new one (`makes_candidate_anew`).
    fn makes_anew(self) -> bool { matches!(self, Op::Build | Op::Base) }
    /// It needs a candidate to act on (`needs_candidate`).
    fn needs(self) -> bool { !matches!(self, Op::Provision | Op::Build | Op::Base) }
    /// It judges the candidate it was given, so its evidence must name that candidate (`evidence_matches`).
    fn judges(self) -> bool {
        matches!(self, Op::Check | Op::Review | Op::Route | Op::Candidate | Op::Gate | Op::GroupReview | Op::Push | Op::Ci
            | Op::PushBranch | Op::OpenPr | Op::PrReady | Op::Land)
    }
    /// A gate a push stands behind: a command whose `passed` judges the candidate.
    fn is_gate(self) -> bool { matches!(self, Op::Check | Op::Gate) }
}
fn route_labels(config: &Value) -> Vec<String> {
    match config.pointer("/route/labels").and_then(Value::as_array) {
        Some(labels) => labels.iter().filter_map(|l| l.as_str().map(str::to_string)).collect(),
        None => vec!["passed".into(), "needs_review".into()],
    }
}
/// The ref every outward step and every landing step is about: `coding:target:<ref>`.
pub fn boundary(config: &Value) -> String {
    let target = config.get("target").and_then(Value::as_str)
        .or_else(|| config.pointer("/pull_request/base").and_then(Value::as_str)).unwrap_or("main");
    format!("coding:target:{target}")
}
fn checks(config: &Value) -> usize { config.get("checks").and_then(Value::as_array).map_or(0, Vec::len) }

/// A token's or a batch's coding facts: `Token::data["coding"]`, `Batch::data["coding"]`, and what a member
/// joined with (`Membership::data`). Fields left out are absent, so an old record reads.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Data {
    /// The worktree the orchestrator named at intake (state.rs `RunTree`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<RunTree>,
    /// What the next judging step acts on: a build's commit, a batch's base, then its folded result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<String>,
    /// A batch: the target's head it folded onto.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// The candidate the last gate passed. A push sends only this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gated: Option<String>,
    /// How many visits there were when the candidate was made: a visit after this that ended `skipped` is the
    /// orchestrator skipping a gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub made_at: Option<usize>,
    /// What a push put on the target, or what a merge made of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pushed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict: Option<Conflict>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<GroupFailure>,
    /// The CI run being rerun; the look after it must see the same run at a newer attempt (ci.rs `CiRetry`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ci_retry: Option<CiRun>,
}
/// A fold that could not apply every member: the files, and the members it names when it could tell.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflict {
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub members: Option<Vec<String>>,
}
/// What the execution half reports with an answer, beyond its label and evidence (`Outcome::Labeled::data`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Facts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict: Option<Conflict>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<GroupFailure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<CiRun>,
    /// Members a failure points at, from the paths each changed against the failing tests.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suspects: Vec<String>,
}
/// One member as an effect names it: the ticket and the candidate it joined with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member { pub token: String, pub candidate: String }
/// `Effect::data["coding"]`: everything the execution half needs to run one step, from the effect alone.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Work {
    pub op: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<RunTree>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<Member>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ci_retry: Option<CiRun>,
}

/// An adapter value as the core stores it: versioned and digested.
pub fn opaque<T: Serialize>(value: &T) -> Opaque {
    let value = serde_json::to_value(value).expect("coding values serialize");
    Opaque { version: VERSION.into(), digest: crate::digest(&value), value }
}
/// This adapter's data among a subject's, wherever the layout keyed it.
pub fn data_of(data: &BTreeMap<String, Opaque>) -> Result<Data, Refusal> {
    match data.values().find(|o| o.version == VERSION) {
        Some(o) => serde_json::from_value(o.value.clone()).map_err(|_| Refusal::Invalid),
        None => Ok(Data::default()),
    }
}
fn valid(id: &str) -> bool { crate::valid_id(id) }
fn valid_failure(f: &GroupFailure) -> bool {
    !f.job.is_empty() && f.job.len() <= 1024 && f.first_line.len() <= 4096
        && !f.job.chars().any(char::is_control) && !f.first_line.chars().any(char::is_control)
}

impl AdapterRules for CodingV1 {
    fn version(&self) -> &'static str { VERSION }
    fn catalog(&self, config: &Value) -> Vec<OpSpec> {
        let target = boundary(config);
        let spec = |op: Op, name: String| OpSpec {
            name, kinds: op.kinds(), labels: op.labels(config),
            outward: op.outward().then(|| Outward { boundary: target.clone(), repeat: Repeat::Once, dedupe: Dedupe::None,
                crossing: vec!["passed".into()] }),
            resources: if op.holds_target() { vec![target.clone()] } else { Vec::new() },
        };
        let mut out: Vec<OpSpec> = Op::ALL.iter().map(|(op, name)| spec(*op, name.to_string())).collect();
        for n in 0..checks(config) {
            out.push(spec(Op::Check, format!("check:{n}")));
            out.push(spec(Op::Gate, format!("gate:{n}")));
        }
        out
    }
    fn validate(&self, layout: &Layout, config: &Value) -> Result<(), Vec<Problem>> {
        let problems = order_rules(self, layout, config);
        if problems.is_empty() { Ok(()) } else { Err(problems) }
    }
    fn start(&self, op: &str, view: &View, config: &Value) -> Result<StartPlan, Refusal> {
        let (kind, check) = Op::parse(op).ok_or(Refusal::Definition)?;
        let d = data_of(view.data)?;
        // A batch runs what its members joined with: each one needs a candidate (reducer.rs `Queue`).
        let members: Vec<Member> = view.memberships.iter().filter(|m| view.members.contains(&m.token))
            .map(|m| Ok(Member { token: m.token.clone(), candidate: data_of(&m.data)?.candidate.ok_or(Refusal::Missing)? }))
            .collect::<Result<_, Refusal>>()?;
        if kind.needs() && d.candidate.is_none() { return Err(Refusal::Missing); }
        if matches!(kind, Op::Push | Op::Land) && d.gated != d.candidate && !skipped_since(view, d.made_at) {
            // definition.rs: a push sends only a candidate a gate passed after the last change, unless the
            // orchestrator skipped the gate with a reason (5.1).
            return Err(Refusal::Evidence);
        }
        let work = Work { op: kind.name().into(), check, tree: d.tree.clone(),
            candidate: if kind.makes_anew() { None } else { d.candidate.clone() }, base: d.base.clone(), members,
            ci_retry: d.ci_retry.clone() };
        Ok(StartPlan { resources: Vec::new(), data: Some(opaque(&work)), spec: crate::digest(&(VERSION, op, config)) })
    }
    fn observe(&self, op: &str, view: &View, outcome: &Outcome, config: &Value) -> Result<Applied, Refusal> {
        let Outcome::Labeled { label, evidence, data } = outcome else { return Err(Refusal::Invalid) };
        let (kind, _) = Op::parse(op).ok_or(Refusal::Definition)?;
        if !kind.labels(config).contains(label) { return Err(Refusal::Evidence); }
        if !valid(&evidence.artifact) || !valid(&evidence.manifest) || !valid(&evidence.receipt) { return Err(Refusal::Evidence); }
        let facts: Facts = match data {
            Some(o) if o.version == VERSION => serde_json::from_value(o.value.clone()).map_err(|_| Refusal::Evidence)?,
            Some(_) => return Err(Refusal::Evidence),
            None => Facts::default(),
        };
        let mut d = data_of(view.data)?;
        if kind.judges() && d.candidate.as_deref() != Some(evidence.artifact.as_str()) { return Err(Refusal::Evidence); }
        let artifact = evidence.artifact.clone();
        let mut again = None;
        match (kind, label.as_str()) {
            (Op::Build | Op::Fold | Op::Merged, "passed") | (Op::Base, "passed") => {
                if kind == Op::Base { d.base = Some(artifact.clone()); d.conflict = None; d.failure = None; }
                if kind == Op::Merged { d.pushed = Some(artifact.clone()); }
                d.candidate = Some(artifact);
                d.gated = None;
                d.made_at = Some(view.visits.len());
            }
            (Op::Fold, "conflict") => {
                let conflict = facts.conflict.unwrap_or_default();
                let named = conflict.members.as_deref().unwrap_or(&[]);
                if conflict.files.len() > 1024 || conflict.files.iter().any(|f| f.is_empty() || f.contains('\0'))
                    || named.iter().collect::<BTreeSet<_>>().len() != named.len()
                    || named.iter().any(|m| !view.members.contains(m)) { return Err(Refusal::Evidence); }
                d.conflict = Some(conflict);
            }
            (Op::Check | Op::Gate, "passed") => d.gated = d.candidate.clone(),
            (Op::Check | Op::Gate | Op::Ci, "red") | (Op::GroupReview, "rejected") if facts.failure.is_some() => {
                let failure = facts.failure.unwrap();
                if !valid_failure(&failure) { return Err(Refusal::Evidence); }
                d.failure = Some(failure);
            }
            (Op::Push | Op::Land, "passed") => d.pushed = Some(artifact),
            _ => {}
        }
        if kind == Op::Ci {
            // ci.rs: the look after a rerun must see the same run at a newer attempt.
            if let Some(retry) = &d.ci_retry {
                if label != "timeout" && !facts.run.as_ref().is_some_and(|r| r.id == retry.id && r.attempt > retry.attempt) {
                    return Err(Refusal::Evidence);
                }
            }
            if facts.run.as_ref().is_some_and(|r| !r.valid()) { return Err(Refusal::Evidence); }
            d.ci_retry = None;
            if label == "red" {
                if let Some(run) = facts.run.as_ref().filter(|r| reruns(config, r)) {
                    d.ci_retry = Some(run.clone());
                    again = Some(opaque(run));
                }
            }
        }
        Ok(Applied { label: label.clone(), data: Some(opaque(&d)), again })
    }
    fn suspects(&self, batch: &View, outcome: &Outcome) -> Vec<String> {
        let Outcome::Labeled { data: Some(o), .. } = outcome else { return Vec::new() };
        let Ok(facts) = serde_json::from_value::<Facts>(o.value.clone()) else { return Vec::new() };
        let named: Vec<String> = match facts.conflict {
            // A conflict that names no member is the whole batch's: each member rebases (reducer.rs `conflict_brief`).
            Some(Conflict { members: None, .. }) => return batch.members.to_vec(),
            Some(Conflict { members: Some(m), .. }) => m,
            None => facts.suspects,
        };
        batch.members.iter().filter(|m| named.contains(m)).cloned().collect()
    }
    fn project(&self, data: &Opaque) -> Vec<(String, String)> {
        let Ok(d) = serde_json::from_value::<Data>(data.value.clone()) else { return Vec::new() };
        let mut out = Vec::new();
        if let Some(t) = &d.tree { out.push(("branch".into(), t.branch.clone())); }
        for (name, value) in [("candidate", &d.candidate), ("base", &d.base), ("pushed", &d.pushed)] {
            if let Some(v) = value { out.push((name.into(), v.clone())); }
        }
        if let Some(f) = &d.failure { out.push(("failure".into(), format!("{}: {}", f.job, f.first_line))); }
        out
    }
}
/// A visit after the candidate was made that the orchestrator ended `skipped`.
fn skipped_since(view: &View, made_at: Option<usize>) -> bool {
    view.visits.iter().skip(made_at.unwrap_or(0)).any(|v| v.ended.as_deref() == Some("skipped"))
}
/// ci.rs `rerun`: every failed job is one the config lists. The count is the station's `again` policy.
fn reruns(config: &Value, run: &CiRun) -> bool {
    let listed: Vec<&str> = config.pointer("/ci_rerun/jobs").and_then(Value::as_array).into_iter().flatten()
        .filter_map(Value::as_str).collect();
    !run.failed_jobs.is_empty() && run.failed_jobs.iter().all(|j| listed.contains(&j.as_str()))
}

/// The order rules (definition.rs `check` and `validate_routes`), over the layout's stations and routes.
fn order_rules(rules: &CodingV1, layout: &Layout, config: &Value) -> Vec<Problem> {
    let linked = Adapters::new(&[rules as &dyn crate::pipeline::AdapterRules]);
    let graph = Graph::new(layout, &linked);
    let mine: BTreeSet<&str> = layout.adapters.iter().filter(|(_, u)| u.version == VERSION && u.config == *config)
        .map(|(k, _)| k.as_str()).collect();
    let op_of = |id: &str| -> Option<Op> {
        let n = graph.node(id)?;
        if !mine.contains(n.adapter.as_deref()?) { return None; }
        Op::parse(n.def.op.as_deref()?.split_once('.')?.1).map(|(o, _)| o)
    };
    let mut problems = Vec::new();
    let mut say = |id: &str, message: &str| problems.push(Problem { station: Some(id.into()), message: message.into() });
    // Edges a step can take, by label: (from, label, to). Only routes to stations; ends and holds are no edge.
    let mut edges: Vec<(String, String, String)> = Vec::new();
    for id in &graph.order {
        let n = &graph.nodes[id];
        let mut labels: BTreeSet<String> = n.def.routes.keys().cloned().collect();
        labels.insert("passed".into());
        if let Some(spec) = &n.spec { labels.extend(spec.labels.iter().cloned()); }
        for label in labels {
            if let Some(Target::Station(to)) = graph.route(id, &label) { edges.push((id.clone(), label, to)); }
        }
    }
    let pushes = graph.order.iter().any(|id| matches!(op_of(id), Some(Op::Push | Op::Land)));
    for id in &graph.order {
        let Some(op) = op_of(id) else { continue };
        let node = &graph.nodes[id];
        if op.in_batch() && node.parent.is_none() { say(id, "a landing step runs inside a batch station"); }
        if op.on_ticket() && node.parent.is_some() { say(id, "a ticket's own step can't run inside a batch station"); }
        let earlier = |want: Op| graph.order.iter().take_while(|o| *o != id).any(|o| op_of(o) == Some(want));
        if matches!(op, Op::PrReady | Op::Merged) && !earlier(Op::OpenPr) { say(id, "a pull request is opened before it is readied or awaited"); }
        if op == Op::Merged {
            if let Some(Target::Station(to)) = graph.route(id, "changes_requested") {
                if op_of(&to) != Some(Op::Build) { say(id, "a requested change goes back to a build, which makes a new candidate"); }
            }
        }
        // A failing gate must stop, return or reject: falling forward or finishing would push what failed.
        if op.is_gate() && pushes {
            for label in op.labels(config).iter().filter(|l| *l != "passed") {
                let forward = match graph.route(id, label) {
                    Some(Target::Done | Target::Finish) => true,
                    Some(Target::Station(to)) => graph.nodes[&to].parent == node.parent && graph.nodes[&to].index > node.index,
                    _ => false,
                };
                if forward { say(id, &format!("its failing label {label} goes on toward a push")); }
            }
        }
    }
    // Entries: the first top-level station and each batch's first inner station start with nothing judged and,
    // inside a batch, no candidate of the batch's own.
    let entries: BTreeSet<String> = graph.order.iter().filter(|id| {
        let n = &graph.nodes[*id];
        n.index == 0
    }).cloned().collect();
    // Every path into a push crosses a gate that passed after the last step that changed what it sends.
    let mut judged: BTreeMap<String, bool> = graph.order.iter().map(|id| (id.clone(), !entries.contains(id))).collect();
    let mut has: BTreeMap<String, bool> = judged.clone();
    loop {
        let mut changed = false;
        for (from, label, to) in &edges {
            let op = op_of(from);
            let out = match op {
                Some(o) if o.is_gate() => label == "passed",
                Some(o) if o.makes() => false,
                _ => judged[from],
            };
            if judged[to] && !out { judged.insert(to.clone(), false); changed = true; }
            if op_of(to).is_some_and(Op::makes_anew) { continue; }
            let batch = graph.nodes[from].def.kind == Kind::Batch;
            let made = has[from] || batch || (label == "passed" && op.is_some_and(Op::makes));
            if has[to] && !made { has.insert(to.clone(), false); changed = true; }
        }
        if !changed { break; }
    }
    for id in &graph.order {
        let needs = match op_of(id) { Some(o) => o.needs(), None => graph.nodes[id].def.kind == Kind::Batch };
        if needs && !has[id] { say(id, "it needs a candidate, and some path reaches it without one"); }
        if matches!(op_of(id), Some(Op::Push | Op::Land)) && !judged[id] {
            say(id, "every path into a push passes a gate after the last change to what it sends");
        }
    }
    problems
}
