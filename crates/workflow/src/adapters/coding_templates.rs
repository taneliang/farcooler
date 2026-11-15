//! The four landing templates (templates.rs) as schema-3 layouts for `coding/v1`. The same `Plan` builds
//! either: `templates::build` for the old reducer, `layout` here for the pipeline core. Station ids keep the
//! old step ids (`join`, `rehearse`, `gate-0`), so a board and a migration can name the same place.
//!
//! What changed in the move, by design (docs/engine-pipeline.md 4.4, 4.5, 8.1):
//! - a group step's `Split` is the batch's `on_reject: bisect`, and a diagnosed fold conflict `returned`;
//! - back-edge limits are station policy: a gate's `rework` (2) and a batch's `returns` (1, or 2 for the
//!   rehearsal, as the join's limit was);
//! - a label the old definition sent to `Fail` (a base or CI timeout, a closed pull request) is left unrouted,
//!   so it holds for the orchestrator instead of failing the run;
//! - a push batch gives up its landing place once it has pushed (`free_on_crossing`), which is ov-1209's
//!   "the next group folds and gates while this one waits for CI".
use std::collections::BTreeMap;
use serde_json::Value;
use crate::pipeline::{AdapterUse, Form, Kind, Layout, OnReject, Policy, Refusal, StationDef};
use crate::templates::{Gate, Plan, Template};
use super::coding::VERSION;

/// The adapter's key in a template's layout.
pub const KEY: &str = "coding";
const FOURTEEN_DAYS_MS: i64 = 14 * 24 * 3_600_000;

fn station(id: &str, kind: Kind, op: Option<&str>, pool: Option<&str>) -> StationDef {
    StationDef { id: id.into(), kind, op: op.map(|o| format!("{KEY}.{o}")), pool: pool.map(str::to_string), ..StationDef::default() }
}
fn route(mut s: StationDef, label: &str, to: &str) -> StationDef { s.routes.insert(label.into(), to.into()); s }
fn decision(id: &str, principal: &str) -> StationDef {
    StationDef { deciders: vec![principal.into()], ..station(id, Kind::Decision, None, None) }
}
fn wait(id: &str, op: &str, patience_ms: u64) -> StationDef {
    StationDef { policy: Policy { patience_ms: Some(patience_ms as i64), ..Policy::default() }, ..station(id, Kind::Wait, Some(op), None) }
}

/// The layout a plan describes, with the adapter's config, or a refusal when it would not be sound: a push
/// with no command gate to stand behind it, or path routes on a template that doesn't push.
pub fn layout(plan: &Plan, config: &Value) -> Result<Layout, Refusal> {
    let t = plan.template;
    let commands: Vec<usize> = plan.gates.iter().filter_map(|g| if let Gate::Command(i) = g { Some(*i) } else { None }).collect();
    if t.pushes() && commands.is_empty() { return Err(Refusal::Definition); }
    if plan.paths.is_some() && !t.pushes() { return Err(Refusal::Definition); }
    let mut config = config.clone();
    let c = config.as_object_mut().ok_or(Refusal::Definition)?;
    c.insert("base".into(), (if plan.green_base { "green" } else { "any" }).into());
    c.insert("every_ms".into(), plan.every_ms.into());
    if let Some(paths) = &plan.paths { c.insert("route".into(), serde_json::json!({ "rules": paths.argument })); }
    let reviewer = plan.gates.iter().find_map(|g| if let Gate::Reviewer { argument } = g { Some(argument.clone()) } else { None });
    if let Some(argument) = reviewer { c.insert("review".into(), argument.into()); }
    if !t.pushes() { c.insert("pull_request_draft".into(), (t == Template::BatchedPr).into()); }

    let mut stations = vec![station("provision", Kind::Command, Some("provision"), Some("work")),
        station("build", Kind::Agent, Some("build"), Some("agents"))];
    // Gates that judge the ticket as it stands. A command gate joins them only where nothing folds.
    let mut judged = 0;
    for gate in &plan.gates {
        let id = format!("gate-{judged}");
        let s = match gate {
            Gate::Command(i) if !t.batched() && !t.pushes() => route(station(&id, Kind::Command, Some(&format!("check:{i}")), Some("work")), "red", "build"),
            Gate::Command(_) => continue,
            Gate::Reviewer { .. } => route(station(&id, Kind::Command, Some("review"), Some("work")), "rejected", "build"),
            Gate::Agent => route(station(&id, Kind::Agent, Some("review"), Some("agents")), "rejected", "build"),
            Gate::Orchestrator { principal } | Gate::Person { principal } => {
                let who = if matches!(gate, Gate::Person { .. }) { format!("person:{principal}") } else { orchestrator(principal) };
                route(decision(&id, &who), "rejected", "build")
            }
        };
        judged += 1;
        stations.push(s);
    }
    let landing = if t.pushes() { "join" } else { "rehearse" };
    if let Some(paths) = &plan.paths {
        let mut r = route(station("route", Kind::Command, Some("route"), Some("work")), "passed", landing);
        if let Some(principal) = &paths.approver {
            r = route(r, "needs_review", "approve");
            stations.push(r);
            stations.push(route(route(decision("approve", &format!("person:{principal}")), "approved", landing), "rejected", "build"));
        } else {
            stations.push(r);
        }
    }
    let batch = if t.batched() { plan.batch } else { Some(1) };
    let size = batch.map(|n| u16::try_from(n).unwrap_or(u16::MAX));
    // reducer.rs: landing infrastructure in a group (base, fold, gates, CI) gets three attempts.
    let infra = |mut s: StationDef| { s.policy.attempts = Some(3); s };
    let folds = || {
        let mut inner = vec![infra(wait("base", "base", plan.base_patience_ms)),
            infra(route(station("fold", Kind::Command, Some("fold"), None), "conflict", "returned"))];
        for (n, i) in commands.iter().enumerate() {
            inner.push(infra(route(station(&format!("gate-{n}"), Kind::Command, Some(&format!("gate:{i}")), None), "red", "rejected")));
        }
        inner
    };
    let batch_station = |inner: Vec<StationDef>, returns: u8, passed: Option<&str>, frees: bool| {
        let mut s = StationDef { form: Some(Form { size, window: None, free_on_crossing: frees }), on_reject: OnReject::Bisect,
            policy: Policy { returns: Some(returns), ..Policy::default() }, stations: inner,
            ..route(station(landing, Kind::Batch, None, Some("landing")), "returned", "build") };
        if let Some(p) = passed { s = route(s, "passed", p); }
        s
    };
    match t {
        Template::BatchedPush | Template::TicketPush => {
            let mut inner = folds();
            inner.push(route(station("push", Kind::Command, Some("push"), None), "refused", "base"));
            if plan.ci { inner.push(infra(ci(plan, &config))); }
            stations.push(batch_station(inner, 1, None, true));
        }
        Template::TicketPr => {
            stations.push(station("branch", Kind::Command, Some("push_branch"), Some("work")));
            stations.push(station("pr", Kind::Command, Some("open_pr"), Some("work")));
            stations.push(route(wait("merged", "merged", FOURTEEN_DAYS_MS as u64), "changes_requested", "build"));
            if plan.ci { stations.push(ci(plan, &config)); }
        }
        Template::BatchedPr => {
            stations.push(station("branch", Kind::Command, Some("push_branch"), Some("work")));
            stations.push(station("pr", Kind::Command, Some("open_pr"), Some("work")));
            stations.push(batch_station(folds(), 2, Some("ready"), false));
            stations.push(station("ready", Kind::Command, Some("pr_ready"), Some("work")));
            stations.push(route(wait("merged", "merged", FOURTEEN_DAYS_MS as u64), "changes_requested", "build"));
        }
    }
    // A decision station routes `approved` explicitly (4.1): to the station after it.
    let ids: Vec<String> = stations.iter().map(|s| s.id.clone()).collect();
    for (i, s) in stations.iter_mut().enumerate() {
        if s.kind == Kind::Decision && !s.routes.contains_key("approved") {
            s.routes.insert("approved".into(), ids.get(i + 1).cloned().unwrap_or_else(|| "done".into()));
        }
    }
    Ok(Layout { schema: 3, name: plan.source.replace('/', "-"), intake: Vec::new(), stations,
        adapters: BTreeMap::from([(KEY.to_string(), AdapterUse { version: VERSION.into(), config })]) })
}
/// The CI wait. A config that lists flaky jobs reruns them once (ci.rs), which is the station's `again`.
fn ci(plan: &Plan, config: &Value) -> StationDef {
    let mut s = wait("ci", "ci", plan.ci_patience_ms);
    if config.pointer("/ci_rerun/jobs").and_then(Value::as_array).is_some_and(|j| !j.is_empty()) { s.policy.again = Some(1); }
    s
}
/// The orchestrator decides as itself; another named principal is a person.
fn orchestrator(principal: &str) -> String {
    if principal == "orchestrator" { "orchestrator".into() } else { format!("person:{principal}") }
}
