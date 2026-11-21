//! Builders pause while a landing gate runs, and resume after (ov-1214).
//!
//! A gate that shares ten cores with four builders and their compilers ran 40 minutes at load 40. Niceness can't
//! be taken back without privilege, so a builder is paused with SIGSTOP and resumed with SIGCONT instead. The
//! engine records each builder's process group (or its native PTY's tty) while it runs, and each landing gate
//! while it runs. Builders yield during a gate once they have no ticket in the machine-wide heavy queue.
//! When the last gate ends every automatically paused builder is continued. A pause is written to disk, so
//! a daemon that died mid-gate continues what it had stopped when it starts again.
mod heavy_queue;

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

/// What a builder's processes are found by.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Target {
    /// A process group the engine created, led by `pid`, which started at `started` (so a recycled pid is never ours).
    Group { pid: u32, started: String },
    /// Legacy pause records, read on restart to continue builders stopped by older engines.
    Tty(String),
    /// A native PTY's waiting shell pins its identity, but must stay running: tmux continues a stopped pane leader's
    /// whole group. Only the agent and its descendants yield, as with a native hand pause (ov-1357).
    Native { tty: String, supervisor: u32, started: String },
}

/// A builder, and whether the orchestrator has paused it by hand (ov-1205). A builder may be stopped by a gate,
/// by hand, or by both. It is continued when neither pause remains.
/// `held_for` and `held_since` count the time a gate has held it stopped and its hand pause has not (ov-1301), so its
/// step's clocks can leave that time out: a hand pause already stops them itself.
struct Builder { target: Target, manual: bool, held_for: Duration, held_since: Option<tokio::time::Instant> }

impl Builder {
    fn count_hold(&mut self, held: bool) {
        match (held && !self.manual, self.held_since) {
            (true, None) => self.held_since = Some(tokio::time::Instant::now()),
            (false, Some(at)) => { self.held_for += at.elapsed(); self.held_since = None; }
            _ => {}
        }
    }

    fn continue_from_gate(&mut self) {
        if self.held_since.is_some() { signal(&self.target, libc::SIGCONT); }
        self.count_hold(false);
    }
}

#[derive(Default)]
struct State {
    gates: BTreeMap<u64, ()>,
    builders: BTreeMap<u64, Builder>,
    next: u64,
    paused: bool,
    ticking: bool,
    queue_latch: Option<File>,
}
pub(crate) struct Inner {
    state: Mutex<State>,
    record: Option<PathBuf>,
    queue: PathBuf,
    #[cfg(test)]
    _scratch: Option<tempfile::TempDir>,
}

/// Shared by every copy of the engine's executor.
#[derive(Clone)]
pub(crate) struct Yielding(Arc<Inner>);
#[cfg(test)]
impl Default for Yielding {
    fn default() -> Self {
        let scratch = tempfile::tempdir().expect("isolated heavy queue");
        let queue = scratch.path().join(heavy_queue::NAME);
        Self(Arc::new(Inner { state: Default::default(), record: None, queue, _scratch: Some(scratch) }))
    }
}

/// Held while a gate runs.
pub(crate) struct GateGuard(Yielding, u64);
/// Held while a builder runs.
pub(crate) struct BuilderGuard(Yielding, u64);
impl Drop for GateGuard {
    fn drop(&mut self) { self.0.change(|s| { s.gates.remove(&self.1); }); }
}
impl Drop for BuilderGuard {
    fn drop(&mut self) {
        // Continued before it is forgotten, so nothing is left stopped that no one will resume.
        self.0.change(|s| { if let Some(target) = s.builders.remove(&self.1) { signal(&target.target, libc::SIGCONT); } });
    }
}
impl BuilderGuard {
    /// The orchestrator paused (`true`) or resumed (`false`) this builder by hand. A hand pause survives the end of
    /// a gate, and a hand resume leaves the builder stopped while a gate still holds it.
    pub(crate) fn manual(&self, on: bool) {
        self.0.change(|s| { if let Some(b) = s.builders.get_mut(&self.1) {
            b.manual = on;
            b.count_hold(!s.gates.is_empty());
            if !on && s.gates.is_empty() { signal(&b.target, libc::SIGCONT); }
        } });
    }
    /// How long gates have held this builder stopped so far, and whether one holds it now (ov-1301). A step's time
    /// limit and quiet periods leave this out: a builder stopped for a gate is waiting, not slow.
    pub(crate) fn held(&self) -> (Duration, bool) { self.0.held(self.1) }
    /// A handle that reads `held`, for a clock that outlives no builder guard.
    pub(crate) fn held_clock(&self) -> HeldClock { HeldClock(self.0.clone(), self.1) }
}
/// Reads how long gates have held one builder stopped; see [`BuilderGuard::held`].
#[derive(Clone)]
pub(crate) struct HeldClock(Yielding, u64);
impl HeldClock {
    pub(crate) fn held(&self) -> Duration { self.0.held(self.1).0 }
}

fn signal(target: &Target, sig: i32) -> bool {
    match target {
        Target::Group { pid, started } => {
            // The leader is the identity: a group whose leader started at another time is not ours.
            if !started.is_empty() && crate::effect_runner::Owner::started_at(*pid).is_ok_and(|actual| actual == *started) {
                unsafe { libc::killpg(*pid as i32, sig); }
                if sig == libc::SIGSTOP {
                    // Wait for the pending stops before another process can enter the heavy queue.
                    let settled = || crate::proc_table::snapshot().is_some_and(|raw|
                        crate::foreground::parse(&raw).in_group(*pid as i32).all(crate::proc_table::stopped_or_gone));
                    return settled() && settled();
                }
            }
        }
        Target::Tty(tty) => {
            let Some(raw) = crate::proc_table::snapshot() else { return false };
            let pids = crate::foreground::parse(&raw).under_tty(tty.trim_start_matches("/dev/"));
            for &pid in &pids {
                if pid > 1 && pid != std::process::id() as i32 { unsafe { libc::kill(pid, sig); } }
            }
            if sig == libc::SIGSTOP { return pids.into_iter().all(crate::proc_table::stopped_or_gone); }
        }
        Target::Native { tty, supervisor, started } => {
            match crate::effect_runner::Owner::started_at(*supervisor) {
                Ok(actual) if actual != *started => return true,
                Ok(_) => {},
                Err(_) => return false,
            }
            let Some(raw) = crate::proc_table::snapshot() else { return false };
            return signal_native(&crate::foreground::parse(&raw), tty, *supervisor as i32, sig, |pid, sig| {
                unsafe { libc::kill(pid, sig); }
            });
        }
    }
    true
}

fn signal_native(table: &crate::foreground::Foreground, tty: &str, supervisor: i32, sig: i32, mut send: impl FnMut(i32, i32)) -> bool {
    let mut pids = table.under_tty(tty.trim_start_matches("/dev/"));
    // A recycled tty belongs to another run. Never signal it, even if the old shell still exists elsewhere.
    if !pids.contains(&supervisor) { return true; }
    pids.retain(|&pid| pid > 1 && pid != supervisor && pid != std::process::id() as i32);
    let ordered = table.parent_first(&pids);
    if sig == libc::SIGCONT {
        // An interactive login shell can see a stopped foreground job and exit
        // successfully. Resume its children before letting it handle SIGCHLD.
        for (pid, _) in ordered.into_iter().rev() { send(pid, sig); }
        return true;
    }
    // If we sent a stop, check a fresh process table before admitting the gate.
    // A parent may have forked after this snapshot but before it stopped.
    // The supervisor alone may still be releasing the native launch barrier.
    let mut settled = !ordered.is_empty();
    let mut stopped = std::collections::HashSet::new();
    for (pid, parent) in ordered {
        if parent.is_some_and(|parent| !stopped.contains(&parent)) {
            settled = false;
            continue;
        }
        if crate::proc_table::stopped_or_gone(pid) {
            stopped.insert(pid);
        } else {
            send(pid, sig);
            settled = false;
            if crate::proc_table::stopped_or_gone(pid) { stopped.insert(pid); }
        }
    }
    settled
}

fn stop_builders(state: &mut State, queue: &Path) -> bool {
    stop_builders_with_snapshot(state, queue, crate::proc_table::snapshot)
}

fn stop_builders_with_snapshot(state: &mut State, queue: &Path, snapshot: impl FnOnce() -> Option<String>) -> bool {
    if state.builders.values().all(|b| b.manual) {
        state.queue_latch = None;
        return true;
    }
    if state.queue_latch.is_none() {
        let Ok(enter) = heavy_queue::try_enter(queue) else { return false };
        state.queue_latch = Some(enter);
    }
    let Some(raw) = snapshot() else { return false };
    let table = crate::foreground::parse(&raw);
    let mut legacy_latches = LegacyLatches::default();
    let pids = state.builders.values().filter(|b| !b.manual).flat_map(|b| target_pids(&table, &b.target));
    let Ok(tickets) = legacy_latches.tickets(queue, pids) else { return false };
    let mut settled = true;
    for b in state.builders.values_mut().filter(|b| !b.manual) {
        if target_has_ticket(&table, &b.target, &tickets) {
            // Leave the whole builder runnable so the job's wrapper can reap its
            // children and remove its ticket. Any queued ticket may become head.
            b.continue_from_gate();
            continue;
        }
        settled &= signal(&b.target, libc::SIGSTOP);
        b.count_hold(true);
    }
    // A legacy wrapper can fork or create its private queue after the first
    // snapshot. Check again while the canonical and known legacy latches are
    // still held. Audit partial stops too: a gate may already be running.
    if state.builders.values().any(|b| !b.manual && b.held_since.is_some()) {
        let checked = (|| {
            let table = pause_snapshot()?;
            let pids = state.builders.values().filter(|b| !b.manual && b.held_since.is_some())
                .flat_map(|b| target_pids(&table, &b.target));
            let tickets = legacy_latches.tickets(queue, pids)?;
            Ok::<_, std::io::Error>((table, tickets))
        })();
        // If a late latch is busy, a stopped wrapper may own it. Let it run
        // before retrying, even if it hasn't published a ticket yet.
        for b in state.builders.values_mut().filter(|b| !b.manual && b.held_since.is_some()) {
            let keep_paused = checked.as_ref().is_ok_and(|(table, tickets)| !target_has_ticket(table, &b.target, tickets));
            if !keep_paused {
                b.continue_from_gate();
            }
        }
        settled &= checked.is_ok();
    }
    if settled { state.queue_latch = None; }
    settled
}

fn pause_snapshot() -> std::io::Result<crate::foreground::Foreground> {
    crate::proc_table::snapshot().map(|raw| crate::foreground::parse(&raw))
        .ok_or_else(|| std::io::Error::other("process table unavailable"))
}

/// Hold each discovered legacy latch alongside the canonical one until the
/// pause has been checked. The second read can discover more queues.
#[derive(Default)]
struct LegacyLatches(BTreeMap<PathBuf, File>);
impl LegacyLatches {
    fn tickets(&mut self, queue: &Path, pids: impl IntoIterator<Item = i32>) -> std::io::Result<Vec<heavy_queue::Ticket>> {
        for old in heavy_queue::legacy_queues(queue, pids) {
            if let std::collections::btree_map::Entry::Vacant(slot) = self.0.entry(old) {
                let latch = heavy_queue::try_enter(slot.key())?;
                slot.insert(latch);
            }
        }
        let mut tickets = heavy_queue::live_tickets(queue)?;
        for old in self.0.keys() {
            tickets.extend(heavy_queue::live_tickets(old)?);
        }
        Ok(tickets)
    }
}

fn target_pids(table: &crate::foreground::Foreground, target: &Target) -> Vec<i32> {
    match target {
        Target::Group { pid, .. } => table.in_group(*pid as i32).collect(),
        Target::Native { tty, .. } | Target::Tty(tty) => table.under_tty(tty.trim_start_matches("/dev/")),
    }
}

fn target_has_ticket(table: &crate::foreground::Foreground, target: &Target, tickets: &[heavy_queue::Ticket]) -> bool {
    match target {
        Target::Group { pid, .. } => tickets.iter().any(|t| unsafe { libc::getpgid(t.pid) } == *pid as i32),
        Target::Native { tty, supervisor, .. } => tty_has_ticket(table, tty, *supervisor as i32, tickets),
        Target::Tty(tty) => tty_has_ticket(table, tty, 0, tickets),
    }
}

fn tty_has_ticket(table: &crate::foreground::Foreground, tty: &str, supervisor: i32, tickets: &[heavy_queue::Ticket]) -> bool {
    let pids = table.under_tty(tty.trim_start_matches("/dev/"));
    (supervisor == 0 || pids.contains(&supervisor))
        && tickets.iter().any(|t| t.pid != supervisor && pids.contains(&t.pid))
}

/// Hand pauses must obey the same admission rule, including before a gate starts.
pub(crate) struct ManualPauseLatch {
    queue: PathBuf,
    _enter: File,
    legacy_latches: LegacyLatches,
}

impl ManualPauseLatch {
    pub(crate) fn check(&mut self, tty: &str, supervisor: i32) -> std::io::Result<()> {
        let table = pause_snapshot()?;
        let tickets = self.legacy_latches.tickets(&self.queue, table.under_tty(tty.trim_start_matches("/dev/")))?;
        if tty_has_ticket(&table, tty, supervisor, &tickets) {
            return Err(std::io::Error::other("builder has a ticket in the machine-wide heavy-job queue"));
        }
        Ok(())
    }
}

pub(crate) fn manual_pause_latch(builder: Option<&BuilderGuard>, tty: &str, supervisor: i32) -> std::io::Result<ManualPauseLatch> {
    let queue = builder.map_or_else(heavy_queue::machine, |builder| builder.0.0.queue.clone());
    let enter = heavy_queue::try_enter(&queue)?;
    let mut latch = ManualPauseLatch { queue, _enter: enter, legacy_latches: LegacyLatches::default() };
    latch.check(tty, supervisor)?;
    Ok(latch)
}

impl Yielding {
    fn new(record: Option<PathBuf>, queue: PathBuf) -> Self {
        Self(Arc::new(Inner { state: Default::default(), record, queue, #[cfg(test)] _scratch: None }))
    }

    /// Persist pauses under `root`, and continue anything an earlier daemon left stopped.
    pub(crate) fn at(root: &Path) -> Self {
        let record = root.join("yielding.json");
        if let Ok(raw) = std::fs::read(&record) {
            match serde_json::from_slice::<Vec<Target>>(&raw) {
                Ok(left) => {
                    for target in &left { signal(target, libc::SIGCONT); }
                    let _ = std::fs::remove_file(&record);
                }
                // Kept for a person to read: deleting it would hide that builders may still be stopped.
                Err(e) => tracing::error!(error = %e, path = %record.display(), "the pause record is unreadable; builders it names may still be stopped"),
            }
        }
        #[cfg(test)]
        let queue = root.join(heavy_queue::NAME);
        #[cfg(not(test))]
        let queue = heavy_queue::machine();
        Self::new(Some(record), queue)
    }

    pub(crate) fn heavy_lock_waits(&self, groups: &std::collections::HashSet<i32>) -> std::collections::HashMap<i32, String> {
        if groups.is_empty() { return Default::default(); }
        let Ok(tickets) = heavy_queue::live_tickets(&self.0.queue) else { return Default::default() };
        heavy_queue::waiting(&self.0.queue, &tickets, groups)
    }
    /// A landing gate started. Builders without heavy tickets yield until every gate has ended.
    pub(crate) fn gate(&self) -> GateGuard {
        let mut id = 0;
        self.change(|s| { s.next += 1; id = s.next; s.gates.insert(id, ()); });
        GateGuard(self.clone(), id)
    }
    /// Hold builders and confirm the stops took effect before admitting gate work.
    pub(crate) async fn admit_gate(&self) -> std::io::Result<GateGuard> {
        let gate = self.gate();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            {
                let mut state = self.0.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if !state.paused { return Err(std::io::Error::other("could not record the builder pause")); }
                if stop_builders(&mut state, &self.0.queue) { return Ok(gate); }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(std::io::Error::other("builder pauses did not settle before the landing gate"));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    /// A builder started. It yields if a gate is running and it has no heavy ticket.
    pub(crate) fn builder(&self, target: Target) -> BuilderGuard {
        let mut id = 0;
        self.change(|s| { s.next += 1; id = s.next; s.builders.insert(id, Builder { target, manual: false, held_for: Duration::ZERO, held_since: None }); });
        BuilderGuard(self.clone(), id)
    }
    /// Whether builders are paused now.
    #[cfg(test)]
    pub(crate) fn paused(&self) -> bool { self.0.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).paused }

    fn held(&self, id: u64) -> (Duration, bool) {
        let state = self.0.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.builders.get(&id).map_or((Duration::ZERO, false),
            |b| (b.held_for + b.held_since.map_or(Duration::ZERO, |at| at.elapsed()), b.held_since.is_some()))
    }
    fn change(&self, edit: impl FnOnce(&mut State)) {
        let mut state = self.0.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        edit(&mut state);
        self.stop_or_continue(&mut state);
    }
    fn stop_or_continue(&self, state: &mut State) {
        if state.gates.is_empty() {
            if state.paused {
                for b in state.builders.values_mut().filter(|b| !b.manual) {
                    b.continue_from_gate();
                }
                if let Some(record) = &self.0.record { let _ = std::fs::remove_file(record); }
            }
            state.paused = false;
            state.queue_latch = None;
            return;
        }
        // The full set that is about to be stopped is on disk, atomically, before any of it stops. If it can't be
        // written, nothing new is stopped and whatever was stopped is continued: a builder never stays stopped with
        // no record that a restarted daemon could resume it from.
        if let Err(e) = self.write(state) {
            tracing::warn!(error = %e, "could not record the builders to pause");
            for b in state.builders.values_mut().filter(|b| !b.manual) {
                b.continue_from_gate();
            }
            state.paused = false;
            state.queue_latch = None;
            return;
        }
        // Also covers a builder that is new, and children a native agent forked before it stopped.
        stop_builders(state, &self.0.queue);
        state.paused = true;
        if !state.ticking { state.ticking = self.tick(); }
    }
    /// Replace the record in one step: a reader sees the whole old set or the whole new set, never a part.
    fn write(&self, state: &State) -> std::io::Result<()> {
        let Some(record) = &self.0.record else { return Ok(()) };
        let targets: Vec<&Target> = state.builders.values().map(|b| &b.target).collect();
        let bytes = serde_json::to_vec(&targets).map_err(std::io::Error::other)?;
        let temp = record.with_extension("json.tmp");
        {
            let mut file = std::fs::File::create(&temp)?;
            std::io::Write::write_all(&mut file, &bytes)?;
            file.sync_all()?;
        }
        std::fs::rename(&temp, record)?;
        if let Some(dir) = record.parent() { std::fs::File::open(dir)?.sync_all()?; }
        Ok(())
    }
    /// While a gate runs, a native PTY may fork descendants before they are stopped: sweep again.
    fn tick(&self) -> bool {
        let Ok(handle) = tokio::runtime::Handle::try_current() else { return false };
        let weak: Weak<Inner> = Arc::downgrade(&self.0);
        handle.spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let Some(inner) = weak.upgrade() else { return };
                let mut state = inner.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if state.gates.is_empty() || !state.paused { state.ticking = false; return; }
                stop_builders(&mut state, &inner.queue);
            }
        });
        true
    }
}

#[cfg(test)]
mod tests;
