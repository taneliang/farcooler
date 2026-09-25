//! Git worktree transaction.
//!
//! Never silently reuses an existing branch or worktree path. If metadata fails
//! after git succeeded, only a newly created CLEAN worktree and newly created
//! UNPUSHED branch are removed; otherwise the artifacts are preserved and
//! manual recovery is surfaced.
//!
//! Serialization is NOT this module's job. `Service::repo_lock` holds a mutex
//! across the whole mutate-git-then-write-the-row sequence, because that is the
//! span the reconciler must not observe half of, and nothing at this level can
//! see the metadata half.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use farcooler_core::{DomainError, Result};
use tokio::process::Command;

/// How long any single git invocation may take before it is killed.
///
/// Worktree operations are local and finish in milliseconds; a call that has not
/// returned in ten seconds is not slow, it is stuck — a repository on a network
/// filesystem that stopped answering, a `.git` on a stalled mount, an index lock
/// held by something that died. Without this the daemon waits forever on a
/// process it can see, and the client waits forever on the daemon.
///
/// Ten seconds rather than one: `git status` on a genuinely large repository with
/// a cold cache can take seconds, and killing honest work is worse than waiting
/// for it.
pub const GIT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub struct GitOutput {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Raw bytes, for the `-z` plumbing whose output is NOT text.
///
/// `git status -z` and friends separate records with NUL and emit path bytes
/// exactly as they are on disk, which on Linux is any byte sequence that is not
/// NUL or `/`. Running that through `from_utf8_lossy` replaces the undecodable
/// bytes with U+FFFD, and a path that has been through that substitution can no
/// longer be opened. Anything that parses paths uses this; anything that parses
/// git's own English uses `git`.
#[derive(Debug)]
pub struct GitBytes {
    pub ok: bool,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

/// Run a git command inside `cwd`, bounded by [`GIT_TIMEOUT`].
pub async fn git(cwd: &Path, args: &[&str]) -> Result<GitOutput> {
    let raw = git_bytes(cwd, args).await?;
    Ok(GitOutput {
        ok: raw.ok,
        stdout: String::from_utf8_lossy(&raw.stdout).into_owned(),
        stderr: raw.stderr,
    })
}

/// Run a git command inside `cwd` and keep its stdout as bytes.
///
/// Two properties beyond spawning the process, and both are the reason this
/// function exists rather than a bare `Command`:
///
/// **It cannot hang.** The call is wrapped in [`GIT_TIMEOUT`]; on expiry the
/// child is killed rather than left to run headless, and the caller gets
/// `OperationFailed` instead of never being answered.
///
/// **It cannot outlive its caller.** `kill_on_drop` means a request whose
/// connection went away takes its git with it. Review recomputes a change set
/// whenever a client asks; without this, a phone that drops off a train tunnel
/// mid-scroll leaves a `git diff` running on the runner for as long as it likes,
/// once per abandoned request.
pub async fn git_bytes(cwd: &Path, args: &[&str]) -> Result<GitBytes> {
    run_bounded(&program(), GIT_TIMEOUT, cwd, args).await
}

/// `git_bytes`, sharing one `deadline` with every other git of the same act.
///
/// For an act that runs several gits one after another, a codex launch (the
/// hooks file's tracked check, then two for each skill file): each git gets
/// what is left of the budget, so a git that hangs costs the act the budget
/// once, not `GIT_TIMEOUT` per call. With nothing left, no git is started
/// and the answer is the same error a timeout gives.
pub async fn git_bytes_by(deadline: tokio::time::Instant, cwd: &Path, args: &[&str]) -> Result<GitBytes> {
    let left = deadline.saturating_duration_since(tokio::time::Instant::now());
    if left.is_zero() {
        tracing::warn!(?args, "no time left for git in this act's budget");
        return Err(DomainError::OperationFailed);
    }
    run_bounded(&program(), left, cwd, args).await
}

/// The program `git_bytes` runs.
#[cfg(not(test))]
fn program() -> std::ffi::OsString {
    std::ffi::OsString::from("git")
}

/// The program `git_bytes` runs: git, unless a test on this thread put
/// something else in its place (`PROGRAM`).
#[cfg(test)]
fn program() -> std::ffi::OsString {
    PROGRAM.with(|p| p.borrow().clone()).unwrap_or_else(|| std::ffi::OsString::from("git"))
}

#[cfg(test)]
thread_local! {
    /// What a test puts where git would be: a program that isn't there, or
    /// one that never answers, to watch what a caller does with the failure.
    /// Per thread, so it changes nothing for a test running beside it; a
    /// `#[tokio::test]` runs its future on its own thread.
    pub(crate) static PROGRAM: std::cell::RefCell<Option<std::ffi::OsString>> =
        const { std::cell::RefCell::new(None) };
}

/// `git_bytes`, with the program and the timeout named.
async fn run_bounded(
    program: &std::ffi::OsStr,
    timeout: Duration,
    cwd: &Path,
    args: &[&str],
) -> Result<GitBytes> {
    let child = Command::new(program)
        .current_dir(cwd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output();

    let out = match tokio::time::timeout(timeout, child).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "failed to spawn git");
            return Err(DomainError::OperationFailed);
        }
        Err(_) => {
            // The timeout dropped the future, and `kill_on_drop` reaped the
            // child with it. Nothing to clean up here; say what happened.
            tracing::warn!(?args, "git timed out and was killed");
            return Err(DomainError::OperationFailed);
        }
    };

    Ok(GitBytes {
        ok: out.status.success(),
        stdout: out.stdout,
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// MVP supports ordinary non-bare repositories with a valid HEAD and a writable
/// working directory. Detached HEAD, bare repositories, and repositories whose
/// common git directory sits outside an allowlisted root are rejected.
pub async fn validate_repository(path: &Path) -> Result<PathBuf> {
    if !path.is_dir() {
        return Err(DomainError::InvalidArgument { what: "repository path" });
    }

    let bare = git(path, &["rev-parse", "--is-bare-repository"]).await?;
    if !bare.ok {
        return Err(DomainError::InvalidArgument { what: "not a git repository" });
    }
    if bare.stdout.trim() == "true" {
        return Err(DomainError::InvalidArgument { what: "bare repository" });
    }

    let common = git(path, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).await?;
    if !common.ok {
        return Err(DomainError::InvalidArgument { what: "unreadable git dir" });
    }

    Ok(PathBuf::from(common.stdout.trim()))
}

/// True when the branch already exists. MVP never silently reuses one.
pub async fn branch_exists(repo: &Path, branch: &str) -> Result<bool> {
    let r = git(repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).await?;
    Ok(r.ok)
}

/// Resolve a base revision to a commit, so a typo fails before any mutation.
pub async fn resolve_revision(repo: &Path, revision: &str) -> Result<String> {
    let r = git(repo, &["rev-parse", "--verify", "--quiet", &format!("{revision}^{{commit}}")]).await?;
    if !r.ok {
        return Err(DomainError::InvalidArgument { what: "base_revision" });
    }
    Ok(r.stdout.trim().to_string())
}

/// Every remote that already carries a branch of this name.
///
/// A list rather than an `Option`, because the COUNT is the decision: git
/// refuses to guess when two remotes both have the name, and so does the caller
/// — anyone with a fork plus an upstream has two, and picking one for them is
/// how a workspace quietly starts from the wrong person's work.
pub async fn remotes_with_branch(repo: &Path, branch: &str) -> Result<Vec<String>> {
    let out = git(repo, &["for-each-ref", "--format", "%(refname)", "refs/remotes"]).await?;
    if !out.ok {
        return Err(DomainError::OperationFailed);
    }
    let mut remotes: Vec<String> = Vec::new();
    for line in out.stdout.lines() {
        let Some(rest) = line.trim().strip_prefix("refs/remotes/") else { continue };
        // `origin/feat/x` splits into remote `origin`, branch `feat/x` — the
        // branch keeps its slashes, so this splits once and only once.
        let mut parts = rest.splitn(2, '/');
        let (Some(remote), Some(name)) = (parts.next(), parts.next()) else { continue };
        if name == branch && !remotes.iter().any(|seen| seen == remote) {
            remotes.push(remote.to_string());
        }
    }
    Ok(remotes)
}

/// What `create_worktree` made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedWorktree {
    /// The commit the worktree was started at.
    pub commit: String,
    /// A new branch cut from the base, as opposed to a remote branch that
    /// already had the name and was checked out with its commits.
    pub forked: bool,
}

/// The worktree transaction.
///
/// Validates, refuses collisions, then creates branch and worktree in one git
/// operation so a half-made branch cannot outlive a failed worktree.
///
/// Returns the commit the worktree was actually started at, which the caller
/// needs and used to compute for itself. It has to come from here now that the
/// answer is not always the base it asked for — and resolving it here rather
/// than twice also closes the window where `HEAD` moved between the two calls
/// and a rollback then refused to remove its own worktree.
///
/// **A branch that already exists on a remote is checked out, not forked.**
/// `git switch feat-branch` creates a local branch from `origin/feat-branch`
/// and tracks it, and this now does the same. Before, a colleague's pushed
/// branch was invisible here: `branch_exists` only reads `refs/heads`, so a
/// remote-only name passed straight through and a fresh branch was cut from
/// `HEAD` — same name as theirs, none of their commits, no upstream, and no
/// sign that any of that had happened.
///
/// Only when the caller asked for the default base. `HEAD` means "wherever this
/// repository is", which is a statement about the repository and not about this
/// branch; naming a base explicitly is a statement about this branch, and it
/// wins. `git switch -c feat origin/main` does not DWIM to `origin/feat`
/// either.
pub async fn create_worktree(
    repo: &Path,
    branch: &str,
    base_revision: &str,
    destination: &Path,
) -> Result<CreatedWorktree> {
    create_worktree_with(repo, branch, base_revision, destination, false).await
}

/// `create_worktree`, or with `fork_only` a NEW branch or nothing: a name any
/// remote already carries is refused as `BranchExists`, whatever the base,
/// rather than checked out.
///
/// For a client that made the name up — the Mac's ⌘N task — where a checkout
/// would start the task on somebody's commits.
///
/// **What makes this safe against a fetch is the create, not a lock.** The
/// caller's repository lock only orders this daemon's own creates; an agent
/// in a sibling worktree can `git fetch` at any moment, including between the
/// remote check below and the `worktree add`. But with `fork_only` there is
/// never a tracking start point: the create is `worktree add -b <branch>
/// <dest> <commit>`, a new branch cut from the base's resolved SHA, and
/// nothing in it can guess a remote branch or check one out. A name fetched
/// in that gap gets a new local branch beside the remote one — still a
/// fork. The check itself is what turns a name already there into a
/// refusal the client can act on.
pub async fn create_worktree_with(
    repo: &Path,
    branch: &str,
    base_revision: &str,
    destination: &Path,
    fork_only: bool,
) -> Result<CreatedWorktree> {
    if branch_exists(repo, branch).await? {
        return Err(DomainError::BranchExists);
    }
    if destination.exists() {
        return Err(DomainError::WorktreeExists);
    }
    if fork_only && !remotes_with_branch(repo, branch).await?.is_empty() {
        return Err(DomainError::BranchExists);
    }

    let tracking = if base_revision == "HEAD" && !fork_only {
        match remotes_with_branch(repo, branch).await?.as_slice() {
            [only] => Some(format!("{only}/{branch}")),
            // None, or too many to choose between. Both fall back to the base
            // that was asked for, which is what this always did.
            _ => None,
        }
    } else {
        None
    };

    // Resolved either way, and before the mutation either way: a typo in a base
    // must still fail before anything is created, and the commit is what the
    // caller rolls back against.
    let start = tracking.clone().unwrap_or_else(|| base_revision.to_string());
    let commit = resolve_revision(repo, &start).await?;

    let dest = destination.to_string_lossy().to_string();
    let r = match &tracking {
        // `--track -b` sets `branch.<name>.remote` and `.merge`, so pushing
        // goes back where the branch came from with nothing further to set. The
        // symbolic ref is passed rather than the commit deliberately: git reads
        // tracking off the START POINT, and a 40-hex SHA is not a
        // remote-tracking branch, so passing the resolved commit here would
        // create the right content with no upstream at all.
        Some(start) => git(repo, &["worktree", "add", "--track", "-b", branch, &dest, start]).await?,
        None => git(repo, &["worktree", "add", "-b", branch, &dest, &commit]).await?,
    };

    if !r.ok {
        tracing::warn!(stderr = %r.stderr, "worktree add failed");
        // Nothing to roll back: `worktree add -b` creates the branch and the
        // worktree together, so a failure leaves neither.
        return Err(DomainError::OperationFailed);
    }
    Ok(CreatedWorktree { commit, forked: tracking.is_none() })
}

/// A branch you could resume work on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchInfo {
    pub name: String,
    /// Present locally. A remote-only branch has to be created before it can be
    /// checked out.
    pub local: bool,
    /// The remote it tracks or came from, if any.
    pub remote: Option<String>,
    /// Already checked out in some worktree. git refuses a second checkout of
    /// the same branch, so this has to be visible BEFORE someone picks it.
    pub checked_out: bool,
    /// Unix seconds of the last commit, for ordering by recency.
    pub updated_at: i64,
    pub subject: String,
}

/// Every branch worth resuming, local and remote, most recent first.
///
/// Both halves matter. Work moves between machines and between people, and a
/// branch pushed from a laptop or by a cloud agent exists only as
/// `origin/whatever` here until something checks it out.
pub async fn list_branches(repo: &Path) -> Result<Vec<BranchInfo>> {
    // One call for both, with a machine-readable separator. `%(if)` would let
    // git do more of this, but keeping the format dumb keeps the parsing
    // obvious.
    let format = "%(refname)\t%(committerdate:unix)\t%(worktreepath)\t%(contents:subject)";
    let out = git(
        repo,
        &["for-each-ref", "--format", format, "refs/heads", "refs/remotes"],
    )
    .await?;
    if !out.ok {
        return Err(DomainError::OperationFailed);
    }

    let mut byname: std::collections::HashMap<String, BranchInfo> = Default::default();
    let mut order: Vec<String> = Vec::new();

    for line in out.stdout.lines() {
        let mut f = line.split('\t');
        let (Some(refname), Some(date), Some(worktree)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        let subject = f.next().unwrap_or("").trim().to_string();
        let updated_at: i64 = date.trim().parse().unwrap_or(0);

        let (name, remote) = if let Some(rest) = refname.strip_prefix("refs/heads/") {
            (rest.to_string(), None)
        } else if let Some(rest) = refname.strip_prefix("refs/remotes/") {
            // `origin/feat/x` splits into remote `origin`, branch `feat/x`.
            let mut parts = rest.splitn(2, '/');
            let (Some(remote), Some(branch)) = (parts.next(), parts.next()) else { continue };
            // HEAD is a symbolic pointer, not a branch anyone resumes.
            if branch == "HEAD" {
                continue;
            }
            (branch.to_string(), Some(remote.to_string()))
        } else {
            continue;
        };

        let entry = byname.entry(name.clone()).or_insert_with(|| {
            order.push(name.clone());
            BranchInfo {
                name: name.clone(),
                local: false,
                remote: None,
                checked_out: false,
                updated_at: 0,
                subject: String::new(),
            }
        });

        // A branch that exists locally AND on a remote is one branch, and the
        // local side is the one that decides whether it is checked out.
        match remote {
            None => {
                entry.local = true;
                entry.checked_out = !worktree.trim().is_empty();
            }
            Some(r) => {
                entry.remote.get_or_insert(r);
            }
        }
        if updated_at > entry.updated_at {
            entry.updated_at = updated_at;
            entry.subject = subject;
        }
    }

    let mut branches: Vec<BranchInfo> = order.into_iter().filter_map(|n| byname.remove(&n)).collect();
    branches.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
    Ok(branches)
}

/// Add a worktree for a branch that already exists.
///
/// The remote-only case is the one that matters: a branch pushed from another
/// machine, by a colleague, or by a cloud agent has no local ref here, and
/// `worktree add <dest> <branch>` would simply fail. `--track -b` creates the
/// local branch pointing at the remote one and sets upstream, so pushing back
/// goes where it came from without further setup.
pub async fn create_worktree_from_branch(
    repo: &Path,
    branch: &str,
    destination: &Path,
) -> Result<()> {
    if destination.exists() {
        return Err(DomainError::WorktreeExists);
    }
    let dest = destination.to_string_lossy().to_string();

    let r = if branch_exists(repo, branch).await? {
        git(repo, &["worktree", "add", &dest, branch]).await?
    } else {
        // Find which remote has it. Guessing `origin` is wrong often enough to
        // matter for anyone with a fork plus an upstream.
        let branches = list_branches(repo).await?;
        let Some(info) = branches.iter().find(|b| b.name == branch) else {
            return Err(DomainError::InvalidArgument { what: "no such branch" });
        };
        let Some(remote) = &info.remote else {
            return Err(DomainError::InvalidArgument { what: "branch has no remote" });
        };
        let start = format!("{remote}/{branch}");
        git(repo, &["worktree", "add", "--track", "-b", branch, &dest, &start]).await?
    };

    if !r.ok {
        // The most common failure is a branch already checked out somewhere
        // else, which git states plainly. Reporting it as such beats a generic
        // failure the user cannot act on.
        if r.stderr.contains("already used by worktree") || r.stderr.contains("already checked out")
        {
            return Err(DomainError::WorktreeExists);
        }
        tracing::warn!(stderr = %r.stderr, "worktree add from branch failed");
        return Err(DomainError::OperationFailed);
    }
    Ok(())
}

/// Roll back a worktree created moments ago, only when it is safe.
///
/// Refuses if the worktree is dirty or the branch has commits that are not on
/// the base, because "make the database look clean" is never worth destroying
/// work. Returns whether anything was removed.
pub async fn rollback_worktree(
    repo: &Path,
    branch: &str,
    destination: &Path,
    base_commit: &str,
) -> Result<bool> {
    let dirty = is_dirty(destination).await.unwrap_or(true);
    if dirty {
        tracing::warn!("refusing to roll back a dirty worktree, preserving artifacts");
        return Ok(false);
    }

    let head = git(destination, &["rev-parse", "HEAD"]).await?;
    if !head.ok || head.stdout.trim() != base_commit {
        tracing::warn!("branch has moved past its base, preserving artifacts");
        return Ok(false);
    }

    let dest = destination.to_string_lossy().to_string();
    let _ = git(repo, &["worktree", "remove", "--force", &dest]).await?;
    let _ = git(repo, &["branch", "-D", branch]).await?;
    Ok(true)
}

/// Uncommitted or untracked changes present.
///
/// The files Far Cooler itself wrote into the worktree are subtracted first.
/// `install_project_hooks` puts `.codex/hooks.json` and `.cursor/hooks.json`
/// into every worktree this runner makes, and `Service::prepare_launch_hooks`
/// puts one of the two into any worktree a codex or cursor pane is opened in —
/// including the checkout the user works in every day, which Far Cooler did not
/// make. `removal_needs_confirmation` derives this same answer from the one
/// `git status` its hooks check also reads, so without the subtraction a
/// workspace created a second ago and never touched by anyone would demand
/// the user type its name back to remove it, on the strength of files Far
/// Cooler wrote and the user has never seen.
///
/// Only an UNTRACKED copy is subtracted (`hook_install::hide_our_untracked`).
/// A hooks file the repository commits is never one Far Cooler wrote, so a
/// change to it is the user's work and counts here like any other.
///
/// The same answer the diff view gets (`change_set::working_tree`), so the two
/// can't disagree about what is the user's.
pub async fn is_dirty(worktree: &Path) -> Result<bool> {
    Ok(crate::change_set::working_tree(worktree).await?.is_dirty())
}

/// A short human summary of the remote, for display only.
pub async fn remote_summary(repo: &Path) -> String {
    match git(repo, &["remote", "get-url", "origin"]).await {
        Ok(r) if r.ok => r.stdout.trim().to_string(),
        _ => "(no remote)".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as SyncCommand;

    fn scratch(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("farcooler-git-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// A worktree destination beside the scratch repo, and unique to this process.
    ///
    /// A worktree cannot live inside the repository it belongs to, so these have
    /// to be siblings. The obvious spelling — joining `../wt-name` onto the
    /// scratch repo — looks like it inherits `scratch`'s per-process name, and
    /// does not: `..` climbs out of the very directory the pid is in, so every
    /// concurrent `cargo test` process aimed at one `/tmp/wt-name` and one of
    /// them lost.
    ///
    /// That made these tests fail only when two runs overlapped, which is the
    /// worst shape a failure can have — it trains people to re-run rather than
    /// read, and it fails in CI on a busy machine while passing on every desk.
    fn sibling(name: &str) -> PathBuf {
        let p = std::env::temp_dir()
            .join(format!("farcooler-git-test-wt-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    fn init_repo(dir: &Path) {
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "t@example.com"],
            vec!["config", "commit.gpgsign", "false"],
            vec!["config", "user.name", "t"],
        ] {
            let status = SyncCommand::new("git").current_dir(dir).args(&args).status().unwrap();
            assert!(status.success(), "git {:?} failed in {}", args, dir.display());
        }
        std::fs::write(dir.join("README.md"), "hello").unwrap();
        let status =
            SyncCommand::new("git").current_dir(dir).args(["add", "."]).status().unwrap();
        assert!(status.success(), "git add failed in {}", dir.display());
        let status = SyncCommand::new("git")
            .current_dir(dir)
            .args(["commit", "-qm", "init"])
            .status()
            .unwrap();
        assert!(status.success(), "git commit failed in {}", dir.display());
    }

    #[tokio::test]
    async fn validates_an_ordinary_repository() {
        let d = scratch("valid");
        init_repo(&d);
        assert!(validate_repository(&d).await.is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn rejects_a_non_repository() {
        let d = scratch("norepo");
        assert!(validate_repository(&d).await.is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn creates_a_worktree_on_a_new_branch() {
        let d = scratch("create");
        init_repo(&d);
        let dest = sibling("create");

        let created = create_worktree(&d, "feature/x", "HEAD", &dest).await.unwrap();
        assert!(created.forked, "a name nobody had is a new branch");

        assert!(dest.join("README.md").exists(), "worktree checked out");
        assert!(branch_exists(&d, "feature/x").await.unwrap());

        let _ = std::fs::remove_dir_all(&dest);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Give `dir` a real remote, at `name`, carrying `branch` with one commit
    /// of its own on it.
    ///
    /// A second repository on disk and a genuine `fetch`, rather than a
    /// hand-written `refs/remotes` ref: what is being tested is that git treats
    /// the result as a remote-tracking branch it will set upstream from, and a
    /// forged ref would test the parsing and none of that.
    fn add_remote_branch(dir: &Path, name: &str, branch: &str, marker: &str) {
        let remote = sibling(&format!("remote-{marker}"));
        std::fs::create_dir_all(&remote).unwrap();
        init_repo(&remote);
        for args in [
            vec!["checkout", "-q", "-b", branch],
            vec!["commit", "-q", "--allow-empty", "-m", "theirs"],
        ] {
            let status = SyncCommand::new("git").current_dir(&remote).args(&args).status().unwrap();
            assert!(status.success(), "git {args:?} failed in the remote");
        }
        for args in [
            vec!["remote", "add", name, remote.to_str().unwrap()],
            vec!["fetch", "-q", name],
        ] {
            let status = SyncCommand::new("git").current_dir(dir).args(&args).status().unwrap();
            assert!(status.success(), "git {args:?} failed in {}", dir.display());
        }
    }

    fn head_of(dir: &Path, rev: &str) -> String {
        let out = SyncCommand::new("git")
            .current_dir(dir)
            .args(["rev-parse", rev])
            .output()
            .unwrap();
        // Asserted rather than trusted: `rev-parse` prints nothing and exits
        // non-zero for a ref it cannot resolve, and an empty string compared
        // against another empty string is a test that passes for the wrong
        // reason.
        assert!(out.status.success(), "rev-parse {rev} failed in {}", dir.display());
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    /// Someone else pushed the branch; checking it out must get THEIR commits.
    ///
    /// The reported bug, exactly: a colleague creates `origin/feat-branch`, you
    /// make a workspace for it, and you get an empty branch of the same name cut
    /// from `main`. `branch_exists` reads only `refs/heads`, so nothing noticed.
    #[tokio::test]
    async fn a_branch_that_exists_on_a_remote_is_checked_out_rather_than_forked() {
        let d = scratch("dwim");
        init_repo(&d);
        add_remote_branch(&d, "origin", "feat-branch", "dwim");
        let dest = sibling("dwim");

        let created = create_worktree(&d, "feat-branch", "HEAD", &dest).await.unwrap();
        let commit = created.commit;
        assert!(!created.forked, "checked out, not forked: it carries their commits");

        assert_eq!(commit, head_of(&d, "refs/remotes/origin/feat-branch"), "started from theirs");
        assert_ne!(commit, head_of(&d, "refs/heads/main"), "and not from HEAD");
        assert_eq!(
            head_of(&dest, "HEAD"),
            head_of(&d, "refs/remotes/origin/feat-branch"),
            "the worktree really is on their commit"
        );
        // And it tracks, so pushing goes back where it came from.
        assert_eq!(
            head_of(&d, "feat-branch@{upstream}"),
            head_of(&d, "refs/remotes/origin/feat-branch"),
        );

        let _ = std::fs::remove_dir_all(&dest);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Fork only: a name a remote already carries is refused, not checked
    /// out, and nothing is made — at `HEAD`, where the plain create would
    /// check it out, and at a named base too.
    #[tokio::test]
    async fn fork_only_refuses_a_name_a_remote_carries_and_makes_nothing() {
        let d = scratch("forkonly");
        init_repo(&d);
        add_remote_branch(&d, "origin", "feat-branch", "forkonly");
        for base in ["HEAD", "main"] {
            let dest = sibling("forkonly");
            let err = create_worktree_with(&d, "feat-branch", base, &dest, true).await.unwrap_err();
            assert!(matches!(err, DomainError::BranchExists), "{base}: {err:?}");
            assert!(!dest.exists(), "{base}: no worktree");
            assert!(!branch_exists(&d, "feat-branch").await.unwrap(), "{base}: no local branch");
        }

        // A name nobody has is made as ever, and is a fork.
        let dest = sibling("forkonly");
        let created = create_worktree_with(&d, "fresh", "HEAD", &dest, true).await.unwrap();
        assert!(created.forked);
        assert_eq!(created.commit, head_of(&d, "refs/heads/main"));

        let _ = std::fs::remove_dir_all(&dest);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// An explicitly named base is a statement about THIS branch and wins.
    /// `git switch -c feat origin/main` does not DWIM to `origin/feat` either.
    #[tokio::test]
    async fn an_explicit_base_is_not_overruled_by_a_remote_of_the_same_name() {
        let d = scratch("dwimbase");
        init_repo(&d);
        add_remote_branch(&d, "origin", "feat-branch", "dwimbase");
        let dest = sibling("dwimbase");

        let commit = create_worktree(&d, "feat-branch", "main", &dest).await.unwrap().commit;

        assert_eq!(commit, head_of(&d, "refs/heads/main"));
        assert_ne!(commit, head_of(&d, "refs/remotes/origin/feat-branch"));

        let _ = std::fs::remove_dir_all(&dest);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Two remotes carrying the name is git's own refusal to guess, and anyone
    /// with a fork plus an upstream has two.
    #[tokio::test]
    async fn an_ambiguous_branch_name_falls_back_to_the_base() {
        let d = scratch("dwimamb");
        init_repo(&d);
        add_remote_branch(&d, "origin", "feat-branch", "dwimamb-a");
        add_remote_branch(&d, "upstream", "feat-branch", "dwimamb-b");
        let dest = sibling("dwimamb");

        assert_eq!(
            remotes_with_branch(&d, "feat-branch").await.unwrap(),
            vec!["origin".to_string(), "upstream".to_string()]
        );
        let commit = create_worktree(&d, "feat-branch", "HEAD", &dest).await.unwrap().commit;
        assert_eq!(commit, head_of(&d, "HEAD"), "neither remote was chosen");

        let _ = std::fs::remove_dir_all(&dest);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A branch nobody else has is still cut from the base, with no upstream —
    /// which is what `git switch -c` does, and what this always did.
    #[tokio::test]
    async fn a_name_no_remote_carries_is_still_a_fresh_branch() {
        let d = scratch("dwimnew");
        init_repo(&d);
        add_remote_branch(&d, "origin", "feat-branch", "dwimnew");
        let dest = sibling("dwimnew");

        let commit = create_worktree(&d, "something-else", "HEAD", &dest).await.unwrap().commit;

        assert_eq!(commit, head_of(&d, "HEAD"));
        let upstream = SyncCommand::new("git")
            .current_dir(&d)
            .args(["rev-parse", "refs/heads/something-else@{upstream}"])
            .output()
            .unwrap();
        assert!(!upstream.status.success(), "no upstream invented for a branch nobody has");

        let _ = std::fs::remove_dir_all(&dest);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn refuses_an_existing_branch_rather_than_reusing_it() {
        let d = scratch("branchdup");
        init_repo(&d);
        let a = sibling("a1");
        let b = sibling("b1");

        create_worktree(&d, "dup", "HEAD", &a).await.unwrap();
        let err = create_worktree(&d, "dup", "HEAD", &b).await.unwrap_err();
        assert!(matches!(err, DomainError::BranchExists));

        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn refuses_an_occupied_destination() {
        let d = scratch("pathdup");
        init_repo(&d);
        let dest = sibling("occupied");
        std::fs::create_dir_all(&dest).unwrap();

        let err = create_worktree(&d, "newbranch", "HEAD", &dest).await.unwrap_err();
        assert!(matches!(err, DomainError::WorktreeExists));

        let _ = std::fs::remove_dir_all(&dest);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn rejects_an_unknown_base_revision_before_mutating() {
        let d = scratch("badbase");
        init_repo(&d);
        let dest = sibling("badbase");

        let err = create_worktree(&d, "b", "no-such-rev", &dest).await.unwrap_err();
        assert!(matches!(err, DomainError::InvalidArgument { .. }));
        assert!(!branch_exists(&d, "b").await.unwrap(), "no branch left behind");
        assert!(!dest.exists());

        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn rollback_removes_a_clean_untouched_worktree() {
        let d = scratch("rbclean");
        init_repo(&d);
        let dest = sibling("rbclean");

        let base = resolve_revision(&d, "HEAD").await.unwrap();
        create_worktree(&d, "rb", "HEAD", &dest).await.unwrap();

        assert!(rollback_worktree(&d, "rb", &dest, &base).await.unwrap());
        assert!(!dest.exists());
        assert!(!branch_exists(&d, "rb").await.unwrap());

        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn rollback_preserves_a_dirty_worktree() {
        let d = scratch("rbdirty");
        init_repo(&d);
        let dest = sibling("rbdirty");

        let base = resolve_revision(&d, "HEAD").await.unwrap();
        create_worktree(&d, "rbd", "HEAD", &dest).await.unwrap();
        std::fs::write(dest.join("scratch.txt"), "work in progress").unwrap();

        assert!(
            !rollback_worktree(&d, "rbd", &dest, &base).await.unwrap(),
            "must refuse to delete uncommitted work"
        );
        assert!(dest.join("scratch.txt").exists(), "the user's work survives");

        let _ = std::fs::remove_dir_all(&dest);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The data-loss half of the tracked-hooks ruling, as the scratch-repo
    /// reproduction: a repository that COMMITS `.codex/hooks.json`, with that
    /// file edited. The edit is the user's work. It must show in the diff view
    /// and count as dirt, because `removal_needs_confirmation` reads this, and
    /// `git worktree remove --force` would otherwise take the edit with it.
    ///
    /// An untracked `.cursor/hooks.json` beside it stays hidden: the exclusion
    /// is decided per file, not dropped for the whole repository.
    #[tokio::test]
    async fn an_edit_to_a_tracked_hooks_file_is_the_users_change() {
        let d = scratch("tracked-hooks");
        init_repo(&d);
        std::fs::create_dir_all(d.join(".codex")).unwrap();
        std::fs::write(d.join(".codex/hooks.json"), "{\"hooks\":{}}\n").unwrap();
        for args in [vec!["add", "--", ".codex/hooks.json"], vec!["commit", "-qm", "codex hooks"]] {
            let status = SyncCommand::new("git").current_dir(&d).args(&args).status().unwrap();
            assert!(status.success(), "git {args:?}");
        }
        std::fs::create_dir_all(d.join(".cursor")).unwrap();
        std::fs::write(d.join(".cursor/hooks.json"), "{\"version\":1}\n").unwrap();
        assert!(!is_dirty(&d).await.unwrap(), "committed and untouched, beside an untracked copy of ours");

        std::fs::write(d.join(".codex/hooks.json"), "{\"hooks\":{\"Stop\":[]}}\n").unwrap();

        assert!(is_dirty(&d).await.unwrap(), "an edit to a tracked hooks file is uncommitted work");
        let tree = crate::change_set::working_tree(&d).await.unwrap();
        let unstaged: Vec<&str> = tree.unstaged.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(unstaged, [".codex/hooks.json"], "the diff view lists the edit");
        assert!(tree.untracked.is_empty(), "the untracked copy is still ours to hide: {:?}", tree.untracked);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A tracked hooks file taken out of the index with `git rm --cached` and
    /// then edited is the user's work twice over: a staged deletion and an
    /// untracked copy. Only `?` records are ever hidden, so the staged
    /// deletion still shows, and so does the change.
    #[tokio::test]
    async fn a_hooks_file_taken_out_of_the_index_is_still_the_users_change() {
        let d = scratch("rm-cached-hooks");
        init_repo(&d);
        std::fs::create_dir_all(d.join(".codex")).unwrap();
        std::fs::write(d.join(".codex/hooks.json"), "{\"hooks\":{}}\n").unwrap();
        for args in [
            vec!["add", "--", ".codex/hooks.json"],
            vec!["commit", "-qm", "codex hooks"],
            vec!["rm", "-q", "--cached", "--", ".codex/hooks.json"],
        ] {
            let status = SyncCommand::new("git").current_dir(&d).args(&args).status().unwrap();
            assert!(status.success(), "git {args:?}");
        }
        std::fs::write(d.join(".codex/hooks.json"), "{\"hooks\":{\"Stop\":[]}}\n").unwrap();

        assert!(is_dirty(&d).await.unwrap(), "a staged deletion is uncommitted work");
        let tree = crate::change_set::working_tree(&d).await.unwrap();
        let staged: Vec<&str> = tree.staged.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(staged, [".codex/hooks.json"], "the diff view lists the deletion");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The case the exclusion exists for, which the per-file answer must keep:
    /// a checkout whose only new files are the two Far Cooler writes is clean,
    /// to `is_dirty` and to the diff view alike.
    #[tokio::test]
    async fn untracked_hooks_files_of_ours_are_still_not_the_users_work() {
        let d = scratch("untracked-hooks");
        init_repo(&d);
        for relative in crate::hook_install::PROJECT_HOOK_FILES {
            let path = d.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "{}\n").unwrap();
        }

        assert!(!is_dirty(&d).await.unwrap(), "nothing here is the user's");
        let tree = crate::change_set::working_tree(&d).await.unwrap();
        assert!(!tree.is_dirty(), "and the diff view opens empty: {tree:?}");
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// A worktree that already exists on disk.
///
/// Found rather than created. People arrive at Far Cooler with a repository they
/// have been using for months and a handful of worktrees already checked out —
/// and until they can see those here, Far Cooler is a tool that only knows about
/// work it started itself, which is a bad first impression and a lot of manual
/// re-creation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    pub path: String,
    /// The branch checked out there, or `None` for a detached HEAD.
    pub branch: Option<String>,
    pub head: String,
    /// The repository's own working tree, as opposed to a linked worktree.
    ///
    /// Listed by git and deliberately excluded from what is offered: the main
    /// checkout is where you work by hand, and turning it into a task workspace
    /// would put an agent in it.
    pub is_main: bool,
    /// git holds a lock, usually because the worktree lives on removable media.
    pub locked: bool,
    /// The directory is gone; git would drop this on the next `worktree prune`.
    pub prunable: bool,
}

/// Every worktree git knows about for this repository, main checkout included.
///
/// `--porcelain` because the human format is not a format: it aligns columns and
/// truncates, and parsing it would break on a long path.
pub async fn list_worktrees(repo: &Path) -> Result<Vec<WorktreeInfo>> {
    let out = git(repo, &["worktree", "list", "--porcelain"]).await?;
    if !out.ok {
        return Err(DomainError::OperationFailed);
    }

    let mut found = Vec::new();
    let mut current: Option<WorktreeInfo> = None;
    // Records are separated by a blank line; the first one is the main checkout.
    let mut first = true;

    for line in out.stdout.lines() {
        if line.is_empty() {
            if let Some(worktree) = current.take() {
                found.push(worktree);
            }
            continue;
        }
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "worktree" => {
                if let Some(worktree) = current.take() {
                    found.push(worktree);
                }
                current = Some(WorktreeInfo {
                    path: value.to_string(),
                    branch: None,
                    head: String::new(),
                    is_main: std::mem::replace(&mut first, false),
                    locked: false,
                    prunable: false,
                });
            }
            "HEAD" => {
                if let Some(w) = current.as_mut() {
                    w.head = value.to_string();
                }
            }
            "branch" => {
                if let Some(w) = current.as_mut() {
                    // `refs/heads/feat/x` -> `feat/x`.
                    w.branch = Some(value.trim_start_matches("refs/heads/").to_string());
                }
            }
            "locked" => {
                if let Some(w) = current.as_mut() {
                    w.locked = true;
                }
            }
            "prunable" => {
                if let Some(w) = current.as_mut() {
                    w.prunable = true;
                }
            }
            // `detached`, `bare`, and anything a newer git adds. A record we do
            // not fully understand is still a worktree at a path.
            _ => {}
        }
    }
    if let Some(worktree) = current.take() {
        found.push(worktree);
    }
    Ok(found)
}

/// The marker naming which install owns a worktree, inside git's own admin
/// directory.
const OWNER_MARKER: &str = "farcooler-install-id";

/// Git's admin directory for a worktree.
///
/// `<repo>/.git/worktrees/<name>` for a linked worktree, `<repo>/.git` for the
/// main checkout. The right place for a marker: invisible to `git status`, so
/// it never turns up in anyone's diff, and removed by `git worktree prune`
/// along with the worktree it describes.
///
/// Not `git config --worktree`, which requires turning on
/// `extensions.worktreeConfig` for the whole repository — a change with its own
/// effects on how `core.worktree` resolves, and not ours to make to someone
/// else's repo just to leave a note.
async fn admin_dir(worktree: &Path) -> Result<PathBuf> {
    let r = git(worktree, &["rev-parse", "--absolute-git-dir"]).await?;
    if !r.ok {
        return Err(DomainError::OperationFailed);
    }
    Ok(PathBuf::from(r.stdout.trim()))
}

/// Record which install owns a worktree.
///
/// Best effort by design. A marker that could not be written leaves the
/// worktree looking unowned, which is exactly the behaviour that existed before
/// ownership did — whereas failing workspace creation over a note would be a
/// worse outcome than the hazard it guards.
pub async fn mark_owner(worktree: &Path, install_id: &str) {
    match admin_dir(worktree).await {
        Ok(dir) => {
            if let Err(e) = std::fs::write(dir.join(OWNER_MARKER), install_id) {
                tracing::warn!(error = %e, "could not mark worktree ownership");
            }
        }
        Err(e) => tracing::warn!(error = ?e, "could not locate the worktree admin dir"),
    }
}

/// The marker saying this install forked the worktree's branch, new, for it.
const FORKED_MARKER: &str = "farcooler-forked";

/// Record that `worktree`'s branch was cut new by `install_id`, rather than
/// checked out with somebody's commits. Read by `forked_by`; cursor's
/// `--trust` is drawn on it. Best effort, like `mark_owner`: a mark that could
/// not be written costs the trust skip, and cursor asks as it always did.
pub async fn mark_forked(worktree: &Path, install_id: &str) {
    match admin_dir(worktree).await {
        Ok(dir) => {
            if let Err(e) = std::fs::write(dir.join(FORKED_MARKER), install_id) {
                tracing::warn!(error = %e, "could not mark a forked worktree");
            }
        }
        Err(e) => tracing::warn!(error = ?e, "could not locate the worktree admin dir"),
    }
}

/// Which install forked this worktree's branch, if one did.
///
/// Synchronous, because a launch is built synchronously: it reads the
/// linked worktree's `.git` FILE (`gitdir: <admin dir>`) rather than asking
/// git. A main checkout has a `.git` directory, not a file, and so is never
/// forked — which is right, nobody forked it.
pub fn forked_by(worktree: &Path) -> Option<String> {
    let pointer = std::fs::read_to_string(worktree.join(".git")).ok()?;
    let admin = pointer.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
    let admin = if Path::new(admin).is_absolute() {
        PathBuf::from(admin)
    } else {
        worktree.join(admin)
    };
    let mark = std::fs::read_to_string(admin.join(FORKED_MARKER)).ok()?;
    let mark = mark.trim();
    (!mark.is_empty()).then(|| mark.to_string())
}

/// Which install owns a worktree, if any claims it.
///
/// `None` means unowned — a worktree someone made by hand — and unowned is
/// adoptable, because picking those up is the whole point of adoption. Only a
/// mark naming a DIFFERENT install is a refusal.
pub async fn owner_of(worktree: &Path) -> Option<String> {
    let dir = admin_dir(worktree).await.ok()?;
    let s = std::fs::read_to_string(dir.join(OWNER_MARKER)).ok()?;
    let s = s.trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

#[cfg(test)]
mod worktree_tests {
    use super::*;

    /// Parsing is exercised through a real repository, because the shape of
    /// `--porcelain` output is the thing under test and a hand-written fixture
    /// would only prove I can copy it.
    #[tokio::test]
    async fn lists_the_main_checkout_and_every_linked_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        for args in [
            vec!["init", "-q", "-b", "main", "."],
            vec!["config", "user.email", "t@example.com"],
            vec!["config", "commit.gpgsign", "false"],
            vec!["config", "user.name", "t"],
            vec!["commit", "-q", "--allow-empty", "-m", "base"],
        ] {
            git(&repo, &args).await.unwrap();
        }

        let extra = dir.path().join("side");
        git(&repo, &["worktree", "add", "-q", "-b", "feat/side", extra.to_str().unwrap()])
            .await
            .unwrap();

        let found = list_worktrees(&repo).await.unwrap();
        assert_eq!(found.len(), 2, "main checkout plus the linked one: {found:?}");

        let main = &found[0];
        assert!(main.is_main, "the first record is always the main checkout");
        assert_eq!(main.branch.as_deref(), Some("main"));

        let side = &found[1];
        assert!(!side.is_main);
        assert_eq!(side.branch.as_deref(), Some("feat/side"), "refs/heads/ is stripped");
        assert!(!side.head.is_empty());
        assert!(!side.locked && !side.prunable);
    }

    #[tokio::test]
    async fn a_detached_worktree_has_no_branch_rather_than_a_fake_one() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        for args in [
            vec!["init", "-q", "-b", "main", "."],
            vec!["config", "user.email", "t@example.com"],
            vec!["config", "commit.gpgsign", "false"],
            vec!["config", "user.name", "t"],
            vec!["commit", "-q", "--allow-empty", "-m", "base"],
        ] {
            git(&repo, &args).await.unwrap();
        }
        let extra = dir.path().join("detached");
        git(&repo, &["worktree", "add", "-q", "--detach", extra.to_str().unwrap()])
            .await
            .unwrap();

        let found = list_worktrees(&repo).await.unwrap();
        let detached = found.iter().find(|w| w.path.ends_with("detached")).expect("found");
        assert_eq!(detached.branch, None);
        assert!(!detached.head.is_empty(), "it still has a commit");
    }

    /// A repository and one linked worktree beside it, `side`, on a new branch.
    async fn a_linked_worktree(dir: &Path) -> (PathBuf, PathBuf) {
        let repo = dir.join("repo");
        std::fs::create_dir(&repo).unwrap();
        for args in [
            vec!["init", "-q", "-b", "main", "."],
            vec!["config", "user.email", "t@example.com"],
            vec!["config", "commit.gpgsign", "false"],
            vec!["config", "user.name", "t"],
            vec!["commit", "-q", "--allow-empty", "-m", "base"],
        ] {
            git(&repo, &args).await.unwrap();
        }
        let side = dir.join("side");
        git(&repo, &["worktree", "add", "-q", "-b", "side", side.to_str().unwrap()]).await.unwrap();
        (repo, side)
    }

    /// The mark names the install that wrote it, whichever that was. The
    /// comparison against this install's id is the caller's
    /// (`forked_this_worktree`), and its own test writes another install's
    /// mark.
    #[tokio::test]
    async fn forked_by_names_the_install_that_marked_it_and_nothing_unmarked() {
        let dir = tempfile::tempdir().unwrap();
        let (repo, side) = a_linked_worktree(dir.path()).await;

        assert_eq!(forked_by(&side), None, "unmarked");
        assert_eq!(forked_by(&repo), None, "a main checkout has a .git directory, not a file");
        mark_forked(&side, "install-b").await;
        assert_eq!(forked_by(&side).as_deref(), Some("install-b"));
    }

    /// git 2.48 and later can write the pointer relative to the worktree
    /// (`worktree.useRelativePaths`). Read against the worktree, not against
    /// whatever directory the daemon happens to run in.
    #[tokio::test]
    async fn forked_by_follows_a_relative_gitdir() {
        let dir = tempfile::tempdir().unwrap();
        let (_repo, side) = a_linked_worktree(dir.path()).await;
        mark_forked(&side, "install-a").await;

        let pointer = std::fs::read_to_string(side.join(".git")).unwrap();
        let admin = pointer.trim().strip_prefix("gitdir:").unwrap().trim().to_string();
        let tail = Path::new(&admin)
            .strip_prefix(dir.path().canonicalize().unwrap())
            .or_else(|_| Path::new(&admin).strip_prefix(dir.path()))
            .expect("the admin dir is under the test's directory")
            .to_path_buf();
        std::fs::write(side.join(".git"), format!("gitdir: ../{}\n", tail.display())).unwrap();

        assert_eq!(forked_by(&side).as_deref(), Some("install-a"));
    }
}
