//! Control: every intervention the orchestrator can make (docs/engine-pipeline.md 5.1), with working
//! work taken off its station safely first (5.2) and outward work settled before it moves (5.3).
use super::flow::Ctx;
use super::layout::{Kind, Skipper};
use super::step::{Command, Grant, Refusal, Scope};
use super::types::*;

impl<'a, 'r> Ctx<'a, 'r> {
    pub(super) fn intervene(&mut self, command: &Command) -> Result<(), Refusal> {
        match &self.input.grant { Grant::Orchestrator | Grant::Person { .. } => {}, _ => return Err(Refusal::Unauthorized) }
        let by = self.input.grant.principal();
        match command {
            Command::Reorder { station, token, position } => {
                if !matches!(self.p.tokens.get(token).map(|t| &t.at), Some(Location::Queued { station: q }) if q == station) {
                    return Err(Refusal::Stale);
                }
                let st = self.p.stations.get_mut(station).ok_or(Refusal::Missing)?;
                st.queue.retain(|k| k != token);
                let at = (*position).min(st.queue.len());
                st.queue.insert(at, token.clone());
            }
            Command::Prioritize { token, priority } => {
                let t = self.p.tokens.get_mut(token).ok_or(Refusal::Missing)?;
                t.priority = *priority;
                if let Location::Queued { station } = t.at.clone() { self.insert_queue(&station, token); }
            }
            Command::Hold { subject, station, reason } | Command::Stop { subject, station, reason } => {
                self.expect_at(subject, station)?;
                if matches!(self.at(subject), Location::Held { .. }) { return Err(Refusal::Invalid); }
                let hold = Hold { reason: reason.clone(), by: by.clone(), revision: self.rev(), kind: HoldKind::Asked, then: None };
                self.take_off(subject, station, Then::Hold(hold))?;
            }
            Command::Release { subject, station, to, reason } => {
                self.expect_at(subject, station)?;
                let Location::Held { hold, .. } = self.at(subject).clone() else { return Err(Refusal::Stale) };
                let overrode = match &hold.kind { HoldKind::PolicyLimit { policy } => Some(policy.clone()), _ => None };
                match to {
                    Some(t) => {
                        self.top_station(t)?;
                        if self.crossed_block(subject, t).is_some() { return Err(Refusal::Crossed); }
                        self.apply_then(subject, station, Then::Arrive { station: t.clone(), by, reason: reason.clone() }, overrode);
                    }
                    None if matches!(hold.kind, HoldKind::Crossed { .. }) => return Err(Refusal::Crossed),
                    None => {
                        self.close_visit(subject, "released", reason.clone(), overrode);
                        self.free_place(subject.key(), station);
                        match hold.then {
                            Some(target) => self.go(subject, station, target, "released"),
                            None if self.p.stations.contains_key(station) => self.arrive(subject, station, &by, reason.clone()),
                            None => return Err(Refusal::Missing),
                        }
                    }
                }
            }
            Command::Send { token, station, to, reason } => {
                let s = Subject::Token(token.clone());
                self.expect_at(&s, station)?;
                self.top_station(to)?;
                if self.crossed_block(&s, to).is_some() { return Err(Refusal::Crossed); }
                let overrode = self.overrode(&s);
                self.p.tokens.get_mut(token).unwrap().brief = Some(reason.clone());
                self.take_off_with(&s, station, Then::Arrive { station: to.clone(), by, reason: Some(reason.clone()) }, overrode)?;
            }
            Command::Skip { subject, station, reason } => {
                self.expect_at(subject, station)?;
                let node = self.graph.node(station).ok_or(Refusal::Missing)?;
                if node.def.policy.skip == Skipper::Person && !matches!(self.input.grant, Grant::Person { .. }) {
                    return Err(Refusal::PersonOnly);
                }
                if let Some(Target::Station(t)) = self.graph.route(station, "skipped") {
                    if self.crossed_block(subject, &t).is_some() { return Err(Refusal::Crossed); }
                }
                let overrode = self.overrode(subject);
                self.take_off_with(subject, station, Then::Leave { label: "skipped".into(), by, reason: Some(reason.clone()) }, overrode)?;
            }
            Command::Pull { token, reason } => self.pull(token, reason, &by)?,
            Command::Cancel { token, reason } => {
                let s = Subject::Token(token.clone());
                let at = self.p.tokens.get(token).ok_or(Refusal::Stale)?.at.clone();
                match at {
                    Location::InBatch { .. } => {
                        self.pull(token, reason, &by)?;
                        self.close_visit(&s, "cancelled", Some(reason.clone()), None);
                        self.close(&s, Closure::Cancelled, Some(reason.clone()));
                    }
                    Location::Resting { .. } => self.close(&s, Closure::Cancelled, Some(reason.clone())),
                    _ => {
                        let station = at.station().ok_or(Refusal::Stale)?.to_string();
                        self.take_off(&s, &station, Then::Close { by, reason: Some(reason.clone()) })?;
                    }
                }
            }
            Command::Reopen { token, station } => {
                self.top_station(station)?;
                let archived = self.p.archive.get(token).ok_or(Refusal::Missing)?;
                if archived.batch.is_some() { return Err(Refusal::Invalid); }
                let archived = self.p.archive.remove(token).unwrap();
                let mut t = super::step::new_token(0, archived.data, Default::default(), Default::default(), None);
                t.fresh_from = archived.crossings.len();
                t.crossings = archived.crossings;
                t.visits = archived.visits;
                self.p.tokens.insert(token.clone(), t);
                self.arrive(&Subject::Token(token.clone()), station, &by, Some("reopened".into()));
            }
            Command::SetCapacity { scope, n, .. } => match scope {
                Scope::Station(s) => self.p.stations.get_mut(s).ok_or(Refusal::Missing)?.capacity = *n,
                Scope::Pool(p) => self.p.pools.get_mut(p).ok_or(Refusal::Missing)?.capacity = *n,
            },
            Command::HoldStation { station, reason } => {
                let rev = self.rev();
                self.p.stations.get_mut(station).ok_or(Refusal::Missing)?.held =
                    Some(Hold { reason: reason.clone(), by, revision: rev, kind: HoldKind::Asked, then: None });
            }
            Command::ResumeStation { station } => {
                let st = self.p.stations.get_mut(station).ok_or(Refusal::Missing)?;
                st.held = None;
                st.draining = false;
            }
            Command::Drain { station } => self.p.stations.get_mut(station).ok_or(Refusal::Missing)?.draining = true,
            _ => return Err(Refusal::Invalid),
        }
        Ok(())
    }
    /// The one staleness check: the work is where the command believes it is.
    fn expect_at(&self, s: &Subject, station: &str) -> Result<(), Refusal> {
        if !self.exists(s) || self.at(s).station() != Some(station) { return Err(Refusal::Stale); }
        Ok(())
    }
    fn overrode(&self, s: &Subject) -> Option<String> {
        match self.at(s) { Location::Held { hold: Hold { kind: HoldKind::PolicyLimit { policy }, .. }, .. } => Some(policy.clone()), _ => None }
    }
    pub(super) fn take_off(&mut self, s: &Subject, station: &str, then: Then) -> Result<(), Refusal> {
        self.take_off_with(s, station, then, None)
    }
    /// Take work off its station. Working work stops first (or settles, if outward); anything else moves now.
    fn take_off_with(&mut self, s: &Subject, station: &str, then: Then, overrode: Option<String>) -> Result<(), Refusal> {
        let at = self.at(s).clone();
        match at {
            Location::Stopping { .. } | Location::Settling { .. } => Err(Refusal::Stopping),
            Location::Working { work, .. } => {
                let spec = self.graph.node(station).and_then(|n| n.spec.clone());
                let outward = spec.as_ref().is_some_and(|sp| sp.outward.is_some());
                let wait = self.graph.node(station).is_some_and(|n| n.def.kind == Kind::Wait);
                if work.uncertain && outward { return Err(Refusal::Stopping); }
                if wait && !outward {
                    // A probe holds nothing: fence its late answer out and move now.
                    self.bump(s);
                    self.apply_then(s, station, then, overrode);
                    return Ok(());
                }
                let stop = self.next_id("s");
                let fence = self.bump(s);
                let original = work.pending.clone().unwrap_or_else(|| work.run.clone());
                let mut w = work.clone();
                w.stopping = Some(original.clone());
                w.pending = Some(stop.clone());
                w.run = format!("stop:{stop}");
                self.effects.push(Effect { id: stop.clone(), subject: s.clone(), station: station.into(), run: w.run.clone(),
                    epoch: self.p.epoch, fence, resources: work.resources.clone(), kind: EffectKind::Stop { operation: original },
                    created_ms: self.now(), model: None, brief: None, members: self.members(s, station),
                    idempotency: stop, data: Default::default() });
                let then = Box::new(then);
                self.set_at(s, if outward { Location::Settling { station: station.into(), work: w, then } }
                    else { Location::Stopping { station: station.into(), work: w, then } });
                Ok(())
            }
            Location::InBatch { .. } | Location::Closed { .. } => Err(Refusal::Invalid),
            _ => { self.apply_then(s, station, then, overrode); Ok(()) }
        }
    }
    /// Where an intervention sends work, once nothing runs for it at `station`.
    pub(super) fn apply_then(&mut self, s: &Subject, station: &str, then: Then, overrode: Option<String>) {
        match then {
            Then::Arrive { station: to, by, reason } => {
                self.close_visit(s, "sent", reason.clone(), overrode);
                self.free_place(s.key(), station);
                self.arrive(s, &to, &by, reason);
            }
            Then::Hold(mut hold) => {
                self.free_place(s.key(), station);
                hold.revision = self.rev();
                if let Some(v) = self.visits_mut(s).iter_mut().rev().find(|v| v.left.is_none()) { v.reason = Some(hold.reason.clone()); }
                self.set_at(s, Location::Held { station: station.into(), hold });
            }
            Then::Leave { label, reason, .. } => {
                if let Some(v) = self.visits_mut(s).iter_mut().rev().find(|v| v.left.is_none()) {
                    v.reason = reason;
                    v.overrode = overrode;
                }
                self.set_at(s, Location::Queued { station: station.into() });
                self.leave(s, station, &label, false);
            }
            Then::Close { reason, .. } => {
                self.close_visit(s, "cancelled", reason.clone(), overrode);
                self.free_place(s.key(), station);
                self.close(s, Closure::Cancelled, reason);
            }
        }
    }
    /// Pull a token out of its batch, even mid-step: it holds at the batch station; a working batch stops and
    /// re-runs its station without it.
    fn pull(&mut self, token: &str, reason: &str, by: &str) -> Result<(), Refusal> {
        let Some(Location::InBatch { batch }) = self.p.tokens.get(token).map(|t| t.at.clone()) else { return Err(Refusal::Stale) };
        if self.p.batches[&batch].crossings.iter().any(|c| self.graph.repeat(&c.boundary) == Some(super::adapter::Repeat::Once)) {
            return Err(Refusal::Crossed);
        }
        let b = Subject::Batch(batch.clone());
        let home = self.p.batches[&batch].station.clone();
        if let Location::Working { station, .. } = self.at(&b).clone() {
            self.take_off(&b, &station, Then::Arrive { station: station.clone(), by: by.into(),
                reason: Some(format!("re-runs without {token}")) })?;
        }
        let entry = self.p.batches.get_mut(&batch).unwrap();
        entry.members.retain(|m| m.token != token);
        if let Some(x) = entry.bisect.as_mut() {
            x.trying.retain(|k| k != token);
            x.cleared.retain(|k| k != token);
            x.found.retain(|k| k != token);
            for w in &mut x.waiting { w.retain(|k| k != token); }
        }
        let empty = entry.members.is_empty();
        let s = Subject::Token(token.into());
        let hold = Hold { reason: reason.into(), by: by.into(), revision: self.rev(), kind: HoldKind::Asked, then: None };
        self.set_at(&s, Location::Held { station: home, hold });
        if let Some(v) = self.visits_mut(&s).iter_mut().rev().find(|v| v.left.is_none()) { v.reason = Some(reason.into()); }
        if empty { self.archive_batch(&batch, Closure::Cancelled, Some("every member was pulled".into())); }
        Ok(())
    }
}
