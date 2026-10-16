//! Pull requests that want this runner's reviewer (ov-315, ov-305 section 4.2
//! and 5.2): the open PRs where the runner's own `gh` login is a requested
//! reviewer, a teammate's included, read from `gh search prs`.
//!
//! **Whose.** The runner's `gh` login, `@me`. There is no identity model
//! behind it: your Mac and your phone show your review requests, next to your
//! board's asks. A team-wide view is Phase 4.
//!
//! **When it runs.** Only while some board on this runner lands through pull
//! requests (`Landing::mode`, the owner's choice), and then every
//! `REFRESH`. A runner with no such board runs no `gh` search and has no
//! review requests, which is what it did before this read existed.
//!
//! **When `gh` cannot answer** (missing, logged out, offline, rate limited) the
//! read is a miss, never an empty list. The last answer stands for `STALE_AFTER`
//! so a blip does not make rows flicker, and then it is dropped: a Needs You
//! with no review rows says nothing about whether there are any, and it is
//! never an error. A miss backs off as the PR watch does (`backoff_after`).
//!
//! **What the relay is told.** That a pull request wants a review, and how
//! many (`watch::pr_review_notice`). Never a repository, a title, a body or a
//! login, because the relay never sees a repository (`services/relay/README.md`).
//! The rows, with all of that, go only to clients on the daemon's own channel.
//!
//! **Who sees them.** Only a peer at Control scope: below it the list holds no
//! PR_REVIEW item (`needs_you::redact_list_below_control`). The search itself
//! is not narrowed to the boards' repositories, and that is deliberate: it
//! returns every review request of the login, the runner's owner's, because
//! Control is a shell on this runner and a Control peer could run `gh` as that
//! user anyway. The scope line is Read versus Control, not which repository.
//!
//! Drafts are skipped: GitHub requests a review of a draft only through
//! CODEOWNERS, and the author has not asked a person to look yet.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;
use std::time::Duration;

use farcooler_store::landing::LandingMode;
use serde::Deserialize;

use crate::service::Service;
use crate::watch::Watcher;

/// How often a runner with a pull-request board asks GitHub for its review
/// requests. One search call: GitHub allows thirty a minute.
pub const REFRESH: Duration = Duration::from_secs(2 * 60);
/// How often a runner with no such board looks for one. A store read; no `gh`.
pub const IDLE: Duration = Duration::from_secs(5 * 60);
/// How long the last answer stands when `gh` cannot give a new one: seven
/// refreshes, so a blip is invisible and a logged-out `gh` is not hidden.
pub const STALE_AFTER_MS: i64 = 15 * 60 * 1000;
/// The most review requests one read keeps. Past this is a backlog nobody is
/// reviewing from a phone, and a longer list is a slower search.
pub const MOST: &str = "50";

/// One pull request waiting on this login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewRequest {
    pub url: String,
    pub number: u32,
    /// `owner/name`.
    pub repository: String,
    pub title: String,
    pub author: String,
    /// When the PR was opened, in unix milliseconds: stable across reads and
    /// restarts, where the request itself carries no time.
    pub created_at: i64,
}

#[derive(Deserialize)]
struct GhRepository {
    #[serde(rename = "nameWithOwner", default)]
    name_with_owner: String,
}

#[derive(Deserialize)]
struct GhAuthor {
    #[serde(default)]
    login: String,
}

#[derive(Deserialize)]
struct GhRequest {
    number: u32,
    #[serde(default)]
    title: String,
    url: String,
    repository: Option<GhRepository>,
    author: Option<GhAuthor>,
    #[serde(rename = "isDraft", default)]
    is_draft: bool,
    #[serde(rename = "createdAt", default)]
    created_at: String,
}

/// What `gh search prs --json ...` printed, as the requests it listed. `None`
/// for anything that is not that JSON, which must never read as "no requests".
pub fn parse(stdout: &[u8]) -> Option<Vec<ReviewRequest>> {
    let listed: Vec<GhRequest> = match serde_json::from_slice(stdout) {
        Ok(listed) => listed,
        Err(e) => {
            // The kind of failure and where, never its text: a decoder quotes its input.
            tracing::warn!(class = ?e.classify(), "could not read gh's review requests");
            return None;
        }
    };
    Some(
        listed
            .into_iter()
            .filter(|r| !r.is_draft && r.url.starts_with("https://"))
            .map(|r| ReviewRequest {
                url: r.url,
                number: r.number,
                repository: r.repository.map(|r| r.name_with_owner).unwrap_or_default(),
                title: r.title,
                author: r.author.map(|a| a.login).unwrap_or_default(),
                created_at: crate::stack::parse_iso8601_millis(&r.created_at).unwrap_or_default(),
            })
            .collect(),
    )
}

/// The read before a new one: what it listed, and whether it was stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Previous {
    pub requests: Vec<ReviewRequest>,
    /// It was older than `STALE_AFTER_MS` when the new read arrived, so the
    /// clients were showing none of it: this read moves their lists even if it
    /// lists the same pull requests (an outage, then recovery).
    pub stale: bool,
}

/// What the cache holds: the last read, when, and whether the clients were
/// already told that it went stale.
#[derive(Debug)]
struct Held {
    at: i64,
    requests: Vec<ReviewRequest>,
    told_stale: bool,
}

/// The last read, when it was made, and what it said.
#[derive(Debug, Default)]
pub struct Cache(Mutex<Option<Held>>);

impl Cache {
    /// Record a read made at `at`, and say what the one before it was:
    /// `None` when there was none, or it had been dropped (the first read of a
    /// board seeds the cache and tells nobody about requests already waiting).
    pub fn put(&self, at: i64, requests: Vec<ReviewRequest>) -> Option<Previous> {
        let held = Held { at, requests, told_stale: false };
        self.0.lock().unwrap_or_else(PoisonError::into_inner).replace(held).map(|was| Previous {
            stale: at.saturating_sub(was.at) > STALE_AFTER_MS,
            requests: was.requests,
        })
    }

    /// What is waiting as of `now`: the last read, unless it is older than
    /// `STALE_AFTER_MS`. Nothing for a read never made, which is not a claim
    /// that no review is wanted.
    pub fn current(&self, now: i64) -> Vec<ReviewRequest> {
        match &*self.0.lock().unwrap_or_else(PoisonError::into_inner) {
            Some(held) if now.saturating_sub(held.at) <= STALE_AFTER_MS => held.requests.clone(),
            _ => Vec::new(),
        }
    }

    /// Whether the read has just gone stale with requests in it, once: the
    /// clients' lists dropped them without any event, so the caller says so.
    pub fn went_stale(&self, now: i64) -> bool {
        match &mut *self.0.lock().unwrap_or_else(PoisonError::into_inner) {
            Some(held) if !held.told_stale && !held.requests.is_empty() && now.saturating_sub(held.at) > STALE_AFTER_MS => {
                held.told_stale = true;
                true
            }
            _ => false,
        }
    }

    /// How long until the read goes stale, from `now`: the longest a caller
    /// should wait before looking again, so the drop is announced when it
    /// happens and not at the next backoff. `None` with nothing to drop.
    pub fn stale_in(&self, now: i64) -> Option<Duration> {
        let guard = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let held = guard.as_ref().filter(|h| !h.told_stale && !h.requests.is_empty())?;
        let ms = (held.at + STALE_AFTER_MS + 1).saturating_sub(now).max(1);
        Some(Duration::from_millis(ms as u64))
    }

    /// Forget the read: no board lands through pull requests any more.
    /// Returns whether there was one.
    pub fn clear(&self) -> bool {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take().is_some()
    }
}

/// The requests in `now` that `before` did not list: the ones to tell the
/// phone about. By URL, so a title edit is not a new request.
pub fn newly_requested<'a>(before: &[ReviewRequest], now: &'a [ReviewRequest]) -> Vec<&'a ReviewRequest> {
    now.iter().filter(|r| !before.iter().any(|b| b.url == r.url)).collect()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// Whether a board lands through pull requests, and a checkout for `gh` to
/// run in if so: `gh` wants a repository's directory for the pins it runs git
/// under, though a search names no repository.
#[derive(Debug, PartialEq, Eq)]
pub enum Tree {
    /// A board does, and this is a checkout of its repository.
    Some(PathBuf),
    /// The store was read and no board does.
    None,
    /// The store could not be read: nothing is known, and nothing changes.
    Unknown,
}

pub fn pull_request_tree(svc: &Service) -> Tree {
    let Ok(workspaces) = svc.store.list_workspaces(None) else { return Tree::Unknown };
    for workspace in workspaces {
        let Ok(landing) = svc.store.get_landing(workspace.id) else { return Tree::Unknown };
        if landing.mode == Some(LandingMode::PullRequests)
            && let Ok(repo) = svc.store.get_repository(workspace.repository_id)
        {
            return Tree::Some(svc.repository_worktree(&repo));
        }
    }
    Tree::None
}

/// The query, one call: open PRs that request this login, as the daemon reads them.
async fn search(svc: &Service, tree: &Path) -> Option<Vec<ReviewRequest>> {
    let mut gh = crate::stack::gh(tree).await.ok()?;
    let _permit = svc.gh_permit().await;
    let out = tokio::time::timeout(
        crate::stack::GH_TIMEOUT,
        gh.current_dir(tree)
            .args(["search", "prs", "--review-requested=@me", "--state=open", "--limit", MOST, "--json"])
            .arg("number,title,url,repository,author,isDraft,createdAt")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await;
    match out {
        Ok(Ok(o)) if o.status.success() => parse(&o.stdout),
        // Missing, logged out, offline, rate limited and slow are one answer: unread.
        _ => None,
    }
}

/// One turn, and how long to wait before the next. `misses` counts reads in a
/// row `gh` could not answer.
pub async fn tick(svc: &Service, watcher: &Watcher, misses: &mut u32) -> Duration {
    match pull_request_tree(svc) {
        Tree::Some(tree) => apply(svc, watcher, misses, search(svc, &tree).await, now_ms()).await,
        other => without_checkout(svc, watcher, misses, &other),
    }
}

/// A turn with no checkout to read in. No board lands through pull requests
/// (`Tree::None`): `forget`. A store that cannot be read (`Tree::Unknown`)
/// says nothing about the boards, so the cache and the miss count stand, and
/// the next look is the idle one.
pub(crate) fn without_checkout(svc: &Service, watcher: &Watcher, misses: &mut u32, tree: &Tree) -> Duration {
    match tree {
        Tree::None => forget(svc, watcher, misses),
        _ => {
            // The rows age out on their own, with no event, whether or not the
            // store can be read: say so when they do.
            if svc.pr_requests.went_stale(now_ms()) {
                watcher.announce_needs_you();
                watcher.schedule_count_notice();
            }
            IDLE
        }
    }
}

/// No board lands through pull requests: there are no review requests, and
/// the clients are told if there were.
pub(crate) fn forget(svc: &Service, watcher: &Watcher, misses: &mut u32) -> Duration {
    *misses = 0;
    if svc.pr_requests.clear() {
        watcher.announce_needs_you();
        watcher.schedule_count_notice();
    }
    IDLE
}

/// What a read made at `now` (or a `gh` that could not make one) changes:
/// the cache, the clients' lists, and the phone's notice for each request it
/// had not seen. Returns how long to wait.
pub(crate) async fn apply(
    svc: &Service,
    watcher: &Watcher,
    misses: &mut u32,
    read: Option<Vec<ReviewRequest>>,
    now: i64,
) -> Duration {
    let Some(read) = read else {
        // Not "no requests": the cache stands until it goes stale, and the
        // clients are told the moment it does (they drop the rows then, with
        // no event of their own), so the wait never runs past that.
        *misses += 1;
        if svc.pr_requests.went_stale(now) {
            watcher.announce_needs_you();
            watcher.schedule_count_notice();
        }
        let backoff = crate::pr_watch::backoff_after(*misses);
        return svc.pr_requests.stale_in(now).map_or(backoff, |until| backoff.min(until));
    };
    *misses = 0;
    let before = svc.pr_requests.put(now, read.clone());
    let new = before.as_ref().map_or(0, |b| newly_requested(&b.requests, &read).len());
    // A stale read had already dropped its rows from the clients' lists, so
    // the same pull requests coming back are a move too.
    let moved = before.as_ref().is_none_or(|b| b.stale || b.requests.len() != read.len() || new > 0);
    if moved {
        watcher.announce_needs_you();
        watcher.schedule_count_notice();
    }
    // The first read after a board chose pull requests seeds the cache and
    // tells nobody about requests that were already waiting.
    if new > 0 {
        watcher.announce_pr_review(new).await;
    }
    REFRESH
}

fn kicked() -> &'static tokio::sync::Notify {
    static KICK: std::sync::OnceLock<tokio::sync::Notify> = std::sync::OnceLock::new();
    KICK.get_or_init(tokio::sync::Notify::new)
}

/// A board was just set to land through pull requests (or away from it): read
/// now, not at the next look. Choosing the mode is the moment the owner
/// expects review requests to appear. At most once in `pr_watch::KICK_FLOOR`,
/// so setting the mode over and over cannot hammer `gh`.
pub fn kick() {
    static FLOOR: Mutex<Option<crate::pr_watch::KickFloor>> = Mutex::new(None);
    let allowed = FLOOR
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_with(|| crate::pr_watch::KickFloor::new(crate::pr_watch::kick_floor()))
        .allows(uuid::Uuid::nil(), Instant::now());
    if allowed {
        kicked().notify_one();
    }
}

/// The read, for the life of the daemon.
pub async fn run(svc: Arc<Service>, watcher: Arc<Watcher>) {
    let mut misses = 0;
    loop {
        let wait = tick(&svc, &watcher, &mut misses).await;
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = kicked().notified() => {}
        }
    }
}

#[cfg(test)]
#[path = "pr_review_tests.rs"]
mod tests;
