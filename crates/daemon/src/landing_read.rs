//! Reading how a repository lands work (ov-313): the runner's own `gh` and git
//! asked the facts `landing::decide` needs, the answer to `repository.landing`,
//! and the daily re-read per workspace.
//!
//! **One read is five questions**, each failing alone. `gh repo view` (the
//! default branch, the merge methods, what this login may do), the base
//! branch's rules and its protection (`gh api`), and two looks at the base
//! branch's own tree (`git ls-tree` and `git show`, never the working
//! directory, which may be on any branch): whether a CODEOWNERS file exists and
//! whether any workflow runs on `merge_group`. A question that fails leaves its
//! fact unread, and `landing::decide` says so rather than guessing.
//!
//! **A dry-run push is not part of it.** The design (section 6.2) pairs
//! `viewerPermission` with a refused dry-run push. A dry run still contacts
//! the remote with the owner's credentials, so a read command that does it
//! isn't read-only to anyone auditing the remote's logs; `viewerPermission`
//! is what GitHub itself says the login may do, and that stands alone.
//!
//! **Daily, and in memory.** `run` reads each workspace's repository once a day
//! and keeps the answer in memory, so the runner can say, beside the
//! workspace, that `direct` cannot work (`refusal`). It never changes the
//! workspace's mode: a person chooses that. After a restart the first read is
//! a minute away and the answer is rebuilt; nothing here is stored.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, result};
use farcooler_store::landing::LandingMode;
use farcooler_store::models::Repository;
use uuid::Uuid;

use crate::landing::{self, Decision, Facts};
use crate::service::Service;
use crate::watch::Watcher;

/// How long a read stands before the daily job asks again.
pub const DAY: Duration = Duration::from_secs(24 * 60 * 60);
/// How long the daily job waits before asking again when nothing could be read
/// (a logged-out `gh`, a repository with no GitHub remote).
pub const RETRY: Duration = Duration::from_secs(6 * 60 * 60);
/// How often the daily job looks for a read that is due. No `gh` runs for
/// the look.
pub const LOOK: Duration = Duration::from_secs(60 * 60);
/// What the job waits after the daemon starts before its first read.
const FIRST: Duration = Duration::from_secs(60);
/// The most workflow files read for one repository: with more than this,
/// whether any runs on `merge_group` is unread, never "none".
const MOST_WORKFLOWS: usize = 50;

/// One read, with when it was made.
#[derive(Debug, Clone)]
pub struct Reading {
    /// The base the board asked for when this was read (`None`: GitHub's
    /// default). A read of another base says nothing about this one.
    pub asked: Option<String>,
    pub facts: Facts,
    pub decision: Decision,
    pub read_at: i64,
}

impl Reading {
    /// Whether it learned anything: a read of nothing is retried sooner.
    fn learned_something(&self) -> bool {
        let f = &self.facts;
        f.rules.is_some() || f.protected.is_some() || f.repo.is_some()
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// The last read of each workspace's repository, by workspace. In memory only.
static READINGS: Mutex<Option<HashMap<Uuid, Reading>>> = Mutex::new(None);

fn remember(workspace: Uuid, reading: &Reading) {
    READINGS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .insert(workspace, reading.clone());
}

/// Forget what was read for `workspace`: its base changed, so the old read
/// is about another branch. The daily job reads it again at its next look.
pub fn forget(workspace: Uuid) {
    if let Some(all) = READINGS.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
        all.remove(&workspace);
    }
}

fn recall(workspace: Uuid) -> Option<Reading> {
    READINGS.lock().unwrap_or_else(PoisonError::into_inner).as_ref()?.get(&workspace).cloned()
}

/// Why direct landing can't work for `workspace`, when its last read found
/// that and the board isn't already landing through pull requests. The
/// sentence an app shows beside the setting. `None` for a board that has
/// not been read, and for one whose read found direct possible.
pub fn refusal(workspace: Uuid, chosen: Option<LandingMode>, base: Option<&str>) -> Option<String> {
    if chosen == Some(LandingMode::PullRequests) {
        return None;
    }
    let reading = recall(workspace).filter(|r| r.decision.direct_impossible && r.asked.as_deref() == base)?;
    let why = reading.decision.reasons.first().cloned().unwrap_or_default();
    Some(format!("Landing straight on {} won't work. {why}", reading.facts.base).trim().to_string())
}

/// Percent-encode a branch name for a URL path: slashes included, because the
/// REST routes take the whole name as one segment.
pub fn path_segment(name: &str) -> String {
    let mut out = String::new();
    for b in name.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// One `gh` read in `tree`, or `None` for every way it can fail.
async fn gh_read(svc: &Service, tree: &Path, args: &[&str]) -> Option<Vec<u8>> {
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

/// A commit-ish for `base` in `tree`: `origin/<base>`, else `<base>`.
async fn base_ref(tree: &Path, base: &str) -> Option<String> {
    if base.is_empty() || base.starts_with('-') {
        return None;
    }
    for candidate in [format!("origin/{base}"), base.to_string()] {
        let spec = format!("{candidate}^{{commit}}");
        if let Ok(r) = crate::git::git(tree, &["rev-parse", "--verify", "--quiet", &spec]).await
            && r.ok
        {
            return Some(candidate);
        }
    }
    None
}

/// Whether `reference` has a CODEOWNERS file in one of the three places GitHub
/// looks, and whether any of its workflows runs on `merge_group`.
pub async fn read_tree(tree: &Path, reference: &str) -> (Option<bool>, Option<bool>) {
    let owners = crate::git::git(
        tree,
        &["ls-tree", "--name-only", reference, "--", "CODEOWNERS", ".github/CODEOWNERS", "docs/CODEOWNERS"],
    )
    .await
    .ok()
    .filter(|r| r.ok)
    .map(|r| !r.stdout.trim().is_empty());

    // Only the files directly in the directory run: GitHub ignores a
    // subdirectory's, so `ls-tree` is not recursive.
    let listed = crate::git::git(tree, &["ls-tree", "--name-only", reference, "--", ".github/workflows/"]).await;
    let workflow = match listed {
        Ok(r) if r.ok => {
            let files: Vec<&str> =
                r.stdout.lines().filter(|f| f.ends_with(".yml") || f.ends_with(".yaml")).collect();
            let over_the_cap = files.len() > MOST_WORKFLOWS;
            let mut found = false;
            let mut failed = false;
            for file in files.into_iter().take(MOST_WORKFLOWS) {
                let spec = format!("{reference}:{file}");
                match crate::git::git_bytes(tree, &["show", &spec]).await {
                    Ok(b) if b.ok => found |= landing::runs_on_merge_group(&String::from_utf8_lossy(&b.stdout)),
                    _ => failed = true,
                }
            }
            // A workflow that could not be read, or was never looked at, might have been the one.
            if found { Some(true) } else if failed || over_the_cap { None } else { Some(false) }
        }
        _ => None,
    };
    (owners, workflow)
}

/// Read `repo`'s base branch: `base` when the board named one, else GitHub's
/// default. Never fails: what could not be read is absent from the facts.
pub async fn read(svc: &Service, repo: &Repository, base: Option<&str>) -> Reading {
    let tree = svc.repository_worktree(repo);
    let view = gh_read(
        svc,
        &tree,
        &[
            "repo",
            "view",
            "--json",
            "defaultBranchRef,squashMergeAllowed,rebaseMergeAllowed,mergeCommitAllowed,viewerPermission",
        ],
    )
    .await
    .and_then(|b| landing::parse_repo_view(&b));

    let asked = base.map(str::trim).filter(|b| !b.is_empty()).map(str::to_string);
    let base = base
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .map(str::to_string)
        .or_else(|| view.as_ref().and_then(|v| v.default_branch.clone()))
        .unwrap_or_else(|| "main".into());
    let mut facts = Facts { base: base.clone(), repo: view, ..Default::default() };

    // With no `gh` answer to the first question, the next two would fail the same way.
    if facts.repo.is_some() {
        let segment = path_segment(&base);
        let rules_path = format!("repos/{{owner}}/{{repo}}/rules/branches/{segment}?per_page=100");
        let branch_path = format!("repos/{{owner}}/{{repo}}/branches/{segment}");
        // Every page, one array each. Without --slurp, which needs gh 2.48.
        let args = ["api", "--paginate", "-X", "GET", rules_path.as_str()];
        facts.rules = gh_read(svc, &tree, &args).await.and_then(|b| landing::parse_rules(&b));
        let branch = gh_read(svc, &tree, &["api", "-X", "GET", &branch_path]).await;
        facts.protected = branch.as_deref().and_then(landing::parse_protected);
        facts.protection_checks = branch.as_deref().map(landing::parse_protection_checks).unwrap_or_default();
    }
    if let Some(reference) = base_ref(&tree, &base).await {
        let (owners, workflow) = read_tree(&tree, &reference).await;
        facts.codeowners = owners;
        facts.merge_group_workflow = workflow;
    }
    let decision = landing::decide(&facts);
    Reading { asked, facts, decision, read_at: now_ms() }
}

/// The base a workspace named, else none.
fn chosen_base(svc: &Service, workspace: Uuid) -> Option<String> {
    svc.store.get_landing(workspace).ok().and_then(|l| l.base)
}

/// `repository.landing`: read the repository now, against its Main workspace's
/// base, and remember what was found for that workspace.
pub async fn handle(svc: &Service, req: pb::Request) -> Result<result::Value> {
    let id = req.target_resource_id.as_deref().and_then(crate::wire::parse_id).ok_or(DomainError::NotFound)?;
    let repo = svc.store.get_repository(id)?;
    let main = svc.store.ensure_main_workspace(repo.id)?;
    let base = chosen_base(svc, main.id);
    let reading = read(svc, &repo, base.as_deref()).await;
    remember(main.id, &reading);
    Ok(result::Value::RepositoryLanding(to_wire(&reading)))
}

pub fn to_wire_mode(mode: LandingMode) -> pb::LandingMode {
    match mode {
        LandingMode::Direct => pb::LandingMode::Direct,
        LandingMode::PullRequests => pb::LandingMode::PullRequests,
    }
}

pub fn from_wire_mode(raw: i32) -> Option<LandingMode> {
    match pb::LandingMode::try_from(raw).ok()? {
        pb::LandingMode::Direct => Some(LandingMode::Direct),
        pb::LandingMode::PullRequests => Some(LandingMode::PullRequests),
        pb::LandingMode::Unspecified => None,
    }
}

/// A read as the wire carries it.
pub fn to_wire(r: &Reading) -> pb::RepositoryLanding {
    let f = &r.facts;
    let rules = f.rules.as_ref();
    let view = f.repo.as_ref();
    pb::RepositoryLanding {
        base: f.base.clone(),
        suggested: r.decision.suggested.map_or(pb::LandingMode::Unspecified, to_wire_mode) as i32,
        direct_impossible: r.decision.direct_impossible,
        reasons: r.decision.reasons.clone(),
        warnings: r.decision.warnings.clone(),
        facts: Some(pb::LandingFacts {
            pull_request_rule: rules.map(|r| r.pull_request),
            required_approvals: rules.filter(|r| r.pull_request).map(|r| r.approvals),
            merge_queue: rules.map(|r| r.merge_queue),
            required_checks: {
                let mut all = rules.map(|r| r.required_checks.clone()).unwrap_or_default();
                all.extend(f.protection_checks.iter().filter(|c| !all.contains(c)).cloned().collect::<Vec<_>>());
                all
            },
            branch_protected: f.protected,
            squash_allowed: view.and_then(|v| v.squash),
            rebase_allowed: view.and_then(|v| v.rebase),
            merge_commit_allowed: view.and_then(|v| v.merge_commit),
            merge_method: r.decision.merge_method.map(str::to_string),
            codeowners: f.codeowners,
            viewer_permission: view.and_then(|v| v.viewer_permission.clone()),
            merge_group_workflow: f.merge_group_workflow,
        }),
        read_at: r.read_at,
    }
}

/// Whether a workspace's last read is old enough to take again at `now`.
pub fn due(last: Option<&Reading>, now: i64) -> bool {
    let Some(last) = last else { return true };
    let wait = if last.learned_something() { DAY } else { RETRY };
    now.saturating_sub(last.read_at) >= wait.as_millis() as i64
}

/// One look: read each workspace whose last read is due, and announce a
/// fleet change when whether direct landing is refused moved, so the apps
/// re-read the workspace and its sentence. Returns how many it read.
pub async fn look(svc: &Service, watcher: &Watcher) -> usize {
    let mut read_now = 0;
    for ws in svc.store.list_workspaces(None).unwrap_or_default() {
        let before = recall(ws.id);
        if !due(before.as_ref(), now_ms()) {
            continue;
        }
        let Ok(repo) = svc.store.get_repository(ws.repository_id) else { continue };
        let base = chosen_base(svc, ws.id);
        let reading = read(svc, &repo, base.as_deref()).await;
        remember(ws.id, &reading);
        read_now += 1;
        let was = before.is_some_and(|b| b.decision.direct_impossible);
        if was != reading.decision.direct_impossible {
            watcher.announce_fleet_changed();
        }
    }
    read_now
}

/// The daily job, for the life of the daemon: only the real daemon runs it
/// (`main.rs`), never a `--stdio` session.
pub async fn run(svc: Arc<Service>, watcher: Arc<Watcher>) {
    tokio::time::sleep(FIRST).await;
    loop {
        look(&svc, &watcher).await;
        tokio::time::sleep(LOOK).await;
    }
}

#[cfg(test)]
#[path = "landing_read_tests.rs"]
mod tests;
