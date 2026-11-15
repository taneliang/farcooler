//! Control of the pipeline (docs/engine-pipeline.md 4.3, 4.4, 5): order, capacity, policies that hold and never
//! close, and every intervention the orchestrator makes, against the content layout of 8.4.
#[path = "pipeline_support/mod.rs"]
mod support;
use farcooler_workflow::pipeline::*;
use support::*;

/// research → outline → pick (orchestrator approves) → write → fact-check, for each key.
fn to_fact_check(r: &mut Run, keys: &[&str]) {
    for k in keys {
        let s = tok(k);
        r.answer(&s, "passed");
        r.answer(&s, "passed");
        r.orch(Command::Decide { subject: s.clone(), station: "pick".into(), verdict: "approved".into(), suspects: vec![], reason: None }).unwrap();
        r.answer(&s, "passed");
        assert_eq!(r.station(k).as_deref(), Some("fact-check"));
    }
}
fn working(r: &Run, station: &str) -> Vec<String> { r.p.stations[station].working.iter().cloned().collect() }

#[test]
fn queues_order_by_priority_then_arrival_and_the_orchestrator_reorders() {
    let mut r = Run::new(&[("agents", 3), ("work", 1)], content());
    r.orch(Command::HoldStation { station: "research".into(), reason: "collect first".into() }).unwrap();
    r.intake("t1", None, 0);
    r.intake("t2", None, 5);
    r.intake("t3", None, 0);
    r.intake("t4", None, 5);
    assert_eq!(r.queue("research"), vec!["t2", "t4", "t1", "t3"], "higher priority first; equal priority first come, first served");
    r.orch(Command::Reorder { station: "research".into(), token: "t3".into(), position: 0 }).unwrap();
    assert_eq!(r.queue("research"), vec!["t3", "t2", "t4", "t1"]);
    r.orch(Command::Prioritize { token: "t1".into(), priority: 9 }).unwrap();
    assert_eq!(r.queue("research"), vec!["t1", "t3", "t2", "t4"]);
    assert_eq!(r.orch(Command::Reorder { station: "outline".into(), token: "t1".into(), position: 0 }), Err(Refusal::Stale),
        "a command names where it believes the token is");
    r.orch(Command::ResumeStation { station: "research".into() }).unwrap();
    assert_eq!(working(&r, "research"), vec!["t1", "t2", "t3"], "the head of the queue starts, up to the pool");
    assert_eq!(r.queue("research"), vec!["t4"]);
    // The priority follows the token to every later station.
    r.answer(&tok("t1"), "passed");
    assert_eq!(r.p.tokens["t1"].priority, 9);
    r.assert_replays();
}

#[test]
fn capacity_binds_per_station_per_pool_and_per_host() {
    let mut r = Run::new(&[("agents", 2), ("work", 1)], content());
    for k in ["t1", "t2", "t3"] { r.intake(k, None, 0); }
    assert_eq!(working(&r, "research"), vec!["t1", "t2"], "the agents pool holds two");
    assert_eq!(r.queue("research"), vec!["t3"]);
    r.orch(Command::SetCapacity { scope: Scope::Pool("agents".into()), n: 10, reason: "more agents".into() }).unwrap();
    assert_eq!(working(&r, "research"), vec!["t1", "t2", "t3"]);
    r.orch(Command::SetCapacity { scope: Scope::Station("research".into()), n: 1, reason: "one at a time".into() }).unwrap();
    assert_eq!(working(&r, "research").len(), 3, "lowering a capacity stops nothing running");
    r.intake("t4", None, 0);
    r.answer(&tok("t1"), "passed");
    r.answer(&tok("t2"), "passed");
    assert_eq!(r.queue("research"), vec!["t4"], "the station's own capacity binds while the pool has room");
    r.answer(&tok("t3"), "passed");
    assert_eq!(working(&r, "research"), vec!["t4"]);
    r.assert_replays();

    // A host limit binds on top of the workspace's pool: the tighter limit wins.
    let mut h = Run::new(&[("agents", 4), ("work", 1)], content());
    h.host.insert("agents".into(), HostPool { limit: 2, busy: 1 });
    for k in ["t1", "t2", "t3"] { h.intake(k, None, 0); }
    assert_eq!(working(&h, "research"), vec!["t1"], "one host place is taken by another workspace");
    h.host.clear();
    h.advance();
    assert_eq!(working(&h, "research").len(), 3);
    h.assert_replays();
}

#[test]
fn a_policy_limit_holds_and_never_closes_and_an_override_is_recorded() {
    let mut r = Run::new(&[("agents", 3), ("work", 1)], content());
    r.intake("t1", None, 0);
    to_fact_check(&mut r, &["t1"]);
    for _ in 0..2 {
        r.answer(&tok("t1"), "rejected");
        assert_eq!(r.station("t1").as_deref(), Some("write"));
        r.answer(&tok("t1"), "passed");
    }
    r.answer(&tok("t1"), "rejected");
    match r.at("t1") {
        Location::Held { station, hold } => {
            assert_eq!(station, "fact-check");
            assert_eq!(hold.kind, HoldKind::PolicyLimit { policy: "fact-check.rework".into() });
            assert_eq!(hold.then, Some(Target::Station("write".into())));
        }
        other => panic!("a third rejection holds by rework: 2, got {other:?}"),
    }
    assert!(!r.p.archive.contains_key("t1"), "a policy never closes a token");
    assert!(r.p.pools["agents"].held_by.is_empty(), "a held token gives up its place");
    // The orchestrator sends it on; the visit records which policy it overrode.
    r.orch(Command::Release { subject: tok("t1"), station: "fact-check".into(), to: None, reason: Some("one more pass".into()) }).unwrap();
    assert_eq!(r.station("t1").as_deref(), Some("write"));
    let visit = r.p.tokens["t1"].visits.iter().rev().find(|v| v.station == "fact-check").unwrap();
    assert_eq!(visit.overrode.as_deref(), Some("fact-check.rework"));
    assert_eq!(visit.reason.as_deref(), Some("one more pass"));
    // Counts are history, not a budget: the next rejection holds again, and a Send overrides it too.
    r.answer(&tok("t1"), "passed");
    r.answer(&tok("t1"), "rejected");
    assert!(matches!(r.at("t1"), Location::Held { hold: Hold { kind: HoldKind::PolicyLimit { .. }, .. }, .. }));
    r.orch(Command::Send { token: "t1".into(), station: "fact-check".into(), to: "research".into(), reason: "start over".into() }).unwrap();
    assert_eq!(r.station("t1").as_deref(), Some("research"));
    let overrides = r.p.tokens["t1"].visits.iter().filter(|v| v.overrode.is_some()).count();
    assert_eq!(overrides, 2);

    // `attempts`: a step that keeps failing holds StepFailed; it is never closed.
    let t1 = tok("t1");
    r.observe(&t1, Outcome::Failed { reason: Some("source unreachable".into()) }).unwrap();
    assert_eq!(r.work(&t1).attempt, 2, "the first failure re-runs");
    r.observe(&t1, Outcome::Failed { reason: Some("source unreachable".into()) }).unwrap();
    assert!(matches!(r.at("t1"), Location::Held { hold: Hold { kind: HoldKind::StepFailed, .. }, .. }));
    assert_eq!(r.p.tokens["t1"].failure.as_deref(), Some("source unreachable"));
    r.assert_replays();
}

#[test]
fn hold_stops_a_running_step_safely_and_release_runs_it_again() {
    let mut r = Run::new(&[("agents", 3), ("work", 1)], content());
    r.intake("t1", None, 0);
    let t1 = tok("t1");
    let original = r.work(&t1).pending.unwrap();
    let late_fence = r.fence(&t1);
    let effects = r.orch(Command::Hold { subject: t1.clone(), station: "research".into(), reason: "wrong topic".into() }).unwrap();
    assert!(matches!(&effects[0].kind, EffectKind::Stop { operation } if *operation == original));
    assert!(matches!(r.at("t1"), Location::Stopping { .. }));
    assert!(r.p.pools["agents"].held_by.contains("t1"), "a stopping step keeps its place");
    // The original's late answer is fenced out.
    let evidence = r.evidence(&t1);
    let late = r.send(Grant::Operation { effect: original.clone(), fence: late_fence }, Command::Observe { subject: t1.clone(),
        effect: original, fence: late_fence, outcome: Outcome::Labeled { label: "passed".into(), evidence, data: None } });
    assert_eq!(late, Err(Refusal::Stale));
    assert_eq!(r.orch(Command::Send { token: "t1".into(), station: "research".into(), to: "outline".into(), reason: "x".into() }),
        Err(Refusal::Stopping));
    r.stopped(&t1);
    assert!(matches!(r.at("t1"), Location::Held { hold: Hold { kind: HoldKind::Asked, .. }, .. }));
    assert!(!r.p.pools["agents"].held_by.contains("t1"));
    r.orch(Command::Release { subject: t1.clone(), station: "research".into(), to: None, reason: None }).unwrap();
    assert!(matches!(r.at("t1"), Location::Working { .. }));
    r.assert_replays();
}

#[test]
fn send_back_any_number_of_times_and_skip_any_station_with_a_reason() {
    let mut r = Run::new(&[("agents", 3), ("work", 1)], content());
    r.intake("t1", None, 0);
    to_fact_check(&mut r, &["t1"]);
    let t1 = tok("t1");
    // No cap on send-backs: back to research, then run research again, five times over.
    r.orch(Command::Send { token: "t1".into(), station: "fact-check".into(), to: "research".into(), reason: "thin sources".into() }).unwrap();
    r.stopped(&t1);
    for i in 0..5 {
        r.orch(Command::Send { token: "t1".into(), station: "research".into(), to: "research".into(), reason: format!("again {i}") }).unwrap();
        r.stopped(&t1);
        assert!(matches!(r.at("t1"), Location::Working { station, .. } if station == "research"));
    }
    assert_eq!(r.p.tokens["t1"].brief.as_deref(), Some("again 4"), "the reason becomes the brief");
    assert_eq!(r.effect_for(&t1).brief.as_deref(), Some("again 4"));
    // Skip outline, which hasn't started: it leaves as `skipped` along `passed`, with the reason in its visit.
    r.orch(Command::HoldStation { station: "outline".into(), reason: "pause".into() }).unwrap();
    r.answer(&t1, "passed");
    r.orch(Command::Skip { subject: t1.clone(), station: "outline".into(), reason: "the research is the outline".into() }).unwrap();
    assert!(matches!(r.at("t1"), Location::Awaiting { station, .. } if station == "pick"));
    let visit = r.p.tokens["t1"].visits.iter().rev().find(|v| v.station == "outline").unwrap();
    assert_eq!((visit.ended.as_deref(), visit.reason.as_deref()), (Some("skipped"), Some("the research is the outline")));
    // Skip a running step: it stops first, then leaves as skipped.
    r.orch(Command::Skip { subject: t1.clone(), station: "pick".into(), reason: "obvious pick".into() }).unwrap();
    assert_eq!(r.station("t1").as_deref(), Some("write"), "skipping a decision goes where approved goes");
    r.orch(Command::Skip { subject: t1.clone(), station: "write".into(), reason: "already written".into() }).unwrap();
    r.stopped(&t1);
    assert_eq!(r.station("t1").as_deref(), Some("fact-check"));
    r.assert_replays();
}

#[test]
fn a_station_may_reserve_skipping_to_a_person() {
    let mut layout = content();
    layout["stations"][2]["policy"] = serde_json::json!({ "skip": "Person" });
    let mut r = Run::new(&[("agents", 3), ("work", 1)], layout);
    r.intake("t1", None, 0);
    let t1 = tok("t1");
    r.answer(&t1, "passed");
    r.answer(&t1, "passed");
    let skip = Command::Skip { subject: t1.clone(), station: "pick".into(), reason: "waive".into() };
    assert_eq!(r.orch(skip.clone()), Err(Refusal::PersonOnly));
    r.person("owner", skip).unwrap();
    assert_eq!(r.station("t1").as_deref(), Some("write"));
}

#[test]
fn pull_from_a_working_batch_holds_the_token_and_the_batch_runs_again_without_it() {
    let mut r = Run::new(&[("agents", 3), ("work", 1)], content());
    for k in ["a1", "a2"] { r.intake(k, None, 0); }
    to_fact_check(&mut r, &["a1", "a2"]);
    for k in ["a1", "a2"] { r.answer(&tok(k), "passed"); }
    r.now = (r.now / (7 * DAY) + 1) * 7 * DAY;
    r.advance();
    let batch = Subject::Batch(r.batch_of("a1"));
    assert_eq!(r.location(&batch).station(), Some("issue.assemble"));
    let effects = r.orch(Command::Pull { token: "a1".into(), reason: "needs another source".into() }).unwrap();
    assert!(matches!(effects[0].kind, EffectKind::Stop { .. }), "a working batch stops");
    assert!(matches!(r.at("a1"), Location::Held { station, .. } if station == "issue"));
    r.stopped(&batch);
    assert_eq!(r.effect_for(&batch).members, vec!["a2"], "the batch re-runs its station without the pulled member");
    // Released, a1 waits at the batch station for the next issue.
    r.orch(Command::Release { subject: tok("a1"), station: "issue".into(), to: None, reason: None }).unwrap();
    assert_eq!(r.queue("issue"), vec!["a1"]);
    // Duplicate and stale inputs change nothing.
    let last = r.log.last().unwrap().clone();
    assert_eq!(step(&r.p, &last, &r.adapters).map(|_| ()), Err(Refusal::Duplicate));
    r.assert_replays();
}

#[test]
fn crossed_work_is_never_sent_back_before_a_repeat_once_boundary() {
    let mut r = Run::new(&[("agents", 3), ("work", 1)], content());
    r.intake("a1", None, 0);
    to_fact_check(&mut r, &["a1"]);
    r.answer(&tok("a1"), "passed");
    r.now = (r.now / (7 * DAY) + 1) * 7 * DAY;
    r.advance();
    let batch = Subject::Batch(r.batch_of("a1"));
    r.answer(&batch, "passed");
    r.answer(&batch, "passed");
    r.orch(Command::Decide { subject: batch.clone(), station: "issue.editor".into(), verdict: "approved".into(), suspects: vec![], reason: None }).unwrap();
    r.answer(&batch, "passed");
    r.answer(&batch, "measured");
    assert_eq!(r.station("a1").as_deref(), Some("archive"));
    for to in ["write", "research", "fact-check"] {
        assert_eq!(r.orch(Command::Send { token: "a1".into(), station: "archive".into(), to: to.into(), reason: "fix".into() }),
            Err(Refusal::Crossed), "{to} leads back to cms:issue");
    }
    // A reopen starts it afresh: its crossings stay as history but no longer bind.
    r.orch(Command::Cancel { token: "a1".into(), reason: "superseded".into() }).unwrap();
    r.stopped(&tok("a1"));
    assert_eq!(r.closed("a1").crossings.len(), 1, "a cancel after a crossing keeps it");
    r.orch(Command::Reopen { token: "a1".into(), station: "write".into() }).unwrap();
    assert_eq!(r.station("a1").as_deref(), Some("write"));
    assert_eq!(r.p.tokens["a1"].crossings.len(), 1);
    r.assert_replays();
}
