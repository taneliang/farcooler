//! `migrate`: the old engine's state onto the pipeline (docs/engine-pipeline.md 10, step 2). Pure and unwired.
//!
//! It reads the old `State`, the old log (only for queue order), each pending effect's outbox dispatch status,
//! and the layout and pools the workspace cuts over to, and returns the pipeline the cutover stores as its
//! snapshot. Every item lands on that one layout, whatever pin it ran under, at the station with its step's
//! id (10.1). Work in flight keeps going: admitted work keeps its effect, fence and place; work that never
//! started queues in outbox order, and its outbox row is listed for suppression. Nothing is guessed:
//! - an item the pipeline can hold but not run (an effect whose fate is unknown, a wait that never began) is
//!   held `Quarantined` at its station and listed in `Migrated::quarantined`;
//! - an item that has no faithful place at all (a step with no station, followers, a running script, a
//!   read-time phase) refuses the whole migration, each reason naming the item.
//!
//! What each `Item` field becomes is `fields::ITEM_FIELDS`. The coding facts (candidate, tree, what was
//! pushed) go into `coding/v1`'s data, keyed as the layout keys that adapter.
mod fields;
mod items;
pub use fields::{Decision, ITEM_FIELDS};

use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};
use crate::adapters::coding::{self, Data};
use crate::adapters::linked;
use crate::pipeline::{self as p, Graph, Layout, Opaque};
use crate::{CloseResolution, Command, Input, Phase, State};

/// The layout and pools a workspace cuts over to (section 6: from today's caps).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Setup { pub layout: Layout, pub pools: BTreeMap<String, u16> }
/// An outbox row's `dispatch_state` (store/src/workflow.rs), in outbox order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DispatchState { Pending, Claimed, Admitted, Uncertain, Settled, Suppressed }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dispatch { pub effect: String, pub state: DispatchState }
/// One item and why it is held, or why the migration is refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quarantine { pub item: String, pub reason: String }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Migrated {
    pub pipeline: p::Pipeline,
    /// Items held `Quarantined` in the pipeline, each with its reason.
    pub quarantined: Vec<Quarantine>,
    /// Effects of work that never started: the cutover marks their outbox rows `suppressed` (10.1).
    pub suppress: Vec<String>,
}

/// The old state on the new pipeline, or every reason it can't be.
pub fn migrate(state: &State, log: &[Input], outbox: &[Dispatch], setup: &Setup) -> Result<Migrated, Vec<Quarantine>> {
    let refuse = |item: &str, reason: &str| Err(vec![Quarantine { item: item.into(), reason: reason.into() }]);
    if state.schema != 1 { return refuse("state", "only a schema 1 state migrates"); }
    let adapters = linked();
    let Some((key, config)) = setup.layout.adapters.iter().find(|(_, u)| u.version == coding::VERSION)
        .map(|(k, u)| (k.clone(), u.config.clone())) else { return refuse("layout", "the layout doesn't use coding/v1") };
    let now = log.last().map_or(0, |i| i.now_ms);
    let pipeline = published(state, setup, now).map_err(|r| vec![Quarantine { item: "layout".into(),
        reason: format!("the layout doesn't publish: {r:?}") }])?;
    let mut m = Migration { graph: Graph::new(&setup.layout, &adapters), p: pipeline, key, boundary: coding::boundary(&config),
        state, outbox: outbox.iter().enumerate().map(|(i, d)| (d.effect.clone(), (i, d.state))).collect(),
        mentioned: mentions(log), ranks: BTreeMap::new(), quarantined: Vec::new(), refused: Vec::new(), suppress: Vec::new() };
    m.items();
    m.archive();
    if !m.refused.is_empty() { return Err(m.refused); }
    m.queues();
    let outbox = &m.outbox;
    m.suppress.sort_by_key(|e| outbox[e].0);
    Ok(Migrated { pipeline: m.p, quarantined: m.quarantined, suppress: m.suppress })
}

/// An empty pipeline with the layout published, carrying the old state's revision, inputs seen and sources.
/// Publishing goes through `step`, so the layout is validated and the stations are made exactly as a publish
/// makes them; the publish's own input is then taken back out, as `Genesis` is the next event (10.2).
fn published(state: &State, setup: &Setup, now: i64) -> Result<p::Pipeline, p::Refusal> {
    let empty = p::Pipeline::new(&state.workspace, state.epoch, &setup.pools);
    let input = p::Input { schema: 3, id: "migrate".into(), workspace: state.workspace.clone(), expected_revision: 0,
        epoch: state.epoch, source: "migrate".into(), source_sequence: 1, now_ms: now, grant: p::Grant::Orchestrator,
        command: p::Command::Publish { layout: setup.layout.clone(), moves: BTreeMap::new(), after: BTreeMap::new(),
            reason: "the cutover from the old engine".into() },
        busy_resources: Default::default(), host: BTreeMap::new() };
    let mut out = p::step(&empty, &input, &linked())?.state;
    out.revision = state.revision;
    out.seen = state.seen.clone();
    out.source_sequences = state.source_sequences.clone();
    Ok(out)
}
/// The revision each item was last named at, from the old log: the order work arrived in a queue.
fn mentions(log: &[Input]) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for input in log {
        let item = match &input.command {
            Command::Decide { item, .. } | Command::Enqueue { item, .. } | Command::Approve { item, .. }
            | Command::Review { item, .. } | Command::Observe { item, .. } | Command::Cancel { item, .. }
            | Command::Close { item, .. } | Command::Retry { item } | Command::Queue { item } | Command::Leave { item }
            | Command::Move { item, .. } | Command::Start { item, .. } | Command::Resolve { item, .. }
            | Command::SendBack { item, .. } | Command::Land { item, .. } | Command::Reopen { item, .. }
            | Command::RefreshJoin { item, result: crate::JoinRefresh::Conflict { .. }, .. } => item,
            Command::HoldQueue { .. } | Command::Advance | Command::RefreshGroups { .. } | Command::RepinPending { .. } | Command::AdoptPublication { .. }
            | Command::RefreshJoin { result: crate::JoinRefresh::Rebased { .. }, .. } => continue,
        };
        out.insert(item.clone(), input.expected_revision);
    }
    out
}

/// Where a queued key sorts: work whose effect never started, in outbox order; then the members of a group
/// that hadn't left, in the group's order; then everything else by when the log last named it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Rank { Outbox(usize), Group(String, usize), Log(u64, String) }

struct Migration<'s> {
    graph: Graph,
    p: p::Pipeline,
    /// The layout's key for `coding/v1`, and the boundary its outward steps cross.
    key: String,
    boundary: String,
    state: &'s State,
    outbox: BTreeMap<String, (usize, DispatchState)>,
    mentioned: BTreeMap<String, u64>,
    ranks: BTreeMap<String, Rank>,
    quarantined: Vec<Quarantine>,
    refused: Vec<Quarantine>,
    suppress: Vec<String>,
}

impl Migration<'_> {
    fn refuse(&mut self, item: &str, reason: impl Into<String>) {
        self.refused.push(Quarantine { item: item.into(), reason: reason.into() });
    }
    fn data(&self, d: &Data) -> BTreeMap<String, Opaque> {
        if *d == Data::default() { BTreeMap::new() } else { BTreeMap::from([(self.key.clone(), coding::opaque(d))]) }
    }
    fn crossing(&self, station: &str, artifact: &str, receipt: &str) -> p::Crossing {
        p::Crossing { boundary: self.boundary.clone(), round: 1, effect: String::new(), station: station.into(),
            artifact: artifact.into(), receipt: receipt.into(), revision: self.state.revision }
    }
    /// The old archive: closed tickets and landed groups keep their keys, with the new `Archived` shape.
    fn archive(&mut self) {
        for (id, a) in &self.state.archive {
            let closed = match a.phase {
                Phase::Done => p::Closure::Done,
                Phase::Cancelled => p::Closure::Cancelled,
                _ => { self.refuse(id, format!("an archive entry is {:?}, which is neither done nor cancelled", a.phase)); continue }
            };
            let pushed = a.group.as_ref().and_then(|g| g.pushed.clone());
            let crossings = match (&a.group, &pushed) {
                (Some(g), Some(sha)) => vec![self.crossing(&format!("{}.push", g.name), sha, "")],
                _ => Vec::new(),
            };
            let data = self.data(&Data { candidate: a.candidate.clone(), pushed, ..Data::default() });
            let batch = a.group.as_ref().map(|g| p::ArchivedBatch { name: g.name.clone(), members: g.tickets.clone(), since_ms: g.since_ms });
            self.p.archive.insert(id.clone(), p::Archived { closed, reason: closing(&a.closed, None), crossings, batch, data, visits: Vec::new() });
        }
    }
    /// Each station's queue in rank order, now that every token and batch has its place.
    fn queues(&mut self) {
        let mut queued: BTreeMap<String, Vec<(Rank, String)>> = BTreeMap::new();
        let at = self.p.tokens.iter().map(|(k, t)| (k, &t.at)).chain(self.p.batches.iter().map(|(k, b)| (k, &b.at)));
        for (k, loc) in at {
            if let p::Location::Queued { station } = loc {
                let rank = self.ranks.get(k).cloned().unwrap_or_else(|| Rank::Log(self.mentioned.get(k).copied().unwrap_or(0), k.clone()));
                queued.entry(station.clone()).or_default().push((rank, k.clone()));
            }
        }
        for (station, mut keys) in queued {
            keys.sort();
            self.p.stations.get_mut(&station).expect("queued at a live station").queue = keys.into_iter().map(|(_, k)| k).collect();
        }
    }
}
/// Why a closed item closed, in a sentence: the cancel's reason, or how a controller closed it.
fn closing(closed: &Option<CloseResolution>, cancel: Option<&String>) -> Option<String> {
    match closed {
        Some(CloseResolution::Landed { sha }) => Some(format!("closed as landed in {sha}")),
        Some(CloseResolution::Continued { item }) => Some(format!("continued in {item}")),
        None => cancel.cloned(),
    }
}

/// A kept effect (an admitted one, still running) in the new shape, so the runtime reconciles it as today and
/// its answer binds to the migrated work (11.1, `Effect`). `p` is the migrated pipeline.
pub fn effect(old: &crate::Effect, p: &p::Pipeline) -> Result<p::Effect, Quarantine> {
    let refuse = |reason: &str| Quarantine { item: old.item.clone(), reason: reason.into() };
    let (subject, at, fence) = match (p.tokens.get(&old.item), p.batches.get(&old.item)) {
        (Some(t), _) => (p::Subject::Token(old.item.clone()), &t.at, t.fence),
        (_, Some(b)) => (p::Subject::Batch(old.item.clone()), &b.at, b.fence),
        _ => return Err(refuse("the effect's item is not live in the pipeline")),
    };
    let (station, work) = match at {
        p::Location::Working { station, work } | p::Location::Stopping { station, work, .. }
        | p::Location::Settling { station, work, .. } => (station, work),
        _ => return Err(refuse("the effect's item is not working")),
    };
    let bound = work.pending.as_deref() == Some(old.id.as_str()) || (work.uncertain && work.run == old.id);
    if !bound { return Err(refuse("the effect is not the one its item's work waits on")); }
    let (_, check) = coding::Op::parse(&work.op.op).ok_or_else(|| refuse("the effect's station runs no coding operation"))?;
    let kind = match &old.kind {
        crate::EffectKind::Stop { operation } => p::EffectKind::Stop { operation: operation.clone() },
        crate::EffectKind::Execute(k) if crate::is_wait(k) => p::EffectKind::Probe(work.op.clone()),
        crate::EffectKind::Execute(_) => p::EffectKind::Execute(work.op.clone()),
        crate::EffectKind::Decide { .. } => return Err(refuse("a decision script's effect has no place in the pipeline")),
    };
    let idempotency = match work.round {
        Some(round) if !matches!(kind, p::EffectKind::Stop { .. }) => format!("{}:{}:{round}", old.item, coding_boundary(p, &work.op.adapter)),
        _ => old.id.clone(),
    };
    let data = coding::Work { op: work.op.op.split(':').next().unwrap_or_default().into(), check, tree: old.tree.clone(),
        candidate: old.candidate.clone(), base: None,
        members: old.members.iter().map(|m| coding::Member { token: m.item.clone(), candidate: m.candidate.clone() }).collect(),
        ci_retry: old.ci_retry.as_ref().map(|r| r.run.clone()) };
    Ok(p::Effect { id: old.id.clone(), subject, station: station.clone(), run: old.run.clone(), epoch: old.epoch, fence,
        resources: work.resources.clone(), kind, created_ms: old.created_ms, model: old.model.clone(), brief: old.brief.clone(),
        members: old.members.iter().map(|m| m.item.clone()).collect(), idempotency,
        data: BTreeMap::from([(work.op.adapter.clone(), coding::opaque(&data))]) })
}
fn coding_boundary(p: &p::Pipeline, adapter: &str) -> String {
    let config = p.workflow.as_ref().and_then(|w| w.layout.adapters.get(adapter)).map(|u| u.config.clone()).unwrap_or_default();
    coding::boundary(&config)
}
