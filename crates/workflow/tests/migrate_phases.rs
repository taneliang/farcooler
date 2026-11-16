//! `migrate` for every `Phase` (docs/engine-pipeline.md 11.2), every ticket and group stage of the four templates,
//! and every dispatch status (10.1). The sweep cuts every differential scenario at every point and checks each
//! live item's place; the targeted tests make the phases and statuses the scenarios never stop at.
#[path = "differential_support/mod.rs"]
mod differential;
use std::collections::{BTreeMap, BTreeSet};
use differential::*;
use farcooler_workflow as old;
use farcooler_workflow::adapters::coding::data_of;
use farcooler_workflow::adapters::{coding_templates, linked};
use farcooler_workflow::migrate::{migrate, Dispatch, DispatchState, Migrated, Quarantine, Setup};
use farcooler_workflow::pipeline::{self as new, Closure, HoldKind, Location};
use farcooler_workflow::templates::{build, PathRoutes};

fn setup(s: &Scenario) -> Setup {
    Setup { layout: coding_templates::layout(&s.plan, &config(s)).unwrap(), pools: POOLS.iter().map(|(k, n)| (k.to_string(), *n)).collect() }
}
fn rows(o: &Old, state: DispatchState) -> Vec<Dispatch> {
    o.state.items.values().filter_map(|i| i.pending.clone()).map(|effect| Dispatch { effect, state }).collect()
}
fn admitted(o: &Old) -> Migrated { migrate(&o.state, &o.log, &rows(o, DispatchState::Admitted), &setup_of(o)).unwrap() }
/// The setup for an old run, from the scenario it was started with (kept on the side by `start`).
fn setup_of(o: &Old) -> Setup { SETUP.with(|s| s.borrow().get(&o.tickets.join(",")).cloned().expect("started with `start`")) }
thread_local! { static SETUP: std::cell::RefCell<BTreeMap<String, Setup>> = Default::default(); }
fn start(s: &Scenario) -> Old {
    let o = old_start(s);
    SETUP.with(|m| m.borrow_mut().insert(o.tickets.join(","), setup(s)));
    o
}
/// Run the old reducer until `stop` holds.
fn until(mut o: Old, stop: impl Fn(&Old) -> bool) -> Old {
    while !stop(&o) { assert!(o.act(), "the old run settled first"); }
    o
}
fn stage(o: &Old, id: &str) -> (String, old::Phase) {
    let i = &o.state.items[id];
    (i.definition.stages.get(i.stage).map(|s| s.id.clone()).unwrap_or_default(), i.phase.clone())
}
fn group_of(o: &Old) -> Option<String> { o.state.items.iter().find(|(_, i)| i.group_of.is_some()).map(|(k, _)| k.clone()) }
fn find(name: &str) -> Scenario { scenarios().into_iter().find(|s| s.name.starts_with(name)).unwrap() }

/// Where each live item of `o` must be once migrated with every effect admitted.
fn check(o: &Old, m: &Migrated, seen: &mut BTreeSet<(String, String, String)>) {
    let p = &m.pipeline;
    for (id, item) in &o.state.items {
        let step = item.definition.stages.get(item.stage).map(|s| s.id.clone()).unwrap_or_else(|| "end".into());
        let source = item.definition.source.clone();
        seen.insert((source, step.clone(), format!("{:?}", item.phase)));
        let here = |at: &Location| at.station().map(str::to_string);
        if item.group_of.is_some() {
            if item.members.is_empty() { continue; }
            match p.batches.get(id) {
                Some(b) => {
                    let inner = format!("{}.{step}", b.station);
                    assert_eq!(here(&b.at).as_deref(), Some(inner.as_str()), "{id}");
                    if item.phase == old::Phase::Running && item.ci_retry.as_ref().is_some_and(|r| r.pending) {
                        assert!(matches!(&b.at, Location::Held { hold, .. } if hold.kind == HoldKind::Quarantined), "{id}: a rerun request in flight");
                        assert!(m.quarantined.iter().any(|q| q.item == *id));
                    } else if item.phase == old::Phase::Running {
                        let Location::Working { work, .. } = &b.at else { panic!("{id} is running: {:?}", b.at) };
                        assert_eq!(work.pending, item.pending, "{id} keeps its effect");
                        assert_eq!(b.fence, item.fence);
                    }
                    let members: Vec<&str> = b.members.iter().map(|x| x.token.as_str()).collect();
                    assert_eq!(members, item.members.iter().map(|x| x.item.as_str()).collect::<Vec<_>>());
                    for x in &item.members {
                        assert_eq!(data_of(&b.members.iter().find(|y| y.token == x.item).unwrap().data).unwrap().candidate.as_deref(), Some(x.candidate.as_str()));
                        assert_eq!(p.tokens[&x.item].at, Location::InBatch { batch: id.clone() });
                    }
                    assert_eq!(data_of(&b.data).unwrap().candidate, item.candidate);
                    if item.pushed.is_some() { assert_eq!(b.crossings.len(), 1, "{id} crossed the target"); }
                }
                None => {
                    // A group that hadn't left: its tickets are queued at the batch station, in its order.
                    assert!(item.phase == old::Phase::Blocked && item.stage == 0, "{id} {:?}", item.phase);
                    for x in &item.members { assert!(matches!(&p.tokens[&x.item].at, Location::Queued { station } if p.stations[station].kind == new::Kind::Batch)); }
                }
            }
            continue;
        }
        let t = p.tokens.get(id);
        match item.phase {
            old::Phase::Done => assert_eq!(p.archive[id].closed, Closure::Done),
            old::Phase::Cancelled => assert_eq!(p.archive[id].closed, Closure::Cancelled),
            old::Phase::Running => {
                let Some(Location::Working { station, work }) = t.map(|t| &t.at) else { panic!("{id} is running: {t:?}") };
                assert_eq!((station, &work.pending, work.attempt), (&step, &item.pending, item.attempt), "{id}");
                assert_eq!(t.unwrap().fence, item.fence);
            }
            old::Phase::Cancelling => assert!(matches!(t.map(|t| &t.at), Some(Location::Stopping { station, work, .. }) if *station == step
                && work.pending == item.pending && work.stopping == item.stopping), "{id}: {t:?}"),
            old::Phase::WaitingReview | old::Phase::WaitingApproval =>
                assert!(matches!(t.map(|t| &t.at), Some(Location::Awaiting { station, decision }) if *station == step && decision.run == item.run)),
            old::Phase::WaitingJoin | old::Phase::Blocked => assert_eq!(t.map(|t| t.at.clone()), Some(Location::Queued { station: step.clone() }), "{id}"),
            old::Phase::Joined => assert!(matches!(t.map(|t| &t.at), Some(Location::InBatch { .. } | Location::Queued { .. }))),
            old::Phase::Failed => assert!(matches!(t.map(|t| &t.at), Some(Location::Held { station, hold }) if *station == step && hold.kind == HoldKind::StepFailed)),
            ref other => panic!("{id}: {other:?} at a cut"),
        }
        if let Some(t) = t {
            let d = data_of(&t.data).unwrap();
            if item.phase != old::Phase::Joined { assert_eq!(d.candidate, item.candidate, "{id}"); }
            assert_eq!(t.evidence.as_ref().map(|e| e.artifact.clone()), item.last_evidence.as_ref().map(|e| e.candidate.clone()));
            assert_eq!(t.dependencies, item.dependencies);
        }
    }
    // Places: running work at an agent or command station holds a place there and in its pool.
    for (k, t) in &p.tokens {
        if let Location::Working { station, .. } = &t.at {
            if p.stations[station].kind != new::Kind::Wait { assert!(p.stations[station].working.contains(k), "{k} holds a place at {station}"); }
        }
    }
}

#[test]
fn every_ticket_and_group_stage_of_every_template_migrates_in_place() {
    let mut seen = BTreeSet::new();
    let mut expected = BTreeSet::new();
    for s in scenarios() {
        let definition = build(&s.plan, &Fixture).unwrap();
        expected.extend(definition.stages.iter().map(|x| (definition.source.clone(), x.id.clone())));
        for (name, g) in &definition.groups { expected.extend(g.steps.iter().map(|x| (format!("group/{name}"), x.id.clone()))); }
        let mut o = start(&s);
        loop {
            let m = admitted(&o);
            check(&o, &m, &mut seen);
            if !o.act() { break; }
        }
    }
    let stages: BTreeSet<(String, String)> = seen.iter().map(|(a, b, _)| (a.clone(), b.clone())).collect();
    let missing: Vec<_> = expected.difference(&stages).collect();
    assert!(missing.is_empty(), "stages never migrated: {missing:?}");
    let phases: BTreeSet<&str> = seen.iter().map(|(_, _, p)| p.as_str()).collect();
    for p in ["Running", "WaitingReview", "WaitingJoin", "Joined", "Blocked", "Failed", "Cancelling", "Done", "Cancelled"] {
        assert!(phases.contains(p), "{p} is never migrated in the sweep: {phases:?}");
    }
}

#[test]
fn a_group_mid_gate_keeps_its_effect_its_members_and_the_landing() {
    let s = find("tickets queued while a group lands");
    let o = until(start(&s), |o| group_of(o).is_some_and(|g| stage(o, &g) == ("gate-0".into(), old::Phase::Running)));
    let g = group_of(&o).unwrap();
    let m = admitted(&o);
    let b = &m.pipeline.batches[&g];
    assert_eq!(b.station, "join");
    assert!(matches!(&b.at, Location::Working { station, .. } if station == "join.gate-0"));
    assert!(m.pipeline.pools["landing"].held_by.contains(&g), "the batch holds the landing");
    assert!(m.pipeline.stations["join"].working.contains(&g));
    assert!(m.pipeline.stations["join.gate-0"].working.contains(&g));
    // The batch passed no gate yet, and its base is unknown past the fold: neither is invented.
    let d = data_of(&b.data).unwrap();
    assert_eq!((d.gated, d.base), (None, None));
    // Its answer binds: the gate passes and the batch goes on toward the push, with what it gated.
    let mut r = support::Run::resume(m.pipeline, linked());
    let evidence = new::Evidence { artifact: d.candidate.clone().unwrap(), manifest: "m".into(), receipt: "r".into(), spec: "s".into() };
    let effect = o.state.items[&g].pending.clone().unwrap();
    r.observe(&new::Subject::Batch(g.clone()), new::Outcome::Labeled { label: "passed".into(), evidence, data: None }).unwrap();
    // With CI, it waits on CI for the candidate before the push (ov-1269); without, it pushes.
    let next = if s.plan.ci { "join.ci" } else { "join.push" };
    assert!(matches!(&r.p.batches[&g].at, Location::Working { station, .. } if station == next));
    assert_eq!(data_of(&r.p.batches[&g].data).unwrap().gated, d.candidate, "a push sends what the gate passed");
    let after = r.effects.last().unwrap();
    assert_ne!(after.id, effect);
    r.assert_replays();
}

#[test]
fn a_waiting_group_dissolves_into_the_queue_in_its_order() {
    let s = find("tickets queued while a group lands");
    let o = until(start(&s), |o| o.state.items.values().any(|i| i.group_of.is_some() && i.phase == old::Phase::Blocked && i.members.len() >= 2));
    let waiting = o.state.items.iter().find(|(_, i)| i.group_of.is_some() && i.phase == old::Phase::Blocked).unwrap();
    let m = admitted(&o);
    assert!(!m.pipeline.batches.contains_key(waiting.0), "a group that hadn't left is no batch");
    let order: Vec<String> = waiting.1.members.iter().map(|x| x.item.clone()).collect();
    assert_eq!(m.pipeline.stations["join"].queue[..order.len()], order[..]);
}

#[test]
fn dispatch_status_splits_running_work() {
    let s = find("tickets queued while a group lands");
    let o = until(start(&s), |o| stage(o, "a").0 == "build" && stage(o, "b").0 == "provision");
    let effect = |k: &str| o.state.items[k].pending.clone().unwrap();
    let outbox = vec![
        Dispatch { effect: effect("c"), state: DispatchState::Pending },
        Dispatch { effect: effect("b"), state: DispatchState::Claimed },
        Dispatch { effect: effect("a"), state: DispatchState::Admitted },
    ];
    let m = migrate(&o.state, &o.log, &outbox, &setup_of(&o)).unwrap();
    let p = &m.pipeline;
    // Never started: queued in outbox order, with the outbox rows listed for suppression and the fence kept
    // (a start bumps it, so the old effect can never answer).
    assert_eq!(p.stations["provision"].queue, vec!["c", "b"]);
    assert_eq!(m.suppress, vec![effect("c"), effect("b")]);
    assert_eq!(p.tokens["c"].fence, o.state.items["c"].fence);
    assert!(matches!(&p.tokens["a"].at, Location::Working { work, .. } if work.pending == Some(effect("a"))));
    assert!(p.pools["agents"].held_by.contains("a") && !p.pools["work"].held_by.contains("c"));
    // Admitted work answers into the pipeline; a never-started effect's answer is refused.
    let mut r = support::Run::resume(m.pipeline.clone(), linked());
    let stale = new::Command::Observe { subject: new::Subject::Token("c".into()), effect: effect("c"), fence: o.state.items["c"].fence,
        outcome: new::Outcome::Labeled { label: "passed".into(), evidence: ev("x"), data: None } };
    assert_eq!(r.send(new::Grant::Operation { effect: effect("c"), fence: o.state.items["c"].fence }, stale), Err(new::Refusal::Stale));
    let started = r.send(new::Grant::Runtime, new::Command::Advance).unwrap();
    assert_eq!(started.iter().map(|e| e.subject.key()).collect::<Vec<_>>(), vec!["c", "b"], "the first tick starts them in outbox order");
    assert!(started.iter().all(|e| e.fence == o.state.items[e.subject.key()].fence + 1));

    // Uncertain: still working, and still counted; its late answer binds by the effect alone.
    let outbox = vec![Dispatch { effect: effect("a"), state: DispatchState::Uncertain }];
    let m = migrate(&o.state, &o.log, &outbox, &setup_of(&o)).unwrap();
    assert!(matches!(&m.pipeline.tokens["a"].at, Location::Working { work, .. } if work.uncertain && work.pending.is_none()));
    assert!(m.pipeline.stations["build"].working.contains("a"));
    // Settled, suppressed or missing: held Quarantined at its station, and listed.
    for state in [Some(DispatchState::Settled), Some(DispatchState::Suppressed), None] {
        let outbox: Vec<Dispatch> = state.map(|state| Dispatch { effect: effect("a"), state }).into_iter().collect();
        let m = migrate(&o.state, &o.log, &outbox, &setup_of(&o)).unwrap();
        assert!(matches!(&m.pipeline.tokens["a"].at, Location::Held { station, hold } if station == "build" && hold.kind == HoldKind::Quarantined));
        assert!(m.quarantined.iter().any(|q| q.item == "a"), "{state:?}");
        assert!(m.quarantined.iter().any(|q| q.item == "b"), "b has no row either");
    }
}
fn ev(artifact: &str) -> new::Evidence { new::Evidence { artifact: artifact.into(), manifest: "m".into(), receipt: "r".into(), spec: "s".into() } }

#[test]
fn an_uncertain_item_waits_for_its_late_answer() {
    let s = find("tickets queued while a group lands");
    let mut o = until(start(&s), |o| stage(o, "a") == ("build".into(), old::Phase::Running));
    let i = o.state.items["a"].clone();
    let (effect, run, fence) = (i.pending.clone().unwrap(), i.run.clone(), i.fence);
    o.send(old::Command::Observe { item: "a".into(), effect: effect.clone(), run: run.clone(), fence, outcome: old::Outcome::Uncertain },
        old::Grant::Operation { effect: effect.clone(), run, fence });
    assert_eq!(o.state.items["a"].phase, old::Phase::Uncertain);
    let m = admitted(&o);
    assert!(m.quarantined.is_empty());
    let mut r = support::Run::resume(m.pipeline, linked());
    // Interventions wait for it, as on any uncertain step.
    assert!(matches!(r.at("a"), Location::Working { work, .. } if work.uncertain && work.run == effect));
    // The late answer binds by the effect alone, and the build's candidate is taken.
    r.send(new::Grant::Operation { effect: effect.clone(), fence: 0 }, new::Command::Observe { subject: new::Subject::Token("a".into()),
        effect, fence: 0, outcome: new::Outcome::Labeled { label: "passed".into(), evidence: ev("a-built"), data: None } }).unwrap();
    assert_eq!(data_of(&r.p.tokens["a"].data).unwrap().candidate.as_deref(), Some("a-built"));
    r.assert_replays();
}

#[test]
fn a_cancelling_item_stops_then_closes() {
    let s = find("cancelling a run while it builds");
    let o = until(start(&s), |o| stage(o, "a").1 == old::Phase::Cancelling);
    let i = &o.state.items["a"];
    let m = admitted(&o);
    let mut r = support::Run::resume(m.pipeline, linked());
    let Location::Stopping { work, then, .. } = r.at("a") else { panic!("{:?}", r.at("a")) };
    assert!(matches!(*then, new::Then::Close { .. }));
    assert_eq!((work.pending.as_ref(), work.stopping.as_ref()), (i.pending.as_ref(), i.stopping.as_ref()));
    r.stopped(&new::Subject::Token("a".into()));
    assert_eq!(r.closed("a").closed, Closure::Cancelled);
    assert!(!r.p.pools["agents"].held_by.contains("a"), "the stop gave its place back");
    r.assert_replays();
}

#[test]
fn a_cancelling_merge_settles_and_a_merged_ticket_has_crossed() {
    let s = find("per-ticket pull request merges and CI passes");
    // A ticket past `merged` crossed the target: it can't be sent back across it.
    let o = until(start(&s), |o| stage(o, "a").0 == "ci");
    let m = admitted(&o);
    assert_eq!(m.pipeline.tokens["a"].crossings.len(), 1);
    assert_eq!(m.pipeline.tokens["a"].crossings[0].station, "merged");
    let mut r = support::Run::resume(m.pipeline, linked());
    assert_eq!(r.orch(new::Command::Send { token: "a".into(), station: "ci".into(), to: "build".into(), reason: "again".into() }),
        Err(new::Refusal::Crossed));
    // Cancelled while waiting for the merge: the stop is outward, so it settles first.
    let mut o = until(start(&s), |o| stage(o, "a") == ("merged".into(), old::Phase::Running));
    o.send(old::Command::Cancel { item: "a".into(), reason: None }, old::Grant::Controller);
    let m = admitted(&o);
    assert!(matches!(m.pipeline.tokens["a"].at, Location::Settling { .. }));
}

#[test]
fn failed_blocked_and_resting_items() {
    // Failed: held where it failed, with its reason, for the orchestrator to release.
    let s = find("a build that fails twice stops");
    let o = until(start(&s), |o| stage(o, "a").1 == old::Phase::Failed);
    let mut r = support::Run::resume(admitted(&o).pipeline, linked());
    assert!(matches!(r.at("a"), Location::Held { station, hold } if station == "build" && hold.kind == HoldKind::StepFailed));
    r.orch(new::Command::Release { subject: new::Subject::Token("a".into()), station: "build".into(), to: None, reason: None }).unwrap();
    assert!(matches!(r.at("a"), Location::Working { station, .. } if station == "build"));

    // Blocked on a dependency: queued at its step, and it waits for the dependency there too.
    let s = find("tickets queued while a group lands");
    let mut o = start(&s);
    let definition = o.state.items["a"].definition.clone();
    o.send(old::Command::Enqueue { item: "d".into(), definition, policy_pin: "policy".into(), resource: "res-d".into(),
        dependencies: BTreeSet::from(["a".to_string()]), initial_candidate: None }, old::Grant::Controller);
    assert_eq!(stage(&o, "d"), ("provision".into(), old::Phase::Blocked));
    let m = admitted(&o);
    assert_eq!(m.pipeline.tokens["d"].at, Location::Queued { station: "provision".into() });
    let mut r = support::Run::resume(m.pipeline, linked());
    let started = r.send(new::Grant::Runtime, new::Command::Advance).unwrap();
    assert!(started.iter().all(|e| e.subject.key() != "d"), "d waits for a");

    // A ticket at rest, in its column; done and cancelled ones not yet archived are archived now.
    let mut o = start(&s);
    for (k, column) in [("t1", "todo"), ("t2", "review"), ("t3", "backlog"), ("t4", "backlog")] {
        o.send(old::Command::Enqueue { item: k.into(), definition: old::Definition::ticket_at(column).unwrap(), policy_pin: "policy".into(),
            resource: format!("res-{k}"), dependencies: BTreeSet::new(), initial_candidate: Some("seed".into()) }, old::Grant::Controller);
    }
    o.send(old::Command::Move { item: "t3".into(), to: "done".into() }, old::Grant::Controller);
    o.send(old::Command::Cancel { item: "t4".into(), reason: Some("not needed".into()) }, old::Grant::Controller);
    assert_eq!(stage(&o, "t1").1, old::Phase::WaitingApproval);
    let m = admitted(&o);
    assert_eq!(m.pipeline.tokens["t1"].at, Location::Resting { column: "todo".into() });
    assert_eq!(m.pipeline.tokens["t2"].at, Location::Resting { column: "review".into() });
    assert_eq!(m.pipeline.archive["t3"].closed, Closure::Done);
    assert_eq!((m.pipeline.archive["t4"].closed.clone(), m.pipeline.archive["t4"].reason.as_deref()), (Closure::Cancelled, Some("not needed")));
    let mut r = support::Run::resume(m.pipeline, linked());
    r.orch(new::Command::Start { token: "t1".into(), station: "provision".into() }).unwrap();
}

#[test]
fn archived_items_keep_their_keys_and_a_landed_group_its_crossing() {
    let s = find("tickets queued while a group lands");
    let mut o = start(&s);
    while o.act() {}
    o.send(old::Command::Advance, old::Grant::Controller);
    assert!(o.state.items.is_empty() && !o.state.archive.is_empty());
    let m = admitted(&o);
    assert_eq!(m.pipeline.archive.keys().collect::<Vec<_>>(), o.state.archive.keys().collect::<Vec<_>>());
    for (k, a) in &o.state.archive {
        let n = &m.pipeline.archive[k];
        assert_eq!(n.closed, if a.phase == old::Phase::Done { Closure::Done } else { Closure::Cancelled });
        assert_eq!(data_of(&n.data).unwrap().candidate, a.candidate);
        if let Some(g) = &a.group {
            let b = n.batch.as_ref().unwrap();
            assert_eq!((&b.members, b.since_ms), (&g.tickets, g.since_ms));
            assert_eq!(n.crossings.iter().map(|c| c.artifact.clone()).collect::<Vec<_>>(), g.pushed.iter().cloned().collect::<Vec<_>>());
        }
    }
    // A done ticket stays done for a dependency; a closed name stays taken.
    let mut r = support::Run::resume(m.pipeline, linked());
    assert_eq!(r.send(new::Grant::Runtime, new::Command::Intake { token: "a".into(), to: None, priority: 0, data: BTreeMap::new(),
        dependencies: BTreeSet::new(), models: BTreeMap::new(), brief: None }), Err(new::Refusal::Duplicate));
}

#[test]
fn back_edges_become_visits_the_policies_count() {
    // A review that rejected twice (its limit): migrated, the third rejection holds instead of sending back.
    let s = find("a review rejects three times");
    let o = until(start(&s), |o| o.state.items["a"].back_edges.get("gate-0") == Some(&2) && stage(o, "a").0 == "gate-0");
    let m = admitted(&o);
    let back = m.pipeline.tokens["a"].visits.iter().filter(|v| v.station == "gate-0" && v.ended.as_deref() == Some("rejected")).count();
    assert_eq!(back, 2);
    let mut r = support::Run::resume(m.pipeline, linked());
    let c = data_of(&r.p.tokens["a"].data).unwrap().candidate.unwrap();
    r.observe(&new::Subject::Token("a".into()), new::Outcome::Labeled { label: "rejected".into(), evidence: ev(&c), data: None }).unwrap();
    assert!(matches!(r.at("a"), Location::Held { hold, .. } if hold.kind == HoldKind::PolicyLimit { policy: "gate-0.rework".into() }));
}

#[test]
fn what_has_no_faithful_place_refuses_naming_the_item() {
    let s = find("tickets queued while a group lands");
    let o = until(start(&s), |o| stage(o, "a").0 == "build");
    let refused = |state: &old::State| -> Vec<Quarantine> { migrate(state, &o.log, &rows(&o, DispatchState::Admitted), &setup_of(&o)).unwrap_err() };
    let with = |f: &dyn Fn(&mut old::Item), reason: &str| {
        let mut state = o.state.clone();
        f(state.items.get_mut("a").unwrap());
        let r = refused(&state);
        assert!(r.iter().any(|q| q.item == "a" && q.reason.contains(reason)), "{reason}: {r:?}");
    };
    with(&|i| i.definition.stages[i.stage].id = "elsewhere".into(), "has no station");
    with(&|i| i.followers = vec!["b".into()], "followers");
    with(&|i| i.follows = Some("b".into()), "followers");
    with(&|i| i.phase = old::Phase::WaitingSlot, "read-time phase");
    with(&|i| i.phase = old::Phase::Starting, "read-time phase");
    with(&|i| i.hook_refused = Some("no".into()), "script");
    with(&|i| i.phase = old::Phase::WaitingApproval, "no place");
    with(&|i| { i.phase = old::Phase::Cancelling; i.closed = Some(old::CloseResolution::Landed { sha: "a".repeat(40) }); i.stopping = Some("x".into()); },
        "closed as landed");
    // The layout itself: one that doesn't publish, or doesn't use coding/v1.
    let mut bad = setup_of(&o);
    bad.layout.stations.clear();
    assert!(migrate(&o.state, &o.log, &[], &bad).unwrap_err()[0].reason.contains("doesn't publish"));
    // An old archive entry that is neither done nor cancelled.
    let mut state = o.state.clone();
    state.archive.insert("x".into(), old::Archived { closed: None, phase: old::Phase::Failed, candidate: None, group: None });
    assert!(refused(&state).iter().any(|q| q.item == "x"));
}

#[test]
fn a_kept_effect_takes_the_new_shape() {
    let s = find("tickets queued while a group lands");
    let o = until(start(&s), |o| group_of(o).is_some_and(|g| stage(o, &g) == ("gate-0".into(), old::Phase::Running)));
    let kept = &o.effects;
    let g = group_of(&o).unwrap();
    let pending = o.state.items[&g].pending.clone().unwrap();
    let old_effect = kept.iter().find(|e| e.id == pending).unwrap();
    let m = admitted(&o);
    let e = farcooler_workflow::migrate::effect(old_effect, &m.pipeline).unwrap();
    assert_eq!((e.id.as_str(), e.subject.clone(), e.station.as_str(), e.fence), (pending.as_str(), new::Subject::Batch(g.clone()), "join.gate-0", old_effect.fence));
    let work: farcooler_workflow::adapters::coding::Work = serde_json::from_value(e.data["coding"].value.clone()).unwrap();
    assert_eq!((work.op.as_str(), work.check, work.candidate.clone()), ("gate", Some(0), old_effect.candidate.clone()));
    assert_eq!(work.members.iter().map(|x| (x.token.clone(), x.candidate.clone())).collect::<Vec<_>>(),
        old_effect.members.iter().map(|x| (x.item.clone(), x.candidate.clone())).collect::<Vec<_>>());
    // An effect its item no longer waits on has no new shape.
    let other = kept.iter().find(|e| e.id != pending && e.item == g).or_else(|| kept.iter().find(|e| e.id != pending)).unwrap();
    assert!(farcooler_workflow::migrate::effect(other, &m.pipeline).is_err());
}

#[test]
fn a_path_route_ticket_awaits_its_approver() {
    let mut s = find("a path rule asks a person");
    s.plan.paths = Some(PathRoutes { argument: "rules".into(), approver: Some("owner".into()) });
    let o = until(start(&s), |o| stage(o, "a").0 == "approve");
    let m = admitted(&o);
    let Location::Awaiting { decision, .. } = &m.pipeline.tokens["a"].at else { panic!() };
    assert_eq!(decision.deciders, vec!["person:owner".to_string()]);
    let mut r = support::Run::resume(m.pipeline, linked());
    r.person("owner", new::Command::Decide { subject: new::Subject::Token("a".into()), station: "approve".into(),
        verdict: "approved".into(), suspects: vec![], reason: None }).unwrap();
    assert!(matches!(r.at("a"), Location::InBatch { .. }), "approved, it goes on to land: {:?}", r.at("a"));
}
