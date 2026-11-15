//! Changing the process (docs/engine-pipeline.md 7): a publish takes effect in the event that publishes it.
//! No token remembers an older layout; running steps finish where they are and answer into the new one.
use std::collections::{BTreeMap, BTreeSet};
use super::flow::Ctx;
use super::layout::{check, BoundaryMove, Graph, Kind, Layout, Move};
use super::step::Refusal;
use super::types::*;

/// Where an old station's work goes under a publish.
#[derive(Clone, Debug, PartialEq)]
enum Mapped { Same(StationId), Renamed(StationId), Replaced { to: StationId, labels: BTreeMap<String, Target> }, Unbatched(StationId), Gone }

impl<'a, 'r> Ctx<'a, 'r> {
    pub(super) fn publish(&mut self, layout: &Layout, moves: &BTreeMap<StationId, Move>, after: &BTreeMap<String, After>,
        reason: &str) -> Result<(), Refusal> {
        let pools: BTreeSet<String> = self.p.pools.keys().cloned().collect();
        let new = check(layout, self.adapters, &pools)?;
        let old = std::mem::replace(&mut self.graph, new.clone());
        let map = mapping(&old, &new, moves)?;
        for (boundary, a) in after {
            if let After::Station(s) = a {
                if !new.nodes.contains_key(s) || new.leads_to(s, boundary) { return Err(Refusal::Definition); }
            }
        }
        self.refuse_stranding(&old, &map)?;
        let remap = |id: &str| -> Option<StationId> { match map.get(id) {
            Some(Mapped::Same(n) | Mapped::Renamed(n)) => Some(n.clone()),
            Some(Mapped::Replaced { to, .. } | Mapped::Unbatched(to)) => Some(to.clone()),
            Some(Mapped::Gone) => None,
            None => new.nodes.contains_key(id).then(|| id.to_string()),
        } };
        // Successive publishes compose: a retired translation always names live stations (7.3).
        for r in self.p.retired.values_mut() {
            r.to = remap(&r.to).ok_or(Refusal::Stranded)?;
            for t in r.labels.values_mut() {
                if let Target::Station(s) = t { *s = remap(s).ok_or(Refusal::Stranded)?; }
            }
        }
        self.rename_everywhere(&map);
        let adapters = layout.adapters.iter().map(|(k, u)| (k.clone(), AdapterRef { version: u.version.clone(),
            config_digest: crate::digest(&u.config) })).collect();
        self.p.workflow = Some(Published { digest: crate::digest(layout), layout: layout.clone(), adapters,
            after: after.clone(), reason: reason.into() });
        let mut removed = BTreeMap::new();
        let mut stations = BTreeMap::new();
        let old_states = std::mem::take(&mut self.p.stations);
        for (id, st) in old_states {
            let key = match map.get(&id) { Some(Mapped::Renamed(n)) => n.clone(), _ => id.clone() };
            if new.nodes.contains_key(&key) { stations.insert(key, st); } else { removed.insert(id, st); }
        }
        for id in &new.order {
            let def = &new.nodes[id].def;
            // A station without its own capacity is bound by its pool alone, so raising the pool raises it.
            let capacity = def.capacity.unwrap_or(u16::MAX);
            let window = def.form.as_ref().and_then(|f| f.window.as_ref()).map(|w| w.index(self.now()));
            let st = stations.entry(id.clone()).or_insert_with(|| Station { kind: def.kind, queue: Vec::new(),
                working: BTreeSet::new(), capacity, held: None, draining: false, window });
            st.kind = def.kind;
            st.capacity = capacity;
            if st.window.is_none() { st.window = window; }
        }
        self.p.stations = stations;
        self.p.stations.extend(removed.iter().map(|(k, v)| (k.clone(), v.clone())));
        for (id, mapped) in &map {
            match mapped {
                Mapped::Replaced { to, labels } => self.move_off(&old, id, to, labels.clone(), false),
                Mapped::Unbatched(to) => self.move_off(&old, id, to, BTreeMap::new(), true),
                _ => {}
            }
        }
        for id in removed.keys() { self.p.stations.remove(id); }
        self.recheck_crossed();
        Ok(())
    }
    /// Refused `Stranded` if any work would end at a station that neither exists nor is in `moves`.
    fn refuse_stranding(&self, old: &Graph, map: &BTreeMap<StationId, Mapped>) -> Result<(), Refusal> {
        let gone = |id: &str| matches!(map.get(id), Some(Mapped::Gone));
        let busy = |id: &str| self.p.tokens.values().any(|t| t.at.station() == Some(id) || then_names(&t.at, id))
            || self.p.batches.values().any(|b| b.at.station() == Some(id) || b.station == id || then_names(&b.at, id))
            || self.p.retired.values().any(|r| r.to == id || r.labels.values().any(|t| *t == Target::Station(id.into())));
        if old.order.iter().any(|id| gone(id) && busy(id)) { return Err(Refusal::Stranded); }
        Ok(())
    }
    /// A rename re-keys every place the old id is written: locations, visits, crossings, models.
    fn rename_everywhere(&mut self, map: &BTreeMap<StationId, Mapped>) {
        let renamed: BTreeMap<&str, &str> = map.iter().filter_map(|(k, m)| match m {
            Mapped::Renamed(n) => Some((k.as_str(), n.as_str())), _ => None }).collect();
        let fix = |s: &mut String| if let Some(n) = renamed.get(s.as_str()) { *s = n.to_string(); };
        let fix_at = |at: &mut Location| {
            match at {
                Location::Queued { station } | Location::Working { station, .. } | Location::Awaiting { station, .. }
                | Location::Held { station, .. } => fix(station),
                Location::Stopping { station, then, .. } | Location::Settling { station, then, .. } => {
                    fix(station);
                    if let Then::Arrive { station, .. } = then.as_mut() { fix(station); }
                }
                _ => {}
            }
        };
        for t in self.p.tokens.values_mut() {
            fix_at(&mut t.at);
            for v in &mut t.visits { fix(&mut v.station); }
            for c in &mut t.crossings { fix(&mut c.station); }
            t.models = std::mem::take(&mut t.models).into_iter().map(|(mut k, v)| { fix(&mut k); (k, v) }).collect();
        }
        for b in self.p.batches.values_mut() {
            fix_at(&mut b.at);
            fix(&mut b.station);
            for v in &mut b.visits { fix(&mut v.station); }
            for c in &mut b.crossings { fix(&mut c.station); }
            if let Some(x) = &mut b.bisect { fix(&mut x.station); }
        }
    }
    /// Move every piece of work off a removed station `id`: queued, held, awaiting and probing work moves to
    /// `to` now; running work finishes in a retired station that keeps only its translation.
    fn move_off(&mut self, old: &Graph, id: &str, to: &str, labels: BTreeMap<String, Target>, unbatch: bool) {
        let node = &old.nodes[id];
        let (kind, op, pool) = (node.def.kind, node.def.op.clone(), node.def.pool.clone());
        let outward = node.spec.as_ref().is_some_and(|s| s.outward.is_some());
        let here: Vec<Subject> = self.p.tokens.iter().filter(|(_, t)| t.at.station() == Some(id))
            .map(|(k, _)| Subject::Token(k.clone()))
            .chain(self.p.batches.iter().filter(|(_, b)| b.at.station() == Some(id) || (unbatch && b.station == id && b.at.station().is_none()))
                .map(|(k, _)| Subject::Batch(k.clone())))
            .collect();
        for s in here {
            let at = self.at(&s).clone();
            let running = matches!(at, Location::Stopping { .. } | Location::Settling { .. })
                || matches!(at, Location::Working { .. } if kind != Kind::Wait || outward);
            if running {
                let r = self.p.retired.entry(id.into()).or_insert_with(|| Retired { kind, op: op.clone(), pool: pool.clone(),
                    working: BTreeSet::new(), to: to.into(), labels: labels.clone(), unbatch });
                r.working.insert(s.key().into());
                if let Some(st) = self.p.stations.get_mut(id) { st.working.remove(s.key()); }
                continue;
            }
            if let Subject::Batch(b) = &s {
                let home = self.p.batches[b].station.clone();
                if unbatch || self.graph.node(to).and_then(|n| n.parent.clone()) != Some(home) { self.unbatch(b, to, "passed"); continue; }
            }
            if let Location::Working { .. } = at { self.bump(&s); }
            self.free_place(s.key(), id);
            match at {
                Location::Held { hold, .. } => {
                    self.close_visit(&s, "moved", Some(format!("{id} was removed")), None);
                    match self.crossed_block(&s, to) {
                        Some(k) => self.continue_after(&s, to, &k, "held", Some(hold)),
                        None => { self.open_visit(&s, to, "publish", None); self.set_at(&s, Location::Held { station: to.into(), hold }); }
                    }
                }
                _ => {
                    self.close_visit(&s, "moved", Some(format!("{id} was removed")), None);
                    self.place(&s, to, "passed");
                }
            }
        }
        // A removed batch station's queued tokens go to `to` as well; its inner stations' work is handled above.
        // Its running batches finish in their retired inner station (the inner stations map `Unbatched` too).
        if unbatch && kind == Kind::Batch {
            for (k, b) in self.p.batches.clone() {
                if b.station == id && !matches!(b.at, Location::Working { .. } | Location::Stopping { .. } | Location::Settling { .. }) {
                    self.unbatch(&k, to, "passed");
                }
            }
        }
    }
    /// The migration rule for crossed work that stays at a live station the new layout moved before its boundary.
    fn recheck_crossed(&mut self) {
        let queued: Vec<(String, String)> = self.p.tokens.iter().filter_map(|(k, t)| match &t.at {
            Location::Queued { station } => Some((k.clone(), station.clone())), _ => None }).collect();
        for (k, station) in queued {
            let s = Subject::Token(k);
            if let Some(boundary) = self.crossed_block(&s, &station) {
                self.free_place(s.key(), &station);
                self.close_visit(&s, "moved", Some(format!("the new layout leads {station} back to {boundary}")), None);
                self.continue_after(&s, &station, &boundary, "passed", None);
            }
        }
    }
}
fn then_names(at: &Location, id: &str) -> bool {
    match at {
        Location::Stopping { then, .. } | Location::Settling { then, .. } => matches!(then.as_ref(), Then::Arrive { station, .. } if station == id),
        _ => false,
    }
}
/// Every old station's fate under a publish, with the moves checked against both layouts.
fn mapping(old: &Graph, new: &Graph, moves: &BTreeMap<StationId, Move>) -> Result<BTreeMap<StationId, Mapped>, Refusal> {
    let mut map = BTreeMap::new();
    for (id, mv) in moves {
        let node = old.nodes.get(id).ok_or(Refusal::Invalid)?;
        let mapped = match mv {
            Move::Rename { to } => {
                let n = new.nodes.get(to).ok_or(Refusal::Stranded)?;
                if n.def.kind != node.def.kind || n.def.op != node.def.op || new.nodes.contains_key(id) { return Err(Refusal::Definition); }
                Mapped::Renamed(to.clone())
            }
            Move::Replace { to, labels, boundary } => {
                let n = new.nodes.get(to).ok_or(Refusal::Stranded)?;
                if new.nodes.contains_key(id) { return Err(Refusal::Definition); }
                if let Some(o) = node.spec.as_ref().and_then(|s| s.outward.as_ref()) {
                    let same = n.spec.as_ref().and_then(|s| s.outward.as_ref()).is_some_and(|x| x.boundary == o.boundary);
                    match boundary { Some(BoundaryMove::Keep) if same => {}, Some(BoundaryMove::New) => {}, _ => return Err(Refusal::Definition) }
                }
                let mut resolved = BTreeMap::new();
                for (label, raw) in labels { resolved.insert(label.clone(), new.resolve(to, raw).ok_or(Refusal::Stranded)?); }
                Mapped::Replaced { to: to.clone(), labels: resolved }
            }
            Move::Unbatch { to } => {
                if node.def.kind != Kind::Batch || new.nodes.contains_key(id) || !new.nodes.get(to).is_some_and(|n| n.parent.is_none()) { return Err(Refusal::Definition); }
                Mapped::Unbatched(to.clone())
            }
        };
        map.insert(id.clone(), mapped);
    }
    for id in &old.order {
        if map.contains_key(id) { continue; }
        let parent = old.nodes[id].parent.clone();
        let inner = id.rsplit_once('.').map(|(_, i)| i.to_string());
        let mapped = match (parent.as_ref().and_then(|p| map.get(p)), inner) {
            (Some(Mapped::Renamed(np)), Some(i)) if new.nodes.contains_key(&format!("{np}.{i}")) => Mapped::Renamed(format!("{np}.{i}")),
            (Some(Mapped::Unbatched(to)), _) => Mapped::Unbatched(to.clone()),
            _ if new.nodes.contains_key(id) => Mapped::Same(id.clone()),
            _ => Mapped::Gone,
        };
        map.insert(id.clone(), mapped);
    }
    Ok(map)
}
