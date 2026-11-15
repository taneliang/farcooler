//! How work moves: arrivals, starts, answers, routing, holds and closing. Every function here is a step
//! of one accepted input, so it reads time only from the input and names things only from the state.
use std::collections::BTreeMap;
use super::adapter::{Adapters, View};
use super::layout::{Graph, Kind};
use super::step::{Input, Refusal};
use super::types::*;

pub(super) struct Ctx<'a, 'r> {
    pub p: Pipeline,
    pub graph: Graph,
    pub adapters: &'a Adapters<'r>,
    pub input: &'a Input,
    pub effects: Vec<Effect>,
}

impl<'a, 'r> Ctx<'a, 'r> {
    pub fn rev(&self) -> u64 { self.p.revision }
    pub fn now(&self) -> i64 { self.input.now_ms }
    pub fn next_id(&mut self, prefix: &str) -> String {
        self.p.sequence += 1;
        format!("{prefix}{}-{}", self.p.revision, self.p.sequence)
    }
    pub fn exists(&self, s: &Subject) -> bool {
        match s { Subject::Token(k) => self.p.tokens.contains_key(k), Subject::Batch(k) => self.p.batches.contains_key(k) }
    }
    pub fn at(&self, s: &Subject) -> &Location {
        match s { Subject::Token(k) => &self.p.tokens[k].at, Subject::Batch(k) => &self.p.batches[k].at }
    }
    pub fn set_at(&mut self, s: &Subject, at: Location) {
        match s { Subject::Token(k) => self.p.tokens.get_mut(k).unwrap().at = at,
            Subject::Batch(k) => self.p.batches.get_mut(k).unwrap().at = at }
    }
    pub fn visits_mut(&mut self, s: &Subject) -> &mut Vec<Visit> {
        match s { Subject::Token(k) => &mut self.p.tokens.get_mut(k).unwrap().visits,
            Subject::Batch(k) => &mut self.p.batches.get_mut(k).unwrap().visits }
    }
    pub fn fence(&self, s: &Subject) -> u64 {
        match s { Subject::Token(k) => self.p.tokens[k].fence, Subject::Batch(k) => self.p.batches[k].fence }
    }
    pub fn bump(&mut self, s: &Subject) -> u64 {
        let f = match s { Subject::Token(k) => &mut self.p.tokens.get_mut(k).unwrap().fence,
            Subject::Batch(k) => &mut self.p.batches.get_mut(k).unwrap().fence };
        *f += 1;
        *f
    }
    pub fn crossings(&self, s: &Subject) -> &[Crossing] {
        match s { Subject::Token(k) => &self.p.tokens[k].crossings, Subject::Batch(k) => &self.p.batches[k].crossings }
    }
    /// Crossings that still bind: a token's since its last `Reopen`, plus its batch's.
    pub fn binding_crossings(&self, s: &Subject) -> Vec<Crossing> {
        match s {
            Subject::Token(k) => {
                let t = &self.p.tokens[k];
                let mut out = t.crossings[t.fresh_from.min(t.crossings.len())..].to_vec();
                if let Location::InBatch { batch } = &t.at { out.extend(self.p.batches[batch].crossings.clone()); }
                out
            }
            Subject::Batch(k) => self.p.batches[k].crossings.clone(),
        }
    }
    pub fn priority(&self, s: &Subject) -> i32 {
        match s {
            Subject::Token(k) => self.p.tokens[k].priority,
            Subject::Batch(k) => self.p.batches[k].members.iter()
                .filter_map(|m| self.p.tokens.get(&m.token)).map(|t| t.priority).max().unwrap_or(0),
        }
    }
    pub fn subject_of(&self, key: &str) -> Subject {
        if self.p.batches.contains_key(key) { Subject::Batch(key.into()) } else { Subject::Token(key.into()) }
    }
    pub fn members(&self, s: &Subject, station: &str) -> Vec<String> {
        let Subject::Batch(k) = s else { return Vec::new() };
        let b = &self.p.batches[k];
        match &b.bisect { Some(x) if x.station == station => x.trying.clone(),
            _ => b.members.iter().map(|m| m.token.clone()).collect() }
    }
    pub fn view<'v>(&'v self, s: &'v Subject, station: &'v str, members: &'v [String]) -> View<'v> {
        match s {
            Subject::Token(k) => { let t = &self.p.tokens[k];
                View { subject: s, station, data: &t.data, evidence: t.evidence.as_ref(), crossings: &t.crossings,
                    visits: &t.visits, members } }
            Subject::Batch(k) => { let b = &self.p.batches[k];
                View { subject: s, station, data: &b.data, evidence: None, crossings: &b.crossings,
                    visits: &b.visits, members } }
        }
    }

    // ---- visits ----
    pub fn open_visit(&mut self, s: &Subject, station: &str, by: &str, reason: Option<String>) {
        let rev = self.rev();
        self.visits_mut(s).push(Visit { station: station.into(), arrived: rev, started: None, left: None,
            ended: None, by: by.into(), reason, overrode: None });
    }
    fn open(&mut self, s: &Subject) -> Option<&mut Visit> { self.visits_mut(s).iter_mut().rev().find(|v| v.left.is_none()) }
    pub fn mark_started(&mut self, s: &Subject) {
        let rev = self.rev();
        if let Some(v) = self.open(s) { if v.started.is_none() { v.started = Some(rev); } }
    }
    pub fn mark_ended(&mut self, s: &Subject, label: &str) {
        if let Some(v) = self.open(s) { v.ended = Some(label.into()); }
    }
    pub fn close_visit(&mut self, s: &Subject, ended: &str, reason: Option<String>, overrode: Option<String>) {
        let rev = self.rev();
        if let Some(v) = self.open(s) {
            v.left = Some(rev);
            if v.ended.is_none() || overrode.is_none() { v.ended = Some(ended.into()); }
            if reason.is_some() { v.reason = reason; }
            if overrode.is_some() { v.overrode = overrode; }
        }
    }

    // ---- places ----
    pub fn insert_queue(&mut self, station: &str, key: &str) {
        let p = self.priority(&self.subject_of(key));
        let queue = self.p.stations.get(station).map(|s| s.queue.clone()).unwrap_or_default();
        let at = queue.iter().position(|k| self.priority(&self.subject_of(k)) < p).unwrap_or(queue.len());
        let st = self.p.stations.get_mut(station).expect("arrivals name live stations");
        st.queue.retain(|k| k != key);
        st.queue.insert(at.min(st.queue.len()), key.into());
    }
    /// Give up every place `key` holds at `station`: its queue entry, its working slot and its pool place.
    pub fn free_place(&mut self, key: &str, station: &str) {
        let pool = self.graph.node(station).and_then(|n| n.def.pool.clone())
            .or_else(|| self.p.retired.get(station).and_then(|r| r.pool.clone()));
        if let Some(pl) = pool.and_then(|pool| self.p.pools.get_mut(&pool)) { pl.held_by.remove(key); }
        if let Some(st) = self.p.stations.get_mut(station) { st.queue.retain(|k| k != key); st.working.remove(key); }
        let mut emptied = false;
        if let Some(r) = self.p.retired.get_mut(station) { r.working.remove(key); emptied = r.working.is_empty(); }
        // A retired station is deleted once nothing runs in it; crossed work it can't place holds elsewhere.
        if emptied {
            self.p.retired.remove(station);
        }
    }

    /// A token or batch arrives at a live station: queued, awaiting a decision, or probing a wait.
    pub fn arrive(&mut self, s: &Subject, station: &str, by: &str, reason: Option<String>) {
        let kind = self.graph.node(station).map(|n| n.def.kind).expect("arrivals name live stations");
        self.open_visit(s, station, by, reason);
        match kind {
            Kind::Agent | Kind::Command | Kind::Batch => {
                self.set_at(s, Location::Queued { station: station.into() });
                self.insert_queue(station, s.key());
            }
            Kind::Decision => {
                let def = &self.graph.nodes[station].def;
                let decision = Decision { run: format!("decide:{station}:{}", self.rev()), deciders: def.deciders.clone(),
                    needs: def.needs.unwrap_or(1).max(1), given: Vec::new() };
                self.set_at(s, Location::Awaiting { station: station.into(), decision });
                self.mark_started(s);
            }
            Kind::Wait => { self.mark_started(s); self.begin(s, station, 1, 0, None); }
        }
    }
    /// Start the station's operation for a subject: one effect, a working location, and for agent and
    /// command stations a place. An adapter that refuses the start holds the work with its reason.
    pub fn begin(&mut self, s: &Subject, station: &str, attempt: u8, reruns: u8, again: Option<Opaque>) -> bool {
        let node = self.graph.nodes[station].clone();
        let (Some(adapter), Some(spec)) = (node.adapter.clone(), node.spec.clone()) else { return false };
        let use_ = self.p.workflow.as_ref().unwrap().layout.adapters[&adapter].clone();
        let rules = self.adapters.get(&use_.version).expect("published adapters are linked");
        let members = self.members(s, station);
        let plan = rules.start(&spec.name, &self.view(s, station, &members), &use_.config);
        let plan = match plan {
            Ok(plan) => plan,
            Err(refusal) => {
                self.hold(s, station, HoldKind::StepFailed, &format!("{adapter}.{} refused to start: {refusal:?}", spec.name), None);
                return false;
            }
        };
        let mut resources = spec.resources.clone();
        resources.extend(plan.resources.iter().cloned());
        resources.sort();
        resources.dedup();
        let round = spec.outward.as_ref().map(|o| self.crossings(s).iter().filter(|c| c.boundary == o.boundary)
            .map(|c| c.round).max().unwrap_or(0) + 1);
        let id = self.next_id("e");
        let fence = self.bump(s);
        let op = OpRef { adapter: adapter.clone(), op: spec.name.clone(), spec: plan.spec.clone() };
        let wait = node.def.kind == Kind::Wait;
        let work = Work { run: id.clone(), attempt, pending: Some(id.clone()), op: op.clone(), stopping: None,
            uncertain: false, wait_since: wait.then_some(self.now()), again: again.clone(), reruns, round,
            resources: resources.clone() };
        let idempotency = match (&spec.outward, round) {
            (Some(o), Some(r)) => format!("{}:{}:{r}", s.key(), o.boundary),
            _ => id.clone(),
        };
        let (model, brief) = match s {
            Subject::Token(k) => { let t = &self.p.tokens[k];
                (t.models.get(station).cloned().or_else(|| node.def.models.first().cloned()), t.brief.clone()) }
            Subject::Batch(_) => (node.def.models.first().cloned(), None),
        };
        let mut data = BTreeMap::new();
        if let Some(d) = plan.data { data.insert(adapter.clone(), d); }
        if let Some(a) = again { data.insert(format!("{adapter}:again"), a); }
        self.effects.push(Effect { id: id.clone(), subject: s.clone(), station: station.into(), run: id.clone(),
            epoch: self.p.epoch, fence, resources, kind: if wait { EffectKind::Probe(op) } else { EffectKind::Execute(op) },
            created_ms: self.now(), model: if node.def.kind == Kind::Agent { model } else { None }, brief,
            members, idempotency, data });
        if !wait {
            if let Some(st) = self.p.stations.get_mut(station) { st.queue.retain(|k| k != s.key()); st.working.insert(s.key().into()); }
            if let Some(pool) = &node.def.pool { if let Some(pl) = self.p.pools.get_mut(pool) { pl.held_by.insert(s.key().into()); } }
        }
        self.set_at(s, Location::Working { station: station.into(), work });
        self.mark_started(s);
        true
    }

    /// Stop work at its station with a reason; only a person or the orchestrator lets it go.
    pub fn hold(&mut self, s: &Subject, station: &str, kind: HoldKind, reason: &str, then: Option<Target>) {
        self.free_place(s.key(), station);
        let rev = self.rev();
        self.set_at(s, Location::Held { station: station.into(),
            hold: Hold { reason: reason.into(), by: "pipeline".into(), revision: rev, kind, then } });
    }
    pub fn close(&mut self, s: &Subject, closed: Closure, reason: Option<String>) {
        let Subject::Token(k) = s else { return };
        if let Some(st) = self.at(s).station().map(str::to_string) { self.free_place(k, &st); }
        let t = self.p.tokens.remove(k).unwrap();
        self.p.archive.insert(k.clone(), Archived { closed, reason, crossings: t.crossings, batch: None, data: t.data,
            visits: t.visits });
    }

    /// Leave `station` with `label`, by the routing rules of 4.1 (or a retired station's translation, 7.3).
    pub fn leave(&mut self, s: &Subject, station: &str, label: &str, automatic: bool) {
        if let Subject::Batch(b) = s {
            if self.p.batches[b].bisect.as_ref().is_some_and(|x| x.station == station) {
                return self.bisect_answer(b, station, label);
            }
        }
        if let Some(retired) = self.p.retired.get(station).cloned() {
            return self.leave_retired(s, station, &retired, label);
        }
        let target = self.graph.route(station, label);
        if automatic {
            if let Some(policy) = self.rework_limit(s, station, label, &target) {
                self.mark_ended(s, label);
                return self.hold(s, station, HoldKind::PolicyLimit { policy: policy.clone() },
                    &format!("{policy} reached: the station sent this back as often as its policy allows"), target);
            }
        }
        let Some(target) = target else {
            self.mark_ended(s, label);
            return self.hold(s, station, HoldKind::StepFailed, label, None);
        };
        self.close_visit(s, label, None, None);
        self.free_place(s.key(), station);
        self.go(s, station, target, label);
    }
    /// Apply a routing target from `station`, checking crossed work against `after` (7.3).
    pub fn go(&mut self, s: &Subject, station: &str, target: Target, label: &str) {
        match target {
            Target::Station(t) => self.place(s, &t, label),
            Target::Done => self.close(s, Closure::Done, None),
            Target::Finish => if let Subject::Batch(b) = s { self.finish(b) },
            Target::Rejected => if let Subject::Batch(b) = s { self.rejected(b, station, label) },
            Target::Returned => if let Subject::Batch(b) = s { self.returned(b, station, label) },
        }
    }
    /// Arrive at `t` unless crossed work would reach its boundary again from there (the migration rule).
    pub fn place(&mut self, s: &Subject, t: &str, label: &str) {
        match self.crossed_block(s, t) {
            None => self.arrive(s, t, "route", None),
            Some(boundary) => self.continue_after(s, t, &boundary, label, None),
        }
    }
    pub fn continue_after(&mut self, s: &Subject, t: &str, boundary: &str, label: &str, held: Option<Hold>) {
        let after = self.p.workflow.as_ref().and_then(|w| w.after.get(boundary).cloned());
        let passing = label == "passed" || label == "skipped";
        match after {
            Some(After::Station(a)) if self.crossed_block(s, &a).is_none() => {
                if passing && held.is_none() { self.arrive(s, &a, "publish", Some(format!("crossed {boundary}: resumes after it"))) }
                else {
                    self.open_visit(s, &a, "publish", Some(format!("crossed {boundary}: resumes after it")));
                    let reason = held.map_or(label.to_string(), |h| h.reason);
                    self.hold(s, &a, HoldKind::StepFailed, &reason, None);
                }
            }
            Some(After::Done) if passing => self.close(s, Closure::Done, Some(format!("crossed {boundary}"))),
            _ => {
                self.open_visit(s, t, "publish", None);
                self.hold(s, t, HoldKind::Crossed { boundary: boundary.into() },
                    &format!("crossed {boundary}; {t} would lead back to it"), None);
            }
        }
    }
    /// The first `repeat: once` boundary this work crossed that `station` would lead back to.
    pub fn crossed_block(&self, s: &Subject, station: &str) -> Option<String> {
        self.binding_crossings(s).iter().map(|c| c.boundary.clone())
            .find(|b| self.graph.repeat(b) == Some(super::adapter::Repeat::Once) && self.graph.leads_to(station, b))
    }
    /// `rework`: automatic send-backs from this station, counted from visits.
    fn rework_limit(&self, s: &Subject, station: &str, label: &str, target: &Option<Target>) -> Option<String> {
        let node = self.graph.node(station)?;
        if node.def.kind == Kind::Decision || label == "passed" || label == "skipped" { return None; }
        let back = |l: &str| match self.graph.route(station, l) {
            Some(Target::Station(t)) => self.graph.node(&t).is_some_and(|n| n.parent == node.parent && n.index <= node.index),
            _ => false,
        };
        if !matches!(target, Some(Target::Station(_))) || !back(label) { return None; }
        let visits = match s { Subject::Token(k) => &self.p.tokens[k].visits, Subject::Batch(k) => &self.p.batches[k].visits };
        let count = visits.iter().filter(|v| v.station == station && v.left.is_some()
            && v.ended.as_deref().is_some_and(&back)).count();
        (count >= node.def.policy.rework() as usize).then(|| format!("{station}.rework"))
    }
    fn leave_retired(&mut self, s: &Subject, station: &str, retired: &Retired, label: &str) {
        self.close_visit(s, label, Some("finished at a retired station".into()), None);
        self.free_place(s.key(), station);
        let target = if label == "passed" || label == "skipped" { Some(Target::Station(retired.to.clone())) }
            else { retired.labels.get(label).cloned() };
        if retired.unbatch {
            if let Subject::Batch(b) = s { return self.unbatch(b, &retired.to, label); }
        }
        match target {
            Some(t) => self.go(s, station, t, label),
            None => {
                self.open_visit(s, &retired.to, "publish", None);
                self.hold(s, &retired.to, HoldKind::StepFailed, label, None);
            }
        }
    }

    /// Start work wherever there is room, waits that ran out of patience, and batches whose time came.
    pub fn schedule(&mut self) {
        for id in self.graph.order.clone() {
            let node = self.graph.nodes[&id].clone();
            if node.def.kind == Kind::Wait { self.expire(&id, node.def.policy.patience_ms); continue; }
            let Some(st) = self.p.stations.get(&id) else { continue };
            if st.held.is_some() || st.draining { continue; }
            match node.def.kind {
                Kind::Agent | Kind::Command => self.start_from_queue(&id),
                Kind::Batch => self.form(&id),
                _ => {}
            }
        }
    }
    fn expire(&mut self, station: &str, patience: Option<i64>) {
        let Some(patience) = patience else { return };
        let now = self.now();
        let mut late: Vec<Subject> = Vec::new();
        for (k, t) in &self.p.tokens { if let Location::Working { station: st, work } = &t.at {
            if st == station && work.wait_since.is_some_and(|w| w + patience <= now) { late.push(Subject::Token(k.clone())); } } }
        for (k, b) in &self.p.batches { if let Location::Working { station: st, work } = &b.at {
            if st == station && work.wait_since.is_some_and(|w| w + patience <= now) { late.push(Subject::Batch(k.clone())); } } }
        for s in late { self.bump(&s); self.leave(&s, station, "timeout", true); }
    }
    pub fn room(&self, station: &str, key: &str) -> bool {
        let st = &self.p.stations[station];
        if st.working.len() >= st.capacity as usize { return false; }
        let Some(pool) = &self.graph.nodes[station].def.pool else { return true };
        let Some(pl) = self.p.pools.get(pool) else { return false };
        if pl.held_by.contains(key) { return true; }
        if pl.held_by.len() >= pl.capacity as usize { return false; }
        match self.input.host.get(pool) { Some(h) => h.busy as usize + pl.held_by.len() < h.limit as usize, None => true }
    }
    fn held_resources(&self) -> Vec<String> {
        let mut out: Vec<String> = self.input.busy_resources.iter().cloned().collect();
        let work = |at: &Location| match at { Location::Working { work, .. } | Location::Stopping { work, .. }
            | Location::Settling { work, .. } => work.resources.clone(), _ => Vec::new() };
        for t in self.p.tokens.values() { out.extend(work(&t.at)); }
        for b in self.p.batches.values() { out.extend(work(&b.at)); }
        out
    }
    fn start_from_queue(&mut self, station: &str) {
        let queue = self.p.stations[station].queue.clone();
        for key in queue {
            let s = self.subject_of(&key);
            if !self.exists(&s) || !matches!(self.at(&s), Location::Queued { station: q } if q == station) { continue; }
            if let Subject::Token(k) = &s {
                if !self.p.tokens[k].dependencies.iter().all(|d| matches!(self.p.archive.get(d), Some(a) if a.closed == Closure::Done)) { continue; }
            }
            if !self.room(station, &key) { break; }
            let spec = self.graph.nodes[station].spec.clone();
            let held = self.held_resources();
            if spec.is_some_and(|sp| sp.resources.iter().any(|r| held.contains(r))) { continue; }
            self.begin(&s, station, 1, 0, None);
        }
    }

    /// An operation answered. The core maps it through the adapter and routes the label.
    pub fn answer(&mut self, s: &Subject, station: &str, work: Work, outcome: &Outcome) -> Result<(), Refusal> {
        let kind = self.graph.node(station).map(|n| n.def.kind)
            .or_else(|| self.p.retired.get(station).map(|r| r.kind)).ok_or(Refusal::Missing)?;
        let policy = self.graph.node(station).map(|n| n.def.policy.clone()).unwrap_or_default();
        match outcome {
            Outcome::Labeled { evidence, .. } => {
                let use_ = self.p.workflow.as_ref().unwrap().layout.adapters.get(&work.op.adapter).cloned().ok_or(Refusal::Definition)?;
                let rules = self.adapters.get(&use_.version).ok_or(Refusal::Definition)?;
                let members = self.members(s, station);
                let applied = rules.observe(&work.op.op, &self.view(s, station, &members), outcome)?;
                let spec = rules.catalog(&use_.config).into_iter().find(|o| o.name == work.op.op);
                match s {
                    Subject::Token(k) => { let t = self.p.tokens.get_mut(k).unwrap();
                        t.evidence = Some(evidence.clone());
                        if let Some(d) = &applied.data { t.data.insert(work.op.adapter.clone(), d.clone()); } }
                    Subject::Batch(k) => { let b = self.p.batches.get_mut(k).unwrap();
                        if let Some(d) = &applied.data { b.data.insert(work.op.adapter.clone(), d.clone()); } }
                }
                if applied.again.is_some() && work.reruns < policy.again() {
                    self.begin(s, station, work.attempt, work.reruns + 1, applied.again.clone());
                    return Ok(());
                }
                if let Some(o) = spec.and_then(|sp| sp.outward) {
                    if o.crossing.contains(&applied.label) {
                        let effect = work.pending.clone().unwrap_or_default();
                        self.cross(s, station, &o.boundary, work.round.unwrap_or(1), &effect, evidence);
                    }
                }
                if let Subject::Batch(b) = s {
                    if self.graph.route(station, &applied.label) == Some(Target::Returned) || self.graph.route(station, &applied.label) == Some(Target::Rejected) {
                        let suspects = rules.suspects(&self.view(s, station, &members), outcome);
                        self.p.batches.get_mut(b).unwrap().suspects = suspects;
                    }
                }
                self.leave(s, station, &applied.label, true);
            }
            Outcome::Failed { reason } => {
                if let Subject::Token(k) = s { self.p.tokens.get_mut(k).unwrap().failure = reason.clone(); }
                if work.attempt < policy.attempts(kind) && self.p.stations.contains_key(station) {
                    self.begin(s, station, work.attempt + 1, work.reruns, None);
                } else {
                    let why = reason.clone().unwrap_or_else(|| "the step failed".into());
                    self.mark_ended(s, "failed");
                    self.hold(s, station, HoldKind::StepFailed, &format!("{station}.attempts reached: {why}"), None);
                }
            }
            Outcome::Interrupted => { self.begin(s, station, work.attempt, work.reruns, None); }
            Outcome::Uncertain => {
                let mut w = work;
                w.uncertain = true;
                w.pending = None;
                self.set_at(s, Location::Working { station: station.into(), work: w });
            }
            Outcome::Stopped { .. } | Outcome::Settled { .. } => return Err(Refusal::Evidence),
        }
        Ok(())
    }
    /// Record an outward act: on the subject, and for a batch on every member too. Never removed.
    pub fn cross(&mut self, s: &Subject, station: &str, boundary: &str, round: u32, effect: &str, evidence: &Evidence) {
        let c = Crossing { boundary: boundary.into(), round, effect: effect.into(), station: station.into(),
            artifact: evidence.artifact.clone(), receipt: evidence.receipt.clone(), revision: self.rev() };
        match s {
            Subject::Token(k) => self.p.tokens.get_mut(k).unwrap().crossings.push(c),
            Subject::Batch(k) => {
                let b = self.p.batches.get_mut(k).unwrap();
                b.crossings.push(c.clone());
                for m in b.members.clone() { if let Some(t) = self.p.tokens.get_mut(&m.token) { t.crossings.push(c.clone()); } }
            }
        }
    }
}
