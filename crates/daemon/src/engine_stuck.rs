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
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use farcooler_core::activity::Registry;
use farcooler_tmux::TmuxServer;

use crate::native_hold;

/// How often a running build's pane is read.
#[cfg(not(test))]
const LOOK: Duration = Duration::from_secs(2);
#[cfg(test)]
const LOOK: Duration = Duration::from_millis(50);

/// How long an agent has to be heard from once its launch barrier is released.
const START_WITHIN: Duration = Duration::from_secs(60);
pub(crate) const QUARANTINE_PREFIX: &str = "Agent launch blocked by macOS quarantine.";

/// How long a build may print nothing and use no CPU before it is reported stalled (ov-1515): long enough for
/// any quiet wait an agent is likely to be in, short enough that its slot isn't lost for the turn's three hours.
const STALLED_AFTER: Duration = Duration::from_secs(10 * 60);
/// How often the process table is read for it.
const PULSE_EVERY: Duration = Duration::from_secs(30);

/// CPU time each of an agent's processes has used, by pid.
pub(crate) type Cpu = HashMap<i32, Duration>;

/// How long an agent has been quiet: no output, and next to no CPU.
#[derive(Default)]
pub(crate) struct Quiet { since: Duration, output: u64, cpu: Option<Cpu>, used: Duration }
impl Quiet {
    /// One look at `now`, with the bytes its terminal has printed so far and its processes' CPU times. True once
    /// it has been quiet for `after`. CPU that can't be read starts the quiet again: it is not "no CPU".
    pub(crate) fn look(&mut self, now: Duration, output: u64, cpu: Option<Cpu>, after: Duration) -> bool {
        let (Some(cpu), Some(before)) = (&cpu, &self.cpu) else {
            *self = Quiet { since: now, output, cpu, used: Duration::ZERO };
            return false;
        };
        // What each process used since the last look; a new one's whole time. One that has ended takes nothing back.
        self.used += cpu.iter().map(|(pid, time)| time.saturating_sub(before.get(pid).copied().unwrap_or_default())).sum();
        let quiet = now.saturating_sub(self.since);
        // An idle terminal program still wakes now and then: up to 2% of one core, or three seconds in all.
        if output != self.output || self.used > (quiet / 50).max(Duration::from_secs(3)) {
            *self = Quiet { since: now, output, cpu: Some(cpu.clone()), used: Duration::ZERO };
            return false;
        }
        self.cpu = Some(cpu.clone());
        quiet >= after
    }
}

/// `ps`'s CPU time: `M:SS.ss` on macOS, `[D-]HH:MM:SS` on Linux.
fn cpu_time(text: &str) -> Option<Duration> {
    let (days, clock) = match text.split_once('-') { Some((d, rest)) => (d.parse::<u64>().ok()?, rest), None => (0, text) };
    let mut seconds = 0.0;
    for part in clock.split(':') { seconds = seconds * 60.0 + part.parse::<f64>().ok()?; }
    Duration::try_from_secs_f64(days as f64 * 86_400.0 + seconds).ok()
}

/// The CPU times of `root` and everything under it, from `ps -axo pid=,ppid=,time=`. `None` when `root` isn't there.
pub(crate) fn cpu_under(table: &str, root: i32) -> Option<Cpu> {
    let rows: Vec<(i32, i32, Duration)> = table.lines().filter_map(|line| {
        let mut words = line.split_whitespace();
        Some((words.next()?.parse().ok()?, words.next()?.parse().ok()?, cpu_time(words.next()?)?))
    }).collect();
    let mut found = Cpu::new();
    let mut parents = vec![root];
    found.insert(root, rows.iter().find(|r| r.0 == root)?.2);
    while let Some(parent) = parents.pop() {
        for (pid, _, time) in rows.iter().filter(|r| r.1 == parent) {
            if found.insert(*pid, *time).is_none() { parents.push(*pid); }
        }
    }
    Some(found)
}

/// Where the watched agent's processes start.
pub(crate) enum Agent {
    /// The agent's hold, which knows its leader.
    Hold(PathBuf),
    #[cfg(test)]
    Process(i32),
}

/// What tells whether a build is still doing anything: the log of what its terminal prints, and its processes.
pub(crate) struct Pulse { pub(crate) log: PathBuf, pub(crate) agent: Agent, pub(crate) after: Duration, pub(crate) every: Duration }
impl Pulse {
    pub(crate) fn of(log: PathBuf, hold: PathBuf) -> Self {
        Self { log, agent: Agent::Hold(hold), after: STALLED_AFTER, every: PULSE_EVERY }
    }

    pub(crate) async fn cpu(&self) -> Option<Cpu> {
        let leader = match &self.agent {
            Agent::Hold(socket) => match native_hold::ask(socket, "group", Duration::from_secs(1)).await {
                native_hold::Answer::Said(line) => line.trim().parse().ok()?,
                _ => return None,
            },
            #[cfg(test)]
            Agent::Process(pid) => *pid,
        };
        let ps = tokio::process::Command::new("ps").args(["-axo", "pid=,ppid=,time="]).kill_on_drop(true).output();
        let out = tokio::time::timeout(Duration::from_secs(5), ps).await.ok()?.ok()?;
        cpu_under(&String::from_utf8_lossy(&out.stdout), leader)
    }
}

/// What a stalled build is reported as: how long, and the last lines of its terminal.
fn stalled_report(command: &str, after: Duration, screen: &str) -> String {
    let lines: Vec<&str> = screen.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
    let last: Vec<String> = lines[lines.len().saturating_sub(8)..].iter().map(|l| l.chars().take(200).collect()).collect();
    format!("{command} has printed nothing and used no CPU for {} minutes. The last lines of its terminal:\n{}",
        after.as_secs() / 60, last.join("\n"))
}

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
    if let Ok(waiting) = leftovers().lock() {
        all.retain(|s| !waiting.values().any(|w| w.workspace == s.workspace && w.item == s.item));
        all.extend(waiting.values().cloned());
    }
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
            if run.phase != Phase::Uncertain || leftover_wait(&workspace, &run).is_some() { continue; }
            let Some(question) = run.failure else { continue };
            items.push(Stuck { workspace: workspace.clone(), item, since_ms: run.since_ms, question });
        }
    }
    items
}

/// Hold observations are keyed by effect so a replaced step cannot inherit
/// an earlier agent's wait. The active turn or recovery owns their lifetime.
type LeftoverTable = Mutex<HashMap<Key, Stuck>>;
#[cfg(not(test))]
static LEFTOVERS: LazyLock<LeftoverTable> = LazyLock::new(Default::default);
#[cfg(test)]
thread_local! { static LEFTOVERS: &'static LeftoverTable = Box::leak(Box::default()); }
fn leftovers() -> &'static LeftoverTable {
    #[cfg(not(test))]
    { &LEFTOVERS }
    #[cfg(test)]
    { LEFTOVERS.with(|t| *t) }
}

pub(crate) struct Leftovers { key: Key, item: String }
impl Leftovers {
    pub(crate) fn new(workspace: &str, effect: &farcooler_workflow::Effect) -> Self {
        Self { key: (workspace.into(), effect.id.clone()), item: effect.item.clone() }
    }

    pub(crate) fn report(&self, reason: Option<String>) {
        let changed = leftovers().lock().is_ok_and(|mut table| {
            let Some(question) = reason else { return table.remove(&self.key).is_some() };
            let since_ms = table.get(&self.key).map_or_else(now_ms, |s| s.since_ms);
            let new = Stuck { workspace: self.key.0.clone(), item: self.item.clone(), since_ms, question };
            table.insert(self.key.clone(), new.clone()).as_ref() != Some(&new)
        });
        if changed { announce_change(uuid::Uuid::parse_str(&self.key.0).ok()); }
    }
}
impl Drop for Leftovers {
    fn drop(&mut self) { self.report(None); }
}

fn leftover_wait(workspace: &str, item: &farcooler_workflow::Item) -> Option<String> {
    let table = leftovers().lock().ok()?;
    [item.pending.as_ref(), item.stopping.as_ref()].into_iter().flatten()
        .find_map(|effect| table.get(&(workspace.into(), effect.clone())).map(|s| s.question.clone()))
}

pub(crate) fn project_leftover_wait(workspace: &str, item: &mut farcooler_workflow::Item) -> bool {
    let Some(reason) = leftover_wait(workspace, item) else { return false };
    item.execution = Some(farcooler_workflow::ExecutionView {
        phase: item.phase.clone(), position: None, holders: Vec::new(), text: reason.clone(), heavy_lock_wait: None,
    });
    item.reason = Some(reason);
    true
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// A quarantine refusal stays listed while its run is failed. Retry clears it from the run.
pub(crate) fn include_refusals(store: &farcooler_store::Store, stuck: &mut Vec<Stuck>) {
    match store.workflow_failures_with_prefix(QUARANTINE_PREFIX) {
        Ok(failures) => {
            for failure in failures {
                if stuck.iter().any(|s| s.workspace == failure.workspace && s.item == failure.item) {
                    continue;
                }
                stuck.push(Stuck {
                    workspace: failure.workspace,
                    item: failure.item,
                    since_ms: failure.since_ms,
                    question: failure.reason,
                });
            }
        }
        Err(error) => tracing::warn!(%error, "could not read refused agent launches"),
    }
}

/// What an item that speaks for a provider has in place of a ticket's key.
pub(crate) const LIMIT_ITEM: &str = "provider-limit:";

/// One entry for each provider in each workspace whose runs stopped at its limit, however many tickets that
/// stopped (ov-1515). It stays while any of them is failed there.
pub(crate) fn provider_limits(failures: &[farcooler_store::workflow::WorkflowFailure]) -> Vec<Stuck> {
    let mut grouped: std::collections::BTreeMap<(&str, &str), Vec<(&farcooler_store::workflow::WorkflowFailure, &str)>> = Default::default();
    for failure in failures {
        if let Some((harness, until)) = crate::provider_limit::parse(&failure.reason) {
            grouped.entry((&failure.workspace, harness)).or_default().push((failure, until));
        }
    }
    grouped.into_iter().map(|((workspace, harness), mut stopped)| {
        stopped.sort_by(|a, b| (a.0.since_ms, &a.0.item).cmp(&(b.0.since_ms, &b.0.item)));
        // The latest word on when it lifts.
        let (_, until) = stopped[stopped.len() - 1];
        let mut keys: Vec<&str> = stopped.iter().map(|(f, _)| f.item.as_str()).collect();
        keys.sort_unstable();
        let name = if harness == "codex" { "Codex" } else { "Claude" };
        Stuck {
            workspace: workspace.into(),
            item: format!("{LIMIT_ITEM}{name}"),
            since_ms: stopped[0].0.since_ms,
            question: format!("{name} is at its usage limit until {until}. It stopped {}. To move one to another model, run \
                `farcooler workflow model {workspace} <key> <model>`. Or retry it after the limit lifts.", keys.join(", ")),
        }
    }).collect()
}

/// List each provider's limit, and return the cards it speaks for: each stopped run's, and those of the tickets
/// that follow it.
pub(crate) fn include_limits(store: &farcooler_store::Store, stuck: &mut Vec<Stuck>) -> std::collections::HashSet<uuid::Uuid> {
    let mut cards = std::collections::HashSet::new();
    let failures = match store.workflow_failures_with_prefix(crate::provider_limit::PREFIX) {
        Ok(failures) => failures,
        Err(error) => { tracing::warn!(%error, "could not read runs stopped at a usage limit"); return cards; }
    };
    for failure in &failures {
        let followers = store.workflow_state(&failure.workspace).ok().flatten()
            .and_then(|state| state.items.get(&failure.item).map(|item| item.followers.clone())).unwrap_or_default();
        for key in std::iter::once(&failure.item).chain(&followers) {
            if let Ok(Some(card)) = store.workflow_item_task(&failure.workspace, key) { cards.insert(card); }
        }
    }
    stuck.extend(provider_limits(&failures));
    cards
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

    /// `quiet` is the report of a build that has gone quiet (`Quiet`), which counts as stalled whether or not it
    /// was ever heard from.
    pub(crate) fn observe(&self, heard: bool, held: bool, question: Option<&str>, quiet: Option<&str>) {
        let stalled = !heard && self.opened.elapsed() >= START_WITHIN;
        let fallback = stalled.then(|| format!(
            "{} has not reported its start within {} seconds. {}",
            self.command, START_WITHIN.as_secs(),
            if held { "The builder is paused for a landing check." }
            else { "The agent may be blocked during launch or waiting for input." },
        ));
        let reason = question.or(quiet).or(fallback.as_deref());
        if let Some(activity) = &self.activity
            && let Ok(mut state) = activity.startup_stall.lock() {
            *state = reason.filter(|_| stalled || quiet.is_some()).map(str::to_owned);
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

pub(crate) fn announce() { announce_change(None); }

fn announce_change(board: Option<uuid::Uuid>) {
    let live: Vec<_> = listeners().lock().map(|mut all| {
        all.retain(|w| w.strong_count() > 0);
        all.iter().filter_map(|w| w.upgrade()).collect()
    }).unwrap_or_default();
    for watcher in live {
        watcher.announce_needs_you();
        if let Some(board) = board {
            watcher.announce_plan_changed(board, farcooler_store::models::Actor::Runner);
        }
    }
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
    builder: Option<&crate::yielding::BuilderGuard>, pulse: Option<Pulse>,
) -> std::convert::Infallible {
    let mut quiet = Quiet::default();
    let mut pulsed: Option<tokio::time::Instant> = None;
    let mut stalled: Option<String> = None;
    let mut last_screen = String::new();
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
                if let Ok(screen) = screen {
                    question = asking(&startup.command, &screen);
                    last_screen = screen;
                }
            }
        }
        let held = builder.is_some_and(|b| b.held().1);
        if let Some(pulse) = &pulse
            && pulsed.is_none_or(|at| at.elapsed() >= pulse.every) {
            pulsed = Some(tokio::time::Instant::now());
            let output = std::fs::metadata(&pulse.log).map_or(0, |m| m.len());
            // A build held for a landing check is quiet because the engine stopped it.
            let cpu = if held { None } else { pulse.cpu().await };
            stalled = quiet.look(startup.opened.elapsed(), output, cpu, pulse.after)
                .then(|| stalled_report(&startup.command, pulse.after, &last_screen));
        }
        startup.observe(heard.load(Ordering::Relaxed), held, question.as_deref(), stalled.as_deref());
    }
}

#[cfg(test)]
#[path = "engine_stuck_tests.rs"]
mod tests;
