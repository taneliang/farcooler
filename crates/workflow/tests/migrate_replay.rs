//! Replay across the cutover (docs/engine-pipeline.md 10.2), on the differential test's scenarios (ov-1273).
//!
//! Each scenario runs on the old reducer, and is cut at every point of that run: the old state there is
//! migrated (every pending effect's outbox row `admitted`, as the old runtime had sent them all), and the
//! pipeline driver carries on from the migrated snapshot with the same world, answering the old effects the
//! migrated work still waits on. The run must then end where one of the two engines ends it alone: where the old
//! reducer does, or, past a point where the design changes the rules (the four differences in the ov-1273
//! report), where the pipeline does. Each migrated run also replays from its snapshot to the same state.
//!
//! One difference is intended and allowed: when two or more tickets are waiting at the join for the old
//! orchestrator's `Queue`, the pipeline has them queued at the batch station already (11.2, 11.3: `Queue` is
//! removed, arriving at a batch station queues), so it forms one batch where the old run queued them one by
//! one. The same tickets land in the same order with the same builds; only how they are grouped differs.
#[path = "differential_support/mod.rs"]
mod differential;
use differential::*;
use farcooler_workflow::adapters::{coding_templates, linked};
use farcooler_workflow::migrate::{migrate, Dispatch, DispatchState, Setup};
use farcooler_workflow::pipeline::{Command, Kind, Location, Subject};

/// Scenarios whose quarantined work, once released, ends differently, and how every ticket ends. A CI rerun
/// whose request was in flight is quarantined (its old answer means "the rerun went out", which the pipeline
/// would read as CI's verdict); released, the CI look starts a new visit, and the station's `again` policy,
/// counted per visit, allows one rerun again, so a second listed failure reruns and lands.
const RELEASED: &[(&str, &str)] = &[("a second red after the rerun stops the landed group (groups.rs)", "done")];

/// Every pending effect of `o`'s state, admitted.
fn admitted(o: &Old) -> Vec<Dispatch> {
    o.state.items.values().filter_map(|i| i.pending.clone()).map(|effect| Dispatch { effect, state: DispatchState::Admitted }).collect()
}

#[test]
fn a_run_cut_anywhere_and_migrated_ends_where_an_engine_ends_it() {
    let all = scenarios();
    let mut unexpected = Vec::new();
    let (mut scenarios_cut, mut cuts, mut quarantined, mut exact, mut batched, mut allowed) = (0, 0, 0, 0, 0, 0);
    for s in &all {
        let (old_alone, new_alone) = run(s);
        let layout = coding_templates::layout(&s.plan, &config(s)).unwrap();
        let setup = Setup { layout, pools: POOLS.iter().map(|(k, n)| (k.to_string(), *n)).collect() };
        let mut o = old_start(s);
        let mut points = vec![o.clone()];
        while o.act() { points.push(o.clone()); }
        for (k, at) in points.iter().enumerate() {
            let migrated = match migrate(&at.state, &at.log, &admitted(at), &setup) {
                Ok(m) => m,
                Err(refused) => { unexpected.push(format!("{} cut at {k}: refused {refused:?}", s.name)); continue }
            };
            assert!(migrated.suppress.is_empty(), "{} cut at {k}: every effect was admitted", s.name);
            // Tickets the pipeline queues at a batch station: two or more may form one batch where the old
            // orchestrator queued them one at a time.
            let queued_at_batch = migrated.pipeline.tokens.values().filter(|t| matches!(&t.at, Location::Queued { station }
                if migrated.pipeline.stations[station].kind == Kind::Batch)).count();
            let mut n = new_driver(s, support::Run::resume(migrated.pipeline, linked()), Some(at));
            // Quarantined work is held for a person, who looks and releases it where it stands.
            let released = !migrated.quarantined.is_empty();
            for q in &migrated.quarantined {
                quarantined += 1;
                let subject = if n.r.p.batches.contains_key(&q.item) { Subject::Batch(q.item.clone()) } else { Subject::Token(q.item.clone()) };
                let station = n.r.location(&subject).station().unwrap().to_string();
                n.r.person("owner", Command::Release { subject, station, to: None, reason: Some(q.reason.clone()) }).unwrap();
            }
            // Genesis emits nothing: queued work starts on the runtime's first tick after the cutover.
            n.r.advance();
            settle_new(s, &mut n);
            n.r.assert_replays();
            let found = n.outcomes();
            let regrouped = |alone: &Outcomes| found.tickets == alone.tickets && found.builds == alone.builds
                && found.landings.concat() == alone.landings.concat();
            if found == old_alone || found == new_alone { exact += 1; }
            else if queued_at_batch >= 2 && (regrouped(&old_alone) || regrouped(&new_alone)) { batched += 1; }
            else if released && RELEASED.iter().any(|(name, end)| *name == s.name && found.tickets.values().all(|o| o == end)) { allowed += 1; }
            else {
                unexpected.push(format!("{} cut at {k}:\n  migrated {found:?}\n  old      {old_alone:?}\n  new      {new_alone:?}", s.name));
            }
            cuts += 1;
        }
        scenarios_cut += 1;
    }
    assert!(unexpected.is_empty(), "{}", unexpected.join("\n"));
    assert!(scenarios_cut >= 20, "at least twenty scenarios");
    assert!(cuts > scenarios_cut * 10, "each scenario is cut at many points, not just its ends: {cuts}");
    eprintln!("{scenarios_cut} scenarios, {cuts} cuts: {exact} end exactly as an engine alone, {batched} batch differently, {allowed} differ after a release; {quarantined} quarantined");
    assert!(quarantined > 0, "a CI rerun being requested is quarantined, and released by a person");
}
