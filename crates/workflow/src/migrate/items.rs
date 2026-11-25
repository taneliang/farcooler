//! Items: tickets become tokens, live groups become batches (docs/engine-pipeline.md 10.1, 11.1, 11.2).
use std::collections::BTreeSet;
use crate::adapters::coding::{self, Data};
use crate::pipeline::{self as p, HoldKind, Kind, Location as L};
use crate::{Item, Phase, StageKind};
use super::{closing, DispatchState, Migration, Quarantine, Rank};

/// How a ticket stands to a group it was queued into.
enum Member { None, In { batch: String, station: String, crossings: Vec<p::Crossing> }, Waiting { station: String, rank: Rank } }

impl Migration<'_> {
    pub(super) fn items(&mut self) {
        let state = self.state;
        let mut placed = BTreeSet::new();
        for (id, g) in state.items.iter().filter(|(_, i)| i.group_of.is_some()) { self.group(id, g, &mut placed); }
        for (id, item) in state.items.iter().filter(|(id, i)| i.group_of.is_none() && !placed.contains(*id)) {
            self.ticket(id, item, Member::None);
        }
    }
    /// The fields with no place in the pipeline. Every field is named, so a new one fails to build until
    /// `fields::ITEM_FIELDS` and this list say what becomes of it.
    fn placeable(&mut self, id: &str, item: &Item) -> bool {
        let Item { start_load: _, execution, definition: _, pin: _, policy_pin: _, dependencies: _, resource: _, stage: _, iteration: _,
            attempt: _, fence: _, phase, run: _, pending: _, candidate: _, last_evidence: _, stopping: _, rework_history: _,
            back_edges: _, retries: _, members, group_of, pushed, handed, since_ms: _, grouped_in: _, wait_since: _, models: _,
            tree: _, sent_back: _, brief: _, follows, followers, cancel_reason: _, closed: _, failure: _, ci_reruns: _,
            ci_retry: _, deciding, hook_calls: _, hook_refused, queue_hold: _, group_failure: _, landed: _, restart: _, reuse_checkout, carried_progress: _, history: _, reason: _, landing_ci: _, stopped_from: _ } = item;
        let before = self.refused.len();
        if *reuse_checkout { self.refuse(id, "it still needs to reuse its checkout after a workflow publication"); }
        if execution.is_some() { self.refuse(id, "it carries a read-time execution view, which is never stored"); }
        if follows.is_some() || !followers.is_empty() { self.refuse(id, "it follows or carries another ticket's run, and the pipeline has no followers yet"); }
        if deciding.is_some() { self.refuse(id, "a decision script is running for it, and the pipeline has no scripts"); }
        if hook_refused.is_some() { self.refuse(id, "a script refused its decision, and the pipeline has no scripts"); }
        if matches!(phase, Phase::WaitingSlot | Phase::Starting | Phase::Stalled) { self.refuse(id, format!("{phase:?} is a read-time phase, never stored")); }
        if group_of.is_none() && (!members.is_empty() || pushed.is_some() || !handed.is_empty()) {
            self.refuse(id, "a ticket carries a group's members or push");
        }
        self.refused.len() == before
    }

    fn ticket(&mut self, id: &str, item: &Item, member: Member) {
        if !self.placeable(id, item) { return; }
        if item.definition.is_ticket() {
            // A ticket at rest: its column is its step. Its candidate is the tracker template's pin, which
            // nothing reads, so it isn't carried.
            return match item.phase {
                Phase::WaitingApproval => {
                    let column = item.definition.stages[item.stage].id.clone();
                    let mut t = self.token(item, None, Vec::new());
                    t.data.clear();
                    t.at = L::Resting { column };
                    self.p.tokens.insert(id.into(), t);
                }
                Phase::Done | Phase::Cancelled => self.close(id, item),
                ref other => self.refuse(id, format!("a ticket at rest is {other:?}")),
            };
        }
        if matches!(item.phase, Phase::Done | Phase::Cancelled) { return self.close(id, item); }
        let Some(stage) = item.definition.stages.get(item.stage) else { return self.refuse(id, "its step is past its definition's end") };
        let step = stage.id.clone();
        if self.graph.node(&step).is_none_or(|n| n.parent.is_some()) {
            return self.refuse(id, format!("its step {step} has no station in the layout"));
        }
        let crossings = self.landed_before(item);
        match member {
            Member::None if item.phase == Phase::Joined => self.refuse(id, "it is joined to a group that isn't live"),
            Member::None => {
                let Some(at) = self.locate(id, item, &step, false) else { return };
                let mut t = self.token(item, Some(&step), crossings);
                t.at = at;
                self.occupy(id, &t.at.clone());
                self.p.tokens.insert(id.into(), t);
            }
            Member::In { batch, station, crossings: more } => {
                let mut all = crossings;
                all.extend(more);
                let mut t = self.token(item, Some(&station), all);
                t.at = L::InBatch { batch };
                if let Some(v) = t.visits.last_mut() { v.started = Some(self.state.revision); }
                self.p.tokens.insert(id.into(), t);
            }
            Member::Waiting { station, rank } => {
                let mut t = self.token(item, Some(&station), crossings);
                t.at = L::Queued { station };
                self.ranks.insert(id.into(), rank);
                self.p.tokens.insert(id.into(), t);
            }
        }
    }
    /// A closed item the old reducer hadn't archived yet (it archives on the next step): archive it now.
    fn close(&mut self, id: &str, item: &Item) {
        let closed = if item.phase == Phase::Done { p::Closure::Done } else { p::Closure::Cancelled };
        let t = self.token(item, None, self.landed_before(item));
        let mut d = coding::data_of(&t.data).unwrap_or_default();
        d.pushed = item.landed.clone().or(d.pushed);
        let data = self.data(&d);
        self.p.archive.insert(id.into(), p::Archived { closed, reason: closing(&item.closed, item.cancel_reason.as_ref()),
            crossings: t.crossings, batch: None, data, visits: t.visits });
    }
    /// A token's fields from an item. `station` is where it is now, for its open visit.
    fn token(&self, item: &Item, station: Option<&str>, crossings: Vec<p::Crossing>) -> p::Token {
        let rerun = item.ci_retry.as_ref().filter(|r| !r.pending).map(|r| r.run.clone());
        let d = Data { tree: item.tree.clone(), candidate: item.candidate.clone(), ci_retry: rerun, ..Data::default() };
        p::Token { at: L::Closed { crossed: false }, priority: 0, data: self.data(&d), evidence: item.last_evidence.as_ref().map(evidence),
            crossings, fresh_from: 0, brief: item.brief.clone(), failure: item.failure.clone(), models: item.models.clone(),
            dependencies: item.dependencies.clone(), visits: self.visits(item, "", station), fence: item.fence }
    }
    /// A ticket crossed the target if its definition lands before the step it is at: a merged pull request.
    fn landed_before(&self, item: &Item) -> Vec<p::Crossing> {
        let end = item.stage.min(item.definition.stages.len());
        item.definition.stages[..end].iter().rev().find(|s| crate::lands(&s.kind))
            .map(|s| self.crossing(&s.id, item.candidate.as_deref().unwrap_or_default(),
                item.last_evidence.as_ref().map_or("", |e| e.receipt.as_str())))
            .into_iter().collect()
    }
    /// History as visits: each back edge taken, ended with the label the station sends back with (so its rework
    /// or returns policy counts it), each send-back, and an open visit where it is now. Their order across
    /// stations isn't known; their counts are.
    fn visits(&self, item: &Item, prefix: &str, station: Option<&str>) -> Vec<p::Visit> {
        let rev = self.state.revision;
        let visit = |station: String, ended: Option<String>, by: &str, reason: Option<String>| p::Visit { station, arrived: rev,
            started: None, left: ended.as_ref().map(|_| rev), ended, by: by.into(), reason, overrode: None };
        let mut out = Vec::new();
        for (step, n) in &item.back_edges {
            let full = format!("{prefix}{step}");
            if let Some(label) = self.back_label(&full) {
                out.extend((0..*n).map(|_| visit(full.clone(), Some(label.clone()), "migrate", None)));
            }
        }
        for s in &item.sent_back { out.push(visit(format!("{prefix}{}", s.from), Some("sent".into()), "orchestrator", Some(s.reason.clone()))); }
        if let Some(st) = station { out.push(visit(st.into(), None, "migrate", Some("the cutover from the old engine".into()))); }
        out
    }
    /// The one label `station` sends work back with, to itself or an earlier sibling.
    fn back_label(&self, station: &str) -> Option<String> {
        let node = self.graph.node(station)?;
        let back: Vec<&String> = node.def.routes.keys().filter(|l| match self.graph.route(station, l) {
            Some(p::Target::Station(t)) => self.graph.node(&t).is_some_and(|n| n.parent == node.parent && n.index <= node.index),
            _ => false,
        }).collect();
        (back.len() == 1).then(|| back[0].clone())
    }

    /// Where an item at `station` stands, by its phase and its pending effect's outbox row (10.1, 11.2).
    /// `None` when it refuses; a quarantined item is held and listed.
    fn locate(&mut self, id: &str, item: &Item, station: &str, batch: bool) -> Option<L> {
        let node = self.graph.node(station).expect("located at a live station").clone();
        let kind = node.def.kind;
        let outward = node.spec.as_ref().is_some_and(|s| s.outward.is_some());
        let row = item.pending.as_ref().and_then(|e| self.outbox.get(e)).map(|(_, s)| *s);
        let decision = || L::Awaiting { station: station.into(), decision: p::Decision { run: item.run.clone(),
            deciders: node.def.deciders.clone(), needs: node.def.needs.unwrap_or(1).max(1), given: Vec::new() } };
        let failed = |why: &str| item.failure.clone().unwrap_or_else(|| why.to_string());
        Some(match item.phase {
            Phase::Blocked | Phase::WaitingJoin if matches!(kind, Kind::Agent | Kind::Command | Kind::Batch)
                && (kind == Kind::Batch) == (item.phase == Phase::WaitingJoin) => L::Queued { station: station.into() },
            Phase::Blocked => self.quarantine(id, station, "it waits to begin a step that can't queue: holding it lets a person release it"),
            Phase::WaitingApproval | Phase::WaitingReview if kind == Kind::Decision => decision(),
            Phase::Running | Phase::Uncertain | Phase::Cancelling if node.spec.is_none() => {
                self.refuse(id, format!("it is {:?} at {station}, which runs nothing", item.phase)); return None
            }
            Phase::Running | Phase::Uncertain if item.pending.is_none() => {
                self.refuse(id, format!("it is {:?} with no effect", item.phase)); return None
            }
            Phase::Running if item.ci_retry.as_ref().is_some_and(|r| r.pending) =>
                self.quarantine(id, station, "a CI rerun was being requested, and whether it went out is unknown"),
            Phase::Running => match row {
                Some(DispatchState::Admitted) => L::Working { station: station.into(), work: self.work(item, &node, false) },
                Some(DispatchState::Uncertain) => L::Working { station: station.into(), work: self.work(item, &node, true) },
                Some(DispatchState::Pending | DispatchState::Claimed) if kind != Kind::Wait => {
                    self.never_started(id, item);
                    L::Queued { station: station.into() }
                }
                Some(DispatchState::Pending | DispatchState::Claimed) => {
                    self.never_started(id, item);
                    self.quarantine(id, station, "its wait never began, and a wait begins only when work arrives")
                }
                other => self.quarantine(id, station, &format!("its effect's outbox row is {}", row_name(other))),
            },
            Phase::Uncertain if item.stopping.is_some() => self.quarantine(id, station, "its stop's answer was unknown"),
            Phase::Uncertain => match row {
                Some(DispatchState::Admitted | DispatchState::Uncertain) => L::Working { station: station.into(), work: self.work(item, &node, true) },
                other => self.quarantine(id, station, &format!("its answer was unknown and its outbox row is {}", row_name(other))),
            },
            Phase::Cancelling if batch => { self.refuse(id, "a group is stopping, and the pipeline closes tokens, not batches"); return None }
            Phase::Cancelling if matches!(item.closed, Some(crate::CloseResolution::Landed { .. })) => {
                self.refuse(id, "it was closed as landed and waits for its stop, but a stopped token closes cancelled"); return None
            }
            Phase::Cancelling => {
                let (Some(stop), Some(original)) = (item.pending.clone(), item.stopping.clone()) else {
                    self.refuse(id, "it is cancelling with no stop effect"); return None
                };
                let mut work = self.work(item, &node, false);
                work.pending = Some(stop.clone());
                work.stopping = Some(original);
                work.run = format!("stop:{stop}");
                let then = Box::new(p::Then::Close { by: "orchestrator".into(), reason: closing(&item.closed, item.cancel_reason.as_ref()) });
                if outward { L::Settling { station: station.into(), work, then } } else { L::Stopping { station: station.into(), work, then } }
            }
            Phase::Failed => self.hold(station, HoldKind::StepFailed, &failed("it failed under the old engine")),
            ref other => { self.refuse(id, format!("{other:?} has no place at {station}")); return None }
        })
    }
    fn never_started(&mut self, id: &str, item: &Item) {
        let effect = item.pending.clone().expect("running work has an effect");
        self.ranks.insert(id.into(), Rank::Outbox(self.outbox[&effect].0));
        self.suppress.push(effect);
    }
    fn hold(&self, station: &str, kind: HoldKind, reason: &str) -> L {
        L::Held { station: station.into(), hold: p::Hold { reason: reason.into(), by: "migrate".into(),
            revision: self.state.revision, kind, then: None } }
    }
    fn quarantine(&mut self, id: &str, station: &str, reason: &str) -> L {
        self.quarantined.push(Quarantine { item: id.into(), reason: reason.into() });
        self.hold(station, HoldKind::Quarantined, reason)
    }
    /// The running step: its effect is the one whose answer binds, so `run` and `pending` name it.
    fn work(&self, item: &Item, node: &p::Node, uncertain: bool) -> p::Work {
        let effect = item.pending.clone().unwrap_or_default();
        let spec = node.spec.clone().expect("a running station has an operation");
        let ci = node.def.op.as_deref().is_some_and(|o| o.ends_with(".ci"));
        let again = item.ci_retry.as_ref().filter(|r| ci && !r.pending).map(|r| coding::opaque(&r.run));
        let reruns = node.def.id.as_str();
        p::Work { run: effect.clone(), attempt: item.attempt, pending: (!uncertain).then_some(effect),
            op: p::OpRef { adapter: node.adapter.clone().unwrap_or_default(), op: spec.name.clone(),
                spec: crate::digest(&item.definition.stages[item.stage].operation) },
            stopping: None, uncertain, wait_since: if node.def.kind == Kind::Wait { item.wait_since } else { None },
            again, reruns: if ci { item.ci_reruns.get(reruns).copied().unwrap_or(0) } else { 0 },
            round: spec.outward.as_ref().map(|_| 1), resources: spec.resources.clone() }
    }
    /// Running work holds its station's place and its pool's, as `begin` takes them; a wait holds neither.
    fn occupy(&mut self, key: &str, at: &L) {
        let (L::Working { station, .. } | L::Stopping { station, .. } | L::Settling { station, .. }) = at else { return };
        let node = &self.graph.nodes[station];
        if node.def.kind == Kind::Wait { return; }
        let pool = node.def.pool.clone();
        self.p.stations.get_mut(station).expect("a live station").working.insert(key.into());
        if let Some(pl) = pool.and_then(|pool| self.p.pools.get_mut(&pool)) { pl.held_by.insert(key.into()); }
    }

    /// A group item: a live one becomes a batch at its members' join station, a waiting one dissolves into that
    /// station's queue (a batch forms when the landing has room, as the old group would have started), and a
    /// finished one is archived.
    fn group(&mut self, gid: &str, g: &Item, placed: &mut BTreeSet<String>) {
        if !self.placeable(gid, g) { return; }
        let name = g.group_of.as_ref().expect("a group").name.clone();
        let since = g.since_ms;
        let archive = |m: &mut Self, closed: p::Closure, reason: Option<String>| {
            let crossings: Vec<p::Crossing> = g.pushed.iter().map(|sha| m.crossing(&format!("{name}.push"), sha, "")).collect();
            let data = m.data(&Data { candidate: g.candidate.clone(), pushed: g.pushed.clone(), failure: g.group_failure.clone(), ..Data::default() });
            m.p.archive.insert(gid.into(), p::Archived { closed, reason, crossings, data, visits: Vec::new(),
                batch: Some(p::ArchivedBatch { name: name.clone(), members: g.handed.clone(), since_ms: since }) });
        };
        if g.members.is_empty() {
            return match g.phase {
                Phase::Done => archive(self, p::Closure::Done, None),
                // A failed group already handed its tickets back; what's left is its record.
                Phase::Failed | Phase::Cancelled => archive(self, p::Closure::Cancelled, g.failure.clone()),
                ref other => self.refuse(gid, format!("a group with no tickets is {other:?}")),
            };
        }
        let joins: BTreeSet<String> = g.members.iter().filter_map(|m| self.state.items.get(&m.item)).filter_map(|t| {
            t.definition.stages.iter().find(|s| matches!(&s.kind, StageKind::Join { group, .. } if *group == name)).map(|s| s.id.clone())
        }).collect();
        let station = match (joins.len(), joins.first()) {
            (1, Some(s)) if self.graph.node(s).is_some_and(|n| n.def.kind == Kind::Batch && n.parent.is_none()) => s.clone(),
            _ => return self.refuse(gid, format!("its tickets' join step for {name} is not one batch station in the layout")),
        };
        if g.members.iter().any(|m| !self.state.items.get(&m.item).is_some_and(|t| t.grouped_in.as_deref() == Some(gid))) {
            return self.refuse(gid, "a ticket it names isn't queued into it");
        }
        placed.extend(g.members.iter().map(|m| m.item.clone()));
        let waiting = g.phase == Phase::Blocked && g.stage == 0 && g.candidate.is_none() && g.pushed.is_none() && g.pending.is_none();
        let state = self.state;
        if waiting {
            for (i, m) in g.members.iter().enumerate() {
                self.ticket(&m.item, &state.items[&m.item], Member::Waiting { station: station.clone(), rank: Rank::Group(gid.into(), i) });
            }
            return;
        }
        let Some(step) = g.definition.stages.get(g.stage).map(|s| s.id.clone()) else { return self.refuse(gid, "its step is past its definition's end") };
        let inner = format!("{station}.{step}");
        if self.graph.node(&inner).is_none() { return self.refuse(gid, format!("its step {step} has no station inside {station}")); }
        if matches!(g.phase, Phase::Failed) { return self.refuse(gid, "a failed group still holds tickets"); }
        let Some(at) = self.locate(gid, g, &inner, true) else { return };
        let crossings: Vec<p::Crossing> = g.pushed.iter().map(|sha| self.crossing(&format!("{station}.push"), sha,
            g.last_evidence.as_ref().map_or("", |e| e.receipt.as_str()))).collect();
        let mut members = Vec::new();
        for m in &g.members {
            self.ticket(&m.item, &state.items[&m.item], Member::In { batch: gid.into(), station: station.clone(), crossings: crossings.clone() });
            let mut d = self.p.tokens.get(&m.item).map(|t| coding::data_of(&t.data).unwrap_or_default()).unwrap_or_default();
            d.candidate = Some(m.candidate.clone());
            members.push(p::Membership { token: m.item.clone(), joined: self.mentioned.get(&m.item).copied().unwrap_or(self.state.revision),
                data: self.data(&d) });
        }
        // What the gates passed: a group past its last gate pushes the candidate it has (definition.rs: every
        // path into a push passes a gate after the last change).
        let last_gate = g.definition.stages.iter().rposition(|s| matches!(s.kind, StageKind::Run | StageKind::Check));
        let gated = last_gate.is_some_and(|i| g.stage > i).then(|| g.candidate.clone()).flatten();
        let base = (g.definition.stages[g.stage].kind == StageKind::Fold).then(|| g.candidate.clone()).flatten();
        let rerun = g.ci_retry.as_ref().filter(|r| !r.pending).map(|r| r.run.clone());
        let data = self.data(&Data { candidate: g.candidate.clone(), base, gated, pushed: g.pushed.clone(),
            failure: g.group_failure.clone(), ci_retry: rerun, ..Data::default() });
        let mut visits = self.visits(g, &format!("{station}."), Some(&inner));
        if matches!(at, L::Working { .. } | L::Stopping { .. } | L::Settling { .. } | L::Awaiting { .. }) {
            if let Some(v) = visits.last_mut() { v.started = Some(self.state.revision); }
        }
        self.occupy(gid, &at);
        let node = &self.graph.nodes[&station];
        let frees = node.def.form.as_ref().is_some_and(|f| f.free_on_crossing) && g.pushed.is_some();
        if !frees {
            let pool = node.def.pool.clone();
            self.p.stations.get_mut(&station).expect("a live batch station").working.insert(gid.into());
            if let Some(pl) = pool.and_then(|pool| self.p.pools.get_mut(&pool)) { pl.held_by.insert(gid.into()); }
        }
        self.p.batches.insert(gid.into(), p::Batch { station, at, members, data, crossings, suspects: Vec::new(), bisect: None,
            visits, fence: g.fence, since_ms: since });
    }
}
fn evidence(e: &crate::Evidence) -> p::Evidence {
    p::Evidence { artifact: e.candidate.clone(), manifest: e.manifest.clone(), receipt: e.receipt.clone(), spec: e.policy_pin.clone() }
}
fn row_name(row: Option<DispatchState>) -> String {
    row.map_or_else(|| "missing".into(), |s| format!("{s:?}").to_lowercase())
}
