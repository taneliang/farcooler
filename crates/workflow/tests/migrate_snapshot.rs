//! A real engine's state, migrated (docs/engine-pipeline.md 10.1).
//!
//! tests/fixtures/migrate-snapshot.json is a live workspace's old `State`, sanitized and cut down by
//! reports/ov-1281/sanitize.py: ticket keys are t-N, every free string is a stand-in, and it keeps every live
//! group with its members, every ticket that is working or waiting, and a few of each other (phase, step) kind.
//! It holds a group mid-gate, a waiting group, tickets waiting at the join, uncertain provisions and builds,
//! failed tickets and groups, tracked tickets at rest and archived groups. There is no log to go with it, so
//! queues fall back to the outbox's order and the groups' own.
#[path = "pipeline_support/mod.rs"]
mod support;
use std::collections::BTreeMap;
use farcooler_workflow as old;
use farcooler_workflow::adapters::{coding_templates, linked};
use farcooler_workflow::migrate::{migrate, Dispatch, DispatchState, Migrated, Setup};
use farcooler_workflow::pipeline::{HoldKind, Kind, Location};
use farcooler_workflow::templates::{Gate, PathRoutes, Plan, Template};
use serde_json::json;

fn snapshot() -> old::State {
    serde_json::from_str(include_str!("fixtures/migrate-snapshot.json")).expect("the snapshot parses as the old State")
}
fn setup() -> Setup {
    let mut plan = Plan::new(Template::BatchedPush, vec![Gate::Agent, Gate::Command(0)]);
    plan.paths = Some(PathRoutes { argument: "rules".into(), approver: Some("owner".into()) });
    plan.batch = Some(32);
    let config = json!({ "repository": ".", "target": "main", "ci": "ci.yml", "checks": ["landing-gate"] });
    Setup { layout: coding_templates::layout(&plan, &config).unwrap(),
        pools: [("agents", 8), ("work", 8), ("landing", 1)].iter().map(|(k, n)| (k.to_string(), *n)).collect() }
}
/// The outbox as the old runtime left it: running work's effects admitted, uncertain work's uncertain.
fn outbox(state: &old::State) -> Vec<Dispatch> {
    state.items.values().filter_map(|i| {
        let state = match i.phase { old::Phase::Running => DispatchState::Admitted, old::Phase::Uncertain => DispatchState::Uncertain, _ => return None };
        Some(Dispatch { effect: i.pending.clone()?, state })
    }).collect()
}
fn place(m: &Migrated, at: &Location) -> String {
    match at {
        Location::Resting { .. } => "resting".into(),
        Location::Queued { station } => format!("queued at {station}"),
        Location::Working { station, work } => format!("working{} at {station}", if work.uncertain { " uncertain" } else { "" }),
        Location::Awaiting { station, .. } => format!("awaiting at {station}"),
        Location::InBatch { batch } => format!("in batch at {}", m.pipeline.batches[batch].at.station().unwrap()),
        Location::Held { station, hold } => format!("held {:?} at {station}", hold.kind),
        other => format!("{other:?}"),
    }
}

#[test]
fn a_real_workspace_migrates() {
    let state = snapshot();
    let m = migrate(&state, &[], &outbox(&state), &setup()).unwrap_or_else(|r| panic!("refused: {r:#?}"));
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for t in m.pipeline.tokens.values() { *seen.entry(place(&m, &t.at)).or_default() += 1; }
    for b in m.pipeline.batches.values() { *seen.entry(format!("batch {}", place(&m, &b.at))).or_default() += 1; }
    let want: BTreeMap<String, usize> = [
        ("batch working at join.gate-0", 1), ("in batch at join.gate-0", 3),
        // 16 tickets waiting at the join, and the 5 of the waiting group, which dissolves into the queue.
        ("queued at join", 21),
        ("working at build", 3), ("working uncertain at build", 2), ("working uncertain at provision", 3),
        // An uncertain provision whose stop was asked and never answered: no one knows if it stopped.
        ("held Quarantined at provision", 2),
        ("held StepFailed at build", 3), ("held StepFailed at gate-0", 3), ("held StepFailed at join", 3),
        ("resting", 9),
    ].iter().map(|(k, n)| (k.to_string(), *n)).collect();
    assert_eq!(seen, want);
    assert!(m.quarantined.iter().all(|q| q.reason == "its stop's answer was unknown"), "{:?}", m.quarantined);
    assert!(m.suppress.is_empty(), "every running effect was admitted");

    // The group mid-gate keeps its effect and its members; it holds the landing.
    let (gid, g) = state.items.iter().find(|(_, g)| g.group_of.is_some() && g.phase == old::Phase::Running).unwrap();
    let b = &m.pipeline.batches[gid];
    let Location::Working { work, .. } = &b.at else { panic!() };
    assert_eq!((work.pending.as_ref(), b.fence), (g.pending.as_ref(), g.fence));
    assert_eq!(b.members.iter().map(|x| &x.token).collect::<Vec<_>>(), g.members.iter().map(|x| &x.item).collect::<Vec<_>>());
    assert_eq!(m.pipeline.pools["landing"].held_by.iter().collect::<Vec<_>>(), [gid]);
    // The waiting group's tickets queue in its order, ahead of tickets no group had taken.
    let (_, w) = state.items.iter().find(|(_, g)| g.group_of.is_some() && g.phase == old::Phase::Blocked).unwrap();
    let queue = &m.pipeline.stations["join"].queue;
    assert_eq!(queue[..w.members.len()], w.members.iter().map(|x| x.item.clone()).collect::<Vec<_>>()[..]);
    assert_eq!(m.pipeline.stations["join"].kind, Kind::Batch);
    // Uncertain work stays uncertain; failed work is held for a person, with its failure.
    for (k, i) in &state.items {
        match (&i.phase, m.pipeline.tokens.get(k).map(|t| &t.at)) {
            (old::Phase::Uncertain, Some(Location::Working { work, .. })) => assert!(work.uncertain, "{k}"),
            (old::Phase::Failed, Some(Location::Held { hold, .. })) => assert_eq!(hold.kind, HoldKind::StepFailed, "{k}"),
            _ => {}
        }
    }
    // Every archived key stays, and every failed group with no tickets left is archived with its record.
    for k in state.archive.keys() { assert!(m.pipeline.archive.contains_key(k), "{k}"); }
    let failed = state.items.iter().filter(|(_, g)| g.group_of.is_some() && g.members.is_empty()).count();
    assert_eq!(m.pipeline.archive.len(), state.archive.len() + failed);

    // The core takes it from here: the first tick, then replay from the migrated snapshot.
    let mut r = support::Run::resume(m.pipeline, linked());
    r.advance();
    r.assert_replays();
}
