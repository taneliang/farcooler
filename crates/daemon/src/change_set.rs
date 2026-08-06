//! What a branch changed, and how dirty the worktree is.
//!
//! Derived, never stored. Everything here is recomputed from git on request;
//! there is no row in which a stale summary could be written, which is the same
//! rule terminal state follows.
//!
//! ## The range trap
//!
//! `diff` and `log` disagree about what three dots mean, and getting it wrong is
//! silent:
//!
//! ```text
//!   git diff base...HEAD   ==  git diff $(git merge-base base HEAD) HEAD   CORRECT
//!   git log  base...HEAD   ==  commits reachable from EITHER, not both     WRONG
//!   git log  base..HEAD    ==  commits on HEAD since they diverged         CORRECT
//! ```
//!
//! A `log base...HEAD` includes commits that only base has — work this branch
//! never did — and nothing about the output looks wrong. So the merge base is
//! resolved once, explicitly, and both calls are given two dots against it.
//!
//! ## Why the digest hashes contents
//!
//! `worktree_digest` decides whether a comment written earlier needs re-reading.
//! Hashing porcelain output alone does not change when a file that was ALREADY
//! listed as modified is modified again — so an unanchored comment, which is
//! most of them, would never notice an agent's edit. Hashing every tracked file
//! is correct and unaffordable. Hashing the contents of the files git already
//! says are dirty is both: the list is the size of the work under review, not
//! the size of the repository.

use std::path::Path;

use farcooler_core::{DomainError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::git::{git, git_bytes};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Untracked,
    Conflicted,
}

impl FileStatus {
    fn from_xy(code: u8) -> FileStatus {
        match code {
            b'A' => FileStatus::Added,
            b'D' => FileStatus::Deleted,
            b'R' => FileStatus::Renamed,
            b'C' => FileStatus::Copied,
            b'T' => FileStatus::TypeChanged,
            _ => FileStatus::Modified,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub status: FileStatus,
    pub old_path: Option<String>,
    pub insertions: u32,
    pub deletions: u32,
    /// git reported `-` for both counts, which is how it says "not text".
    pub binary: bool,
    pub submodule: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    pub sha: String,
    pub subject: String,
    pub body: String,
    pub author: String,
    pub timestamp: i64,
    pub files_changed: u32,
    pub insertions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkingTree {
    pub staged: Vec<FileChange>,
    pub unstaged: Vec<FileChange>,
    pub untracked: Vec<String>,
    pub conflicted: Vec<String>,
}

impl WorkingTree {
    pub fn is_dirty(&self) -> bool {
        !self.staged.is_empty()
            || !self.unstaged.is_empty()
            || !self.untracked.is_empty()
            || !self.conflicted.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaseSource {
    /// The user pinned it.
    Recorded,
    /// `branch.<name>.merge`.
    Upstream,
    /// The repository's default branch, because nothing better was known.
    Guessed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeSet {
    pub branch: String,
    pub base_ref: String,
    pub base_source: BaseSource,
    /// The resolved merge base. Displayed, because a wrong base silently
    /// produces a wrong diff and the user is the only one who can spot it.
    pub base_commit: String,
    pub head_commit: String,
    pub commits: Vec<Commit>,
    pub working_tree: WorkingTree,
    pub files: Vec<FileChange>,
    pub insertions: u32,
    pub deletions: u32,
    pub worktree_digest: String,
}

impl ChangeSet {
    pub fn is_dirty(&self) -> bool {
        self.working_tree.is_dirty()
    }
}

/// Resolve the merge base of `base_ref` and HEAD.
pub async fn merge_base(repo: &Path, base_ref: &str) -> Result<String> {
    let r = git(repo, &["merge-base", base_ref, "HEAD"]).await?;
    if !r.ok {
        return Err(DomainError::InvalidArgument { what: "base_ref" });
    }
    Ok(r.stdout.trim().to_string())
}

/// The commits this branch made, oldest first.
///
/// TWO dots against the resolved merge base. See the module note.
pub async fn commits_since(repo: &Path, base_commit: &str) -> Result<Vec<Commit>> {
    // A record separator that cannot occur in a commit message, and a field
    // separator likewise. %x1e / %x1f are the ASCII record and unit separators,
    // which is what they are for.
    let range = format!("{base_commit}..HEAD");
    let r = git(
        repo,
        &["log", "--reverse", "--no-color", "--format=%H%x1f%an%x1f%at%x1f%s%x1f%b%x1e", &range],
    )
    .await?;
    if !r.ok {
        return Err(DomainError::OperationFailed);
    }

    let mut out = Vec::new();
    for rec in r.stdout.split('\u{1e}') {
        let rec = rec.trim_start_matches('\n');
        if rec.trim().is_empty() {
            continue;
        }
        let mut f = rec.split('\u{1f}');
        let (Some(sha), Some(author), Some(ts), Some(subject)) =
            (f.next(), f.next(), f.next(), f.next())
        else {
            continue;
        };
        let body = f.next().unwrap_or("").trim_end().to_string();
        out.push(Commit {
            sha: sha.trim().to_string(),
            subject: subject.to_string(),
            body,
            author: author.to_string(),
            timestamp: ts.parse().unwrap_or(0),
            files_changed: 0,
            insertions: 0,
            deletions: 0,
        });
    }
    Ok(out)
}

/// Per-file counts for `base_commit..HEAD`.
///
/// `-z` because a path is bytes, not a line: one containing a newline or a quote
/// breaks the ordinary format, and `core.quotePath` mangles anything non-ASCII.
pub async fn numstat(repo: &Path, base_commit: &str) -> Result<Vec<FileChange>> {
    let raw = git_bytes(
        repo,
        &["diff", "--numstat", "-z", "--find-renames", base_commit, "HEAD"],
    )
    .await?;
    if !raw.ok {
        return Err(DomainError::OperationFailed);
    }
    Ok(parse_numstat_z(&raw.stdout))
}

/// `insertions \t deletions \t path NUL` — and for a rename, the path field is
/// replaced by TWO NUL-separated fields, old then new.
pub fn parse_numstat_z(bytes: &[u8]) -> Vec<FileChange> {
    let mut out = Vec::new();
    let mut fields = bytes.split(|b| *b == 0).map(|f| String::from_utf8_lossy(f).into_owned());

    while let Some(head) = fields.next() {
        if head.trim().is_empty() {
            continue;
        }
        let mut parts = head.splitn(3, '\t');
        let (Some(ins), Some(del)) = (parts.next(), parts.next()) else { continue };
        let binary = ins == "-" || del == "-";
        let insertions = ins.parse().unwrap_or(0);
        let deletions = del.parse().unwrap_or(0);

        // With `-z`, a non-rename keeps its path in the third tab field; a
        // rename leaves it EMPTY and puts old and new in the next two records.
        let third = parts.next().unwrap_or("").to_string();
        let (path, old_path, status) = if third.is_empty() {
            let old = fields.next().unwrap_or_default();
            let new = fields.next().unwrap_or_default();
            (new, Some(old), FileStatus::Renamed)
        } else {
            (third, None, FileStatus::Modified)
        };

        out.push(FileChange {
            path,
            status,
            old_path,
            insertions,
            deletions,
            binary,
            submodule: false,
        });
    }
    out
}

/// The working tree, split into the four groups a reviewer needs apart.
pub async fn working_tree(repo: &Path) -> Result<WorkingTree> {
    let raw =
        git_bytes(repo, &["status", "--porcelain=v2", "--untracked-files=all", "-z"]).await?;
    if !raw.ok {
        return Err(DomainError::OperationFailed);
    }
    Ok(parse_porcelain_v2_z(&raw.stdout))
}

/// Parse `git status --porcelain=v2 -z`.
///
/// Record kinds: `1` ordinary, `2` rename/copy (one extra NUL field for the old
/// path), `u` unmerged, `?` untracked, `!` ignored.
pub fn parse_porcelain_v2_z(bytes: &[u8]) -> WorkingTree {
    let mut wt = WorkingTree::default();
    let mut records = bytes
        .split(|b| *b == 0)
        .filter(|r| !r.is_empty())
        .map(|r| String::from_utf8_lossy(r).into_owned())
        .peekable();

    while let Some(rec) = records.next() {
        let mut it = rec.splitn(2, ' ');
        let kind = it.next().unwrap_or("");
        let rest = it.next().unwrap_or("");

        match kind {
            "?" => wt.untracked.push(rest.to_string()),
            "u" => {
                // `u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>`
                if let Some(path) = rest.split(' ').nth(9) {
                    wt.conflicted.push(path.to_string());
                }
            }
            "1" | "2" => {
                // `<XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>`; kind 2 adds a
                // score field before the path and the OLD path in the next
                // NUL-separated record.
                let f: Vec<&str> = rest.split(' ').collect();
                if f.len() < 8 {
                    continue;
                }
                let xy = f[0].as_bytes();
                let sub = f[1];
                let submodule = sub.starts_with('S');
                let path_index = if kind == "2" { 8 } else { 7 };
                let path = f.get(path_index).copied().unwrap_or("").to_string();
                let old_path =
                    if kind == "2" { records.next() } else { None };

                let staged_code = xy.first().copied().unwrap_or(b'.');
                let unstaged_code = xy.get(1).copied().unwrap_or(b'.');

                if staged_code != b'.' {
                    wt.staged.push(FileChange {
                        path: path.clone(),
                        status: FileStatus::from_xy(staged_code),
                        old_path: old_path.clone(),
                        insertions: 0,
                        deletions: 0,
                        binary: false,
                        submodule,
                    });
                }
                if unstaged_code != b'.' {
                    wt.unstaged.push(FileChange {
                        path,
                        status: FileStatus::from_xy(unstaged_code),
                        old_path,
                        insertions: 0,
                        deletions: 0,
                        binary: false,
                        submodule,
                    });
                }
            }
            _ => {}
        }
    }
    wt
}

/// A digest that moves whenever anything a reviewer could have commented on
/// moved. See the module note for why it reads file contents.
pub async fn worktree_digest(repo: &Path, head_commit: &str) -> Result<String> {
    let wt = working_tree(repo).await?;

    let mut paths: Vec<(&str, &'static str)> = Vec::new();
    for f in &wt.staged {
        paths.push((&f.path, "staged"));
    }
    for f in &wt.unstaged {
        paths.push((&f.path, "unstaged"));
    }
    for p in &wt.untracked {
        paths.push((p, "untracked"));
    }
    for p in &wt.conflicted {
        paths.push((p, "conflicted"));
    }
    // Sorted, so that git's ordering never changes the answer on its own.
    paths.sort();

    let mut h = Sha256::new();
    h.update(head_commit.as_bytes());
    h.update([0]);
    for (path, status) in paths {
        h.update(path.as_bytes());
        h.update([0]);
        h.update(status.as_bytes());
        h.update([0]);
        // Content, not mtime. A checkout that restores identical bytes must not
        // mark a whole review stale, and a second edit to an already-dirty file
        // must not go unnoticed.
        match tokio::fs::read(repo.join(path)).await {
            Ok(bytes) => h.update(Sha256::digest(&bytes)),
            // Deleted between the status call and now. Its absence is itself a
            // fact worth hashing, so the digest still moves.
            Err(_) => h.update(b"absent"),
        };
        h.update([0]);
    }
    Ok(format!("{:x}", h.finalize()))
}

/// The whole summary for one branch.
pub async fn change_set(
    repo: &Path,
    branch: &str,
    base_ref: &str,
    base_source: BaseSource,
) -> Result<ChangeSet> {
    let head = git(repo, &["rev-parse", "HEAD"]).await?;
    if !head.ok {
        return Err(DomainError::OperationFailed);
    }
    let head_commit = head.stdout.trim().to_string();

    let base_commit = merge_base(repo, base_ref).await?;
    let commits = commits_since(repo, &base_commit).await?;
    let files = numstat(repo, &base_commit).await?;
    let working_tree = working_tree(repo).await?;
    let worktree_digest = worktree_digest(repo, &head_commit).await?;

    let insertions = files.iter().map(|f| f.insertions).sum();
    let deletions = files.iter().map(|f| f.deletions).sum();

    Ok(ChangeSet {
        branch: branch.to_string(),
        base_ref: base_ref.to_string(),
        base_source,
        base_commit,
        head_commit,
        commits,
        working_tree,
        files,
        insertions,
        deletions,
        worktree_digest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_numstat_record_carries_its_counts_and_path() {
        let raw = b"3\t1\tsrc/main.rs\0";
        let v = parse_numstat_z(raw);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].path, "src/main.rs");
        assert_eq!((v[0].insertions, v[0].deletions), (3, 1));
        assert!(!v[0].binary);
    }

    #[test]
    fn a_binary_file_reports_dashes_not_zero_counts() {
        // Zero-and-zero would be indistinguishable from an empty text change.
        let raw = b"-\t-\tassets/icon.png\0";
        let v = parse_numstat_z(raw);
        assert!(v[0].binary);
    }

    #[test]
    fn a_rename_takes_its_two_paths_from_the_following_records() {
        let raw = b"1\t1\t\0old/name.rs\0new/name.rs\0";
        let v = parse_numstat_z(raw);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].status, FileStatus::Renamed);
        assert_eq!(v[0].old_path.as_deref(), Some("old/name.rs"));
        assert_eq!(v[0].path, "new/name.rs");
    }

    #[test]
    fn a_path_containing_a_newline_survives_because_records_are_nul_separated() {
        let raw = b"1\t0\tweird\nname.rs\0";
        let v = parse_numstat_z(raw);
        assert_eq!(v.len(), 1, "a newline in a path is not a record boundary");
        assert_eq!(v[0].path, "weird\nname.rs");
    }

    #[test]
    fn a_staged_and_unstaged_file_appears_in_both_groups() {
        // XY = "MM": modified in the index AND modified again in the worktree.
        // A reviewer needs those apart, because only one of them is going to be
        // in the next commit.
        let raw = b"1 MM N... 100644 100644 100644 aaa bbb src/x.rs\0";
        let wt = parse_porcelain_v2_z(raw);
        assert_eq!(wt.staged.len(), 1);
        assert_eq!(wt.unstaged.len(), 1);
        assert_eq!(wt.staged[0].path, "src/x.rs");
    }

    #[test]
    fn a_file_staged_only_does_not_appear_as_unstaged() {
        let raw = b"1 M. N... 100644 100644 100644 aaa bbb src/x.rs\0";
        let wt = parse_porcelain_v2_z(raw);
        assert_eq!(wt.staged.len(), 1);
        assert!(wt.unstaged.is_empty());
    }

    #[test]
    fn untracked_and_conflicted_are_their_own_groups() {
        let raw = b"? notes.txt\0u UU N... 100644 100644 100644 100644 aaa bbb ccc src/c.rs\0";
        let wt = parse_porcelain_v2_z(raw);
        assert_eq!(wt.untracked, vec!["notes.txt"]);
        assert_eq!(wt.conflicted, vec!["src/c.rs"]);
        assert!(wt.is_dirty());
    }

    #[test]
    fn a_renamed_entry_takes_its_old_path_from_the_next_record() {
        let raw = b"2 R. N... 100644 100644 100644 aaa bbb R100 new.rs\0old.rs\0";
        let wt = parse_porcelain_v2_z(raw);
        assert_eq!(wt.staged.len(), 1);
        assert_eq!(wt.staged[0].path, "new.rs");
        assert_eq!(wt.staged[0].old_path.as_deref(), Some("old.rs"));
    }

    #[test]
    fn a_submodule_entry_is_flagged_as_one() {
        let raw = b"1 .M SCMU 160000 160000 160000 aaa bbb vendor/lib\0";
        let wt = parse_porcelain_v2_z(raw);
        assert!(wt.unstaged[0].submodule);
    }

    #[test]
    fn a_clean_tree_is_not_dirty() {
        assert!(!parse_porcelain_v2_z(b"").is_dirty());
    }
}
