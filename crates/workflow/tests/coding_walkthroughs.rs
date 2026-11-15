//! The coding walkthroughs of docs/engine-pipeline.md section 8 (8.1 fully agentic, 8.2 human pull request
//! review), end to end through the pipeline reducer with `coding/v1` linked (ov-1273). Each test ends with a
//! replay that must reach the same state, effects and digest.
//!
//! Where the layout differs from the YAML in the design, it is because schema 3 as built spells it so:
//! patience is `patience_ms`; 8.2's `group:` deciders and `needs: { total, from }` are two named people and
//! `needs: 2` (the core has no group membership yet, ov-1266 report item 9); `source: coding.pr_reviews` is
//! the reviewers' own `Decide`; and `command/v1` (deploy) is a fixture here, as it is a later ticket (10.6).
#[path = "pipeline_support/mod.rs"]
mod support;
use std::collections::{BTreeMap, BTreeSet};
use farcooler_workflow::adapters::coding::{data_of, opaque, CodingV1, Conflict, Data, Facts, Op};
use farcooler_workflow::pipeline::*;
use farcooler_workflow::{CiRun, GroupFailure, RunTree};
use serde_json::{json, Value};
use support::*;

const H: i64 = HOUR;

/// 8.1, as written in the design.
fn agentic() -> Value {
    json!({ "schema": 3, "name": "agentic-batched-push",
        "adapters": { "coding": { "version": "coding/v1", "config": { "repository": ".", "target": "main", "ci": "ci.yml",
            "checks": ["cargo test"], "ci_rerun": { "jobs": ["flaky"] } } } },
        "stations": [
            { "id": "provision", "kind": "Command", "op": "coding.provision", "pool": "work" },
            { "id": "build", "kind": "Agent", "op": "coding.build", "pool": "agents",
              "models": ["claude:opus", "codex:gpt-5.5", "claude:sonnet"], "policy": { "attempts": 2 } },
            { "id": "review", "kind": "Agent", "op": "coding.review", "pool": "agents", "models": ["claude:opus", "codex:gpt-5.5"],
              "routes": { "rejected": "build" }, "policy": { "rework": 2 } },
            { "id": "route", "kind": "Command", "op": "coding.route", "pool": "work",
              "routes": { "passed": "land", "needs_review": "decide" } },
            { "id": "decide", "kind": "Decision", "deciders": ["orchestrator"], "routes": { "approved": "land", "rejected": "build" } },
            { "id": "land", "kind": "Batch", "pool": "landing", "form": { "size": 8, "free_on_crossing": true },
              "on_reject": "Bisect", "policy": { "returns": 1 }, "routes": { "returned": "build", "passed": "done" },
              "stations": [
                { "id": "base", "kind": "Wait", "op": "coding.base", "policy": { "patience_ms": 3 * H } },
                { "id": "fold", "kind": "Command", "op": "coding.fold", "routes": { "conflict": "returned" } },
                { "id": "checks", "kind": "Command", "op": "coding.check", "routes": { "red": "rejected" } },
                { "id": "group-review", "kind": "Agent", "op": "coding.group_review", "models": ["claude:opus"],
                  "routes": { "rejected": "rejected" } },
                { "id": "gate", "kind": "Command", "op": "coding.gate", "routes": { "red": "rejected" } },
                { "id": "push", "kind": "Command", "op": "coding.push", "routes": { "refused": "base" } },
                { "id": "ci", "kind": "Wait", "op": "coding.ci", "policy": { "patience_ms": 3 * H, "again": 1 } } ] }
        ] })
}
/// 8.2, as written in the design (see the module comment for the three places schema 3 spells differently).
fn company() -> Value {
    json!({ "schema": 3, "name": "company-human-review",
        "adapters": { "coding": { "version": "coding/v1", "config": { "repository": ".", "pull_request": { "repository": "acme/app", "base": "main" } } },
                      "deploy": { "version": "command/v1", "config": { "staging": ["./deploy", "staging"],
                          "production": { "run": ["./deploy", "production"], "outward": "fresh", "reconcile": ["./deploy", "status"] } } } },
        "stations": [
            { "id": "provision", "kind": "Command", "op": "coding.provision", "pool": "work" },
            { "id": "build", "kind": "Agent", "op": "coding.build", "pool": "agents", "models": ["claude:opus"] },
            { "id": "self-review", "kind": "Agent", "op": "coding.review", "pool": "agents", "routes": { "rejected": "build" },
              "policy": { "rework": 2 } },
            { "id": "open-pr", "kind": "Command", "op": "coding.open_pr", "pool": "work" },
            { "id": "pr-ci", "kind": "Wait", "op": "coding.ci", "policy": { "patience_ms": 2 * H }, "routes": { "red": "build" } },
            { "id": "code-review", "kind": "Decision", "deciders": ["person:ana", "person:bo"], "needs": 2,
              "routes": { "approved": "merged", "changes_requested": "build" }, "policy": { "skip": "Person" } },
            { "id": "merged", "kind": "Wait", "op": "coding.merged", "policy": { "patience_ms": 14 * DAY },
              "routes": { "passed": "main-ci" } },
            { "id": "main-ci", "kind": "Wait", "op": "coding.ci", "policy": { "patience_ms": 2 * H } },
            { "id": "staging", "kind": "Command", "op": "deploy.staging", "pool": "work" },
            { "id": "release", "kind": "Decision", "deciders": ["person:release-manager"], "routes": { "approved": "production" },
              "policy": { "skip": "Person" } },
            { "id": "production", "kind": "Command", "op": "deploy.production", "pool": "work", "routes": { "passed": "done" } }
        ] })
}
/// `command/v1` for 8.2's deploys: a fixture until the generic command adapter lands.
fn deploy() -> Fixture {
    let op = |name: &str| OpSpec { name: name.into(), kinds: vec![Kind::Command], labels: vec!["passed".into(), "failed".into()],
        outward: None, resources: Vec::new() };
    let mut production = op("production");
    production.outward = Some(Outward { boundary: "deploy:production".into(), repeat: Repeat::Fresh, dedupe: Dedupe::None,
        crossing: vec!["passed".into()] });
    Fixture { version: "command/v1", ops: vec![op("staging"), production] }
}
fn coding_run(pools: &[(&str, u16)], workflow: Value) -> Run {
    let linked: Vec<&'static dyn AdapterRules> = vec![&CodingV1, Box::leak(Box::new(deploy()))];
    Run::with(pools, workflow, Adapters::new(&linked))
}
fn data(r: &Run, s: &Subject) -> Data {
    let d = match s { Subject::Token(k) => &r.p.tokens[k].data, Subject::Batch(k) => &r.p.batches[k].data };
    data_of(d).unwrap()
}
fn station_of(r: &Run, s: &Subject) -> String { r.location(s).station().unwrap().to_string() }
/// Answer the subject's running step. A judging step answers for the candidate it was given; a step that
/// makes a candidate answers with a new one, named after the subject and the station.
fn say(r: &mut Run, s: &Subject, label: &str, facts: Option<Facts>) {
    let op = r.work(s).op.op.clone();
    let kind = if r.work(s).op.adapter == "coding" { Op::parse(&op).map(|(o, _)| o) } else { None };
    let n = r.p.revision;
    let artifact = match (kind, label) {
        (None, _) => format!("{}-{op}-{n}", s.key()),
        (Some(Op::Build | Op::Base | Op::Fold | Op::Merged), "passed") => format!("{}-{}-{n}", s.key().replace('#', "-"), station_of(r, s).replace('.', "-")),
        _ => data(r, s).candidate.unwrap_or_else(|| "none".into()),
    };
    let evidence = Evidence { artifact, manifest: "manifest".into(), receipt: format!("receipt-{n}"), spec: "spec".into() };
    r.observe(s, Outcome::Labeled { label: label.into(), evidence, data: facts.map(|f| opaque(&f)) })
        .unwrap_or_else(|e| panic!("{s:?} answering {label} at {}: {e:?}", station_of(r, s)));
}
fn pass(r: &mut Run, s: &Subject) { say(r, s, "passed", None) }
fn decide(r: &mut Run, by: Option<&str>, s: &Subject, station: &str, verdict: &str) -> Result<Vec<Effect>, Refusal> {
    let command = Command::Decide { subject: s.clone(), station: station.into(), verdict: verdict.into(), suspects: vec![],
        reason: Some(format!("{verdict}, by the decider")) };
    match by { Some(name) => r.person(name, command), None => r.orch(command) }
}
fn intake(r: &mut Run, key: &str, tree: bool) {
    let tree = tree.then(|| RunTree { name: key.into(), branch: format!("ticket/{key}"), path: format!("/trees/{key}"),
        base: "a".repeat(40), adopted: false });
    let data = BTreeMap::from([("coding".to_string(), opaque(&Data { tree, ..Data::default() }))]);
    r.send(Grant::Runtime, Command::Intake { token: key.into(), to: Some("provision".into()), priority: 0, data,
        dependencies: BTreeSet::new(), models: BTreeMap::new(), brief: None }).unwrap();
}
/// provision → build → review → route, to the landing batch or the orchestrator's decision.
fn to_route(r: &mut Run, key: &str) {
    let s = tok(key);
    for station in ["provision", "build", "review"] {
        assert_eq!(r.station(key).as_deref(), Some(station));
        pass(r, &s);
    }
    assert_eq!(r.station(key).as_deref(), Some("route"));
}
fn batch(r: &Run, key: &str) -> Subject { Subject::Batch(r.batch_of(key)) }
fn members(r: &Run, b: &Subject) -> Vec<String> { r.p.batches[b.key()].members.iter().map(|m| m.token.clone()).collect() }
fn coding_effect(e: &Effect) -> farcooler_workflow::adapters::coding::Work {
    serde_json::from_value(e.data["coding"].value.clone()).unwrap()
}

#[test]
fn agentic_tickets_build_review_route_and_land_in_batches() {
    let mut r = coding_run(&[("agents", 4), ("work", 2), ("landing", 1)], agentic());
    intake(&mut r, "t1", true);
    // The tree named at intake reaches every effect, and the build's agent gets the station's first model.
    let build = { pass(&mut r, &tok("t1")); r.effect_for(&tok("t1")) };
    assert_eq!(coding_effect(&build).tree.unwrap().branch, "ticket/t1");
    assert_eq!(build.model.as_deref(), Some("claude:opus"));
    pass(&mut r, &tok("t1"));
    // review rejected → build, with the reason as the brief; then it passes.
    say(&mut r, &tok("t1"), "rejected", None);
    assert_eq!(r.station("t1").as_deref(), Some("build"));
    pass(&mut r, &tok("t1"));
    pass(&mut r, &tok("t1"));
    // route passed → land: the landing is free, so a batch of one forms at once (as a queued group left at once).
    pass(&mut r, &tok("t1"));
    let b1 = batch(&r, "t1");
    assert_eq!(station_of(&r, &b1), "land.base");
    pass(&mut r, &b1);
    pass(&mut r, &b1);
    let fold = r.effects.iter().rev().find(|e| e.station == "land.fold").unwrap();
    assert_eq!(coding_effect(fold).members.iter().map(|m| m.token.as_str()).collect::<Vec<_>>(), ["t1"]);
    assert_eq!(fold.resources, ["coding:target:main"], "a fold holds the landing target");
    // Meanwhile t2 needs a decision and t3 and t4 route straight to the landing: they queue behind b1.
    intake(&mut r, "t2", false);
    to_route(&mut r, "t2");
    say(&mut r, &tok("t2"), "needs_review", None);
    decide(&mut r, None, &tok("t2"), "decide", "approved").unwrap();
    for k in ["t3", "t4"] { intake(&mut r, k, false); to_route(&mut r, k); pass(&mut r, &tok(k)); }
    assert_eq!(r.queue("land"), ["t2", "t3", "t4"], "one landing at a time");
    for station in ["land.checks", "land.group-review", "land.gate"] {
        assert_eq!(station_of(&r, &b1), station);
        pass(&mut r, &b1);
    }
    // The push crosses coding:target:main for the batch and each member, and frees the landing place: the next
    // batch forms and folds while b1 waits for CI (ov-1209).
    pass(&mut r, &b1);
    assert_eq!(r.p.batches[b1.key()].crossings[0].boundary, "coding:target:main");
    assert_eq!(r.p.tokens["t1"].crossings.len(), 1);
    assert_eq!(station_of(&r, &b1), "land.ci");
    let b2 = batch(&r, "t2");
    assert_eq!(members(&r, &b2), ["t2", "t3", "t4"]);
    // b1's CI goes red on a listed flaky job: the adapter asks for it again, once, and the look after it names
    // the same run at a newer attempt.
    let run = |attempt| CiRun { id: 7, attempt, failed_jobs: vec!["flaky".into()] };
    say(&mut r, &b1, "red", Some(Facts { run: Some(run(1)), ..Facts::default() }));
    assert_eq!(station_of(&r, &b1), "land.ci");
    assert_eq!(coding_effect(&r.effect_for(&b1)).ci_retry, Some(run(1)), "the rerun names the run");
    let stale = Outcome::Labeled { label: "passed".into(), evidence: Evidence { artifact: data(&r, &b1).candidate.unwrap(),
        manifest: "m".into(), receipt: "r".into(), spec: "s".into() }, data: Some(opaque(&Facts { run: Some(run(1)), ..Facts::default() })) };
    assert_eq!(r.observe(&b1, stale), Err(Refusal::Evidence), "the same attempt can't answer for the rerun");
    say(&mut r, &b1, "passed", Some(Facts { run: Some(run(2)), ..Facts::default() }));
    let closed = r.closed("t1");
    assert_eq!((closed.closed.clone(), closed.crossings.len()), (Closure::Done, 1));
    assert!(r.p.archive[b1.key()].batch.is_some());

    // b2: the fold names t4 as conflicting; t4 goes back to build, the rest go on.
    pass(&mut r, &b2);
    say(&mut r, &b2, "conflict", Some(Facts { conflict: Some(Conflict { files: vec!["src/lib.rs".into()],
        members: Some(vec!["t4".into()]) }), ..Facts::default() }));
    assert_eq!(r.station("t4").as_deref(), Some("build"));
    assert!(r.p.tokens["t4"].brief.as_deref().unwrap().contains("conflict"));
    assert_eq!(members(&r, &b2), ["t2", "t3"]);
    assert_eq!(station_of(&r, &b2), "land.checks");
    pass(&mut r, &b2);
    pass(&mut r, &b2);
    // The gate goes red: the batch bisects on the gate, and finds t2.
    let failure = GroupFailure { job: "cargo test".into(), first_line: "test parse ... FAILED".into(), split: false };
    say(&mut r, &b2, "red", Some(Facts { failure: Some(failure.clone()), ..Facts::default() }));
    assert_eq!(r.effect_for(&b2).members, ["t2"]);
    say(&mut r, &b2, "red", None);
    assert_eq!(r.effect_for(&b2).members, ["t3"]);
    pass(&mut r, &b2);
    assert_eq!(r.station("t2").as_deref(), Some("build"));
    assert_eq!(data(&r, &b2).failure, Some(failure), "the gate's failure stays on the batch");
    assert_eq!((members(&r, &b2), station_of(&r, &b2)), (vec!["t3".to_string()], "land.push".to_string()));
    // The target moved: the push is refused, nothing crossed, and the batch refolds on the new base.
    say(&mut r, &b2, "refused", None);
    assert!(r.p.batches[b2.key()].crossings.is_empty());
    assert_eq!(station_of(&r, &b2), "land.base");
    for _ in ["base", "fold", "checks", "group-review", "gate", "push"] { pass(&mut r, &b2); }
    pass(&mut r, &b2);
    assert_eq!(r.closed("t3").closed, Closure::Done);
    // t2 and t4 come back through build and review; t2 reaches the free landing first and forms at once.
    for k in ["t2", "t4"] {
        for _ in ["build", "review", "route"] { pass(&mut r, &tok(k)); }
    }
    assert_eq!(members(&r, &batch(&r, "t2")), ["t2"]);
    assert_eq!(r.queue("land"), ["t4"]);
    r.assert_replays();
}

#[test]
fn the_orchestrator_skips_a_gate_and_the_push_takes_it() {
    let mut r = coding_run(&[("agents", 4), ("work", 2), ("landing", 1)], agentic());
    intake(&mut r, "t1", false);
    to_route(&mut r, "t1");
    pass(&mut r, &tok("t1"));
    let b = batch(&r, "t1");
    for _ in ["base", "fold"] { pass(&mut r, &b); }
    // Both gates are skipped, so nothing passed the folded result: only the orchestrator's skip lets it push.
    r.orch(Command::Skip { subject: b.clone(), station: "land.checks".into(), reason: "the gate's runner is down; checks passed".into() }).unwrap();
    r.stopped(&b);
    pass(&mut r, &b);
    assert_eq!(station_of(&r, &b), "land.gate");
    assert_eq!(data(&r, &b).gated, None);
    r.orch(Command::Skip { subject: b.clone(), station: "land.gate".into(), reason: "the gate's runner is down; checks passed".into() }).unwrap();
    r.stopped(&b);
    assert_eq!(station_of(&r, &b), "land.push", "the push starts: the skip is the orchestrator's, with a reason");
    let skipped = r.p.batches[b.key()].visits.iter().find(|v| v.station == "land.gate").unwrap();
    assert_eq!((skipped.ended.as_deref(), skipped.reason.as_deref()), (Some("skipped"), Some("the gate's runner is down; checks passed")));
    pass(&mut r, &b);
    pass(&mut r, &b);
    assert_eq!(r.closed("t1").closed, Closure::Done);
    r.assert_replays();
}

#[test]
fn a_crossed_batch_settles_before_it_moves_and_a_red_ci_holds_for_the_orchestrator() {
    let mut r = coding_run(&[("agents", 4), ("work", 2), ("landing", 1)], agentic());
    for k in ["t1", "t2"] { intake(&mut r, k, false); to_route(&mut r, k); }
    pass(&mut r, &tok("t1"));
    let b = batch(&r, "t1");
    for _ in ["base", "fold", "checks", "group-review", "gate"] { pass(&mut r, &b); }
    pass(&mut r, &tok("t2"));
    assert_eq!(r.queue("land"), ["t2"]);
    // 5.3's walk-through: the push is out and its answer is unknown; a cancel is only recorded.
    r.observe(&b, Outcome::Uncertain).unwrap();
    assert_eq!(r.orch(Command::Pull { token: "t1".into(), reason: "late".into() }), Err(Refusal::Stopping),
        "uncertain outward work settles first");
    // A Settle with evidence that it went out records the crossing and goes on.
    let receipt = Evidence { artifact: data(&r, &b).candidate.unwrap(), manifest: "ref read".into(), receipt: "origin/main".into(), spec: "s".into() };
    r.orch(Command::Settle { subject: b.clone(), station: "land.push".into(), crossed: true, evidence: receipt }).unwrap();
    assert_eq!(r.p.tokens["t1"].crossings.len(), 1);
    assert_eq!(station_of(&r, &b), "land.ci");
    // Red, not a listed job: no rerun, and the batch holds. It crossed, so nothing sends it back before the push.
    say(&mut r, &b, "red", Some(Facts { run: Some(CiRun { id: 9, attempt: 1, failed_jobs: vec!["unit".into()] }), ..Facts::default() }));
    assert!(matches!(r.location(&b), Location::Held { station, .. } if station == "land.ci"));
    assert_eq!(r.orch(Command::Release { subject: b.clone(), station: "land.ci".into(), to: Some("build".into()), reason: None }),
        Err(Refusal::Crossed));
    // The orchestrator skips the red CI with a reason: the members close, crossed.
    r.orch(Command::Skip { subject: b.clone(), station: "land.ci".into(), reason: "red on main before this landed".into() }).unwrap();
    assert_eq!(r.closed("t1").closed, Closure::Done);
    // t2 was waiting for the landing place: it formed when t1's push crossed.
    assert_eq!(station_of(&r, &batch(&r, "t2")), "land.base");
    r.assert_replays();
}

#[test]
fn review_sends_back_twice_then_holds_and_the_orchestrator_overrides() {
    let mut r = coding_run(&[("agents", 4), ("work", 2), ("landing", 1)], agentic());
    intake(&mut r, "t1", false);
    pass(&mut r, &tok("t1"));
    pass(&mut r, &tok("t1"));
    for _ in 0..2 {
        say(&mut r, &tok("t1"), "rejected", None);
        pass(&mut r, &tok("t1"));
    }
    say(&mut r, &tok("t1"), "rejected", None);
    match r.at("t1") {
        Location::Held { station, hold } => {
            assert_eq!(station, "review");
            assert_eq!(hold.kind, HoldKind::PolicyLimit { policy: "review.rework".into() });
        }
        other => panic!("a third rejection holds: {other:?}"),
    }
    r.orch(Command::Release { subject: tok("t1"), station: "review".into(), to: None, reason: Some("one more try".into()) }).unwrap();
    assert_eq!(r.station("t1").as_deref(), Some("build"));
    let overrode = r.p.tokens["t1"].visits.iter().rev().find(|v| v.station == "review").unwrap().overrode.clone();
    assert_eq!(overrode.as_deref(), Some("review.rework"));
    // An answer for another candidate than the one it was given is refused.
    pass(&mut r, &tok("t1"));
    let wrong = Outcome::Labeled { label: "passed".into(), evidence: Evidence { artifact: "someone-else".into(), manifest: "m".into(),
        receipt: "r".into(), spec: "s".into() }, data: None };
    assert_eq!(r.observe(&tok("t1"), wrong), Err(Refusal::Evidence));
    r.assert_replays();
}

#[test]
fn company_human_review_merge_and_release() {
    let mut r = coding_run(&[("agents", 2), ("work", 2)], company());
    intake(&mut r, "t1", true);
    for station in ["provision", "build", "self-review", "open-pr"] {
        assert_eq!(r.station("t1").as_deref(), Some(station));
        pass(&mut r, &tok("t1"));
    }
    // pr-ci red → build (the branch push isn't outward), then green → code-review.
    say(&mut r, &tok("t1"), "red", None);
    assert_eq!(r.station("t1").as_deref(), Some("build"));
    for _ in ["build", "self-review", "open-pr", "pr-ci"] { pass(&mut r, &tok("t1")); }
    assert!(matches!(r.at("t1"), Location::Awaiting { station, .. } if station == "code-review"));
    // Changes requested by one reviewer: back to build with the comments as the brief.
    decide(&mut r, Some("bo"), &tok("t1"), "code-review", "changes_requested").unwrap();
    assert_eq!(r.station("t1").as_deref(), Some("build"));
    assert_eq!(r.p.tokens["t1"].brief.as_deref(), Some("changes_requested, by the decider"));
    for _ in ["build", "self-review", "open-pr", "pr-ci"] { pass(&mut r, &tok("t1")); }
    // Only a person may waive review; one approval is not two.
    assert_eq!(r.orch(Command::Skip { subject: tok("t1"), station: "code-review".into(), reason: "trivial".into() }),
        Err(Refusal::PersonOnly));
    assert_eq!(decide(&mut r, None, &tok("t1"), "code-review", "approved"), Err(Refusal::Unauthorized));
    decide(&mut r, Some("ana"), &tok("t1"), "code-review", "approved").unwrap();
    assert!(matches!(r.at("t1"), Location::Awaiting { .. }), "two approvals are needed");
    decide(&mut r, Some("bo"), &tok("t1"), "code-review", "approved").unwrap();
    // merged is an outward wait: a person merges on GitHub, and the merge crosses the target.
    assert_eq!(r.station("t1").as_deref(), Some("merged"));
    assert_eq!(r.effect_for(&tok("t1")).idempotency, "t1:coding:target:main:1");
    pass(&mut r, &tok("t1"));
    assert_eq!(r.p.tokens["t1"].crossings[0].boundary, "coding:target:main");
    assert_eq!(data(&r, &tok("t1")).pushed, data(&r, &tok("t1")).candidate, "the merge commit is the candidate main CI reads");
    // Nothing after the merge may lead back before it.
    assert_eq!(r.orch(Command::Send { token: "t1".into(), station: "main-ci".into(), to: "build".into(), reason: "fix".into() }),
        Err(Refusal::Crossed));
    pass(&mut r, &tok("t1"));
    pass(&mut r, &tok("t1"));
    assert!(matches!(r.at("t1"), Location::Awaiting { station, .. } if station == "release"));
    assert_eq!(r.orch(Command::Skip { subject: tok("t1"), station: "release".into(), reason: "Friday".into() }), Err(Refusal::PersonOnly));
    decide(&mut r, Some("release-manager"), &tok("t1"), "release", "approved").unwrap();
    assert_eq!(r.effect_for(&tok("t1")).idempotency, "t1:deploy:production:1");
    let evidence = r.evidence(&tok("t1"));
    r.observe(&tok("t1"), Outcome::Labeled { label: "passed".into(), evidence, data: None }).unwrap();
    let closed = r.closed("t1");
    assert_eq!(closed.crossings.iter().map(|c| c.boundary.as_str()).collect::<Vec<_>>(), ["coding:target:main", "deploy:production"]);

    // A pull request closed unmerged holds for the orchestrator, who sends it back to build.
    intake(&mut r, "t2", false);
    for _ in ["provision", "build", "self-review", "open-pr", "pr-ci"] { pass(&mut r, &tok("t2")); }
    decide(&mut r, Some("ana"), &tok("t2"), "code-review", "approved").unwrap();
    decide(&mut r, Some("bo"), &tok("t2"), "code-review", "approved").unwrap();
    say(&mut r, &tok("t2"), "closed", None);
    assert!(matches!(r.at("t2"), Location::Held { station, .. } if station == "merged"));
    assert!(r.p.tokens["t2"].crossings.is_empty(), "closed is not a crossing label");
    r.orch(Command::Release { subject: tok("t2"), station: "merged".into(), to: Some("build".into()), reason: Some("reopen the work".into()) }).unwrap();
    assert_eq!(r.station("t2").as_deref(), Some("build"));
    r.assert_replays();
}

#[test]
fn the_agentic_and_company_layouts_validate_as_written() {
    let linked: Vec<&'static dyn AdapterRules> = vec![&CodingV1, Box::leak(Box::new(deploy()))];
    let adapters = Adapters::new(&linked);
    let pools: BTreeSet<String> = ["agents", "work", "landing"].iter().map(|s| s.to_string()).collect();
    for l in [agentic(), company()] {
        let l: Layout = serde_json::from_value(l).unwrap();
        assert!(Graph::validate(&l, &adapters, &pools).is_ok(), "{}", l.name);
    }
}
