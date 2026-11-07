//! The four ways to land, as plain definitions (ov-950, lane 8): batched or per ticket, by push or by
//! pull request. They are built from a short plan, so a workspace tunes them by changing numbers and gates,
//! and every step is one a published definition could spell out by hand. Nothing here grants anything:
//! publishing the result still takes a person.
use std::collections::BTreeMap;
use crate::{Definition, GroupDef, OperationSpec, Progression, Refusal, Reviewer, Stage, StageKind, Target};

/// What pins an operation to the host's configuration. The daemon answers from its configured
/// adapter; tests answer from fixtures. Anything a template needs from the host comes through here.
pub trait Pins {
    fn operation(&self, version: &str, argument: &str) -> OperationSpec;
    fn wait(&self, version: &str, argument: &str, every_ms: u64, patience_ms: u64) -> OperationSpec;
    /// The resource one landing group at a time holds.
    fn landing_resource(&self) -> String;
    /// The branch a push goes to, as the host names it.
    fn landing_target(&self) -> String;
    /// What CI is read for.
    fn ci_subject(&self) -> String;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Template { BatchedPush, TicketPush, TicketPr, BatchedPr }
impl Template {
    pub const ALL: [Template; 4] = [Self::BatchedPush, Self::TicketPush, Self::TicketPr, Self::BatchedPr];
    pub fn name(self) -> &'static str {
        match self { Self::BatchedPush => "batched-push", Self::TicketPush => "ticket-push", Self::TicketPr => "ticket-pr", Self::BatchedPr => "batched-pr" }
    }
    pub fn parse(name: &str) -> Option<Self> { Self::ALL.into_iter().find(|t| t.name() == name) }
    pub fn batched(self) -> bool { matches!(self, Self::BatchedPush | Self::BatchedPr) }
    pub fn pushes(self) -> bool { matches!(self, Self::BatchedPush | Self::TicketPush) }
    /// What the template does, in a sentence for a person choosing one.
    pub fn about(self) -> &'static str {
        match self {
            Self::BatchedPush => "Tickets queue together, fold onto the base, pass the gates once, and one push lands them.",
            Self::TicketPush => "Each ticket folds onto the base, passes the gates and pushes on its own.",
            Self::TicketPr => "Each ticket opens a pull request that a person merges, then waits for CI.",
            Self::BatchedPr => "Open pull requests are folded and gated together as a rehearsal; a person merges each one.",
        }
    }
}

/// One gate, in the order it runs. Any of the three can stand at any place a gate can.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gate {
    /// A command the host pinned, by its place in the configured checks. Runs on the folded result when
    /// the template folds, else on the ticket.
    Command(usize),
    /// An agent reviews the ticket, with the host's configured reviewer.
    Agent { argument: String },
    /// A named person decides. Nothing runs; the ticket waits for their word.
    Person { principal: String },
}

/// Rules that send each ticket to the route its changed paths earn. The host writes `argument` in the form its
/// route verb reads and pins it; the template only places the step. A ticket answers `push` (straight to the
/// gates and the landing) or `ask` (through `approver`, a person, first). Any other label ends the run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathRoutes { pub argument: String, pub approver: Option<String> }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub template: Template,
    pub gates: Vec<Gate>,
    /// Whether the base must be green before a group starts (`green`), or any head will do (`any`).
    pub green_base: bool,
    /// Whether the pushed commit waits for CI.
    pub ci: bool,
    /// Most tickets in one batch. A per-ticket template is always one.
    pub batch: u8,
    pub every_ms: u64,
    pub base_patience_ms: u64,
    pub ci_patience_ms: u64,
    pub source: String,
    /// Route tickets by the paths they change, after the ticket gates and before they join a group.
    pub paths: Option<PathRoutes>,
}
impl Plan {
    pub fn new(template: Template, gates: Vec<Gate>) -> Self {
        Self { template, gates, green_base: true, ci: true, batch: 8, every_ms: 60_000,
            base_patience_ms: 3 * 3_600_000, ci_patience_ms: 3 * 3_600_000, source: format!("landing/{}", template.name()), paths: None }
    }
}

struct Maker<'a> { pins: &'a dyn Pins }
impl Maker<'_> {
    fn stage(&self, id: &str, kind: StageKind, version: &str, argument: &str) -> Stage {
        Stage { id: id.into(), kind, progression: Progression::Automatic, max_attempts: 2,
            operation: self.pins.operation(version, argument), routes: BTreeMap::new(), limit: 0 }
    }
    fn wait(&self, id: &str, kind: StageKind, version: &str, argument: &str, every: u64, patience: u64) -> Stage {
        Stage { operation: self.pins.wait(version, argument, every, patience), max_attempts: 5, ..self.stage(id, kind, version, "") }
    }
}
fn routed(mut stage: Stage, label: &str, to: Target, limit: u8) -> Stage {
    stage.routes.insert(label.into(), to); stage.limit = limit; stage
}
/// A route that ends the run: it takes no back-edge budget, so it leaves the step's limit alone.
fn ending(mut stage: Stage, label: &str, to: Target) -> Stage { stage.routes.insert(label.into(), to); stage }
fn step(id: &str) -> Target { Target::Step(id.into()) }
fn split(to: &str) -> Target { Target::Split { step: to.into(), otherwise: Box::new(Target::Fail) } }

/// The definition a plan describes, or a refusal when it would not be sound: a push with no command
/// gate to stand behind it, or a gate list longer than a definition holds.
pub fn build(plan: &Plan, pins: &dyn Pins) -> Result<Definition, Refusal> {
    let m = Maker { pins };
    let commands: Vec<usize> = plan.gates.iter().filter_map(|g| if let Gate::Command(i) = g { Some(*i) } else { None }).collect();
    if plan.template.pushes() && commands.is_empty() { return Err(Refusal::Definition); }
    let t = plan.template;
    let mut stages = vec![m.stage("provision", StageKind::Provision, "coding-worktree/v1", ""),
        m.stage("build", StageKind::Build, "coding-harness/v1", "")];
    // Gates that judge the ticket as it stands. A command gate joins them only where nothing folds.
    let mut judged = 0;
    for gate in &plan.gates {
        let id = format!("gate-{judged}");
        let stage = match gate {
            Gate::Command(i) if !t.batched() && !t.pushes() => m.stage(&id, StageKind::Check, "coding-check/v1", &i.to_string()),
            Gate::Command(_) => continue,
            Gate::Agent { argument } => m.stage(&id, StageKind::Review(Reviewer::Dispatch), "coding-review/v1", argument),
            Gate::Person { principal } => m.stage(&id, StageKind::Review(Reviewer::Person(principal.clone())), "coding-review/v1", &format!("person:{principal}")),
        };
        judged += 1;
        stages.push(routed(stage, "rejected", step("build"), 2));
    }
    // Where a ticket that passed its gates goes next: the join of a push template, else the end of the steps.
    if let (Some(paths), true) = (&plan.paths, t.pushes()) {
        let mut route = m.stage("route", StageKind::Route, "coding-route/v1", &paths.argument);
        route.routes.insert("push".into(), step("join"));
        stages.push(route);
        if let Some(principal) = &paths.approver {
            stages.last_mut().unwrap().routes.insert("ask".into(), step("approve"));
            let approve = m.stage("approve", StageKind::Review(Reviewer::Person(principal.clone())), "coding-review/v1", &format!("person:{principal}"));
            stages.push(routed(approve, "rejected", step("build"), 2));
        }
    } else if plan.paths.is_some() {
        return Err(Refusal::Definition);
    }
    let batch = if t.batched() { plan.batch } else { 1 };
    let (every, base_wait, ci_wait) = (plan.every_ms, plan.base_patience_ms, plan.ci_patience_ms);
    let mode = if plan.green_base { "green" } else { "any" };
    let mut groups = BTreeMap::new();
    let folds = |m: &Maker| -> Vec<Stage> {
        let mut steps = vec![routed(m.wait("base", StageKind::AwaitBase, "coding-await-base/v1", mode, every, base_wait), "timeout", Target::Fail, 0),
            routed(m.stage("fold", StageKind::Fold, "coding-fold/v1", ""), "conflict", split("base"), 3)];
        for (n, i) in commands.iter().enumerate() {
            steps.push(routed(m.stage(&format!("gate-{n}"), StageKind::Run, "coding-gate/v1", &i.to_string()), "rejected", split("base"), 3));
        }
        steps
    };
    match t {
        Template::BatchedPush | Template::TicketPush => {
            let mut join = m.stage("join", StageKind::Join { group: "landing".into(), adopt: true }, "coding-join/v1", "landing");
            join.max_attempts = 1;
            stages.push(routed(join, "rejected", step("build"), 1));
            let mut steps = folds(&m);
            steps.push(routed(m.stage("push", StageKind::Push, "coding-push/v1", &pins.landing_target()), "base_moved", step("base"), 2));
            if plan.ci {
                steps.push(m.wait("ci", StageKind::AwaitCi, "coding-await-ci/v1", &pins.ci_subject(), every, ci_wait));
            }
            groups.insert("landing".to_string(), GroupDef { limit: batch, exclusive: pins.landing_resource(), steps });
        }
        Template::TicketPr => {
            stages.push(m.stage("branch", StageKind::Action("push-branch".into()), "pr/v1", "push-branch"));
            stages.push(m.stage("pr", StageKind::Action("open-pr".into()), "pr/v1", "open-pr"));
            stages.push(ending(routed(m.wait("merged", StageKind::Action("await-pr".into()), "pr/v1", "await-pr", 300_000, 14 * 24 * 3_600_000),
                "changes_requested", step("build"), 2), "closed", Target::Fail));
            if plan.ci {
                stages.push(m.wait("ci", StageKind::AwaitCi, "coding-await-ci/v1", &pins.ci_subject(), every, ci_wait));
            }
        }
        Template::BatchedPr => {
            stages.push(m.stage("branch", StageKind::Action("push-branch".into()), "pr/v1", "push-branch"));
            stages.push(m.stage("pr", StageKind::Action("open-pr".into()), "pr/v1", "open-pr:draft"));
            let mut join = m.stage("rehearse", StageKind::Join { group: "rehearsal".into(), adopt: false }, "coding-join/v1", "rehearsal");
            join.max_attempts = 1;
            stages.push(routed(join, "rejected", step("build"), 2));
            stages.push(m.stage("ready", StageKind::Action("pr-ready".into()), "pr/v1", "pr-ready"));
            stages.push(ending(routed(m.wait("merged", StageKind::Action("await-pr".into()), "pr/v1", "await-pr", 300_000, 14 * 24 * 3_600_000),
                "changes_requested", step("build"), 2), "closed", Target::Fail));
            groups.insert("rehearsal".to_string(), GroupDef { limit: batch, exclusive: pins.landing_resource(), steps: folds(&m) });
        }
    }
    let definition = Definition { schema: 2, source: plan.source.clone(), stages, max_reworks: 0, rework_target: None, groups };
    definition.validate()?;
    Ok(definition)
}
