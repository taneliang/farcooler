//! Batches (docs/engine-pipeline.md 4.1, 4.5): forming by policy, finishing together, returning members,
//! and a rejection's `on_reject`. A batch is a carrier: its members keep their keys and their visits.
use super::flow::Ctx;
use super::layout::{Form, OnReject};
use super::types::*;

impl<'a, 'r> Ctx<'a, 'r> {
    /// Form batches at batch station `station` from its queue, by its `form` policy.
    pub fn form(&mut self, station: &str) {
        let form = self.graph.nodes[station].def.form.clone().unwrap_or(Form { size: Some(8), ..Form::default() });
        let size = form.size.unwrap_or(u16::MAX).max(1) as usize;
        let eligible: Vec<String> = self.p.stations[station].queue.iter().filter(|k| {
            self.p.tokens.get(*k).is_some_and(|t| matches!(&t.at, Location::Queued { station: q } if q == station)
                && t.dependencies.iter().all(|d| matches!(self.p.archive.get(d), Some(a) if a.closed == Closure::Done)))
        }).cloned().collect();
        let groups: Vec<Vec<String>> = match &form.window {
            Some(w) => {
                let index = w.index(self.now());
                let last = self.p.stations[station].window;
                self.p.stations.get_mut(station).unwrap().window = Some(index);
                if last.is_none_or(|l| index <= l) { return; }
                eligible.chunks(size).map(<[String]>::to_vec).collect()
            }
            None => eligible.chunks(size).map(<[String]>::to_vec).collect(),
        };
        for members in groups {
            if !self.room(station, "") { break; }
            self.form_one(station, members);
        }
    }
    fn form_one(&mut self, station: &str, members: Vec<String>) {
        self.p.sequence += 1;
        let id = format!("{station}#{}", self.p.sequence);
        let rev = self.rev();
        let mut memberships = Vec::new();
        for k in &members {
            let t = self.p.tokens.get_mut(k).unwrap();
            t.at = Location::InBatch { batch: id.clone() };
            memberships.push(Membership { token: k.clone(), joined: rev, data: t.data.clone() });
            self.mark_started(&Subject::Token(k.clone()));
        }
        let st = self.p.stations.get_mut(station).unwrap();
        st.queue.retain(|k| !members.contains(k));
        st.working.insert(id.clone());
        if let Some(pool) = &self.graph.nodes[station].def.pool {
            if let Some(pl) = self.p.pools.get_mut(pool) { pl.held_by.insert(id.clone()); }
        }
        self.p.batches.insert(id.clone(), Batch { station: station.into(), at: Location::Resting { column: String::new() },
            members: memberships, data: Default::default(), crossings: Vec::new(), suspects: Vec::new(), bisect: None,
            visits: Vec::new(), fence: 0, since_ms: self.now() });
        let first = self.graph.nodes[station].first.clone().expect("a batch station has inner stations");
        self.arrive(&Subject::Batch(id), &first, "route", None);
    }
    /// The batch is complete: its members leave together with `passed`, routed by the batch station.
    pub fn finish(&mut self, b: &str) {
        let station = self.p.batches[b].station.clone();
        let members: Vec<String> = self.p.batches[b].members.iter().map(|m| m.token.clone()).collect();
        self.archive_batch(b, Closure::Done, None);
        for k in members { self.member_leave(&k, &station, "passed", None); }
    }
    pub fn archive_batch(&mut self, b: &str, closed: Closure, reason: Option<String>) {
        let Some(batch) = self.p.batches.remove(b) else { return };
        if let Some(at) = batch.at.station() { self.free_place(b, at); }
        self.free_place(b, &batch.station);
        // Members still in the batch keep a location that names nothing; give them their batch station.
        for m in &batch.members {
            if let Some(t) = self.p.tokens.get_mut(&m.token) {
                if matches!(&t.at, Location::InBatch { batch: x } if x == b) { t.at = Location::Queued { station: batch.station.clone() }; }
            }
        }
        self.p.archive.insert(b.into(), Archived { closed, reason, crossings: batch.crossings, data: batch.data,
            batch: Some(ArchivedBatch { name: batch.station, members: batch.members.into_iter().map(|m| m.token).collect(),
                since_ms: batch.since_ms }), visits: batch.visits });
    }
    /// One member leaves the batch station with `label`, by its routes, under its `returns` policy.
    pub fn member_leave(&mut self, k: &str, station: &str, label: &str, brief: Option<String>) {
        let s = Subject::Token(k.into());
        if !self.p.tokens.contains_key(k) { return; }
        self.set_at(&s, Location::Queued { station: station.into() });
        if brief.is_some() { self.p.tokens.get_mut(k).unwrap().brief = brief; }
        let target = self.graph.route(station, label);
        if label == "returned" {
            let returns = self.graph.nodes[station].def.policy.returns() as usize;
            let count = self.p.tokens[k].visits.iter()
                .filter(|v| v.station == station && v.left.is_some() && v.ended.as_deref() == Some("returned")).count();
            if count >= returns {
                self.mark_ended(&s, label);
                let policy = format!("{station}.returns");
                return self.hold(&s, station, HoldKind::PolicyLimit { policy: policy.clone() },
                    &format!("{policy} reached: returned from the batch as often as its policy allows"), target);
            }
        }
        match target {
            None => { self.mark_ended(&s, label); self.hold(&s, station, HoldKind::StepFailed, label, None); }
            Some(t) => { self.close_visit(&s, label, None, None); self.go(&s, station, t, label); }
        }
    }
    fn take_suspects(&mut self, b: &str) -> Vec<String> {
        let batch = self.p.batches.get_mut(b).unwrap();
        let suspects: Vec<String> = batch.members.iter().map(|m| m.token.clone())
            .filter(|k| batch.suspects.contains(k)).collect();
        batch.members.retain(|m| !suspects.contains(&m.token));
        batch.suspects.clear();
        suspects
    }
    /// `returned` inside a batch: the named members leave; the rest carry on as if the station passed.
    pub fn returned(&mut self, b: &str, station: &str, label: &str) {
        let s = Subject::Batch(b.into());
        let suspects = self.take_suspects(b);
        if suspects.is_empty() {
            self.open_visit(&s, station, "route", None);
            return self.hold(&s, station, HoldKind::StepFailed, &format!("{label}: the answer named no member"), None);
        }
        let home = self.p.batches[b].station.clone();
        let brief = Some(format!("returned from {station}: {label}"));
        for k in &suspects { self.member_leave(k, &home, "returned", brief.clone()); }
        if self.p.batches[b].members.is_empty() { return self.archive_batch(b, Closure::Cancelled, Some("every member returned".into())); }
        match self.graph.route(station, "passed") {
            Some(t) => self.go(&s, station, t, "passed"),
            None => self.hold(&s, station, HoldKind::StepFailed, label, None),
        }
    }
    /// `rejected` inside a batch: the batch station's `on_reject` (4.5).
    pub fn rejected(&mut self, b: &str, station: &str, label: &str) {
        let s = Subject::Batch(b.into());
        let home = self.p.batches[b].station.clone();
        let outward = self.graph.nodes[station].spec.as_ref().is_some_and(|sp| sp.outward.is_some());
        let mut rule = self.graph.nodes[&home].def.on_reject;
        if outward { rule = OnReject::Hold; }
        match rule {
            OnReject::Hold => {
                self.open_visit(&s, station, "route", None);
                self.hold(&s, station, HoldKind::StepFailed, label, Some(Target::Rejected));
            }
            OnReject::ReturnSuspects => {
                let suspects = self.take_suspects(b);
                if suspects.is_empty() {
                    self.open_visit(&s, station, "route", None);
                    return self.hold(&s, station, HoldKind::StepFailed, &format!("{label}: no suspects named"), None);
                }
                let brief = Some(format!("returned from {station}: {label}"));
                for k in &suspects { self.member_leave(k, &home, "returned", brief.clone()); }
                if self.p.batches[b].members.is_empty() { return self.archive_batch(b, Closure::Cancelled, Some("every member returned".into())); }
                self.arrive(&s, station, "route", Some("re-entered without the returned members".into()));
            }
            OnReject::Bisect => {
                let members: Vec<String> = self.p.batches[b].members.iter().map(|m| m.token.clone()).collect();
                if members.len() == 1 {
                    self.p.batches.get_mut(b).unwrap().members.clear();
                    self.member_leave(&members[0], &home, "returned", Some(format!("rejected at {station}: {label}")));
                    return self.archive_batch(b, Closure::Cancelled, Some("its only member was returned".into()));
                }
                let (first, second) = members.split_at(members.len() / 2);
                self.p.batches.get_mut(b).unwrap().bisect = Some(Bisect { station: station.into(), trying: first.to_vec(),
                    waiting: vec![second.to_vec()], cleared: Vec::new(), found: Vec::new(), passing: None });
                self.arrive(&s, station, "route", Some("bisecting".into()));
            }
        }
    }
    /// A bisecting batch answered for the members it tried: narrow the search or end it.
    pub fn bisect_answer(&mut self, b: &str, station: &str, label: &str) {
        let s = Subject::Batch(b.into());
        self.close_visit(&s, label, Some("bisecting".into()), None);
        self.free_place(b, station);
        let failed = self.graph.route(station, label) == Some(Target::Rejected);
        let mut x = self.p.batches[b].bisect.clone().unwrap();
        let tried = std::mem::take(&mut x.trying);
        if !failed { x.cleared.extend(tried); x.passing = Some(label.into()); }
        else if tried.len() == 1 { x.found.extend(tried); }
        else { let (a, c) = tried.split_at(tried.len() / 2); x.waiting.insert(0, c.to_vec()); x.waiting.insert(0, a.to_vec()); }
        if !x.waiting.is_empty() {
            x.trying = x.waiting.remove(0);
            self.p.batches.get_mut(b).unwrap().bisect = Some(x);
            return self.arrive(&s, station, "route", Some("bisecting".into()));
        }
        let home = self.p.batches[b].station.clone();
        let batch = self.p.batches.get_mut(b).unwrap();
        batch.bisect = None;
        batch.members.retain(|m| !x.found.contains(&m.token));
        for k in &x.found { self.member_leave(k, &home, "returned", Some(format!("bisect found it failing {station}"))); }
        if self.p.batches[b].members.is_empty() { return self.archive_batch(b, Closure::Cancelled, Some("every member failed".into())); }
        let passing = x.passing.unwrap_or_else(|| "passed".into());
        match self.graph.route(station, &passing) {
            Some(t) => self.go(&s, station, t, &passing),
            None => { self.open_visit(&s, station, "route", None); self.hold(&s, station, HoldKind::StepFailed, &passing, None) }
        }
    }
    /// A removed batch station's batch: every member leaves on its own to `to`, checked by the migration rule.
    pub fn unbatch(&mut self, b: &str, to: &str, label: &str) {
        let home = self.p.batches[b].station.clone();
        let members: Vec<String> = self.p.batches[b].members.iter().map(|m| m.token.clone()).collect();
        self.archive_batch(b, Closure::Cancelled, Some("its batch station was removed".into()));
        for k in members {
            let s = Subject::Token(k.clone());
            if !self.p.tokens.contains_key(&k) { continue; }
            // A member carries its batch's crossings already (`cross` appends to every member).
            self.close_visit(&s, label, Some(format!("unbatched from {home}")), None);
            self.place(&s, to, if label == "skipped" { "skipped" } else { "passed" });
        }
    }
}
