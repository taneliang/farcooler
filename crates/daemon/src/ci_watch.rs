//! The runner's CI watch (ov-309, ov-306): reads GitHub Actions through `gh`,
//! read only, for every CI subject a board names, and keeps what it heard in
//! the store's `board_ci` (`farcooler_store::board_ci`).
//!
//! A board names a subject when an engine landing group pushes (`sha:<pushed
//! sha>`, ov-958). The watch reads each landing while CI is going and
//! retains finished results. A read
//! that changed what a board shows announces `plan_changed`. The old landings
//! are history and aren't followed (ov-960).
//!
//! Every `gh` call is `gh api -X GET` with a `--jq` filter, run in the
//! repository's main checkout through the daemon's own sandboxed launch
//! (`stack::gh`). The filters are constants here and the parsers read what they
//! print, so the test fixtures in `test/fixtures/ci/` are that exact output
//! from a real `gh` against this repository. A `gh` that's missing, logged out
//! or offline reads `unknown`, which never replaces a known read.
//!
//! Canary runs after CI and carries main's head, not the commit it builds, so
//! its run is found by name, and a commit reads queued until it exists
//! (`ci_canary`, ov-301).
//!
//! Part of the plan layer and as removable: only the real daemon runs it
//! (`main.rs`), never a `--stdio` session.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use farcooler_store::board_ci::{self, CiJob, CiRead, CiStatus};
use farcooler_store::models::Actor;
use serde::Deserialize;
use uuid::Uuid;

use crate::service::Service;
use crate::watch::Watcher;

/// The name of `.github/workflows/nightly.yml`'s workflow.
pub const NIGHTLY: &str = "Nightly";

/// How often subjects are read while any run is still going.
pub const BUSY: Duration = Duration::from_secs(60);
/// How often they're read once everything watched has finished.
pub const QUIET: Duration = Duration::from_secs(10 * 60);

/// What a list of runs is cut down to: the fields read here, nothing more.
pub const RUNS_JQ: &str =
    "[.workflow_runs[] | {id, name, head_sha, status, conclusion, html_url, created_at, display_title, event, updated_at}]";
/// A run's jobs.
pub const JOBS_JQ: &str = "[.jobs[] | {name, status, conclusion, html_url}]";

/// The watch, for the life of the daemon.
pub async fn run(svc: Arc<Service>, watcher: Arc<Watcher>) {
    let mut memo = Memo::default();
    loop {
        let Ok(ticket) = svc.writers.admit("landing-jobs") else { return };
        let wait = read_all(&svc, &watcher, &mut memo).await;
        ticket.finish();
        tokio::time::sleep(wait).await;
    }
}

/// What the watch remembers between turns: full SHAs for short ones, and a
/// run's jobs while the run hasn't changed, so a finished run costs one call.
#[derive(Default)]
pub struct Memo {
    shas: HashMap<String, String>,
    completed_jobs: HashMap<(Uuid, u64), Option<Vec<GhJob>>>,
    completed_order: VecDeque<(Uuid, u64)>,
    last_attempt: HashMap<(Uuid, String), i64>,
    /// Subjects GitHub didn't answer for, or had no run for: how many reads
    /// in a row, and when to ask again (review train-1005c M2).
    misses: HashMap<(Uuid, String), (u32, i64)>,
}

/// How long after `misses` reads in a row that GitHub couldn't or didn't
/// answer to ask again: a minute, doubling, at most `QUIET`. A runner whose gh
/// is logged out settles at one read
/// every ten minutes instead of one a minute forever.
pub fn backoff_after(misses: u32) -> Duration {
    let doubled = BUSY.saturating_mul(1u32 << misses.saturating_sub(1).min(16));
    doubled.min(QUIET)
}

/// Whether a read keeps the watch at its busy pace: only a run that's going
/// or waiting. A subject GitHub can't answer for, or one with no runs, backs
/// off on its own instead (`backoff_after`).
pub fn keeps_busy(status: CiStatus) -> bool {
    matches!(status, CiStatus::Running | CiStatus::Queued)
}

/// How long the watch sleeps: a minute while anything runs, else until the
/// first backed-off subject is due, between a minute and `QUIET`.
pub fn next_wait(busy: bool, first_due_ms: Option<i64>, now_ms: i64) -> Duration {
    if busy {
        return BUSY;
    }
    match first_due_ms {
        Some(due) => Duration::from_millis(due.saturating_sub(now_ms).max(0) as u64).clamp(BUSY, QUIET),
        None => QUIET,
    }
}

/// One run, as `RUNS_JQ` prints it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GhRun {
    pub id: u64,
    pub name: String,
    pub head_sha: String,
    pub status: String,
    #[serde(default)]
    pub conclusion: Option<String>,
    pub html_url: String,
    pub created_at: String,
    /// The run's name: Canary's carries the commit it builds (`ci_canary`).
    #[serde(default)]
    pub display_title: String,
    #[serde(default)]
    pub event: String,
    #[serde(default)]
    pub updated_at: String,
}

/// One job, as `JOBS_JQ` prints it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GhJob {
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub conclusion: Option<String>,
    #[serde(default)]
    pub html_url: String,
}

/// `RUNS_JQ`'s output.
pub fn parse_runs(bytes: &[u8]) -> Option<Vec<GhRun>> {
    serde_json::from_slice(bytes).map_err(|e| tracing::warn!(error = %e, "could not read gh's runs")).ok()
}

/// One run from a CI receipt.
pub fn parse_run(bytes: &[u8]) -> Option<GhRun> {
    serde_json::from_slice(bytes).map_err(|e| tracing::warn!(error = %e, "could not read gh's run")).ok()
}

/// `JOBS_JQ`'s output.
pub fn parse_jobs(bytes: &[u8]) -> Option<Vec<GhJob>> {
    serde_json::from_slice(bytes).map_err(|e| tracing::warn!(error = %e, "could not read gh's jobs")).ok()
}

/// The newest run of each workflow: a workflow run again on the same commit (a
/// re-trigger, or a chained run) replaces its older run. In the order GitHub
/// listed them, newest first.
///
/// Never the nightly iOS UI run (`NIGHTLY`, ov-534). It carries main's head at
/// 11:23 UTC, so it would sit beside a train's CI as an hour of "running" and,
/// with one red shard, turn a green push into "failed". It is a verdict on
/// main, reported by its own issue, not on the commit a train pushed.
pub fn latest_per_workflow(runs: &[GhRun]) -> Vec<GhRun> {
    let mut newest: BTreeMap<&str, &GhRun> = BTreeMap::new();
    for run in runs.iter().filter(|r| r.name != NIGHTLY) {
        let slot = newest.entry(run.name.as_str()).or_insert(run);
        if run.id > slot.id {
            *slot = run;
        }
    }
    let kept: BTreeSet<u64> = newest.values().map(|r| r.id).collect();
    runs.iter().filter(|r| kept.contains(&r.id)).cloned().collect()
}

fn run_state(run: &GhRun) -> &'static str {
    board_ci::job_state(&run.status, run.conclusion.as_deref())
}

/// How long a completion event has to create a retry or recovery workflow run.
const RETRY_EVENT_WAIT_SECS: i64 = 15 * 60;
// Keep this ceiling in sync with scripts/ci-runner-retry.py.
const MAX_CI_ATTEMPTS: u64 = 8;

pub(crate) enum RetryRead {
    Data(Vec<u8>),
    Missing,
    Unavailable,
}

impl RetryRead {
    pub(crate) fn error(stderr: &[u8]) -> Self {
        // gh's error when this repository does not have the retry workflow.
        if String::from_utf8_lossy(stderr).contains("(HTTP 404)") { Self::Missing } else { Self::Unavailable }
    }
}

/// GitHub reports each attempt as completed even while our retry workflow is
/// being scheduled or submitting the rerun. Follow that workflow by source
/// run ID and attempt, then follow the active rerun on main.
pub(crate) async fn retry_pending<F, Fut>(
    run: &GhRun,
    attempt: Option<u64>,
    now_secs: i64,
    read: F,
) -> bool
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = RetryRead>,
{
    if run.name != "CI" || run.event != "push" { return false; }
    let active = run.status != "completed";
    if active {
        // The landing adapter includes the attempt number. The board already
        // shows an active run as queued or running.
        if attempt.is_none_or(|n| n <= 1) { return false; }
    } else if !matches!(run.conclusion.as_deref(), Some("failure" | "cancelled")) {
        return false;
    }

    #[derive(Deserialize)]
    struct Source {
        id: u64,
        head_sha: String,
        head_branch: String,
        status: String,
        conclusion: Option<String>,
        run_attempt: u64,
        #[serde(default)]
        updated_at: String,
    }
    #[derive(Deserialize)]
    struct Controller {
        id: u64,
        display_title: String,
        status: String,
        conclusion: Option<String>,
        #[serde(default)]
        updated_at: String,
    }
    #[derive(Deserialize)]
    struct Controllers {
        workflow_runs: Vec<Controller>,
    }

    let recent = |stamp: &str| crate::ci_canary::epoch_secs(stamp)
        .is_some_and(|finished| now_secs.saturating_sub(finished) < RETRY_EVENT_WAIT_SECS);
    let source_path = format!("actions/runs/{}", run.id);
    let source = || async {
        let RetryRead::Data(raw) = read(source_path.clone()).await else { return None };
        serde_json::from_slice::<Source>(&raw).ok()
            .filter(|s| s.id == run.id && s.head_sha == run.head_sha)
    };
    let Some(current) = source().await else { return true };
    if current.head_branch != "main" { return false; }
    if active || current.status != "completed" || current.conclusion != run.conclusion
        || attempt.is_some_and(|n| current.run_attempt != n)
        || (!run.updated_at.is_empty() && !current.updated_at.is_empty() && current.updated_at != run.updated_at) {
        return true;
    }

    if current.run_attempt >= MAX_CI_ATTEMPTS { return false; }

    let title = format!("Retry CI {} attempt {}", run.id, current.run_attempt);
    let updated_at = if current.updated_at.is_empty() { &run.updated_at } else { &current.updated_at };
    let read = &read;
    let controller = |workflow: &'static str, title: String| async move {
        let mut path = format!("actions/workflows/{workflow}/runs?event=workflow_run&per_page=100");
        if crate::ci_canary::epoch_secs(updated_at).is_some() {
            path.push_str(&format!("&created=%3E%3D{updated_at}"));
        }
        match read(path).await {
            RetryRead::Data(raw) => serde_json::from_slice::<Controllers>(&raw)
                .map(|runs| runs.workflow_runs.into_iter()
                    .filter(|r| r.display_title == title).max_by_key(|r| r.id))
                .map_err(|_| RetryRead::Unavailable),
            error => Err(error),
        }
    };
    let first = match controller("ci-runner-retry.yml", title.clone()).await {
        Ok(Some(first)) => first,
        Ok(None) => return recent(updated_at),
        Err(RetryRead::Missing) => return false,
        Err(_) => return true,
    };
    if first.status != "completed" { return true; }

    if first.conclusion.as_deref() != Some("success") {
        // A failed controller gets one independent recovery workflow. Measure
        // its delivery grace from the primary controller's completion.
        match controller("ci-runner-retry-recover.yml", format!("Recover {title}")).await {
            Ok(Some(recovery)) if recovery.status != "completed" => return true,
            Ok(Some(_)) => {}
            Ok(None) | Err(RetryRead::Missing) if recent(&first.updated_at) => return true,
            Ok(None) | Err(RetryRead::Missing) => {}
            Err(_) => return true,
        }
    }

    // A POST may have advanced CI even when the controller was interrupted.
    source().await.is_none_or(|fresh| fresh.status != "completed"
        || fresh.run_attempt != current.run_attempt)
}

/// What the runs (each with its jobs, when they were read) say about a
/// subject. Jobs are named "workflow / job"; a run whose jobs couldn't be read
/// stands as one line of its own.
pub fn summarize(subject: &str, runs: &[(GhRun, Option<Vec<GhJob>>)]) -> CiRead {
    let states: Vec<&str> = runs.iter().map(|(r, _)| run_state(r)).collect();
    let status = board_ci::status_of(&states);
    let failing = runs.iter().find(|(r, _)| run_state(r) == "failed");
    let url = failing.or(runs.first()).map(|(r, _)| r.html_url.clone()).unwrap_or_default();
    let mut jobs = Vec::new();
    for (run, its) in runs {
        match its {
            Some(its) if !its.is_empty() => jobs.extend(its.iter().map(|j| CiJob {
                name: format!("{} / {}", run.name, j.name),
                state: board_ci::job_state(&j.status, j.conclusion.as_deref()).to_string(),
                url: j.html_url.clone(),
            })),
            _ => jobs.push(CiJob { name: run.name.clone(), state: run_state(run).to_string(), url: run.html_url.clone() }),
        }
    }
    CiRead {
        subject: subject.to_string(),
        sha: runs.first().map(|(r, _)| r.head_sha.clone()).unwrap_or_default(),
        status,
        url,
        jobs,
        fetched_at: 0,
        changed_at: 0,
        asked_at: 0,
    }
}

/// A read that says `gh` couldn't answer.
fn unknown(subject: &str) -> CiRead {
    CiRead {
        subject: subject.to_string(),
        sha: String::new(),
        status: CiStatus::Unknown,
        url: String::new(),
        jobs: Vec::new(),
        fetched_at: 0,
        changed_at: 0,
        asked_at: 0,
    }
}

/// `gh api -X GET <path> -f <field>... --jq <jq>`'s arguments.
pub fn api_args(path: &str, fields: &[String], jq: &str) -> Vec<String> {
    let mut args = vec!["api".to_string(), "-X".into(), "GET".into(), path.to_string()];
    for field in fields {
        args.push("-f".into());
        args.push(field.clone());
    }
    args.push("--jq".into());
    args.push(jq.to_string());
    args
}

/// One `gh api` read in `tree`, or `None` for every way it can fail.
async fn gh_get(svc: &Service, tree: &Path, args: &[String]) -> Option<Vec<u8>> {
    gh_output(svc, tree, args).await.filter(|o| o.status.success()).map(|o| o.stdout)
}

async fn gh_output(svc: &Service, tree: &Path, args: &[String]) -> Option<std::process::Output> {
    let mut gh = crate::stack::gh(tree).await.ok()?;
    let _permit = svc.gh_permit().await;
    let out = tokio::time::timeout(
        crate::stack::GH_TIMEOUT,
        gh.current_dir(tree)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await;
    out.ok()?.ok()
}

/// A run with its jobs, from `memo` while the run is unchanged.
async fn with_jobs(
    svc: &Service,
    tree: &Path,
    repository: Uuid,
    memo: &mut Memo,
    run: GhRun,
) -> (GhRun, Option<Vec<GhJob>>) {
    if run.id == 0 {
        // The placeholder for a Canary run that has not started (`ci_canary`).
        return (run, None);
    }
    let key = (repository, run.id);
    if let Some(jobs) = memo.completed_jobs.get(&key) {
        return (run, jobs.clone());
    }
    let path = format!("repos/{{owner}}/{{repo}}/actions/runs/{}/jobs", run.id);
    let jobs = gh_get(svc, tree, &api_args(&path, &["per_page=100".into()], JOBS_JQ)).await.and_then(|b| parse_jobs(&b));
    if memo_holds(&run.status) {
        memo.remember_completed_jobs(key, jobs.clone());
    }
    (run, jobs)
}

/// The most completed runs or short SHAs the watch keeps in either cache.
const MEMO_MAX: usize = 512;

impl Memo {
    fn remember_completed_jobs(&mut self, key: (Uuid, u64), jobs: Option<Vec<GhJob>>) {
        // Only called on a cache miss. Evict a single oldest entry so a new
        // completed run does not make the watch forget all the recent ones.
        if self.completed_jobs.len() == MEMO_MAX
            && let Some(oldest) = self.completed_order.pop_front()
        {
            self.completed_jobs.remove(&oldest);
        }
        self.completed_jobs.insert(key, jobs);
        self.completed_order.push_back(key);
    }
}

/// Whether a run's jobs, once read, can be reused while the run reads the
/// same: only when it has finished. A run in progress is read again, so "9 of
/// 15 jobs done" moves and a job failing early shows (review train-1005c L5).
pub fn memo_holds(run_status: &str) -> bool {
    run_status == "completed"
}

/// The full SHA a short SHA names, through gh.
async fn commit_of(svc: &Service, tree: &Path, reference: &str) -> Option<String> {
    let path = format!("repos/{{owner}}/{{repo}}/commits/{reference}");
    let found = gh_get(svc, tree, &api_args(&path, &[], ".sha")).await;
    found.map(|b| String::from_utf8_lossy(&b).trim().to_string()).filter(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// The newest run of each workflow on one commit, with Canary's run found by
/// its name rather than its SHA, and a placeholder while it has yet to start
/// (`ci_canary`).
async fn runs_on(svc: &Service, tree: &Path, sha: &str) -> Option<Vec<GhRun>> {
    const RUNS: &str = "repos/{owner}/{repo}/actions/runs";
    let fields = [format!("head_sha={sha}"), "per_page=50".to_string()];
    let listed = gh_get(svc, tree, &api_args(RUNS, &fields, RUNS_JQ)).await.and_then(|b| parse_runs(&b))?;
    // `None` when the list could not be read, which is not "no Canary run".
    let canary = if crate::ci_canary::worth_looking(&listed) {
        let fields = ["per_page=50".to_string()];
        gh_get(svc, tree, &api_args(crate::ci_canary::WORKFLOW_RUNS, &fields, RUNS_JQ)).await.and_then(|b| parse_runs(&b))
    } else {
        Some(Vec::new())
    };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    crate::ci_canary::settle(sha, listed, canary, now)
}

/// Read one subject now.
async fn read_subject(svc: &Service, repository: Uuid, tree: &Path, subject: &str, memo: &mut Memo) -> CiRead {
    let runs = if let Some(sha) = subject.strip_prefix("sha:") {
        let full = match memo.shas.get(sha) {
            Some(full) => Some(full.clone()),
            None if sha.len() == 40 => Some(sha.to_string()),
            None => {
                let full = commit_of(svc, tree, sha).await;
                if let Some(full) = &full {
                    if memo.shas.len() >= MEMO_MAX {
                        memo.shas.clear();
                    }
                    memo.shas.insert(sha.to_string(), full.clone());
                }
                full
            }
        };
        match full {
            Some(full) => runs_on(svc, tree, &full).await,
            None => None,
        }
    } else {
        None
    };
    let Some(runs) = runs else { return unknown(subject) };
    let mut read = Vec::new();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    for run in runs {
        let (mut run, jobs) = with_jobs(svc, tree, repository, memo, run).await;
        if retry_pending(&run, None, now, |path| async move {
            match gh_output(svc, tree, &["api".into(), format!("repos/{{owner}}/{{repo}}/{path}")]).await {
                Some(out) if out.status.success() => RetryRead::Data(out.stdout),
                Some(out) => RetryRead::error(&out.stderr),
                None => RetryRead::Unavailable,
            }
        }).await {
            run.status = "queued".into();
            run.conclusion = None;
        }
        read.push((run, jobs));
    }
    summarize(subject, &read)
}

/// The main checkout `gh` runs in for a board's repository.
fn tree_of(svc: &Service, workspace: Uuid) -> Option<(Uuid, PathBuf)> {
    let ws = svc.store.get_workspace(workspace).ok()?;
    let repo = svc.store.get_repository(ws.repository_id).ok()?;
    Some((repo.id, svc.repository_worktree(&repo)))
}

/// Every board's CI subjects from its engine landing groups.
pub fn wanted(svc: &Service) -> BTreeMap<Uuid, BTreeSet<String>> {
    let mut wanted: BTreeMap<Uuid, BTreeSet<String>> = BTreeMap::new();
    // A landing group's pushed commit, from the engine's state (ov-958).
    for (workspace, subject) in svc.store.workflow_ci_subjects().unwrap_or_default() {
        wanted.entry(workspace).or_default().insert(subject);
    }
    wanted
}

fn poll_due(last: Option<&CiRead>, last_attempt: Option<i64>, now: i64) -> bool {
    if last.is_some_and(|read| read.status.is_finished()) {
        return false;
    }
    last_attempt.is_none_or(|at| now.saturating_sub(at) >= BUSY.as_millis() as i64)
}

/// One turn: read every subject that's due, forget those nothing names, and
/// announce each board whose reads changed. How long to wait before the next.
pub async fn read_all(svc: &Service, watcher: &Watcher, memo: &mut Memo) -> Duration {
    let wanted = wanted(svc);
    memo.last_attempt.retain(|(workspace, subject), _| wanted.get(workspace).is_some_and(|subjects| subjects.contains(subject)));
    memo.misses.retain(|(workspace, subject), _| wanted.get(workspace).is_some_and(|subjects| subjects.contains(subject)));
    for ws in svc.store.list_workspaces(None).unwrap_or_default() {
        let named: Vec<String> = wanted.get(&ws.id).map(|s| s.iter().cloned().collect()).unwrap_or_default();
        if svc.store.keep_ci(ws.id, &named).unwrap_or(0) > 0 {
            watcher.announce_plan_changed(ws.id, Actor::Runner);
        }
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
    let mut busy = false;
    for (workspace, subjects) in wanted {
        let Some((repository, tree)) = tree_of(svc, workspace) else { continue };
        let mut changed = false;
        for subject in subjects {
            let last = svc.store.ci_read(workspace, &subject).ok().flatten();
            let key = (workspace, subject.clone());
            if !poll_due(last.as_ref(), memo.last_attempt.get(&key).copied(), now) {
                busy |= last.as_ref().is_some_and(|read| keeps_busy(read.status));
                continue;
            }
            if memo.misses.get(&key).is_some_and(|(_, due)| now < *due) {
                continue;
            }
            memo.last_attempt.insert(key.clone(), now);
            let read = read_subject(svc, repository, &tree, &subject, memo).await;
            busy |= keeps_busy(read.status);
            if matches!(read.status, CiStatus::Unknown | CiStatus::None) {
                let misses = memo.misses.get(&key).map_or(0, |(n, _)| *n) + 1;
                memo.misses.insert(key, (misses, now + backoff_after(misses).as_millis() as i64));
            } else {
                memo.misses.remove(&key);
            }
            match svc.store.record_ci(workspace, &read) {
                Ok(w) => changed |= w.changed,
                Err(e) => tracing::warn!(error = %e, subject, "could not keep a CI read"),
            }
        }
        if changed {
            watcher.announce_plan_changed(workspace, Actor::Runner);
        }
    }
    next_wait(busy, memo.misses.values().map(|(_, due)| *due).min(), now)
}

#[cfg(test)]
#[path = "ci_watch_tests.rs"]
mod tests;
