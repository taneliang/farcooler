//! Stacks of branches, and what GitHub says about them.
//!
//! A stack is a chain from the repository's default branch up to a tip, one PR
//! per link. It is derived per REPOSITORY, not per workspace, so two worktrees
//! on two links of the same stack show the same chain from different positions —
//! which is what happens when someone splits a branch and opens a second
//! worktree to review one slice of it.
//!
//! ## Nothing here is authoritative about merging
//!
//! Git is authoritative for local content and ancestry. GitHub is authoritative
//! for PR lifecycle. Neither alone may green-light deleting a worktree, and the
//! reason is squash merges: a squashed PR's commits never appear in `main` with
//! those SHAs, so `merge-base --is-ancestor` reports "not merged" forever. The
//! only thing that proves nothing local is unmerged is comparing the branch head
//! against the PR head GitHub last saw.
//!
//! When GitHub cannot be reached the answer is `Unknown`. Never "not merged",
//! and never a silent yes.

use std::path::Path;
use std::time::Duration;

use farcooler_core::Result;
use serde::Deserialize;
use uuid::Uuid;

use crate::git::git;

/// How long a `gh` invocation may take.
///
/// Longer than git's ten seconds because this one crosses a network, and still
/// bounded because a captive portal or a DNS blackhole otherwise hangs it
/// forever — and `gh` is reached from a fleet refresh that touches every
/// repository.
pub const GH_TIMEOUT: Duration = Duration::from_secs(15);

/// The most `gh` processes the daemon will run at once, across all repositories.
pub const GH_MAX_CONCURRENT: usize = 4;

/// The least time between automatic refreshes of one repository. A user pressing
/// Refresh is exempt.
///
/// The caller is `Service::claim_pr_fill` (`service.rs`), reached from
/// `stack.get`. It bounds two different things at once. The first is a
/// stampede: five clients opening the same repository in the same second would
/// otherwise each see an empty cache and each fork a `gh`. The second is a
/// repository `gh` cannot answer for at all — logged out, or no remote — where
/// the cache never fills and every read would otherwise try again.
pub const GH_MIN_INTERVAL: Duration = Duration::from_secs(60);

/// How old a read of GitHub may be before a client should say so.
///
/// Decided here rather than in each app, so iOS, Android and macOS cannot hold
/// three different opinions about what "a while ago" means — which is what
/// would have happened, because all three already render that sentence and none
/// of them could ever show it: `stale` was hardwired false at both construction
/// sites and never once set true.
///
/// Fifteen minutes, for what it has to separate. It is not `GH_MIN_INTERVAL`:
/// sixty seconds would mark almost every PR stale almost immediately, since a
/// read fills the cache once and nothing polls it afterwards, and a warning
/// that is always on is a warning nobody reads — the same reason `71934f8`
/// turned the last permanent green dot neutral. It is short enough to still be
/// true: a CI run finishes in minutes, so a check state read a quarter of an
/// hour ago is genuinely worth doubting, and the sentence appearing is what
/// tells someone their Refresh would not be pointless.
pub const PR_STALE_AFTER: Duration = Duration::from_secs(15 * 60);

/// Whether a PR read at `fetched_at` is old enough to say so, at `now`. Both in
/// unix milliseconds.
///
/// A reading from the FUTURE is current, not stale. `fetched_at` is the
/// runner's own wall clock and a phone is comparing it against that same
/// runner's clock a moment later, but an NTP correction between the two lands
/// here as a negative age — and the arithmetic must not turn that into a large
/// positive one.
pub fn is_stale(fetched_at: i64, now: i64) -> bool {
    now.saturating_sub(fetched_at) > PR_STALE_AFTER.as_millis() as i64
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentSource {
    Recorded,
    Upstream,
    PrBase,
    Guessed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackLink {
    pub branch: String,
    pub parent_branch: String,
    pub parent_source: ParentSource,
    pub head_commit: String,
    pub ahead: u32,
    pub behind: u32,
    pub pr: Option<PrStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrStatus {
    pub number: u32,
    pub url: String,
    pub state: PrState,
    pub checks: CheckState,
    pub review_decision: ReviewDecision,
    pub head_oid: String,
    pub merged_at: Option<i64>,
    /// When this was read from GitHub, in unix milliseconds.
    ///
    /// There is deliberately no `stale` beside it. There was one, `false` at
    /// every construction site and set true by nothing, behind a sentence three
    /// apps already knew how to render — so the field existed, crossed the wire,
    /// and could never be true. Staleness is a fact about how long ago this
    /// happened, which changes while the value sits in the cache; it is derived
    /// at the moment of answering, by `review_ops::pb_pr` from `is_stale`.
    pub fetched_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Unknown,
    Open,
    Draft,
    Merged,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    Unknown,
    Passing,
    Failing,
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewDecision {
    Unknown,
    Approved,
    ChangesRequested,
    ReviewRequired,
}

/// Walk the parent chain from `branch` down to the default branch.
///
/// Bounded and cycle-checked. A chain that loops is REPORTED rather than
/// followed: a user can absolutely set A's parent to B and B's parent to A, and
/// an unguarded walk would spin until something ran out of memory.
pub fn walk_chain(
    branch: &str,
    default_branch: &str,
    parent_of: &dyn Fn(&str) -> Option<(String, ParentSource)>,
) -> (Vec<StackLink>, bool) {
    let mut links: Vec<StackLink> = Vec::new();
    let mut seen: Vec<String> = vec![branch.to_string()];
    let mut current = branch.to_string();

    loop {
        if current == default_branch {
            break;
        }
        let (parent, source) = parent_of(&current)
            .unwrap_or_else(|| (default_branch.to_string(), ParentSource::Guessed));

        links.push(StackLink {
            branch: current.clone(),
            parent_branch: parent.clone(),
            parent_source: source,
            head_commit: String::new(),
            ahead: 0,
            behind: 0,
            pr: None,
        });

        if seen.iter().any(|b| b == &parent) {
            // The loop closes here. Everything walked so far is still useful and
            // is returned; the caller says so rather than pretending the chain
            // simply ended.
            return (links, true);
        }
        seen.push(parent.clone());
        current = parent;

        if links.len() > 64 {
            return (links, true);
        }
    }

    links.reverse();
    (links, false)
}

/// `gh pr list` for one repository, parsed.
///
/// One call listing every open PR rather than one per branch: a stack of six is
/// six network round trips otherwise, and a fleet refresh multiplies that by the
/// number of repositories.
#[derive(Debug, Deserialize)]
struct GhPr {
    number: u32,
    url: String,
    state: String,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "headRefOid", default)]
    head_ref_oid: String,
    #[serde(rename = "baseRefName", default)]
    base_ref_name: String,
    #[serde(rename = "isDraft", default)]
    is_draft: bool,
    #[serde(rename = "mergedAt", default)]
    merged_at: Option<String>,
    #[serde(rename = "reviewDecision", default)]
    review_decision: Option<String>,
    #[serde(rename = "statusCheckRollup", default)]
    status_check_rollup: Option<Vec<GhCheck>>,
}

#[derive(Debug, Deserialize)]
struct GhCheck {
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

/// Everything GitHub knows about this repository's open and recently merged PRs.
///
/// Returns `Ok(None)` when `gh` is absent, unauthenticated, or unreachable —
/// that is a normal condition, not an error, and it must degrade to `Unknown`
/// rather than failing a review the user is in the middle of.
pub async fn fetch_prs(worktree: &Path) -> Result<Option<Vec<PrInfo>>> {
    let out = tokio::time::timeout(
        GH_TIMEOUT,
        tokio::process::Command::new("gh")
            .current_dir(worktree)
            .args([
                "pr",
                "list",
                "--state",
                "all",
                "--limit",
                "100",
                "--json",
                "number,url,state,headRefName,headRefOid,baseRefName,isDraft,mergedAt,\
                 reviewDecision,statusCheckRollup",
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await;

    let out = match out {
        Ok(Ok(o)) if o.status.success() => o,
        // Every failure mode lands here and means the same thing to a user:
        // Far Cooler cannot say. `gh` missing, logged out, rate limited, offline,
        // or slower than the timeout are not separately actionable.
        _ => return Ok(None),
    };

    let prs: Vec<GhPr> = match serde_json::from_slice(&out.stdout) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "could not read gh output");
            return Ok(None);
        }
    };

    Ok(Some(prs.into_iter().map(parse_pr).collect()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrInfo {
    pub head_ref: String,
    pub base_ref: String,
    pub status: PrStatus,
}

fn parse_pr(p: GhPr) -> PrInfo {
    let state = match (p.state.as_str(), p.is_draft) {
        ("MERGED", _) => PrState::Merged,
        ("CLOSED", _) => PrState::Closed,
        ("OPEN", true) => PrState::Draft,
        ("OPEN", false) => PrState::Open,
        _ => PrState::Unknown,
    };

    let checks = match &p.status_check_rollup {
        None => CheckState::Unknown,
        Some(rollup) if rollup.is_empty() => CheckState::Unknown,
        Some(rollup) => {
            let mut failing = false;
            let mut pending = false;
            for c in rollup {
                match c.conclusion.as_deref() {
                    Some("FAILURE") | Some("TIMED_OUT") | Some("CANCELLED")
                    | Some("ACTION_REQUIRED") => failing = true,
                    Some("SUCCESS") | Some("NEUTRAL") | Some("SKIPPED") => {}
                    // No conclusion yet: still running.
                    _ => {
                        if c.status.as_deref() != Some("COMPLETED") {
                            pending = true;
                        }
                    }
                }
            }
            if failing {
                CheckState::Failing
            } else if pending {
                CheckState::Pending
            } else {
                CheckState::Passing
            }
        }
    };

    let review_decision = match p.review_decision.as_deref() {
        Some("APPROVED") => ReviewDecision::Approved,
        Some("CHANGES_REQUESTED") => ReviewDecision::ChangesRequested,
        Some("REVIEW_REQUIRED") => ReviewDecision::ReviewRequired,
        _ => ReviewDecision::Unknown,
    };

    let merged_at = p.merged_at.as_deref().and_then(parse_iso8601_millis);

    PrInfo {
        head_ref: p.head_ref_name,
        base_ref: p.base_ref_name,
        status: PrStatus {
            number: p.number,
            url: p.url,
            state,
            checks,
            review_decision,
            head_oid: p.head_ref_oid,
            merged_at,
            fetched_at: crate::review::now_millis(),
        },
    }
}

/// `2026-08-05T10:29:37Z` to unix millis.
///
/// Hand-rolled rather than pulling in a date library for one field on one
/// optional code path.
fn parse_iso8601_millis(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20 {
        return None;
    }
    let n = |a: usize, z: usize| -> Option<i64> { s.get(a..z)?.parse().ok() };
    let (y, mo, d) = (n(0, 4)?, n(5, 7)?, n(8, 10)?);
    let (h, mi, sec) = (n(11, 13)?, n(14, 16)?, n(17, 19)?);

    // Days since the epoch, by the civil-from-days algorithm.
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;

    Some(((days * 86_400) + h * 3600 + mi * 60 + sec) * 1000)
}

/// How far a branch is ahead of and behind its parent.
pub async fn ahead_behind(worktree: &Path, branch: &str, parent: &str) -> (u32, u32) {
    let spec = format!("{parent}...{branch}");
    match git(worktree, &["rev-list", "--left-right", "--count", &spec]).await {
        Ok(o) if o.ok => {
            let mut it = o.stdout.split_whitespace();
            let behind = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let ahead = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            (ahead, behind)
        }
        _ => (0, 0),
    }
}

/// Whether a worktree is safe to discard.
///
/// Both halves must hold, and the second is the one that works under a squash
/// merge — where the first half's local form can never succeed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscardVerdict {
    Safe,
    /// Something local would be lost. Never safe.
    UnmergedWork,
    /// Cannot be established. Shown as the ordinary confirmation, never as a
    /// green light.
    Unknown,
}

pub struct DiscardInputs<'a> {
    pub links: &'a [StackLink],
    pub dirty: bool,
    pub untracked: bool,
    /// Any stash in the repository at all.
    ///
    /// `git stash list` gives no structural link between an entry and the
    /// worktree it was made in, so "no stash for this worktree" is not a fact git
    /// can supply. The conservative reading: if the repository has any stashes,
    /// say Unknown and make the person look.
    pub stashes: u32,
    /// Branch head as it stands locally, per link.
    pub local_heads: &'a [(String, String)],
}

pub fn discard_verdict(i: &DiscardInputs<'_>) -> DiscardVerdict {
    if i.dirty || i.untracked {
        return DiscardVerdict::UnmergedWork;
    }
    if i.stashes > 0 {
        return DiscardVerdict::Unknown;
    }
    if i.links.is_empty() {
        return DiscardVerdict::Unknown;
    }

    for link in i.links {
        let Some(pr) = &link.pr else { return DiscardVerdict::Unknown };
        if pr.state != PrState::Merged {
            return DiscardVerdict::Unknown;
        }
        let local = i.local_heads.iter().find(|(b, _)| *b == link.branch).map(|(_, h)| h);
        match local {
            // The squash-merge check: the branch must be exactly what GitHub
            // merged. A local commit past the PR head is work nobody merged.
            Some(h) if *h == pr.head_oid => {}
            Some(_) => return DiscardVerdict::UnmergedWork,
            None => return DiscardVerdict::Unknown,
        }
    }
    DiscardVerdict::Safe
}

/// What one `gh repo view` says about a repository.
///
/// Two fields from one subprocess. The web URL rides along with the default
/// branch rather than being parsed out of a git remote, because parsing
/// `git@github.com:o/r.git` by hand is a guess that is wrong for every GitHub
/// Enterprise host — and `gh` already knows, already runs, and is already
/// cached per repository for the daemon's lifetime.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoFacts {
    /// GitHub's default branch, bare (`main`), never remote-qualified.
    pub default_branch: Option<String>,
    /// The repository's page, e.g. `https://github.com/o/r`.
    pub url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GhRepo {
    #[serde(rename = "defaultBranchRef", default)]
    default_branch_ref: Option<GhBranchRef>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GhBranchRef {
    #[serde(default)]
    name: String,
}

/// The repository's default branch and web URL, according to GitHub.
///
/// One short call, and every field degrades to `None` for every reason
/// `fetch_prs` does — `gh` absent, logged out, offline, rate limited. The caller
/// falls back to `origin/HEAD` for the branch, so a runner without `gh` is not a
/// runner without review; there is no fallback for the URL, and a client shows
/// no link rather than a link to the wrong host.
///
/// Parsed as JSON rather than asked for with `-q`, which is what this used to
/// do. `-q` can only pull one value out, and a second `gh` process to learn a
/// string that arrived in the same response would be a network round trip for
/// nothing.
pub async fn fetch_repo_facts(worktree: &Path) -> RepoFacts {
    let out = tokio::time::timeout(
        GH_TIMEOUT,
        tokio::process::Command::new("gh")
            .current_dir(worktree)
            .args(["repo", "view", "--json", "defaultBranchRef,url"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await;

    let out = match out {
        Ok(Ok(o)) if o.status.success() => o,
        _ => return RepoFacts::default(),
    };

    let repo: GhRepo = match serde_json::from_slice(&out.stdout) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "could not read gh repo output");
            return RepoFacts::default();
        }
    };

    // An empty string is not an answer. `gh` can report a repository with no
    // default branch ref at all — a repository with no commits — and a caller
    // that took `""` for a branch name would run every later git command
    // against nothing.
    let non_empty = |s: String| if s.trim().is_empty() { None } else { Some(s.trim().to_string()) };
    RepoFacts {
        default_branch: repo.default_branch_ref.and_then(|r| non_empty(r.name)),
        url: repo.url.and_then(non_empty),
    }
}

/// Which repository a worktree belongs to, for cache keying.
pub type RepoId = Uuid;

#[cfg(test)]
mod tests {
    use super::*;

    fn link(branch: &str, pr: Option<PrStatus>) -> StackLink {
        StackLink {
            branch: branch.into(),
            parent_branch: "main".into(),
            parent_source: ParentSource::Guessed,
            head_commit: String::new(),
            ahead: 0,
            behind: 0,
            pr,
        }
    }

    fn merged_pr(head_oid: &str) -> PrStatus {
        PrStatus {
            number: 1,
            url: "https://example/1".into(),
            state: PrState::Merged,
            checks: CheckState::Passing,
            review_decision: ReviewDecision::Approved,
            head_oid: head_oid.into(),
            merged_at: Some(1),
            fetched_at: 0,
        }
    }

    #[test]
    fn a_chain_walks_from_the_tip_down_to_the_default_branch() {
        let parents = |b: &str| match b {
            "c" => Some(("b".to_string(), ParentSource::Recorded)),
            "b" => Some(("a".to_string(), ParentSource::Recorded)),
            "a" => Some(("main".to_string(), ParentSource::Recorded)),
            _ => None,
        };
        let (links, cycle) = walk_chain("c", "main", &parents);
        assert!(!cycle);
        let names: Vec<&str> = links.iter().map(|l| l.branch.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c"], "base first, tip last");
    }

    #[test]
    fn a_stack_whose_parents_form_a_cycle_is_reported_not_followed() {
        // A user can set A's parent to B and B's parent to A. An unguarded walk
        // spins forever.
        let parents = |b: &str| match b {
            "a" => Some(("b".to_string(), ParentSource::Recorded)),
            "b" => Some(("a".to_string(), ParentSource::Recorded)),
            _ => None,
        };
        let (links, cycle) = walk_chain("a", "main", &parents);
        assert!(cycle, "the loop must be reported");
        assert!(links.len() < 10, "and must not have been walked far");
    }

    #[test]
    fn an_unknown_parent_is_guessed_at_the_default_branch_and_labelled() {
        let (links, cycle) = walk_chain("orphan", "main", &|_| None);
        assert!(!cycle);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].parent_source, ParentSource::Guessed);
        assert_eq!(links[0].parent_branch, "main");
    }

    #[test]
    fn a_branch_that_is_the_default_branch_has_no_chain() {
        let (links, _) = walk_chain("main", "main", &|_| None);
        assert!(links.is_empty());
    }

    #[test]
    fn a_squash_merged_stack_is_safe_when_every_head_matches_its_pr() {
        // The case local ancestry can never prove: the commits are not in main
        // with these SHAs, and never will be.
        let links = vec![link("part-1", Some(merged_pr("aaa"))), link("part-2", Some(merged_pr("bbb")))];
        let heads = vec![("part-1".to_string(), "aaa".to_string()), ("part-2".to_string(), "bbb".to_string())];
        let v = discard_verdict(&DiscardInputs {
            links: &links,
            dirty: false,
            untracked: false,
            stashes: 0,
            local_heads: &heads,
        });
        assert_eq!(v, DiscardVerdict::Safe);
    }

    #[test]
    fn a_local_commit_past_the_pr_head_is_unmerged_work() {
        let links = vec![link("part-1", Some(merged_pr("aaa")))];
        let heads = vec![("part-1".to_string(), "a-newer-commit".to_string())];
        let v = discard_verdict(&DiscardInputs {
            links: &links,
            dirty: false,
            untracked: false,
            stashes: 0,
            local_heads: &heads,
        });
        assert_eq!(v, DiscardVerdict::UnmergedWork);
    }

    #[test]
    fn any_stash_in_the_repository_forces_unknown() {
        // git cannot attribute a stash to a worktree, so this is the honest
        // answer rather than a guess in the dangerous direction.
        let links = vec![link("part-1", Some(merged_pr("aaa")))];
        let heads = vec![("part-1".to_string(), "aaa".to_string())];
        let v = discard_verdict(&DiscardInputs {
            links: &links,
            dirty: false,
            untracked: false,
            stashes: 2,
            local_heads: &heads,
        });
        assert_eq!(v, DiscardVerdict::Unknown);
    }

    #[test]
    fn an_unreachable_github_forces_unknown_and_never_not_merged() {
        let links = vec![link("part-1", None)];
        let heads = vec![("part-1".to_string(), "aaa".to_string())];
        let v = discard_verdict(&DiscardInputs {
            links: &links,
            dirty: false,
            untracked: false,
            stashes: 0,
            local_heads: &heads,
        });
        assert_eq!(v, DiscardVerdict::Unknown, "no PR state is not evidence of anything");
    }

    #[test]
    fn a_dirty_worktree_is_never_safe_whatever_github_says() {
        let links = vec![link("part-1", Some(merged_pr("aaa")))];
        let heads = vec![("part-1".to_string(), "aaa".to_string())];
        let v = discard_verdict(&DiscardInputs {
            links: &links,
            dirty: true,
            untracked: false,
            stashes: 0,
            local_heads: &heads,
        });
        assert_eq!(v, DiscardVerdict::UnmergedWork);
    }

    #[test]
    fn an_untracked_file_is_unmerged_work_too() {
        let links = vec![link("part-1", Some(merged_pr("aaa")))];
        let heads = vec![("part-1".to_string(), "aaa".to_string())];
        let v = discard_verdict(&DiscardInputs {
            links: &links,
            dirty: false,
            untracked: true,
            stashes: 0,
            local_heads: &heads,
        });
        assert_eq!(v, DiscardVerdict::UnmergedWork);
    }

    #[test]
    fn an_open_pr_is_not_merged() {
        let mut pr = merged_pr("aaa");
        pr.state = PrState::Open;
        let links = vec![link("part-1", Some(pr))];
        let heads = vec![("part-1".to_string(), "aaa".to_string())];
        let v = discard_verdict(&DiscardInputs {
            links: &links,
            dirty: false,
            untracked: false,
            stashes: 0,
            local_heads: &heads,
        });
        assert_eq!(v, DiscardVerdict::Unknown);
    }

    #[test]
    fn a_failing_check_beats_a_pending_one_in_the_rollup() {
        let p = GhPr {
            number: 1,
            url: "u".into(),
            state: "OPEN".into(),
            head_ref_name: "b".into(),
            head_ref_oid: "aaa".into(),
            base_ref_name: "main".into(),
            is_draft: false,
            merged_at: None,
            review_decision: None,
            status_check_rollup: Some(vec![
                GhCheck { conclusion: Some("FAILURE".into()), status: Some("COMPLETED".into()) },
                GhCheck { conclusion: None, status: Some("IN_PROGRESS".into()) },
            ]),
        };
        assert_eq!(parse_pr(p).status.checks, CheckState::Failing);
    }

    #[test]
    fn an_empty_rollup_is_unknown_rather_than_passing() {
        // No checks configured is not the same as all checks green, and showing
        // a green tick for a repository with no CI would be a lie.
        let p = GhPr {
            number: 1,
            url: "u".into(),
            state: "OPEN".into(),
            head_ref_name: "b".into(),
            head_ref_oid: "aaa".into(),
            base_ref_name: "main".into(),
            is_draft: false,
            merged_at: None,
            review_decision: None,
            status_check_rollup: Some(vec![]),
        };
        assert_eq!(parse_pr(p).status.checks, CheckState::Unknown);
    }

    #[test]
    fn a_draft_pr_is_distinguished_from_an_open_one() {
        let mk = |draft: bool| GhPr {
            number: 1,
            url: "u".into(),
            state: "OPEN".into(),
            head_ref_name: "b".into(),
            head_ref_oid: "a".into(),
            base_ref_name: "main".into(),
            is_draft: draft,
            merged_at: None,
            review_decision: None,
            status_check_rollup: None,
        };
        assert_eq!(parse_pr(mk(true)).status.state, PrState::Draft);
        assert_eq!(parse_pr(mk(false)).status.state, PrState::Open);
    }

    /// The sentence three apps already render, given something to render it
    /// for.
    ///
    /// The threshold is decided HERE and nowhere else. `stale` crosses the wire
    /// as a bool precisely so iOS, Android and macOS cannot hold three
    /// different opinions about what "a while ago" means.
    #[test]
    fn a_pr_read_longer_ago_than_the_threshold_is_stale() {
        let now = 1_785_925_777_000;
        let ms = PR_STALE_AFTER.as_millis() as i64;

        assert!(!is_stale(now, now), "a read that just happened is current");
        assert!(!is_stale(now - ms + 1_000, now), "a second inside the threshold is current");
        assert!(is_stale(now - ms - 1_000, now), "a second past it is stale");
        assert!(is_stale(now - 24 * 3_600_000, now), "yesterday is certainly stale");
    }

    /// A clock that went backwards must not make a fresh read look ancient.
    ///
    /// `fetched_at` is this runner's own wall clock, and a phone asking a runner
    /// whose clock was just corrected by NTP would otherwise be told a PR read
    /// four seconds ago was read in the future — which, subtracted, is a large
    /// positive age on some other arrangement of this arithmetic.
    #[test]
    fn a_reading_from_the_future_is_not_stale() {
        let now = 1_785_925_777_000;
        assert!(!is_stale(now + 3_600_000, now));
    }

    #[test]
    fn a_merged_timestamp_is_read_as_unix_millis() {
        // 2026-08-05T10:29:37Z
        let ms = parse_iso8601_millis("2026-08-05T10:29:37Z").expect("parses");
        assert_eq!(ms, 1_785_925_777_000);
    }

    #[test]
    fn a_malformed_timestamp_is_none_rather_than_zero() {
        assert!(parse_iso8601_millis("not a date").is_none());
        assert!(parse_iso8601_millis("").is_none());
    }
}
