//! Builders pause while a landing gate runs, and resume after (ov-1214).
//!
//! A gate that shares ten cores with four builders and their compilers ran 40 minutes at load 40. Niceness can't
//! be taken back without privilege, so a builder is paused with SIGSTOP and resumed with SIGCONT instead. The
//! engine records each builder's process group (or its native PTY's tty) while it runs, and each landing gate
//! while it runs. Whenever any gate is running every recorded builder is stopped, a builder that starts during a
//! gate is stopped at once, and when the last gate ends every builder is continued. A pause is written to disk, so
//! a daemon that died mid-gate continues what it had stopped when it starts again.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

/// What a builder's processes are found by.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Target {
    /// A process group the engine created, led by `pid`, which started at `started` (so a recycled pid is never ours).
    Group { pid: u32, started: String },
    /// A native agent's PTY: its tty and every descendant of what runs on it.
    Tty(String),
}

#[derive(Default)]
struct State {
    gates: BTreeMap<u64, ()>,
    builders: BTreeMap<u64, Target>,
    next: u64,
    paused: bool,
    ticking: bool,
}
pub(crate) struct Inner { state: Mutex<State>, record: Option<PathBuf> }

/// Shared by every copy of the engine's executor.
#[derive(Clone)]
pub(crate) struct Yielding(Arc<Inner>);
impl Default for Yielding { fn default() -> Self { Self(Arc::new(Inner { state: Default::default(), record: None })) } }

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
        self.0.change(|s| { if let Some(target) = s.builders.remove(&self.1) { signal(&target, libc::SIGCONT); } });
    }
}

fn signal(target: &Target, sig: i32) {
    match target {
        Target::Group { pid, started } => {
            // The leader is the identity: a group whose leader started at another time is not ours.
            if !started.is_empty() && crate::effect_runner::Owner::started_at(*pid).is_ok_and(|actual| actual == *started) {
                unsafe { libc::killpg(*pid as i32, sig); }
            }
        }
        Target::Tty(tty) => {
            let Some(raw) = crate::proc_table::snapshot() else { return };
            for pid in crate::foreground::parse(&raw).under_tty(tty.trim_start_matches("/dev/")) {
                if pid > 1 && pid != std::process::id() as i32 { unsafe { libc::kill(pid, sig); } }
            }
        }
    }
}

impl Yielding {
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
        Self(Arc::new(Inner { state: Default::default(), record: Some(record) }))
    }
    /// A landing gate started. The builders stop until every gate has ended.
    pub(crate) fn gate(&self) -> GateGuard {
        let mut id = 0;
        self.change(|s| { s.next += 1; id = s.next; s.gates.insert(id, ()); });
        GateGuard(self.clone(), id)
    }
    /// A builder started. It stops at once if a gate is running.
    pub(crate) fn builder(&self, target: Target) -> BuilderGuard {
        let mut id = 0;
        self.change(|s| { s.next += 1; id = s.next; s.builders.insert(id, target); });
        BuilderGuard(self.clone(), id)
    }
    /// Whether builders are paused now.
    #[cfg(test)]
    pub(crate) fn paused(&self) -> bool { self.0.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).paused }

    fn change(&self, edit: impl FnOnce(&mut State)) {
        let mut state = self.0.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        edit(&mut state);
        if state.gates.is_empty() {
            if state.paused {
                for target in state.builders.values() { signal(target, libc::SIGCONT); }
                if let Some(record) = &self.0.record { let _ = std::fs::remove_file(record); }
            }
            state.paused = false;
            return;
        }
        // The full set that is about to be stopped is on disk, atomically, before any of it stops. If it can't be
        // written, nothing new is stopped and whatever was stopped is continued: a builder never stays stopped with
        // no record that a restarted daemon could resume it from.
        if let Err(e) = self.write(&state) {
            tracing::warn!(error = %e, "could not record the builders to pause; they keep running during this gate");
            for target in state.builders.values() { signal(target, libc::SIGCONT); }
            state.paused = false;
            return;
        }
        // Also covers a builder that is new, and children a native agent forked before it stopped.
        for target in state.builders.values() { signal(target, libc::SIGSTOP); }
        state.paused = true;
        if !state.ticking { state.ticking = true; self.tick(); }
    }
    /// Replace the record in one step: a reader sees the whole old set or the whole new set, never a part.
    fn write(&self, state: &State) -> std::io::Result<()> {
        let Some(record) = &self.0.record else { return Ok(()) };
        let targets: Vec<&Target> = state.builders.values().collect();
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
    fn tick(&self) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else { self.0.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).ticking = false; return };
        let weak: Weak<Inner> = Arc::downgrade(&self.0);
        handle.spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let Some(inner) = weak.upgrade() else { return };
                let mut state = inner.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if state.gates.is_empty() || !state.paused { state.ticking = false; return; }
                for target in state.builders.values() { signal(target, libc::SIGSTOP); }
            }
        });
    }
}

#[cfg(test)]
mod tests;
