//! Gathering the survey's signals (ov-519): git, the named files at the
//! repository's root, the runner's own `gh`, and nothing else. `survey`
//! turns them into answers.
//!
//! **Bounded.** The whole read shares one budget (`LIMITS`): every git gets
//! what is left of it (`git::git_bytes_by`), and every `gh` the lesser of
//! that and its own five seconds. A slow or hanging `gh` costs its answers, never the
//! survey: a read that timed out is a `gh` that didn't answer. The gits run
//! together, and the `gh` calls in two rounds of two, so a 400-commit
//! repository reads in well under the budget.
//!
//! **Read-only, and only what it names.** Every git here reads: no fetch, no
//! write to the index. Files are listed, never opened, except the few in
//! `survey::NAMED` at the root, which are opened without following a link
//! and without waiting on a pipe, and read only when they are small regular
//! files.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::time::{Instant, timeout_at};

use crate::landing::{self, Facts};
use crate::service::Service;
use crate::survey::{Gh, Merged, NAMED, Signals, codeowners, from_pull_request};

/// How long a survey may take.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// The whole survey's time, gits and `gh` together.
    pub budget: Duration,
    /// One `gh` call's most.
    pub gh_call: Duration,
}

/// The runner's: ten seconds in all, five for any one `gh`.
pub const LIMITS: Limits = Limits { budget: Duration::from_secs(10), gh_call: Duration::from_secs(5) };
/// Commits counted at most: past this a repository is simply not new.
pub const MOST_COMMITS: u32 = 1000;
/// The last commits whose subjects are read, within 90 days.
const MOST_RECENT: &str = "400";
/// The first-parent commits looked at for merges.
const FIRST_PARENT: &str = "200";
/// The most paths listed, and the most bytes of listing read to find them.
const MOST_FILES: usize = 100_000;
const MOST_LISTING_BYTES: usize = 8 * 1024 * 1024;
/// The largest named file read.
const MOST_NAMED_BYTES: u64 = 256 * 1024;

/// One `gh` read in the repository, or `None` for every way it can fail.
pub trait GhRead: Sync {
    fn read(&self, args: &[&str], within: Duration) -> impl Future<Output = Option<Vec<u8>>> + Send;
}

/// The runner's own `gh`, as `landing_read` runs it: inside its exec
/// allowlist, with the daemon's git pins, under the runner's `gh` permits.
///
/// All of it inside `within`: the wait for a permit (shared with PR watching
/// and landing, whose holders may each run 15 s), the gits that read the
/// pins, and gh itself. A survey waits for none of them past its budget
/// (review 2a, M2).
pub struct RunnerGh<'a> {
    pub svc: &'a Service,
    pub tree: PathBuf,
}

impl GhRead for RunnerGh<'_> {
    async fn read(&self, args: &[&str], within: Duration) -> Option<Vec<u8>> {
        let read = async {
            let _permit = self.svc.gh_permit().await;
            let mut gh = crate::stack::gh(&self.tree).await.ok()?;
            let o = gh
                .current_dir(&self.tree)
                .args(args)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true)
                .output()
                .await
                .ok()?;
            o.status.success().then_some(o.stdout)
        };
        tokio::time::timeout(within, read).await.ok().flatten()
    }
}

/// A git's stdout as text, or `None` when it failed or ran out of time.
async fn git(deadline: Instant, tree: &Path, args: &[&str]) -> Option<String> {
    let out = crate::git::git_bytes_by(deadline, tree, args).await.ok().filter(|o| o.ok)?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// What `gh` gets of the time left.
fn gh_time(deadline: Instant, limits: Limits) -> Duration {
    deadline.saturating_duration_since(Instant::now()).min(limits.gh_call)
}

/// Every path git lists, tracked and untracked but not ignored, capped: the
/// listing is read only to `MOST_LISTING_BYTES` (`git_capped`), so an
/// unignored `node_modules` costs a cap, not memory.
async fn files(deadline: Instant, tree: &Path) -> Vec<String> {
    let args = ["ls-files", "-z", "--cached", "--others", "--exclude-standard"];
    let Some(out) = crate::git_capped::git_capped(deadline, tree, &args, MOST_LISTING_BYTES).await else {
        return Vec::new();
    };
    let mut listing = out.stdout.as_slice();
    if out.cut {
        // The last path was cut in half: keep only whole ones.
        listing = listing.iter().rposition(|b| *b == 0).map_or(&[][..], |end| &listing[..end]);
    }
    let mut all: Vec<String> = listing
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .take(MOST_FILES)
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect();
    all.sort_unstable();
    all.dedup();
    all
}

/// A named file at the root: a regular file, opened without following a
/// link or waiting on a pipe, and no bigger than `MOST_NAMED_BYTES`.
fn read_named(tree: &Path, name: &str) -> Option<String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(tree.join(name))
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > MOST_NAMED_BYTES {
        return None;
    }
    let mut text = String::new();
    file.take(MOST_NAMED_BYTES).read_to_string(&mut text).ok()?;
    Some(text)
}

/// The branch work lands on, as the checkout knows it.
async fn local_branch(deadline: Instant, tree: &Path) -> String {
    if let Some(remote) = git(deadline, tree, &["symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD"]).await {
        if let Some(name) = remote.trim().strip_prefix("origin/").filter(|n| !n.is_empty()) {
            return name.to_string();
        }
    }
    git(deadline, tree, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .await
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| "main".into())
}

/// `gh pr list --state merged --json number,latestReviews`: how many, how
/// many were reviewed, and who reviewed most.
pub fn parse_merged(bytes: &[u8]) -> Option<Merged> {
    let v: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let prs = v.as_array()?;
    let mut by: BTreeMap<String, u32> = BTreeMap::new();
    let mut reviewed = 0;
    for pr in prs {
        let reviews = pr.get("latestReviews").and_then(|r| r.as_array()).map(Vec::as_slice).unwrap_or_default();
        if !reviews.is_empty() {
            reviewed += 1;
        }
        let mut seen: Vec<&str> = Vec::new();
        for r in reviews {
            if let Some(login) = r.get("author").and_then(|a| a.get("login")).and_then(|l| l.as_str()) {
                if !login.is_empty() && !seen.contains(&login) {
                    seen.push(login);
                    *by.entry(login.to_string()).or_default() += 1;
                }
            }
        }
    }
    let mut ranked: Vec<(String, u32)> = by.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Some(Merged { count: prs.len() as u32, reviewed, reviewers: ranked.into_iter().take(3).map(|(l, _)| l).collect() })
}

/// What GitHub says of the repository and its base branch, as
/// `landing_read::read` asks it, and the merged pull requests. `branch` is
/// the checkout's guess at the base, replaced by GitHub's default.
async fn ask_github<G: GhRead>(gh: &G, deadline: Instant, limits: Limits, branch: &mut String, codeowners: bool, s: &mut Signals) {
    let view_args = [
        "repo",
        "view",
        "--json",
        "defaultBranchRef,squashMergeAllowed,rebaseMergeAllowed,mergeCommitAllowed,viewerPermission",
    ];
    let merged_args = ["pr", "list", "--state", "merged", "--limit", "50", "--json", "number,latestReviews"];
    let (view, merged) = tokio::join!(gh.read(&view_args, gh_time(deadline, limits)), gh.read(&merged_args, gh_time(deadline, limits)));
    let Some(view) = view.as_deref().and_then(landing::parse_repo_view) else {
        s.gh = Gh::Unanswered;
        return;
    };
    s.gh = Gh::Answered;
    s.merged = merged.as_deref().and_then(parse_merged);
    if let Some(default) = view.default_branch.clone() {
        *branch = default;
    }
    let segment = crate::landing_read::path_segment(branch);
    let rules_path = format!("repos/{{owner}}/{{repo}}/rules/branches/{segment}?per_page=100");
    let branch_path = format!("repos/{{owner}}/{{repo}}/branches/{segment}");
    let rules_args = ["api", "--paginate", "-X", "GET", rules_path.as_str()];
    let branch_args = ["api", "-X", "GET", branch_path.as_str()];
    let (rules, protection) =
        tokio::join!(gh.read(&rules_args, gh_time(deadline, limits)), gh.read(&branch_args, gh_time(deadline, limits)));
    let facts = Facts {
        base: branch.clone(),
        rules: rules.as_deref().and_then(landing::parse_rules),
        protected: protection.as_deref().and_then(landing::parse_protected),
        protection_checks: protection.as_deref().map(landing::parse_protection_checks).unwrap_or_default(),
        repo: Some(view),
        codeowners: Some(codeowners),
        merge_group_workflow: None,
    };
    let decision = decision_for(&facts);
    s.landing = Some((facts, decision));
}

/// `landing::decide` for `facts`, its reasons naming the branch as the
/// survey does (`survey::branch_name`): they become signals in the charter,
/// and a name cleaning would change must not appear as a branch that doesn't
/// exist (review 2a, R1).
pub(crate) fn decision_for(facts: &Facts) -> landing::Decision {
    landing::decide(&Facts { base: crate::survey::branch_name(&facts.base), ..facts.clone() })
}

/// Every signal for the repository checked out at `tree`. `gh` is `None`
/// when the runner has none; `agents` is `Host.agents_found`'s list. Never
/// fails: what could not be read is absent.
pub async fn gather<G: GhRead>(tree: &Path, gh: Option<&G>, agents: Vec<String>) -> Signals {
    gather_within(tree, gh, agents, LIMITS).await
}

/// `gather`, within `limits`.
pub async fn gather_within<G: GhRead>(tree: &Path, gh: Option<&G>, agents: Vec<String>, limits: Limits) -> Signals {
    gather_until(tree, gh, agents, Instant::now() + limits.budget, limits).await
}

/// `gather`, done by `deadline`, which the caller set before anything it did
/// first (`survey_ops::signals` looks for the agents), each `gh` call also
/// held to `limits.gh_call`.
pub async fn gather_until<G: GhRead>(
    tree: &Path,
    gh: Option<&G>,
    agents: Vec<String>,
    deadline: Instant,
    limits: Limits,
) -> Signals {
    let head = git(deadline, tree, &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]);
    let shallow = git(deadline, tree, &["rev-parse", "--is-shallow-repository"]);
    let most = format!("--max-count={MOST_COMMITS}");
    let count_args = ["rev-list", "--count", most.as_str(), "HEAD"];
    let count = git(deadline, tree, &count_args);
    let subjects = git(
        deadline,
        tree,
        &["log", "--no-merges", "--since=90.days", "-n", MOST_RECENT, "--format=%s", "HEAD", "--"],
    );
    // Parents, then the subject, split by a unit separator no subject holds.
    let parents = git(deadline, tree, &["log", "--first-parent", "-n", FIRST_PARENT, "--format=%P%x1f%s", "HEAD", "--"]);
    let (head, shallow, count, subjects, parents, files, mut branch) =
        tokio::join!(head, shallow, count, subjects, parents, files(deadline, tree), local_branch(deadline, tree));

    let mut s = Signals {
        empty: head.is_none(),
        shallow: shallow.is_some_and(|o| o.trim() == "true"),
        commits: count.and_then(|c| c.trim().parse().ok()).unwrap_or(0),
        subjects: subjects.map(|t| t.lines().map(str::to_string).filter(|l| !l.is_empty()).collect()).unwrap_or_default(),
        agents,
        ..Default::default()
    };
    for line in parents.as_deref().unwrap_or_default().lines() {
        let (ids, subject) = line.split_once('\u{1f}').unwrap_or((line, ""));
        s.parents.push(ids.split_whitespace().count() as u32);
        s.pr_subjects += u32::from(from_pull_request(subject));
    }
    // Off the runtime: a slow or network disk stalls a blocking thread, not
    // a worker (review 2a, L10).
    let listed: Vec<&'static str> = NAMED.iter().copied().filter(|name| files.iter().any(|f| f == name)).collect();
    let root = tree.to_path_buf();
    let read = tokio::task::spawn_blocking(move || {
        listed.into_iter().filter_map(|name| read_named(&root, name).map(|text| (name.to_string(), text))).collect()
    });
    s.texts = timeout_at(deadline, read).await.ok().and_then(|r| r.ok()).unwrap_or_default();
    let owners = codeowners(&files);
    s.files = files;
    match gh {
        Some(gh) => ask_github(gh, deadline, limits, &mut branch, owners, &mut s).await,
        None => s.gh = Gh::Missing,
    }
    s.branch = branch;
    s
}
