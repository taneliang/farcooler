//! One file's diff, for one selector.
//!
//! Separate from `change_set` because it is requested on demand and the change
//! set never carries patch text: a branch that regenerated a lockfile touches
//! thousands of files, and a phone must not receive that to draw a badge.
//!
//! ## Merges
//!
//! `git show <merge>` prints a COMBINED diff — one column per parent — which the
//! review core refuses on purpose. So a commit is never diffed with `show`. It is
//! diffed against its FIRST PARENT explicitly, which yields an ordinary two-sided
//! patch for merges and non-merges alike. The result is flagged so the client can
//! say "shown by first parent" rather than implying the merge brought nothing.

use std::path::Path;

use farcooler_core::{DomainError, Result};
use farcooler_review::diff::{DiffError, FileDiff, Truncation, parse_unified_from};
use serde::{Deserialize, Serialize};

use crate::git::{git, git_bytes};

/// git's hash of the empty tree. Diffing a root commit against it is how you
/// see what the first commit added, since it has no parent to compare with.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Selector {
    /// The whole branch: merge base to HEAD.
    Range { base_commit: String },
    /// One commit against its first parent.
    Commit { sha: String },
    /// Index against HEAD.
    Staged,
    /// Worktree against index.
    Unstaged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiffResult {
    pub diff: FileDiff,
    /// The commit had more than one parent and this is its first-parent view.
    pub first_parent_of_merge: bool,
    /// Set when the file has no textual diff to show, with the reason. A client
    /// renders the reason rather than an empty diff that reads as "no changes".
    pub unsupported: Option<Unsupported>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unsupported {
    Binary,
    Submodule,
    /// The patch parsed as a combined diff even after asking for first-parent,
    /// which means git was configured to produce one. Refused rather than read.
    CombinedDiff,
    Malformed,
}

/// Whether `sha` has more than one parent.
pub async fn is_merge(repo: &Path, sha: &str) -> Result<bool> {
    let r = git(repo, &["rev-list", "--parents", "-n", "1", sha]).await?;
    if !r.ok {
        return Err(DomainError::OperationFailed);
    }
    // `<sha> <parent1> <parent2>...`
    Ok(r.stdout.split_whitespace().count() > 2)
}

/// The git arguments for a selector, as owned strings.
async fn diff_args(repo: &Path, selector: &Selector) -> Result<(Vec<String>, bool)> {
    Ok(match selector {
        Selector::Range { base_commit } => {
            (vec![base_commit.clone(), "HEAD".to_string()], false)
        }
        Selector::Commit { sha } => {
            let merge = is_merge(repo, sha).await?;
            // `^1` rather than `^`: identical for a single-parent commit, and
            // unambiguous for a merge, which is the case that matters.
            let parent = format!("{sha}^1");
            let resolved = git(repo, &["rev-parse", "--verify", "--quiet", &parent]).await?;
            let left = if resolved.ok { parent } else { EMPTY_TREE.to_string() };
            (vec![left, sha.clone()], merge)
        }
        Selector::Staged => (vec!["--cached".to_string()], false),
        Selector::Unstaged => (Vec::new(), false),
    })
}

/// One file's diff.
pub async fn file_diff(
    repo: &Path,
    selector: &Selector,
    path: &str,
    from_hunk: u32,
) -> Result<FileDiffResult> {
    let (rev_args, first_parent_of_merge) = diff_args(repo, selector).await?;

    let mut args: Vec<&str> = vec!["diff", "--no-color", "--find-renames"];
    for a in &rev_args {
        args.push(a);
    }
    args.push("--");
    args.push(path);

    let raw = git_bytes(repo, &args).await?;
    if !raw.ok {
        return Err(DomainError::OperationFailed);
    }

    // Patch text is read lossily on purpose: a file whose CONTENT is not UTF-8
    // still deserves a hunk count and a "binary" verdict, and the path was
    // already carried separately by the change set.
    let patch = String::from_utf8_lossy(&raw.stdout).into_owned();

    if patch.contains("\nGIT binary patch") || patch.contains("Binary files ") {
        return Ok(FileDiffResult {
            diff: FileDiff {
                path: path.to_string(),
                hunks: Vec::new(),
                truncated: None,
                next_hunk: None,
            },
            first_parent_of_merge,
            unsupported: Some(Unsupported::Binary),
        });
    }
    if patch.contains("\nSubproject commit ") {
        return Ok(FileDiffResult {
            diff: FileDiff {
                path: path.to_string(),
                hunks: Vec::new(),
                truncated: None,
                next_hunk: None,
            },
            first_parent_of_merge,
            unsupported: Some(Unsupported::Submodule),
        });
    }

    match parse_unified_from(path, &patch, from_hunk) {
        Ok(diff) => Ok(FileDiffResult { diff, first_parent_of_merge, unsupported: None }),
        Err(e) => {
            // Never a partial diff. A file whose patch could not be read is
            // reported as unreadable, because half a diff reads as "the rest is
            // unchanged" and that is the one thing it must not say.
            tracing::warn!(error = %e, path, "could not parse patch");
            let unsupported = match e {
                DiffError::CombinedDiff => Unsupported::CombinedDiff,
                _ => Unsupported::Malformed,
            };
            Ok(FileDiffResult {
                diff: FileDiff {
                    path: path.to_string(),
                    hunks: Vec::new(),
                    truncated: Some(Truncation::ByteCap),
                    next_hunk: None,
                },
                first_parent_of_merge,
                unsupported: Some(unsupported),
            })
        }
    }
}

/// The files one commit touched, against its first parent.
pub async fn commit_files(repo: &Path, sha: &str) -> Result<Vec<crate::change_set::FileChange>> {
    let merge = is_merge(repo, sha).await?;
    let parent = format!("{sha}^1");
    let resolved = git(repo, &["rev-parse", "--verify", "--quiet", &parent]).await?;
    let left = if resolved.ok { parent } else { EMPTY_TREE.to_string() };
    let _ = merge;

    let raw =
        git_bytes(repo, &["diff", "--numstat", "-z", "--find-renames", &left, sha]).await?;
    if !raw.ok {
        return Err(DomainError::OperationFailed);
    }
    Ok(crate::change_set::parse_numstat_z(&raw.stdout))
}
