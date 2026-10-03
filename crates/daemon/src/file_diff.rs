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
    /// Worktree against HEAD: everything uncommitted, whether staged or not.
    Local,
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
        // Not `--cached` and not nothing: `git diff HEAD` is both halves at
        // once, which is the only form that shows a file the same way before
        // and after somebody stages it.
        Selector::Local => (vec!["HEAD".to_string()], false),
    })
}

/// Whether `path` is a file git lists as untracked (and not ignored).
///
/// Also the confinement check for the untracked read: `ls-files` refuses a
/// pathspec outside the worktree, matches nothing for a `..` that climbs out,
/// and does not list a file behind a symlinked directory, because git sees the
/// link and never walks into it. The absolute and `..` forms are refused
/// outright first, so nothing depends on that alone.
async fn is_untracked(repo: &Path, path: &str) -> bool {
    let p = Path::new(path);
    let confined = !path.is_empty()
        && p.components().all(|c| matches!(c, std::path::Component::Normal(_)));
    if !confined {
        return false;
    }
    match git_bytes(
        repo,
        &["ls-files", "--others", "--exclude-standard", "-z", "--full-name", "--", path],
    )
    .await
    {
        Ok(r) => r.ok && r.stdout.split(|b| *b == 0).any(|n| n == path.as_bytes()),
        Err(_) => false,
    }
}

/// What reading an untracked file gave.
enum Untracked {
    TooLarge,
    Binary,
    /// A unified patch adding every line (or, for a link, its target).
    Patch(String),
}

/// Read an untracked file for diffing, without git.
///
/// Read here rather than by `git diff --no-index` so the read is bounded
/// (`take(MAX_BYTES + 1)`, never the whole of a file that grew) and so the
/// type is checked on the opened handle: the path is `lstat`ed, opened, and the
/// handle's device and inode must match, so a swap to something else between
/// the check and the read is refused. A symlink is never opened; its target
/// text is the content. Only the final component is guarded; the parent
/// directories are the agent's own and `is_untracked` has already refused any
/// that git does not walk.
fn read_untracked(repo: &Path, path: &str) -> std::io::Result<Untracked> {
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;
    let full = repo.join(path);
    let before = std::fs::symlink_metadata(&full)?;
    let max = farcooler_review::limits::MAX_BYTES;
    if before.file_type().is_symlink() {
        let target = std::fs::read_link(&full)?;
        let text = target.to_string_lossy();
        return Ok(Untracked::Patch(format!("@@ -0,0 +1 @@\n+{text}\n")));
    }
    if !before.is_file() {
        return Err(std::io::Error::other("not a regular file"));
    }
    let file = std::fs::File::open(&full)?;
    let held = file.metadata()?;
    if !held.is_file() || held.dev() != before.dev() || held.ino() != before.ino() {
        return Err(std::io::Error::other("the file changed under the read"));
    }
    let mut bytes = Vec::new();
    file.take(max as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > max {
        return Ok(Untracked::TooLarge);
    }
    if bytes.iter().take(8000).any(|b| *b == 0) {
        return Ok(Untracked::Binary);
    }
    let text = String::from_utf8_lossy(&bytes);
    if text.is_empty() {
        return Ok(Untracked::Patch(String::new()));
    }
    let unterminated = !text.ends_with('\n');
    let lines: Vec<&str> = text.lines().collect();
    let mut patch = format!("@@ -0,0 +1,{} @@\n", lines.len());
    for l in &lines {
        patch.push('+');
        patch.push_str(l);
        patch.push('\n');
    }
    if unterminated {
        patch.push_str("\\ No newline at end of file\n");
    }
    Ok(Untracked::Patch(patch))
}

/// One file's diff.
pub async fn file_diff(
    repo: &Path,
    selector: &Selector,
    path: &str,
    from_hunk: u32,
    context: u32,
) -> Result<FileDiffResult> {
    let (rev_args, first_parent_of_merge) = diff_args(repo, selector).await?;

    // A working-tree view of a file git does not track yet. `git diff` has no
    // answer for it, so it is diffed against nothing instead.
    let working_tree_view = matches!(selector, Selector::Local | Selector::Unstaged);
    let untracked = working_tree_view && is_untracked(repo, path).await;
    let mut untracked_patch: Option<String> = None;
    if untracked {
        let (repo_owned, path_owned) = (repo.to_path_buf(), path.to_string());
        let read = tokio::task::spawn_blocking(move || read_untracked(&repo_owned, &path_owned))
            .await
            .map_err(|_| DomainError::OperationFailed)?
            .map_err(|_| DomainError::OperationFailed)?;
        let empty = |truncated, unsupported| FileDiffResult {
            diff: FileDiff {
                path: path.to_string(),
                hunks: Vec::new(),
                truncated,
                next_hunk: None,
            },
            first_parent_of_merge,
            unsupported,
        };
        match read {
            Untracked::TooLarge => return Ok(empty(Some(Truncation::ByteCap), None)),
            Untracked::Binary => return Ok(empty(None, Some(Unsupported::Binary))),
            Untracked::Patch(p) => untracked_patch = Some(p),
        }
    }

    let mut args: Vec<&str> = vec!["diff", "--no-color", "--find-renames"];
    // Capped rather than passed through. A client asking to open one gap sends
    // a number big enough to cover it; an unbounded one would let a caller ask
    // git to print a hundred thousand lines of context per hunk on a file that
    // is a hundred lines long, and the caps further down would then spend their
    // budget truncating it.
    let context_arg;
    if context > 0 {
        context_arg = format!("-U{}", context.min(50_000));
        args.push(&context_arg);
    }
    for a in &rev_args {
        args.push(a);
    }
    args.push("--");
    args.push(path);

    let patch = if let Some(p) = untracked_patch {
        p
    } else {
        let raw = git_bytes(repo, &args).await?;
        if !raw.ok {
            return Err(DomainError::OperationFailed);
        }
        // Patch text is read lossily on purpose: a file whose CONTENT is not
        // UTF-8 still deserves a hunk count and a "binary" verdict, and the
        // path was already carried separately by the change set.
        String::from_utf8_lossy(&raw.stdout).into_owned()
    };

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
///
/// Two diffs over the SAME range, merged — the pattern `change_set::numstat`
/// settled on. `--numstat` counts lines and can spot a rename, but it has no
/// way to say whether a path was created or removed, so `parse_numstat_z`
/// writes `Modified` into every record it does not recognize as a rename. That
/// value is a placeholder and this is the merge that overwrites it: without it
/// a commit that CREATED a file reports it "modified", and so does one that
/// deleted it — which is precisely the distinction a reviewer opens a commit
/// to read.
///
/// `left` is resolved once and handed to BOTH calls. For a root commit it is
/// the empty tree, and a `--name-status` pass against `sha^1` there would not
/// merely fail loudly — `apply_name_status_z` matches by path and ignores what
/// it cannot find, so a mismatched range yields statuses that look plausible
/// and describe a different diff. One binding, two uses, no second resolve.
pub async fn commit_files(repo: &Path, sha: &str) -> Result<Vec<crate::change_set::FileChange>> {
    let merge = is_merge(repo, sha).await?;
    let parent = format!("{sha}^1");
    let resolved = git(repo, &["rev-parse", "--verify", "--quiet", &parent]).await?;
    let left = if resolved.ok { parent } else { EMPTY_TREE.to_string() };
    let _ = merge;

    let raw =
        git_bytes(repo, &["diff", "--numstat", "-z", "--find-renames", &left, sha]).await?;
    let names =
        git_bytes(repo, &["diff", "--name-status", "-z", "--find-renames", &left, sha]).await?;
    if !raw.ok || !names.ok {
        return Err(DomainError::OperationFailed);
    }
    let mut files = crate::change_set::parse_numstat_z(&raw.stdout);
    crate::change_set::apply_name_status_z(&mut files, &names.stdout);
    Ok(files)
}
