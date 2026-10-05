//! The runner's CI watch (ov-309, ov-306): reads GitHub Actions through `gh`,
//! read only, for every CI subject a board names, and keeps what it heard in
//! the store's `board_ci` (`farcooler_store::board_ci`).
//!
//! A board names a subject by pushing a train (`sha:<pushed sha>`) or by a
//! page's CI reference (a SHA, a run, or `main`); publishing a page kicks the
//! watch as a pushed SHA does. The watch reads each one
//! while it's named: every minute while any run is still going, and every ten
//! minutes once everything it watches has finished, so a re-run is still seen.
//! A write that gives a train a SHA kicks it to read at once. A read that
//! changed what a board shows announces `plan_changed`, and the store moves a
//! pushed train to green or red on the way.
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

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use farcooler_store::board_ci::{self, CiJob, CiRead, CiStatus, MAIN_SUBJECT};
use farcooler_store::models::Actor;
use serde::Deserialize;
use uuid::Uuid;

use crate::service::Service;
use crate::watch::Watcher;

/// How often subjects are read while any run is still going.
pub const BUSY: Duration = Duration::from_secs(60);
/// How often they're read once everything watched has finished.
pub const QUIET: Duration = Duration::from_secs(10 * 60);

/// What a list of runs is cut down to: the fields read here, nothing more.
pub const RUNS_JQ: &str =
    "[.workflow_runs[] | {id, name, head_sha, status, conclusion, html_url, created_at, display_title, event, updated_at}]";
/// One run, the same fields.
pub const RUN_JQ: &str = "{id, name, head_sha, status, conclusion, html_url, created_at, display_title, event, updated_at}";
/// A run's jobs.
pub const JOBS_JQ: &str = "[.jobs[] | {name, status, conclusion, html_url}]";

fn kicked() -> &'static tokio::sync::Notify {
    static KICK: OnceLock<tokio::sync::Notify> = OnceLock::new();
    KICK.get_or_init(tokio::sync::Notify::new)
}

/// Read now rather than at the next turn: a train was just given a SHA.
pub fn kick() {
    kicked().notify_one();
}

/// The watch, for the life of the daemon.
pub async fn run(svc: Arc<Service>, watcher: Arc<Watcher>) {
    let mut memo = Memo::default();
    loop {
        let wait = read_all(&svc, &watcher, &mut memo).await;
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = kicked().notified() => {}
        }
    }
}

/// What the watch remembers between turns: full SHAs for short ones, and a
/// run's jobs while the run hasn't changed, so a finished run costs one call.
#[derive(Default)]
pub struct Memo {
    shas: HashMap<String, String>,
    jobs: HashMap<u64, (String, Option<String>, Vec<GhJob>)>,
    default_branch: HashMap<Uuid, String>,
    /// Subjects GitHub didn't answer for, or had no run for: how many reads
    /// in a row, and when to ask again (review train-1005c M2).
    misses: HashMap<(Uuid, String), (u32, i64)>,
}

/// How long after `misses` reads in a row that GitHub couldn't or didn't
/// answer to ask again: a minute, doubling, at most `QUIET`. A runner whose gh
/// is logged out, or a page naming a SHA nobody pushed, settles at one read
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

/// One run, as `RUNS_JQ` and `RUN_JQ` print it.
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

/// `RUN_JQ`'s output.
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
pub fn latest_per_workflow(runs: &[GhRun]) -> Vec<GhRun> {
    let mut newest: BTreeMap<&str, &GhRun> = BTreeMap::new();
    for run in runs {
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
    match out {
        Ok(Ok(o)) if o.status.success() => Some(o.stdout),
        _ => None,
    }
}

/// A run with its jobs, from `memo` while the run is unchanged.
async fn with_jobs(
    svc: &Service,
    tree: &Path,
    memo: &mut HashMap<u64, (String, Option<String>, Vec<GhJob>)>,
    run: GhRun,
) -> (GhRun, Option<Vec<GhJob>>) {
    if run.id == 0 {
        // The placeholder for a Canary run that has not started (`ci_canary`).
        return (run, None);
    }
    if let Some((s, c, jobs)) = memo.get(&run.id) {
        if memo_holds(&run.status) && (s, c) == (&run.status, &run.conclusion) {
            return (run, Some(jobs.clone()));
        }
    }
    let path = format!("repos/{{owner}}/{{repo}}/actions/runs/{}/jobs", run.id);
    let jobs = gh_get(svc, tree, &api_args(&path, &["per_page=100".into()], JOBS_JQ)).await.and_then(|b| parse_jobs(&b));
    if let Some(jobs) = &jobs {
        // A long-lived daemon's memo stays small: forget it all past a few
        // hundred runs rather than keep every run it ever read.
        if memo.len() >= MEMO_MAX {
            memo.clear();
        }
        memo.insert(run.id, (run.status.clone(), run.conclusion.clone(), jobs.clone()));
    }
    (run, jobs)
}

/// The most runs, or short SHAs, the watch remembers before it starts over.
const MEMO_MAX: usize = 512;

/// Whether a run's jobs, once read, can be reused while the run reads the
/// same: only when it has finished. A run in progress is read again, so "9 of
/// 15 jobs done" moves and a job failing early shows (review train-1005c L5).
pub fn memo_holds(run_status: &str) -> bool {
    run_status == "completed"
}

/// The full SHA a ref names (a short SHA, or a branch), through gh.
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
pub async fn read_subject(svc: &Service, repository: Uuid, tree: &Path, subject: &str, memo: &mut Memo) -> CiRead {
    const RUNS: &str = "repos/{owner}/{repo}/actions/runs";
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
    } else if let Some(id) = subject.strip_prefix("run:") {
        let path = format!("{RUNS}/{id}");
        gh_get(svc, tree, &api_args(&path, &[], RUN_JQ)).await.and_then(|b| parse_run(&b)).map(|r| vec![r])
    } else if subject == MAIN_SUBJECT {
        let branch = match memo.default_branch.get(&repository) {
            Some(b) => b.clone(),
            None => {
                let found = svc.default_branch(repository, tree).await.unwrap_or_else(|| "main".into());
                let bare = found.strip_prefix("origin/").unwrap_or(&found).to_string();
                memo.default_branch.insert(repository, bare.clone());
                bare
            }
        };
        // The commit main is at now, asked every time: never the commit of
        // the newest-created run, which a late follow-up run on an older
        // commit can be (review train-1005c M3). Its own runs, or none yet.
        match commit_of(svc, tree, &branch).await {
            Some(head) => runs_on(svc, tree, &head).await,
            None => None,
        }
    } else {
        None
    };
    let Some(runs) = runs else { return unknown(subject) };
    let mut read = Vec::new();
    for run in runs {
        read.push(with_jobs(svc, tree, &mut memo.jobs, run).await);
    }
    summarize(subject, &read)
}

/// The main checkout `gh` runs in for a board's repository.
fn tree_of(svc: &Service, workspace: Uuid) -> Option<(Uuid, PathBuf)> {
    let ws = svc.store.get_workspace(workspace).ok()?;
    let repo = svc.store.get_repository(ws.repository_id).ok()?;
    Some((repo.id, svc.repository_worktree(&repo)))
}

/// Every board's CI subjects: its trains' pushed SHAs, and what its pages'
/// CI references name (ov-306).
pub fn wanted(svc: &Service) -> BTreeMap<Uuid, BTreeSet<String>> {
    let mut wanted: BTreeMap<Uuid, BTreeSet<String>> = BTreeMap::new();
    for train in svc.store.trains_following_ci().unwrap_or_default() {
        if let Some(subject) = train.ci_subject() {
            wanted.entry(train.workspace_id).or_default().insert(subject);
        }
    }
    // The pages' own file reads them, so nothing here names pages.
    for (workspace, subject) in crate::rpc_pages::ci_subjects(svc) {
        wanted.entry(workspace).or_default().insert(subject);
    }
    wanted
}

/// One turn: read every subject that's due, forget those nothing names, and
/// announce each board whose reads changed. How long to wait before the next.
pub async fn read_all(svc: &Service, watcher: &Watcher, memo: &mut Memo) -> Duration {
    let wanted = wanted(svc);
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
            if last.as_ref().is_some_and(|l| l.status.is_finished() && now - l.fetched_at < QUIET.as_millis() as i64) {
                continue;
            }
            let key = (workspace, subject.clone());
            if memo.misses.get(&key).is_some_and(|(_, due)| now < *due) {
                continue;
            }
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
