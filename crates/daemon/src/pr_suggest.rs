//! Whom to ask to review each live lane's pull request (ov-580): read once
//! per head commit, kept in memory beside the PR cache, laid onto
//! `Plan.suggested_reviewers` for a Control peer.
//!
//! The ranking is `farcooler_core::reviewers` (CODEOWNERS, then the commits
//! that last touched the files, then the repository's reviewer map, never the
//! author), and the git reading is `reviewers_git`, run here in the
//! repository's checkout against `origin/<base>` through the runner's own git
//! (`crate::git`: the guard's pins, the sandbox and one time budget), never a
//! bare `git`, so a repository's config can't run a program and a git that
//! hangs can't stall the watch. The files come from `gh pr
//! diff --name-only`, so no branch needs fetching to name them; a diff GitHub
//! won't print (it refuses past a few hundred files) leaves the pull request
//! without a suggestion rather than a partial one.
//!
//! **Read when the head moves, three at a time.** A suggestion depends on the
//! files, and the files on the head commit, so it is read when a pull request
//! first appears and again when its head changes, and at most three per turn
//! of the watch: a repository with twenty live PRs fills in over a few minutes
//! instead of starting twenty `gh` calls at once. Nothing is written to the
//! store, so a restart reads them again. A read that fails leaves the PR
//! without one, and is tried again next turn.
//!
//! **Nobody who has been asked already.** A pull request that asks someone
//! (`reviewRequests`) or that someone has reviewed gets no line: the question
//! "whom should I ask" has been answered.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use farcooler_core::reviewers::{self, Evidence, Source, Suggestion};
use farcooler_core::reviewers_git;
use farcooler_protocol::v1 as pb;
use uuid::Uuid;

use crate::pr_timing::Moment;
use crate::service::Service;
use crate::stack::{PrInfo, PrState};

/// How many pull requests a turn reads.
const PER_TURN: usize = 3;
/// The most files a suggestion is drawn from.
const FILES_MAX: usize = 500;

struct Entry {
    head: String,
    suggestions: Vec<Suggestion>,
}

static CACHE: Mutex<Option<HashMap<(Uuid, u32), Entry>>> = Mutex::new(None);

fn with_cache<T>(f: impl FnOnce(&mut HashMap<(Uuid, u32), Entry>) -> T) -> T {
    f(CACHE.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert_with(HashMap::new))
}

/// Remember what was read of `pr` at `head`.
pub fn put(repository: Uuid, pr: u32, head: &str, suggestions: Vec<Suggestion>) {
    with_cache(|c| c.insert((repository, pr), Entry { head: head.to_string(), suggestions }));
}

/// What was read of `pr`, whatever head it was read at.
pub fn get(repository: Uuid, pr: u32) -> Vec<Suggestion> {
    with_cache(|c| c.get(&(repository, pr)).map(|e| e.suggestions.clone()).unwrap_or_default())
}

fn stale(repository: Uuid, pr: &PrInfo) -> bool {
    with_cache(|c| c.get(&(repository, pr.status.number)).is_none_or(|e| e.head != pr.status.head_oid))
}

/// Whether a pull request still wants a suggestion: open, from this
/// repository, asking nobody and unreviewed.
fn wanted(pr: &PrInfo) -> bool {
    matches!(pr.status.state, PrState::Open | PrState::Draft)
        && !pr.is_fork
        && pr.review.requested.is_empty()
        && !matches!(pr.timing.first_review, Moment::At(_))
}

/// The files `gh` says pull request `number` changes.
async fn files_of(svc: &Service, worktree: &Path, number: u32) -> Option<Vec<String>> {
    let mut gh = crate::stack::gh(worktree).await.ok()?;
    let _permit = svc.gh_permit().await;
    let out = tokio::time::timeout(
        crate::stack::GH_TIMEOUT,
        gh.current_dir(worktree)
            .args(["pr", "diff"])
            .arg(number.to_string())
            .arg("--name-only")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !out.status.success() {
        return None;
    }
    let files: Vec<String> =
        String::from_utf8_lossy(&out.stdout).lines().map(str::trim).filter(|l| !l.is_empty()).take(FILES_MAX).map(str::to_string).collect();
    (!files.is_empty()).then_some(files)
}

/// Suggestions for `pr` from `files`, read in `worktree` through the runner's
/// own git (`crate::git`): the guard's pins, the sandbox and one shared time
/// budget for everything read, so a repository's config can't run a program
/// and a git that hangs costs the watch the budget once.
pub async fn suggest_for(worktree: &Path, pr: &PrInfo, files: Vec<String>) -> Vec<Suggestion> {
    // Without the author there is no telling who must not be asked; the
    // branch's committers alone are not enough to say who opened it.
    if pr.author.is_empty() {
        return Vec::new();
    }
    let (dir, base, head) = (worktree.to_path_buf(), format!("origin/{}", pr.base_ref), format!("origin/{}", pr.head_ref));
    let paths = files.clone();
    let deadline = tokio::time::Instant::now() + crate::git::timeout();
    let handle = tokio::runtime::Handle::current();
    let found = tokio::task::spawn_blocking(move || {
        let mut run = |args: &[&str]| {
            let out = handle.block_on(crate::git::git_bytes_by(deadline, &dir, args)).ok()?;
            out.ok.then(|| String::from_utf8_lossy(&out.stdout).into_owned())
        };
        reviewers_git::gather_with(&mut run, &base, &head, &paths)
    })
    .await
    .unwrap_or_default();
    let mut authors: Vec<String> = vec![pr.author.clone()];
    authors.extend(found.committers.iter().cloned());
    reviewers::suggest(&Evidence {
        authors: &authors,
        paths: &files,
        codeowners: found.codeowners.as_deref(),
        history: &found.history,
        map: found.map.as_deref(),
    })
}

/// One turn: read what is missing or moved, for the live lanes' pull requests
/// of `repository`.
pub async fn refresh(svc: &Service, repository: Uuid, boards: &BTreeSet<Uuid>, worktree: &Path) {
    let reads = crate::plan_stage::Reads::of(svc, repository);
    let mut numbers: BTreeSet<u32> = BTreeSet::new();
    for board in boards {
        let Ok(plan) = svc.store.plan(*board, i64::MIN) else { continue };
        for lane in plan.lanes.iter().filter(|l| crate::plan_stage::phase_of(l.lane.state).is_active()) {
            numbers.extend(crate::plan_stage::prs_of(lane, &plan.cards, &reads));
        }
    }
    let todo: Vec<&PrInfo> = reads.prs.iter().filter(|p| numbers.contains(&p.status.number) && wanted(p) && stale(repository, p)).take(PER_TURN).collect();
    for pr in todo {
        let Some(files) = files_of(svc, worktree, pr.status.number).await else { continue };
        let found = suggest_for(worktree, pr, files).await;
        put(repository, pr.status.number, &pr.status.head_oid, found);
    }
}

/// `wire.suggested_reviewers`: for each live lane, each of its pull requests
/// that still wants a suggestion and has one. Control peers only (the caller
/// decides): these are GitHub logins.
pub fn annotate(svc: &Service, plan: &farcooler_store::plan_read::Plan, wire: &mut pb::Plan) {
    for view in plan.lanes.iter().filter(|v| crate::plan_stage::phase_of(v.lane.state).is_active()) {
        let Ok(ws) = svc.store.get_workspace(view.lane.workspace_id) else { continue };
        let reads = crate::plan_stage::Reads::of(svc, ws.repository_id);
        for number in crate::plan_stage::prs_of(view, &plan.cards, &reads) {
            let Some(pr) = reads.prs.iter().find(|p| p.status.number == number).filter(|p| wanted(p)) else { continue };
            let found = get(ws.repository_id, pr.status.number);
            if found.is_empty() {
                continue;
            }
            wire.suggested_reviewers.push(pb::LaneReviewers {
                lane_id: view.lane.id.as_bytes().to_vec().into(),
                pr: number,
                reviewers: found.iter().map(|s| s.reviewer.clone()).collect(),
                sources: found.iter().map(|s| Source::words(s.source).to_string()).collect(),
            });
        }
    }
}

#[cfg(test)]
#[path = "pr_suggest_tests.rs"]
mod tests;
