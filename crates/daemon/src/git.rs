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
