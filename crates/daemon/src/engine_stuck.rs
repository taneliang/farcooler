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

/// How long an agent has to be heard from before it is reported as not started.
#[cfg(not(test))]
pub(crate) const START_WITHIN: Duration = Duration::from_secs(20);
#[cfg(test)]
pub(crate) const START_WITHIN: Duration = Duration::from_millis(300);

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

/// Unlists the build when its run ends, however it ends.
struct Listed(Key);
impl Drop for Listed {
    fn drop(&mut self) {
        clear(&self.0);
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

/// Every watcher that tells clients the Needs You list moved. Nothing polls that list, so a build that
/// becomes stuck, or stops being, has to say so itself.
static LISTENERS: LazyLock<Mutex<Vec<std::sync::Weak<crate::watch::Watcher>>>> = LazyLock::new(Default::default);

/// A watcher asks to hear when the list changes.
pub(crate) fn listen(watcher: &std::sync::Arc<crate::watch::Watcher>) {
    if let Ok(mut all) = LISTENERS.lock() { all.push(std::sync::Arc::downgrade(watcher)); }
}

fn announce() {
    let live: Vec<_> = LISTENERS.lock().map(|mut all| {
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
    workspace: &str, item: &str, command: &str, tmux: &TmuxServer, pane: &str, heard: &AtomicBool,
) -> std::convert::Infallible {
    let key = (workspace.to_string(), item.to_string());
    let _listed = Listed(key.clone());
    let opened = tokio::time::Instant::now();
    loop {
        tokio::time::sleep(LOOK).await;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        // A screen that can't be read says nothing, so what was said stays.
        let screen = tmux.capture_screen(pane).await.unwrap_or_default();
        match asking(command, &screen) {
            Some(question) => set(&key, question, now_ms),
            None if !heard.load(Ordering::Relaxed) && opened.elapsed() >= START_WITHIN => {
                set(&key, format!("Hasn’t started: nothing from {command} in the first {} seconds.", START_WITHIN.as_secs()), now_ms)
            }
            None => clear(&key),
        }
    }
}

#[cfg(test)]
#[path = "engine_stuck_tests.rs"]
mod tests;
