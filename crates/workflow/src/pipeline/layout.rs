//! Schema 3: the published layout (docs/engine-pipeline.md 4, 7.1, 8), its routing rules and its validation.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use serde::{Deserialize, Serialize};
use super::adapter::{Adapters, OpSpec, Problem, Repeat};
use super::step::Refusal;
use super::types::{StationId, Target};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    pub schema: u32,
    pub name: String,
    pub adapters: BTreeMap<String, AdapterUse>,
    #[serde(default)]
    pub intake: Vec<Intake>,
    pub stations: Vec<StationDef>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterUse { pub version: String, pub config: serde_json::Value }
/// An adapter source that creates tokens at a station.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Intake { pub op: String, pub to: String }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Kind { Agent, #[default] Command, Decision, Wait, Batch }
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationDef {
    pub id: String,
    pub kind: Kind,
    #[serde(default)]
    pub op: Option<String>,
    #[serde(default)]
    pub pool: Option<String>,
    #[serde(default)]
    pub capacity: Option<u16>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub deciders: Vec<String>,
    #[serde(default)]
    pub needs: Option<u8>,
    #[serde(default)]
    pub routes: BTreeMap<String, String>,
    #[serde(default)]
    pub policy: Policy,
    #[serde(default)]
    pub form: Option<Form>,
    #[serde(default)]
    pub on_reject: OnReject,
    /// A batch station's inner stations, in order.
    #[serde(default)]
    pub stations: Vec<StationDef>,
}
/// Station policy replaces every ticket-level limit (4.4). A policy never closes a token; at worst it holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    #[serde(default)]
    pub attempts: Option<u8>,
    #[serde(default)]
    pub rework: Option<u8>,
    #[serde(default)]
    pub returns: Option<u8>,
    #[serde(default)]
    pub again: Option<u8>,
    #[serde(default)]
    pub patience_ms: Option<i64>,
    #[serde(default)]
    pub skip: Skipper,
}
impl Policy {
    pub fn attempts(&self, kind: Kind) -> u8 { self.attempts.unwrap_or(if kind == Kind::Wait { 5 } else { 2 }) }
    pub fn rework(&self) -> u8 { self.rework.unwrap_or(2) }
    pub fn returns(&self) -> u8 { self.returns.unwrap_or(1) }
    pub fn again(&self) -> u8 { self.again.unwrap_or(0) }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Skipper { #[default] Orchestrator, Person }
/// How a batch station forms batches from its queue. A window forms when `now_ms` enters a new window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Form {
    #[serde(default)]
    pub size: Option<u16>,
    #[serde(default)]
    pub window: Option<Window>,
}
/// Windows of `every_ms`, each starting `at_ms` past a multiple of it (a weekly Friday 09:00 window is
/// `every_ms` one week and `at_ms` the offset of Friday 09:00 from the epoch's week start).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window { pub every_ms: i64, pub at_ms: i64 }
impl Window {
    pub fn index(&self, now_ms: i64) -> i64 { (now_ms - self.at_ms).div_euclid(self.every_ms.max(1)) }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OnReject { #[default] Bisect, ReturnSuspects, Hold }
/// What a publish says about a station it removes or renames (7.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Move {
    Rename { to: StationId },
    Replace { to: StationId, #[serde(default)] labels: BTreeMap<String, String>, #[serde(default)] boundary: Option<BoundaryMove> },
    Unbatch { to: StationId },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoundaryMove { Keep, New }

pub const RESERVED: [&str; 4] = ["done", "finish", "rejected", "returned"];

/// A layout flattened to full station ids, with each station's operation resolved from its adapter.
#[derive(Clone, Debug)]
pub struct Graph {
    pub nodes: BTreeMap<StationId, Node>,
    /// Every station id in layout order, a batch station before its inner stations.
    pub order: Vec<StationId>,
}
#[derive(Clone, Debug)]
pub struct Node {
    pub def: StationDef,
    pub parent: Option<StationId>,
    pub next: Option<StationId>,
    pub first: Option<StationId>,
    /// Position among its siblings: a route to a lower or equal position is a send-back.
    pub index: usize,
    pub spec: Option<OpSpec>,
    pub adapter: Option<String>,
}
impl Graph {
    pub fn new(layout: &Layout, adapters: &Adapters) -> Self {
        let mut graph = Graph { nodes: BTreeMap::new(), order: Vec::new() };
        graph.add(layout, adapters, &layout.stations, None);
        graph
    }
    fn add(&mut self, layout: &Layout, adapters: &Adapters, defs: &[StationDef], parent: Option<&str>) {
        let full = |id: &str| parent.map_or(id.to_string(), |p| format!("{p}.{id}"));
        for (index, def) in defs.iter().enumerate() {
            let id = full(&def.id);
            let next = defs.get(index + 1).map(|d| full(&d.id));
            let first = def.stations.first().map(|d| format!("{id}.{}", d.id));
            let (adapter, spec) = match def.op.as_deref().and_then(|op| op.split_once('.')) {
                Some((key, name)) => {
                    let spec = layout.adapters.get(key).and_then(|u| adapters.get(&u.version)
                        .and_then(|r| r.catalog(&u.config).into_iter().find(|s| s.name == name)));
                    (Some(key.to_string()), spec)
                }
                None => (None, None),
            };
            let mut flat = def.clone();
            flat.stations = Vec::new();
            self.order.push(id.clone());
            self.nodes.insert(id.clone(), Node { def: flat, parent: parent.map(str::to_string), next, first,
                index, spec, adapter });
            if !def.stations.is_empty() { self.add(layout, adapters, &def.stations, Some(&id)); }
        }
    }
    pub fn node(&self, id: &str) -> Option<&Node> { self.nodes.get(id) }
    /// A raw route target written in the station's scope.
    pub fn resolve(&self, station: &str, raw: &str) -> Option<Target> {
        let parent = self.nodes.get(station)?.parent.as_deref();
        match (raw, parent) {
            ("done", None) => Some(Target::Done),
            ("finish", Some(_)) => Some(Target::Finish),
            ("rejected", Some(_)) => Some(Target::Rejected),
            ("returned", Some(_)) => Some(Target::Returned),
            (r, _) if RESERVED.contains(&r) => None,
            (r, p) => {
                let id = p.map_or(r.to_string(), |p| format!("{p}.{r}"));
                self.nodes.contains_key(&id).then_some(Target::Station(id))
            }
        }
    }
    /// Routing, exactly (4.1). `None` means the label holds the work at the station.
    pub fn route(&self, station: &str, label: &str) -> Option<Target> {
        let node = self.nodes.get(station)?;
        if let Some(raw) = node.def.routes.get(label) { return self.resolve(station, raw); }
        if label == "skipped" {
            let via = if node.def.kind == Kind::Decision { "approved" } else { "passed" };
            if let Some(raw) = node.def.routes.get(via) { return self.resolve(station, raw); }
        }
        if label != "passed" && label != "skipped" { return None; }
        Some(match (&node.next, &node.parent) {
            (Some(next), _) => Target::Station(next.clone()),
            (None, None) => Target::Done,
            (None, Some(_)) => Target::Finish,
        })
    }
    /// Stations one move away from `station`, by any label it can answer.
    fn successors(&self, station: &str) -> Vec<StationId> {
        let Some(node) = self.nodes.get(station) else { return Vec::new() };
        let mut labels: BTreeSet<&str> = node.def.routes.keys().map(String::as_str).collect();
        labels.insert("passed");
        if let Some(spec) = &node.spec { labels.extend(spec.labels.iter().map(String::as_str)); }
        let mut out = Vec::new();
        if let Some(first) = &node.first { out.push(first.clone()); }
        for label in labels {
            out.extend(self.exits(station, self.route(station, label)));
        }
        out
    }
    /// The stations a target from `station` arrives at: `finish` leaves by the batch's `passed` route,
    /// `returned` by its `returned` route, and `rejected` re-runs the station or returns members.
    fn exits(&self, station: &str, target: Option<Target>) -> Vec<StationId> {
        let parent = self.nodes.get(station).and_then(|n| n.parent.clone());
        let up = |label: &str| match (&parent, parent.as_ref().and_then(|p| self.route(p, label))) {
            (Some(_), Some(Target::Station(s))) => vec![s],
            _ => Vec::new(),
        };
        match target {
            Some(Target::Station(s)) => vec![s],
            Some(Target::Finish) => up("passed"),
            Some(Target::Returned) => up("returned"),
            Some(Target::Rejected) => { let mut v = up("returned"); v.push(station.to_string()); v }
            _ => Vec::new(),
        }
    }
    /// Every station reachable from `from`, including itself.
    pub fn reachable(&self, from: &str) -> BTreeSet<StationId> {
        let mut seen = BTreeSet::new();
        let mut queue = VecDeque::from([from.to_string()]);
        while let Some(s) = queue.pop_front() {
            if !self.nodes.contains_key(&s) || !seen.insert(s.clone()) { continue; }
            queue.extend(self.successors(&s));
        }
        seen
    }
    /// Stations whose operation crosses `boundary`.
    pub fn boundary_stations(&self, boundary: &str) -> BTreeSet<StationId> {
        self.nodes.iter().filter(|(_, n)| n.spec.as_ref().and_then(|s| s.outward.as_ref())
            .is_some_and(|o| o.boundary == boundary)).map(|(id, _)| id.clone()).collect()
    }
    /// The repeat rule of a boundary as this layout's stations declare it.
    pub fn repeat(&self, boundary: &str) -> Option<Repeat> {
        self.nodes.values().filter_map(|n| n.spec.as_ref()?.outward.as_ref())
            .find(|o| o.boundary == boundary).map(|o| o.repeat)
    }
    /// Whether arriving at `station` could lead back to a station on `boundary` (5.3, rule 1).
    pub fn leads_to(&self, station: &str, boundary: &str) -> bool {
        let targets = self.boundary_stations(boundary);
        self.reachable(station).iter().any(|s| targets.contains(s))
    }
    /// Validate a layout against the adapters linked into this build and the workspace's pools.
    pub fn validate(layout: &Layout, adapters: &Adapters, pools: &BTreeSet<String>) -> Result<Graph, Vec<Problem>> {
        let graph = Graph::new(layout, adapters);
        let mut problems = Vec::new();
        let mut say = |station: Option<&str>, message: &str| problems.push(Problem {
            station: station.map(str::to_string), message: message.to_string() });
        if layout.schema != 3 || layout.name.is_empty() || layout.stations.is_empty() {
            say(None, "a layout is schema 3, named, with at least one station");
        }
        let mut ids = BTreeSet::new();
        for def in &layout.stations {
            if !ids.insert(def.id.clone()) { say(Some(&def.id), "station ids are unique"); }
            let mut inner = BTreeSet::new();
            for d in &def.stations {
                if !inner.insert(d.id.clone()) { say(Some(&d.id), "inner station ids are unique"); }
                if d.kind == Kind::Batch { say(Some(&d.id), "a batch station cannot sit inside another"); }
            }
            if (def.kind == Kind::Batch) == def.stations.is_empty() {
                say(Some(&def.id), "a batch station, and only a batch station, has inner stations");
            }
        }
        for (key, use_) in &layout.adapters {
            match adapters.get(&use_.version) {
                None => say(None, &format!("adapter {key} names an unknown version {}", use_.version)),
                Some(rules) => if let Err(found) = rules.validate(layout, &use_.config) { problems_extend(&mut say, found) },
            }
        }
        for intake in &layout.intake {
            if !matches!(graph.resolve_top(&intake.to), Some(Target::Station(_))) { say(None, "intake names a station") }
        }
        let mut repeats: BTreeMap<String, Repeat> = BTreeMap::new();
        for id in &graph.order {
            let node = &graph.nodes[id];
            let def = &node.def;
            if def.id.is_empty() || def.id.contains(['/', '.']) || RESERVED.contains(&def.id.as_str())
                || !def.id.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)) {
                say(Some(id), "a station id is letters, digits, - and _, and not a reserved target");
            }
            let needs_op = matches!(def.kind, Kind::Agent | Kind::Command | Kind::Wait);
            match (&def.op, &node.spec) {
                (Some(_), None) => say(Some(id), "its op is not in its adapter's catalog"),
                (None, _) if needs_op => say(Some(id), "an agent, command or wait station names an op"),
                (Some(_), _) if !needs_op => say(Some(id), "a decision or batch station runs no op"),
                (Some(_), Some(spec)) if !spec.kinds.contains(&def.kind) => say(Some(id), "its op may not run under this kind"),
                _ => {}
            }
            if let Some(o) = node.spec.as_ref().and_then(|s| s.outward.as_ref()) {
                if *repeats.entry(o.boundary.clone()).or_insert(o.repeat) != o.repeat {
                    say(Some(id), "two operations share a boundary but differ in repeat");
                }
            }
            if let Some(pool) = &def.pool { if !pools.contains(pool) { say(Some(id), "its pool is not in the workspace's limits"); } }
            if def.kind == Kind::Decision && (def.deciders.is_empty() || !def.routes.contains_key("approved")) {
                say(Some(id), "a decision station has deciders and routes approved");
            }
            for raw in def.routes.values() {
                if graph.resolve(id, raw).is_none() { say(Some(id), &format!("route target {raw} is not a station here")); }
            }
        }
        // A `repeat: once` station can't be reached again from any of its crossing labels (4.1, 5.3).
        for id in &graph.order {
            let Some(o) = graph.nodes[id].spec.as_ref().and_then(|s| s.outward.clone()) else { continue };
            if o.repeat != Repeat::Once { continue; }
            for label in &o.crossing {
                if graph.target_reaches(id, graph.route(id, label), &o.boundary) {
                    say(Some(id), &format!("its crossing label {label} leads back to boundary {}", o.boundary));
                }
            }
        }
        if problems.is_empty() { Ok(graph) } else { Err(problems) }
    }
    fn resolve_top(&self, raw: &str) -> Option<Target> {
        self.nodes.get(raw).filter(|n| n.parent.is_none()).map(|_| Target::Station(raw.to_string()))
    }
    /// Whether work sent from `station` to `target` can reach a station on `boundary`.
    pub fn target_reaches(&self, station: &str, target: Option<Target>, boundary: &str) -> bool {
        self.exits(station, target).iter().any(|s| self.leads_to(s, boundary))
    }
}
fn problems_extend(say: &mut impl FnMut(Option<&str>, &str), found: Vec<Problem>) {
    for p in found { say(p.station.as_deref(), &p.message); }
}
/// Validation as a refusal, for the reducer.
pub fn check(layout: &Layout, adapters: &Adapters, pools: &BTreeSet<String>) -> Result<Graph, Refusal> {
    Graph::validate(layout, adapters, pools).map_err(|_| Refusal::Definition)
}
