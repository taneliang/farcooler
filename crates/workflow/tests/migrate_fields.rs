//! Every `Item` field has a decision (docs/engine-pipeline.md 11.1): mapped, with a test here or in
//! migrate_phases.rs, dropped with a reason, or refused with a reason. An item with every field set must
//! serialize exactly the table's keys, so a new field fails here until the table names it; `migrate` also
//! destructures `Item` with no `..`, so it fails to build first.
#[path = "differential_support/mod.rs"]
mod differential;
use std::collections::BTreeSet;
use differential::*;
use farcooler_workflow as old;
use farcooler_workflow::adapters::coding::data_of;
use farcooler_workflow::adapters::{coding_templates, linked};
use farcooler_workflow::migrate::{migrate, Decision, Dispatch, DispatchState, Setup, ITEM_FIELDS};
use farcooler_workflow::pipeline::{HoldKind, Location};
use serde_json::{json, Value};

/// Which test shows each mapped field landing where the table says.
const MAPPED_BY: &[(&str, &str)] = &[
    ("policy_pin", "a_running_ticket_carries_its_fields"), ("dependencies", "a_running_ticket_carries_its_fields"),
    ("stage", "migrate_phases::every_ticket_and_group_stage_of_every_template_migrates_in_place"),
    ("attempt", "a_running_ticket_carries_its_fields"), ("fence", "a_running_ticket_carries_its_fields"),
    ("phase", "migrate_phases::every_ticket_and_group_stage_of_every_template_migrates_in_place"),
    ("run", "a_running_ticket_carries_its_fields"), ("pending", "a_running_ticket_carries_its_fields"),
    ("candidate", "a_running_ticket_carries_its_fields"), ("last_evidence", "a_running_ticket_carries_its_fields"),
    ("stopping", "migrate_phases::a_cancelling_item_stops_then_closes"),
    ("back_edges", "migrate_phases::back_edges_become_visits_the_policies_count"),
    ("members", "a_live_group_carries_its_fields"), ("group_of", "a_live_group_carries_its_fields"),
    ("pushed", "a_live_group_carries_its_fields"), ("handed", "a_finished_group_and_closed_tickets_are_archived"),
    ("since_ms", "a_live_group_carries_its_fields"), ("grouped_in", "a_live_group_carries_its_fields"),
    ("wait_since", "a_waiting_step_keeps_its_patience"), ("models", "a_running_ticket_carries_its_fields"),
    ("tree", "a_running_ticket_carries_its_fields"), ("sent_back", "a_running_ticket_carries_its_fields"),
    ("brief", "a_running_ticket_carries_its_fields"), ("cancel_reason", "a_finished_group_and_closed_tickets_are_archived"),
    ("closed", "a_finished_group_and_closed_tickets_are_archived"), ("failure", "a_running_ticket_carries_its_fields"),
    ("ci_reruns", "a_waiting_step_keeps_its_patience"), ("ci_retry", "a_waiting_step_keeps_its_patience"),
    ("group_failure", "a_live_group_carries_its_fields"), ("landed", "a_finished_group_and_closed_tickets_are_archived"),
];

/// An item from a real run, with every optional field set.
fn every_field_set() -> Value {
    let s = scenarios().into_iter().next().unwrap();
    let o = old_start(&s);
    let mut v = serde_json::to_value(&o.state.items["a"]).unwrap();
    let evidence = json!({ "candidate": "c", "manifest": "m", "receipt": "r", "policy_pin": "p" });
    let set = json!({
        "start_load": { "sample_milli": 4100, "below_milli": 6000 },
        "execution": { "phase": "Running", "position": null, "holders": [], "text": "" },
        "rework_history": [{ "id": "i", "principal": "owner", "revision": 1, "run": "r", "definition_pin": "d", "evidence": evidence, "iteration": 0, "reason": "why" }],
        "back_edges": { "build": 1 }, "retries": 1, "refused_launch": 1, "members": [{ "item": "b", "candidate": "c" }],
        "group_of": { "name": "landing", "pin": "p" }, "pushed": "c", "handed": ["b"], "since_ms": 5, "grouped_in": "g", "wait_since": 5,
        "models": { "build": "claude:opus" },
        "moved": ["build"], "resume_after_stop": true,
        "tree": { "name": "n", "branch": "b", "path": "/p", "base": "0123456789012345678901234567890123456789" },
        "sent_back": [{ "id": "i", "revision": 1, "run": "r", "from": "gate-0", "to": "build", "reason": "why" }],
    });
    let more = json!({
        "restart": "restart-input", "reuse_checkout": true, "carried_progress": true, "reason": "gate: failed", "stopped_from": "stopped-run",
        "history": [{ "phase": "Cancelled", "run": "old-run", "pin": "p", "fence": 1, "candidate": null,
            "tree": null, "models": {}, "reason": null, "rework_history": [], "sent_back": [], "back_edges": {} }],
        "brief": "why", "follows": "b", "followers": ["b"], "cancel_reason": "why", "closed": { "Continued": { "item": "b" } },
        "failure": "why", "ci_reruns": { "ci": 1 },
        "ci_retry": { "run": { "id": 1, "attempt": 1, "failed_jobs": [] }, "pending": false, "policy": "p", "revision": 1, "input_digest": "d" },
        "deciding": { "policy": "p", "deadline_ms": 0, "attempt": 1, "observation": null, "input": { "schema": "s", "point": "group.splits",
            "call": "c", "revision": 0, "now_ms": 0, "subject": "a", "fence": 0, "policy_pin": "p", "label": null, "candidate": null,
            "failure": null, "evidence": null, "groups": [], "members": [], "retries": 0, "allowed": [] } },
        "queue_hold": { "reason": "held", "since_ms": 5, "notified": false },
        "hook_calls": { "p": 1 }, "hook_refused": "no", "group_failure": { "job": "j", "firstLine": "l" }, "landed": "c", "landing_ci": { "Green": { "observed_ms": 1 } },
    });
    for (k, x) in set.as_object().unwrap().iter().chain(more.as_object().unwrap()) { v[k] = x.clone(); }
    v
}

#[test]
fn every_item_field_has_a_decision() {
    let v = every_field_set();
    let item: old::Item = serde_json::from_value(v.clone()).expect("every field parses");
    let keys: BTreeSet<String> = serde_json::to_value(&item).unwrap().as_object().unwrap().keys().cloned().collect();
    let table: BTreeSet<String> = ITEM_FIELDS.iter().map(|(k, _)| k.to_string()).collect();
    assert_eq!(table.len(), ITEM_FIELDS.len(), "each field once");
    assert_eq!(keys, table, "every Item field has a decision in migrate::ITEM_FIELDS, and nothing else does");
    for (field, decision) in ITEM_FIELDS {
        let (Decision::Mapped(why) | Decision::Dropped(why) | Decision::Refused(why)) = decision;
        assert!(why.len() > 10, "{field}: say why");
    }
    let mapped: BTreeSet<&str> = ITEM_FIELDS.iter().filter(|(_, d)| matches!(d, Decision::Mapped(_))).map(|(k, _)| *k).collect();
    let tested: BTreeSet<&str> = MAPPED_BY.iter().map(|(k, _)| *k).collect();
    assert_eq!(mapped, tested, "every mapped field names the test that shows it");
    let (here, phases) = (include_str!("migrate_fields.rs"), include_str!("migrate_phases.rs"));
    for (field, test) in MAPPED_BY {
        let (file, name) = test.split_once("::").map_or((here, *test), |(_, n)| (phases, n));
        assert!(file.contains(&format!("fn {name}()")), "{field}: no test {test}");
    }
}

fn setup(s: &Scenario) -> Setup {
    Setup { layout: coding_templates::layout(&s.plan, &config(s)).unwrap(), pools: POOLS.iter().map(|(k, n)| (k.to_string(), *n)).collect() }
}
fn admitted(o: &Old) -> Vec<Dispatch> {
    o.state.items.values().filter_map(|i| i.pending.clone()).map(|effect| Dispatch { effect, state: DispatchState::Admitted }).collect()
}
fn until(mut o: Old, stop: impl Fn(&Old) -> bool) -> Old {
    while !stop(&o) { assert!(o.act(), "the old run settled first"); }
    o
}
fn at(o: &Old, k: &str) -> String { let i = &o.state.items[k]; i.definition.stages.get(i.stage).map(|s| s.id.clone()).unwrap_or_default() }
fn find(name: &str) -> Scenario { scenarios().into_iter().find(|s| s.name.starts_with(name)).unwrap() }

#[test]
fn every_refused_field_refuses_naming_the_item() {
    let s = find("tickets queued while a group lands");
    let o = until(old_start(&s), |o| at(o, "a") == "build");
    let full = every_field_set();
    for (field, decision) in ITEM_FIELDS {
        let Decision::Refused(_) = decision else { continue };
        let mut state = o.state.clone();
        let mut v = serde_json::to_value(&state.items["a"]).unwrap();
        v[*field] = full[*field].clone();
        state.items.insert("a".into(), serde_json::from_value(v).unwrap());
        let refused = migrate(&state, &o.log, &admitted(&o), &setup(&s)).unwrap_err();
        assert!(refused.iter().any(|q| q.item == "a"), "{field}: {refused:?}");
    }
}

#[test]
fn a_running_ticket_carries_its_fields() {
    let s = find("tickets queued while a group lands");
    let mut o = until(old_start(&s), |o| at(o, "a") == "build" && o.state.items["a"].pending.is_some());
    // Fields a run picks up along the way, set on the old item as the old reducer would have.
    let i = o.state.items.get_mut("a").unwrap();
    i.dependencies.insert("b".into());
    i.models.insert("build".into(), "claude:opus".into());
    i.tree = Some(old::RunTree { name: "n".into(), branch: "a-branch".into(), path: "/w/a".into(), base: "0".repeat(40), adopted: false });
    i.brief = Some("rebase first".into());
    i.failure = Some("the last try said why".into());
    i.attempt = 2;
    i.last_evidence = Some(old::Evidence { candidate: "earlier".into(), manifest: "m".into(), receipt: "r".into(), policy_pin: "policy".into() });
    i.sent_back.push(old::SendBack { id: "i".into(), revision: 1, run: "r".into(), from: "build".into(), to: "build".into(), reason: "again".into() });
    let i = o.state.items["a"].clone();
    let m = migrate(&o.state, &o.log, &admitted(&o), &setup(&s)).unwrap();
    let t = &m.pipeline.tokens["a"];
    let Location::Working { work, .. } = &t.at else { panic!() };
    assert_eq!((work.run.as_str(), work.pending.as_ref(), work.attempt), (i.pending.as_deref().unwrap(), i.pending.as_ref(), 2));
    assert_eq!(work.op.spec, old::digest(&i.definition.stages[i.stage].operation), "the op spec is what the effect runs");
    assert_eq!(t.fence, i.fence);
    assert_eq!(t.dependencies, i.dependencies);
    assert_eq!(t.models, i.models);
    assert_eq!((t.brief.as_deref(), t.failure.as_deref()), (Some("rebase first"), Some("the last try said why")));
    let e = t.evidence.as_ref().unwrap();
    assert_eq!((e.artifact.as_str(), e.spec.as_str()), ("earlier", "policy"));
    let d = data_of(&t.data).unwrap();
    assert_eq!((d.tree, d.candidate), (i.tree.clone(), i.candidate.clone()));
    assert!(t.visits.iter().any(|v| v.station == "build" && v.ended.as_deref() == Some("sent") && v.reason.as_deref() == Some("again")));
    // `run` of a decision is its Decision::run: migrate_phases.rs checks it for every awaiting item.
}

#[test]
fn a_live_group_carries_its_fields() {
    let s = find("a listed CI failure reruns once");
    // A group pushes before CI only on a definition from before ov-1269, which a live item may still carry.
    let mut o = until(old_start_on(&s, before_candidates(&s)), |o| o.state.items.values().any(|g| g.group_of.is_some() && g.pushed.is_some() && g.pending.is_some()
        && !g.ci_retry.as_ref().is_some_and(|r| r.pending)));
    let (gid, _) = o.state.items.iter().find(|(_, g)| g.group_of.is_some()).map(|(k, g)| (k.clone(), g.clone())).unwrap();
    o.state.items.get_mut(&gid).unwrap().group_failure = Some(old::GroupFailure { job: "unit".into(), first_line: "boom".into(), split: false });
    let g = o.state.items[&gid].clone();
    let m = migrate(&o.state, &o.log, &admitted(&o), &setup(&s)).unwrap();
    let b = &m.pipeline.batches[&gid];
    assert_eq!((b.station.as_str(), b.since_ms, b.fence), ("join", g.since_ms, g.fence));
    assert_eq!(b.members.iter().map(|x| x.token.clone()).collect::<Vec<_>>(), g.members.iter().map(|x| x.item.clone()).collect::<Vec<_>>());
    let d = data_of(&b.data).unwrap();
    assert_eq!((d.pushed.clone(), d.failure.map(|f| f.job)), (g.pushed.clone(), Some("unit".to_string())));
    assert_eq!(b.crossings[0].artifact, g.pushed.clone().unwrap());
    for x in &g.members {
        assert_eq!(m.pipeline.tokens[&x.item].at, Location::InBatch { batch: gid.clone() }, "grouped_in");
        assert_eq!(m.pipeline.tokens[&x.item].crossings, b.crossings, "a member crossed with its batch");
    }
    // Pushed, the batch gave up the landing (free_on_crossing), so the next one can fold and gate.
    assert!(!m.pipeline.pools["landing"].held_by.contains(&gid));
}

#[test]
fn a_waiting_step_keeps_its_patience() {
    let s = find("a listed CI failure reruns once");
    // The look after a rerun: patience kept, the rerun spent, and the look bound to the run it reran.
    let o = until(old_start(&s), |o| o.state.items.values().any(|g| g.ci_retry.as_ref().is_some_and(|r| !r.pending) && g.pending.is_some()));
    let (gid, g) = o.state.items.iter().find(|(_, g)| g.ci_retry.is_some()).map(|(k, g)| (k.clone(), g.clone())).unwrap();
    let m = migrate(&o.state, &o.log, &admitted(&o), &setup(&s)).unwrap();
    let Location::Working { station, work } = &m.pipeline.batches[&gid].at else { panic!() };
    assert_eq!(station, "join.ci");
    assert_eq!((work.wait_since, work.reruns), (g.wait_since, 1));
    assert!(work.again.is_some());
    assert_eq!(data_of(&m.pipeline.batches[&gid].data).unwrap().ci_retry, Some(g.ci_retry.clone().unwrap().run));
    // The rerun being requested is quarantined instead.
    let o = until(old_start(&s), |o| o.state.items.values().any(|g| g.ci_retry.as_ref().is_some_and(|r| r.pending)));
    let m = migrate(&o.state, &o.log, &admitted(&o), &setup(&s)).unwrap();
    assert!(m.pipeline.batches.values().any(|b| matches!(&b.at, Location::Held { hold, .. } if hold.kind == HoldKind::Quarantined)));
    let mut r = support::Run::resume(m.pipeline, linked());
    r.advance();
}

#[test]
fn a_finished_group_and_closed_tickets_are_archived() {
    let s = find("tickets queued while a group lands");
    // The first group has landed, and the next has not formed yet.
    let mut o = until(old_start(&s), |o| o.state.items.values().any(|g| g.group_of.is_some() && g.phase == old::Phase::Done)
        && o.state.items.values().all(|g| g.members.is_empty()));
    // Closed this step, not yet archived by the old reducer: a ticket cancelled with a reason, and one closed
    // as landed.
    for (k, phase, closed, reason) in [("c", old::Phase::Cancelled, None, Some("not needed")),
        ("b", old::Phase::Done, Some(old::CloseResolution::Landed { sha: "f".repeat(40) }), None)] {
        let i = o.state.items.get_mut(k).unwrap();
        i.phase = phase; i.closed = closed; i.cancel_reason = reason.map(str::to_string); i.pending = None;
        i.grouped_in = None;
        if i.phase == old::Phase::Done { i.landed = Some("f".repeat(40)); i.stage = i.definition.stages.len(); }
    }
    let (gid, g) = o.state.items.iter().find(|(_, g)| g.group_of.is_some() && g.phase == old::Phase::Done).map(|(k, g)| (k.clone(), g.clone())).unwrap();
    let m = migrate(&o.state, &o.log, &admitted(&o), &setup(&s)).unwrap();
    let a = &m.pipeline.archive;
    assert_eq!(a["c"].reason.as_deref(), Some("not needed"));
    assert_eq!(a["b"].reason, Some(format!("closed as landed in {}", "f".repeat(40))));
    assert_eq!(data_of(&a["b"].data).unwrap().pushed, Some("f".repeat(40)), "landed");
    let batch = a[&gid].batch.as_ref().unwrap();
    assert_eq!((&batch.members, batch.since_ms), (&g.handed, g.since_ms), "handed");
}
