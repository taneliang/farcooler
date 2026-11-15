//! The rehearsal of docs/engine-pipeline.md 10.3: twelve builds, four places.
//!
//! One workspace: pool `agents` of 4, station `build` (pool `agents`) of capacity 4, twelve tickets `Running` at
//! build: A1-A4 `admitted`, P1-P8 `pending` in that outbox order. A second workspace holds one admitted build,
//! which the store stamps on every input as `host.agents.busy`; the host limit for `agents` is 5. A start needs
//! room at three levels (station, pool, host): in each case one level alone is full, nothing starts, and the
//! start comes on the event that frees that level.
//!
//! The rest of 10.3 (the crash inside the cutover transaction, publishes and rounds) needs the store and the
//! mail adapter; it belongs to the steps that wire them (section 10, steps 3 and 4).
#[path = "differential_support/mod.rs"]
mod differential;
use std::collections::BTreeMap;
use differential::*;
use farcooler_workflow as old;
use farcooler_workflow::adapters::{coding_templates, linked};
use farcooler_workflow::migrate::{migrate, Dispatch, DispatchState, Migrated, Setup};
use farcooler_workflow::pipeline::{Command, Effect, EffectKind, HostPool, Location, Scope};
use support::{tok, Run};

const A: [&str; 4] = ["A1", "A2", "A3", "A4"];
const P: [&str; 8] = ["P1", "P2", "P3", "P4", "P5", "P6", "P7", "P8"];

/// The old state before the cutover, its outbox, and the setup it migrates onto.
fn before() -> (old::State, Vec<old::Input>, Vec<Dispatch>, Setup) {
    let s = scenarios().into_iter().find(|s| s.name.starts_with("batched push lands one ticket")).unwrap();
    let mut o = old_start(&s);
    while !(o.state.items["a"].definition.stages[o.state.items["a"].stage].id == "build" && o.state.items["a"].pending.is_some()) {
        assert!(o.act());
    }
    let a = o.state.items["a"].clone();
    assert_eq!(a.phase, old::Phase::Running);
    let mut state = o.state.clone();
    state.items.clear();
    let mut outbox = Vec::new();
    for (n, k) in A.iter().chain(P.iter()).enumerate() {
        let mut i = a.clone();
        i.pending = Some(format!("build-{k}"));
        i.run = format!("run-{k}");
        i.fence = 10 + n as u64;
        state.items.insert(k.to_string(), i);
        let admitted = k.starts_with('A');
        outbox.push(Dispatch { effect: format!("build-{k}"), state: if admitted { DispatchState::Admitted } else { DispatchState::Pending } });
    }
    let mut layout = coding_templates::layout(&s.plan, &config(&s)).unwrap();
    let build = layout.stations.iter_mut().find(|d| d.id == "build").unwrap();
    assert_eq!(build.pool.as_deref(), Some("agents"));
    build.capacity = Some(4);
    let pools = [("agents", 4), ("work", 100), ("landing", 1)].iter().map(|(k, n)| (k.to_string(), *n)).collect();
    (state, o.log.clone(), outbox, Setup { layout, pools })
}

/// The cutover: migrated, asserted as 10.3 states it, prioritized, and handed to a driver under the host limit.
fn cutover() -> (Run, BTreeMap<String, u64>) {
    let (state, log, outbox, setup) = before();
    let Migrated { pipeline, quarantined, suppress } = migrate(&state, &log, &outbox, &setup).unwrap();
    assert!(quarantined.is_empty());
    let st = &pipeline.stations["build"];
    assert_eq!(st.working.iter().cloned().collect::<Vec<_>>(), A.map(String::from), "build working is exactly A1-A4");
    assert_eq!(st.queue, P.map(String::from), "the build queue is P1-P8 in outbox order");
    assert_eq!(suppress, P.map(|k| format!("build-{k}")), "their rows are suppressed");
    for k in A {
        let t = &pipeline.tokens[k];
        let Location::Working { work, .. } = &t.at else { panic!("{k}: {:?}", t.at) };
        assert_eq!((work.run.clone(), work.pending.clone(), t.fence), (format!("build-{k}"), Some(format!("build-{k}")), state.items[k].fence));
    }
    assert_eq!(pipeline.pools["agents"].held_by.len(), 4, "pool 4 of 4");
    let fences = P.iter().map(|k| (k.to_string(), pipeline.tokens[*k].fence)).collect();
    let mut r = Run::resume(pipeline, linked());
    r.host = host(Some(1));
    // A P row's late Observe is refused: its effect never went out.
    let refused = r.send(farcooler_workflow::pipeline::Grant::Operation { effect: "build-P1".into(), fence: fences_of(&r, "P1") },
        Command::Observe { subject: tok("P1"), effect: "build-P1".into(), fence: fences_of(&r, "P1"),
            outcome: farcooler_workflow::pipeline::Outcome::Failed { reason: Some("late".into()) } });
    assert!(refused.is_err(), "{refused:?}");
    assert!(r.orch(Command::Prioritize { token: "P6".into(), priority: 10 }).unwrap().is_empty());
    assert!(r.orch(Command::Prioritize { token: "P7".into(), priority: 10 }).unwrap().is_empty());
    assert_eq!(r.queue("build"), ["P6", "P7", "P1", "P2", "P3", "P4", "P5", "P8"]);
    (r, fences)
}
fn fences_of(r: &Run, k: &str) -> u64 { r.p.tokens[k].fence }
/// The host's `agents` limit of 5 with what the other workspace holds, or no host limit.
fn host(other: Option<u16>) -> BTreeMap<String, HostPool> {
    other.map(|busy| ("agents".to_string(), HostPool { limit: 5, busy })).into_iter().collect()
}
/// The builds an event started, by ticket.
fn builds(effects: &[Effect]) -> Vec<String> {
    effects.iter().filter(|e| matches!(e.kind, EffectKind::Execute(_)) && e.station == "build").map(|e| e.subject.key().to_string()).collect()
}
fn set(r: &mut Run, scope: Scope, n: u16) -> Vec<String> {
    builds(&r.person("owner", Command::SetCapacity { scope, n, reason: "rehearsal".into() }).unwrap())
}
fn station(n: &str) -> Scope { Scope::Station(n.into()) }
fn pool() -> Scope { Scope::Pool("agents".into()) }
/// A2's build passes; its next step (the command gate, pool `work`) needs no agent.
fn a2_passes(r: &mut Run) -> Vec<String> {
    let before = r.effects.len();
    r.answer(&tok("A2"), "passed");
    builds(&r.effects[before..])
}
/// The other workspace's build ends: the store stamps the next input with what it holds now.
fn other_ends(r: &mut Run, limit: bool) -> Vec<String> {
    r.host = host(limit.then_some(0));
    let before = r.effects.len();
    r.advance();
    builds(&r.effects[before..])
}
fn started(r: &Run, k: &str, fences: &BTreeMap<String, u64>) {
    let w = r.work(&tok(k));
    let e = r.effect_for(&tok(k));
    assert_ne!(e.id, format!("build-{k}"), "{k} starts with a new effect id");
    assert_eq!((w.pending, r.fence(&tok(k))), (Some(e.id), fences[k] + 1), "{k}: fence old + 1");
}
fn counts(r: &Run) -> (usize, u16, usize, u16) {
    let st = &r.p.stations["build"];
    (st.working.len(), st.capacity, r.p.pools["agents"].held_by.len(), r.p.pools["agents"].capacity)
}

#[test]
fn only_the_workspace_pool_binds() {
    let (mut r, fences) = cutover();
    assert_eq!(set(&mut r, station("build"), 8), Vec::<String>::new(), "pool and host full");
    assert_eq!(other_ends(&mut r, true), Vec::<String>::new(), "host 4 of 5, station 4 of 8: the pool alone blocks");
    assert_eq!(counts(&r), (4, 8, 4, 4));
    assert_eq!(set(&mut r, pool(), 5), ["P6"], "the start comes on the event that frees the pool");
    started(&r, "P6", &fences);
    assert_eq!(counts(&r), (5, 8, 5, 5));
    assert_eq!(a2_passes(&mut r), ["P7"]);
    assert_ne!(r.station("A2").as_deref(), Some("build"), "A2 moved on, out of the agents pool: {:?}", r.at("A2"));
    r.assert_replays();
}

#[test]
fn only_the_station_binds() {
    let (mut r, fences) = cutover();
    r.host = host(None);
    assert_eq!(set(&mut r, pool(), 8), Vec::<String>::new(), "station 4 of 4, pool 4 of 8, no host limit");
    assert_eq!(set(&mut r, station("build"), 5), ["P6"], "the start comes on the event that frees the station");
    started(&r, "P6", &fences);
    assert_eq!(counts(&r), (5, 5, 5, 8), "P7 waits");
    assert_eq!(a2_passes(&mut r), ["P7"]);
    r.assert_replays();
}

#[test]
fn only_the_host_limit_binds() {
    let (mut r, fences) = cutover();
    assert_eq!(set(&mut r, pool(), 8), Vec::<String>::new());
    assert_eq!(set(&mut r, station("build"), 8), Vec::<String>::new(), "four free places each: host 5 of 5 alone blocks");
    assert_eq!(a2_passes(&mut r), ["P6"], "host 5 of 5 again");
    started(&r, "P6", &fences);
    assert_eq!(other_ends(&mut r, true), ["P7"], "exactly one start, on the event that frees the host");
    assert_eq!(r.queue("build")[0], "P1", "P1 still waits with station and pool places free");
    assert_eq!(counts(&r), (5, 8, 5, 8));
    r.assert_replays();
}

#[test]
fn no_host_limit_and_default_limits() {
    let (mut r, fences) = cutover();
    r.host = host(None);
    assert_eq!(a2_passes(&mut r), ["P6"], "the other workspace no longer counts; station and pool 4 of 4");
    started(&r, "P6", &fences);
    assert_eq!(counts(&r), (4, 4, 4, 4));
    assert_eq!(r.queue("build"), ["P7", "P1", "P2", "P3", "P4", "P5", "P8"]);
    r.assert_replays();
}

#[test]
fn nothing_starts_at_the_cutover() {
    let (state, log, outbox, setup) = before();
    let m = migrate(&state, &log, &outbox, &setup).unwrap();
    let mut r = Run::resume(m.pipeline, linked());
    r.host = host(Some(1));
    r.advance();
    assert!(r.effects.is_empty(), "Genesis and the first tick emit nothing: station 4 of 4, pool 4 of 4, host 5 of 5");
}
