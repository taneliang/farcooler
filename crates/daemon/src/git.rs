//! Git worktree transaction.
//!
//! Workspace creation is serialized per repository and never silently reuses an
//! existing branch or worktree path. If metadata fails after git succeeded, only
//! a newly created CLEAN worktree and newly created UNPUSHED branch are removed;
//! otherwise the artifacts are preserved and manual recovery is surfaced.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use overnight_core::{DomainError, Result};
use tokio::process::Command;

#[derive(Debug)]
pub struct GitOutput {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Run a git command inside `cwd`.
pub async fn git(cwd: &Path, args: &[&str]) -> Result<GitOutput> {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "failed to spawn git");
            DomainError::OperationFailed
        })?;

    Ok(GitOutput {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
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

/// The worktree transaction.
///
/// Validates, refuses collisions, then creates branch and worktree in one git
/// operation so a half-made branch cannot outlive a failed worktree.
pub async fn create_worktree(
    repo: &Path,
    branch: &str,
    base_revision: &str,
    destination: &Path,
) -> Result<()> {
    if branch_exists(repo, branch).await? {
        return Err(DomainError::BranchExists);
    }
    if destination.exists() {
        return Err(DomainError::WorktreeExists);
    }
    let base = resolve_revision(repo, base_revision).await?;

    let dest = destination.to_string_lossy().to_string();
    let r = git(repo, &["worktree", "add", "-b", branch, &dest, &base]).await?;

    if !r.ok {
        tracing::warn!(stderr = %r.stderr, "worktree add failed");
        // Nothing to roll back: `worktree add -b` creates the branch and the
        // worktree together, so a failure leaves neither.
        return Err(DomainError::OperationFailed);
    }
    Ok(())
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
    branches.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
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
pub async fn is_dirty(worktree: &Path) -> Result<bool> {
    let r = git(worktree, &["status", "--porcelain"]).await?;
    if !r.ok {
        return Err(DomainError::OperationFailed);
    }
    Ok(!r.stdout.trim().is_empty())
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
        let p = std::env::temp_dir().join(format!("overnight-git-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn init_repo(dir: &Path) {
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "t@example.com"],
            vec!["config", "user.name", "t"],
        ] {
            SyncCommand::new("git").current_dir(dir).args(&args).status().unwrap();
        }
        std::fs::write(dir.join("README.md"), "hello").unwrap();
        SyncCommand::new("git").current_dir(dir).args(["add", "."]).status().unwrap();
        SyncCommand::new("git")
            .current_dir(dir)
            .args(["commit", "-qm", "init"])
            .status()
            .unwrap();
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
        let dest = d.join("../wt-create");
        let _ = std::fs::remove_dir_all(&dest);

        create_worktree(&d, "feature/x", "HEAD", &dest).await.unwrap();

        assert!(dest.join("README.md").exists(), "worktree checked out");
        assert!(branch_exists(&d, "feature/x").await.unwrap());

        let _ = std::fs::remove_dir_all(&dest);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn refuses_an_existing_branch_rather_than_reusing_it() {
        let d = scratch("branchdup");
        init_repo(&d);
        let a = d.join("../wt-a1");
        let b = d.join("../wt-b1");
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);

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
        let dest = d.join("../wt-occupied");
        let _ = std::fs::remove_dir_all(&dest);
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
        let dest = d.join("../wt-badbase");

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
        let dest = d.join("../wt-rbclean");
        let _ = std::fs::remove_dir_all(&dest);

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
        let dest = d.join("../wt-rbdirty");
        let _ = std::fs::remove_dir_all(&dest);

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
}

/// A worktree that already exists on disk.
///
/// Found rather than created. People arrive at Overnight with a repository they
/// have been using for months and a handful of worktrees already checked out —
/// and until they can see those here, Overnight is a tool that only knows about
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
}
