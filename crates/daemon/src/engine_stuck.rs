//! An engine agent that is stuck on a question shows in Needs You (ov-1077).
//!
//! The engine answers every hook with `{}`, and its agents run in a tmux server of their own that the
//! daemon's watcher never samples. So a claude that stopped at the keyboard (a trust or permission question)
//! waited unseen until the turn's three-hour timeout. `watch` runs beside each native build's turn: every
//! `LOOK` it reads the pane, and lists the build here when
//!
//! - the pane shows a question (`Registry::blocked_question`), with that question; or
//! - no hook of the agent's session has been heard `START_WITHIN` after it opened, whatever the pane shows.
//!
//! `needs_you::gather` reads `list`; a build leaves it when the question goes, its first hook arrives, or
//! its run ends (`Listed`'s drop). Nothing here types into the pane: the person answers it by watching it
//! (ov-972) or by opening it.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use farcooler_core::activity::Registry;
use farcooler_tmux::TmuxServer;

/// How often a running build's pane is read.
#[cfg(not(test))]
const LOOK: Duration = Duration::from_secs(2);
#[cfg(test)]
const LOOK: Duration = Duration::from_millis(50);

/// How long an agent has to be heard from once its launch barrier is released.
const START_WITHIN: Duration = Duration::from_secs(60);
pub(crate) const QUARANTINE_PREFIX: &str = "Agent launch blocked by macOS quarantine.";

/// One stuck build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stuck {
    pub workspace: String,
    pub item: String,
    /// Unix milliseconds, from when it was first seen stuck.
    pub since_ms: i64,
    /// What the pane asks, or why nothing has begun.
    pub question: String,
}

type Key = (String, String);
type Table = Mutex<HashMap<Key, Stuck>>;

/// One list for the process. Under test, one per thread (each test runs on its own), so a test of the
/// list a runner reads (`needs_you::gather`) never sees another test's builds.
#[cfg(not(test))]
static STUCK: LazyLock<Table> = LazyLock::new(Default::default);
#[cfg(test)]
thread_local! { static TABLE: &'static Table = Box::leak(Box::default()); }
#[cfg(test)]
fn table() -> &'static Table { TABLE.with(|t| *t) }
#[cfg(not(test))]
fn table() -> &'static Table { &STUCK }

/// Every build stuck now, oldest first.
pub fn list() -> Vec<Stuck> {
    let mut all: Vec<Stuck> = table().lock().map(|m| m.values().cloned().collect()).unwrap_or_default();
    all.sort_by(|a, b| (a.since_ms, &a.item).cmp(&(b.since_ms, &b.item)));
    all
}

/// The engine stores the result of an inconclusive probe on the fenced item.
pub(crate) fn uncertain(store: &farcooler_store::Store) -> Vec<Stuck> {
    use farcooler_workflow::Phase;
    let mut items = Vec::new();
    for workspace in store.workflow_workspaces().unwrap_or_default() {
        let Ok(Some(state)) = store.workflow_state(&workspace) else { continue };
        for (item, run) in state.items {
            if run.phase != Phase::Uncertain { continue; }
            let Some(question) = run.failure else { continue };
            items.push(Stuck { workspace: workspace.clone(), item, since_ms: run.since_ms, question });
        }
    }
    items
}

/// Keep the more specific quarantine diagnostic, including while an automatic retry is queued.
pub(crate) fn include_refusals(store: &farcooler_store::Store, stuck: &mut Vec<Stuck>) {
    match store.workflow_failures_with_prefix(QUARANTINE_PREFIX) {
        Ok(failures) => for (workspace, item, since_ms, question) in failures {
            stuck.retain(|s| s.workspace != workspace || s.item != item);
            stuck.push(Stuck { workspace, item, since_ms, question });
        },
        Err(error) => tracing::warn!(%error, "could not read refused agent launches"),
    }
}

/// Unlists the build when its run ends, however it ends.
pub(crate) struct Startup {
    key: Key,
    command: String,
    opened: tokio::time::Instant,
    activity: Option<std::sync::Arc<crate::workflow_runtime::execution::Activity>>,
}
impl Startup {
    pub(crate) fn new(workspace: &str, item: &str, command: &str) -> Self {
        Self {
            key: (workspace.into(), item.into()),
            command: command.into(),
            opened: tokio::time::Instant::now(),
            activity: crate::workflow_runtime::execution::ACTIVITY.try_with(std::sync::Arc::clone).ok(),
        }
    }

    pub(crate) fn observe(&self, heard: bool, held: bool, question: Option<&str>) {
        let stalled = !heard && self.opened.elapsed() >= START_WITHIN;
        let fallback = stalled.then(|| format!(
            "{} has not reported its start within {} seconds. {}",
            self.command, START_WITHIN.as_secs(),
            if held { "The builder is paused for a landing check." }
            else { "The agent may be blocked during launch or waiting for input." },
        ));
        let reason = question.or(fallback.as_deref());
        if let Some(activity) = &self.activity
            && let Ok(mut state) = activity.startup_stall.lock() {
            *state = reason.filter(|_| stalled).map(str::to_owned);
        }
        if let Some(reason) = reason {
            let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64);
            set(&self.key, reason.to_owned(), now_ms);
        } else {
            clear(&self.key);
        }
    }
}
impl Drop for Startup {
    fn drop(&mut self) {
        if let Some(activity) = &self.activity
            && let Ok(mut state) = activity.startup_stall.lock() {
            *state = None;
        }
        clear(&self.key);
    }
}

fn set(key: &Key, question: String, now_ms: i64) {
    let changed = table().lock().is_ok_and(|mut m| {
        let since_ms = m.get(key).map_or(now_ms, |s| s.since_ms);
        let new = Stuck { workspace: key.0.clone(), item: key.1.clone(), since_ms, question };
        m.insert(key.clone(), new.clone()).as_ref() != Some(&new)
    });
    if changed { announce(); }
}

fn clear(key: &Key) {
    if table().lock().is_ok_and(|mut m| m.remove(key).is_some()) { announce(); }
}

type Listeners = Mutex<Vec<std::sync::Weak<crate::watch::Watcher>>>;
/// Every watcher that tells clients the Needs You list moved. Nothing polls that list, so a build that
/// becomes stuck, or stops being, has to say so itself.
#[cfg(not(test))]
static LISTENERS: LazyLock<Listeners> = LazyLock::new(Default::default);
// Under test, per thread like `TABLE`, so one test's builds never wake another test's watcher.
#[cfg(test)]
thread_local! { static LISTENER_TABLE: &'static Listeners = Box::leak(Box::default()); }
#[cfg(test)]
fn listeners() -> &'static Listeners { LISTENER_TABLE.with(|t| *t) }
#[cfg(not(test))]
fn listeners() -> &'static Listeners { &LISTENERS }

/// A watcher asks to hear when the list changes.
pub(crate) fn listen(watcher: &std::sync::Arc<crate::watch::Watcher>) {
    if let Ok(mut all) = listeners().lock() { all.push(std::sync::Arc::downgrade(watcher)); }
}

pub(crate) fn announce() {
    let live: Vec<_> = listeners().lock().map(|mut all| {
        all.retain(|w| w.strong_count() > 0);
        all.iter().filter_map(|w| w.upgrade()).collect()
    }).unwrap_or_default();
    for watcher in live { watcher.announce_needs_you(); }
}

/// What a screen says of an agent that has not been heard from: its question, else nothing.
fn asking(command: &str, screen: &str) -> Option<String> {
    static RULES: LazyLock<Registry> = LazyLock::new(Registry::built_in);
    RULES.blocked_question(command, screen).filter(|q| !q.trim().is_empty())
}

/// Watch one build's pane until the caller drops this future. `heard` is set once the agent's session has
/// spoken. Never returns.
pub(crate) async fn watch(
    startup: Startup, tmux: &TmuxServer, pane: &str, heard: &AtomicBool,
    builder: Option<&crate::yielding::BuilderGuard>,
) -> std::convert::Infallible {
    let mut look = tokio::time::interval_at(startup.opened + LOOK, LOOK);
    look.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let deadline = tokio::time::sleep_until(startup.opened + START_WITHIN);
    tokio::pin!(deadline);
    let mut deadline_passed = false;
    let mut question = None;
    loop {
        tokio::select! {
            biased;
            _ = &mut deadline, if !deadline_passed => deadline_passed = true,
            screen = async {
                look.tick().await;
                tmux.capture_screen(pane).await
            } => {
                if let Ok(screen) = screen { question = asking(&startup.command, &screen); }
            }
        }
        startup.observe(heard.load(Ordering::Relaxed), builder.is_some_and(|b| b.held().1), question.as_deref());
    }
}

#[cfg(test)]
#[path = "engine_stuck_tests.rs"]
mod tests;
