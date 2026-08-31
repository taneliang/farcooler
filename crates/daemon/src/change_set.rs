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

/// The four groups a reviewer needs apart, and what each one's counts mean.
///
/// `staged` is the index against HEAD and `unstaged` is the worktree against the
/// index — the two comparisons `DiffSelector::Staged` and `Unstaged` draw, so a
/// group's `+N -M` and the patch a client opens from it are the same diff.
/// `untracked` carries records rather than bare paths because a file git has
/// never seen still has a size, and it is the half `git diff` cannot report at
/// all: see `untracked_counts`.
///
/// Counts are zero until `apply_uncommitted_counts` fills them; `working_tree`
/// alone reads `git status`, which reports status and no numbers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkingTree {
    pub staged: Vec<FileChange>,
    pub unstaged: Vec<FileChange>,
    pub untracked: Vec<FileChange>,
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
    /// The branch's pull request says what it is based on. Exact, and the only
    /// source that is right for a stacked branch.
    PrBase,
    /// The repository's own default branch.
    DefaultBranch,
    /// Nothing knew. A local `main` or `master`, and labeled as a guess because
    /// it is the only one of these that can quietly be wrong.
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

/// The commits this branch made, oldest first, each with its own counts.
///
/// TWO dots against the resolved merge base. See the module note.
///
/// ## Where a commit's counts come from
///
/// `--shortstat` on the SAME `git log`, not a diff per commit.
///
/// The three count fields were hardcoded zeroes for as long as this function
/// existed, and both clients worked around it the same way — summing the file
/// list of whichever commit the reader had SELECTED — so a history row could
/// not say `+12 -4` until it was opened, and `ChangeCommit`'s three fields
/// crossed the wire as decoration.
///
/// One invocation rather than one per commit, because this is a recompute path:
/// `change_set` runs behind the diff pane's three-second poll and behind the
/// sidebar's inbox refresh. Both are cheap-gated in `review::ChangeSets::get`,
/// but every real edit gets through the gate, so the per-recompute cost is the
/// one that matters. Measured warm on this repository: a 30-commit range goes
/// from 4 ms to 40 ms, a 200-commit one from 10 ms to 200 ms — about a
/// millisecond of tree diff per commit, against roughly 20 ms EACH for the two
/// `git diff --numstat` calls `numstat()` already makes over the same range. A
/// `git diff` per commit would instead be N processes and N repository opens per
/// recompute, which on a 200-commit branch is not a cost this poll can carry.
///
/// `--diff-merges=first-parent` because `git log` prints NO stat for a merge by
/// default, and a merge whose row reads 0/0 is exactly the silence this replaces.
/// First parent is what `DiffSelector` documents and what `file_diff` computes
/// for a single commit, so a merge's row and its file list agree; `git show`'s
/// combined diff would agree with neither. That option is git 2.31 (2021), and a
/// runner whose distribution still packages an older git would fail the ENTIRE
/// call — no commits, no change set, a review pane with nothing in it. So a
/// rejected invocation is retried once without it: on such a runner merges keep
/// the zeroes they have today and every other commit gains real counts.
///
/// A root commit needs no `EMPTY_TREE` here, unlike `file_diff::commit_files`:
/// `git log` already diffs a parentless commit against the empty tree, so the
/// first commit of an orphan branch reports everything it added rather than
/// nothing.
/// When each commit on this worktree's HEAD landed, out of the reflog.
///
/// The activity trace wants commit marks on its axis, per bucket, going back a
/// day — and there is no periodic source for that. `commits_since` is the only
/// thing that produces timestamped commits and it runs one `git log` per call,
/// on the demand-driven `ReviewCache::get` path only; nothing recomputes it on
/// a clock, so a trace built from it would be blank for every workspace nobody
/// had opened a diff for.
///
/// `<git dir>/logs/HEAD` is the same fact for free. git appends one line to it
/// every time HEAD moves, each line carrying the author time in Unix seconds
/// and what moved it, so a commit landing is a few bytes appended to a file the
/// daemon can read without spawning anything. A linked worktree keeps its own,
/// which is why the caller passes a git dir resolved through
/// `review::git_dir` rather than `<worktree>/.git`.
///
/// **Only lines whose action is a commit count.** A reflog records checkouts,
/// merges, resets, rebases and pulls in the same file, and every one of them
/// moves HEAD without anybody having written code. `commit`, `commit (initial)`
/// and `commit (amend)` are the three git writes for "a commit was made here";
/// an amend counts, because the work it carries is work that landed.
///
/// **A repository with reflogs turned off has no commit marks**, and that is a
/// silent zero rather than an error: `core.logAllRefUpdates` defaults on for
/// any non-bare repository, so this is a deliberate local setting, and the
/// honest reading of "git kept no record" is no mark rather than a guess.
pub fn reflog_commits(text: &str) -> Vec<i64> {
    text.lines()
        .filter_map(|line| {
            // `<old> <new> <name> <email> <seconds> <tz>\t<action>: <message>`.
            // The tab is the only delimiter git guarantees, because a
            // committer's name may contain anything at all.
            let (left, right) = line.split_once('\t')?;
            let action = right.split(':').next()?.trim();
            if action != "commit" && !action.starts_with("commit (") {
                return None;
            }
            // From the RIGHT: the timezone is last and the epoch seconds are
            // the field before it. Counting from the left would depend on how
            // many words are in the committer's name.
            let mut fields = left.split_whitespace().rev();
            fields.next()?;
            fields.next()?.parse::<i64>().ok()
        })
        .collect()
}

pub async fn commits_since(repo: &Path, base_commit: &str) -> Result<Vec<Commit>> {
    // A record separator that cannot occur in a commit message, and a field
    // separator likewise. %x1e / %x1f are the ASCII record and unit separators,
    // which is what they are for.
    //
    // The record separator LEADS the record and a unit separator TRAILS the
    // body, which is what makes room for the stat block. git prints the stat
    // AFTER the format text, so with %x1e at the END of the format — where it
    // used to be — commit N's diffstat would land at the head of commit N+1's
    // record, in front of its sha, and `f.next()` would hand back a sha with
    // " 3 files changed, 12 insertions(+)" glued to it. Leading the record puts
    // the stat where it belongs: a sixth field, after the body.
    const FORMAT: &str = "--format=%x1e%H%x1f%an%x1f%at%x1f%s%x1f%b%x1f";
    // Explicit rather than relying on `diff.renames`, which defaults to true
    // since git 2.9 but is a repository's to turn off. `file_diff` asks for
    // rename detection on the same commits; a row that disagreed with its own
    // file list would be worse than either number alone.
    const RENAMES: &str = "--find-renames";
    const FIRST_PARENT: &str = "--diff-merges=first-parent";

    let range = format!("{base_commit}..HEAD");
    let mut args = vec![
        "log", "--reverse", "--no-color", "--shortstat", RENAMES, FIRST_PARENT, FORMAT, &range,
    ];
    let mut r = git(repo, &args).await?;
    if !r.ok {
        // Either the range is bad — in which case the retry fails the same way
        // and costs one process on an already-failing path — or this git is
        // older than `--diff-merges`. See the note above.
        args.retain(|a| *a != FIRST_PARENT);
        r = git(repo, &args).await?;
    }
    if !r.ok {
        return Err(DomainError::OperationFailed);
    }

    let mut out = Vec::new();
    for rec in r.stdout.split('\u{1e}') {
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
        // Absent for an empty commit, and for a merge on a git too old for
        // `--diff-merges`. Zeroes are the honest answer to both.
        let (files_changed, insertions, deletions) = parse_shortstat(f.next().unwrap_or(""));
        out.push(Commit {
            sha: sha.trim().to_string(),
            subject: subject.to_string(),
            body,
            author: author.to_string(),
            timestamp: ts.parse().unwrap_or(0),
            files_changed,
            insertions,
            deletions,
        });
    }
    Ok(out)
}

/// ` 3 files changed, 12 insertions(+), 4 deletions(-)` — the three numbers.
///
/// Word-scanned and keyed on the word AFTER each number rather than on
/// position, because git omits whichever clauses are zero: a commit that only
/// adds lines prints no deletions clause at all, and a two-number line read
/// positionally would report its insertions as deletions. Prefix-matched
/// because the singular forms are real output — git writes `1 file changed, 1
/// insertion(+)` — and this exact text is the one place git's own English is
/// load-bearing here; everything else in this module reads plumbing.
fn parse_shortstat(text: &str) -> (u32, u32, u32) {
    let mut files = 0;
    let mut ins = 0;
    let mut del = 0;
    let words: Vec<&str> = text.split_whitespace().collect();
    for (i, w) in words.iter().enumerate() {
        let n: u32 = match w.parse() {
            Ok(n) => n,
            Err(_) => continue,
        };
        match words.get(i + 1) {
            Some(next) if next.starts_with("file") => files = n,
            Some(next) if next.starts_with("insertion") => ins = n,
            Some(next) if next.starts_with("deletion") => del = n,
            _ => {}
        }
    }
    (files, ins, del)
}

/// Per-file counts for `base_commit..HEAD`.
///
/// `-z` because a path is bytes, not a line: one containing a newline or a quote
/// breaks the ordinary format, and `core.quotePath` mangles anything non-ASCII.
pub async fn numstat(repo: &Path, base_commit: &str) -> Result<Vec<FileChange>> {
    let counts = git_bytes(
        repo,
        &["diff", "--numstat", "-z", "--find-renames", base_commit, "HEAD"],
    )
    .await?;
    let names = git_bytes(
        repo,
        &["diff", "--name-status", "-z", "--find-renames", base_commit, "HEAD"],
    )
    .await?;
    if !counts.ok || !names.ok {
        return Err(DomainError::OperationFailed);
    }
    let mut files = parse_numstat_z(&counts.stdout);
    apply_name_status_z(&mut files, &names.stdout);
    Ok(files)
}

/// Add the exact kind of change to the count records. `--numstat` tells us
/// lines and renames, but calls every other path merely modified; the adjacent
/// `--name-status` read distinguishes adds, deletes, copies, and type changes.
/// Both use NUL records, so paths with tabs, newlines, and non-ASCII survive.
pub fn apply_name_status_z(files: &mut [FileChange], bytes: &[u8]) {
    let mut fields = bytes
        .split(|b| *b == 0)
        .filter(|field| !field.is_empty())
        .map(|field| String::from_utf8_lossy(field).into_owned());

    while let Some(token) = fields.next() {
        let code = token.as_bytes().first().copied().unwrap_or(b'M');
        let status = FileStatus::from_xy(code);
        let (old_path, path) = if matches!(code, b'R' | b'C') {
            (fields.next(), fields.next().unwrap_or_default())
        } else {
            (None, fields.next().unwrap_or_default())
        };
        if let Some(file) = files.iter_mut().find(|file| file.path == path) {
            file.status = status;
            if old_path.is_some() { file.old_path = old_path; }
        }
    }
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
///
/// Paths and statuses only. `--porcelain=v2` reports no line counts of any
/// kind, so every record comes back 0/0; `apply_uncommitted_counts` is the pass
/// that fills them, and it is deliberately not folded in here — this function is
/// also what `untracked_lines` and `worktree_digest` call, and neither of them
/// wants a diff run underneath it.
pub async fn working_tree(repo: &Path) -> Result<WorkingTree> {
    let raw =
        git_bytes(repo, &["status", "--porcelain=v2", "--untracked-files=all", "-z"]).await?;
    if !raw.ok {
        return Err(DomainError::OperationFailed);
    }
    Ok(parse_porcelain_v2_z(&raw.stdout))
}

/// Fill in the line counts `git status` cannot report.
///
/// The count fields on `WorkingTree`'s records were zero from the day they
/// existed and nothing ever filled them, so a client that wanted `+N -M` for
/// uncommitted work had to count the diff it had already drawn — which made the
/// total whatever the reader had scrolled into view, and made a scope switch or
/// a Refresh start it over from nearly nothing.
///
/// ## Three questions, and which field asks which
///
/// ```text
///   git diff --numstat --cached   index vs HEAD       what `staged` MEANS
///   git diff --numstat            worktree vs index   what `unstaged` MEANS
///   git diff --numstat HEAD       worktree vs HEAD    both at once
/// ```
///
/// Each group gets the diff its own `DiffSelector` draws, so a row's counts and
/// the patch opened from that row are the same comparison. The third form is
/// what an "uncommitted" total wants — it is what `Selector::Local` shows — and
/// it is deliberately NOT what either field carries: written onto the staged
/// record it would report, under the word staged, work that is not staged.
///
/// A client after the uncommitted total adds the two groups. That is exact for
/// every file in one group or the other, which is every file an agent produces —
/// agents commit, they do not stage. It is an upper bound for the one shape that
/// is neither: a file staged and then edited AGAIN over the same lines, where
/// `git diff HEAD` counts that line once and the two groups each count it. The
/// alternative is a third `git diff` per recompute to answer a question two
/// existing reads already nearly answer, on a path that runs seven git commands
/// before it gets here.
///
/// ## Cost
///
/// Two more `git diff`s, and only on `change_set` — the per-workspace review
/// path behind `ReviewCache::get`, which runs it when the cheap gate or the
/// worktree digest says the tree actually moved. `shortstat`, the call the fleet
/// sampler puts on every workspace, does not come through here and is unchanged.
/// Both are skipped outright when their group is empty, so a clean tree pays
/// nothing. Measured warm on a clone of this repository with one staged file,
/// five unstaged and one untracked: 16 ms for the whole pass, against 155 ms for
/// the `change_set` around it over a 30-commit range and 390 ms over a
/// 200-commit one. The reads it adds are bounded by the size of the dirty work,
/// not by the size of the branch, so it is the one part of this path that does
/// not grow with history.
///
/// Untracked files are the half `git diff` cannot see at all — `untracked_counts`
/// reads them, and says there why `git add -N` is not an option.
pub async fn apply_uncommitted_counts(repo: &Path, wt: &mut WorkingTree) -> Result<()> {
    if !wt.staged.is_empty() {
        let r =
            git_bytes(repo, &["diff", "--numstat", "-z", "--find-renames", "--cached"]).await?;
        if !r.ok {
            return Err(DomainError::OperationFailed);
        }
        apply_numstat_counts_z(&mut wt.staged, &r.stdout);
    }
    if !wt.unstaged.is_empty() {
        let r = git_bytes(repo, &["diff", "--numstat", "-z", "--find-renames"]).await?;
        if !r.ok {
            return Err(DomainError::OperationFailed);
        }
        apply_numstat_counts_z(&mut wt.unstaged, &r.stdout);
    }
    for f in &mut wt.untracked {
        if let Some((lines, binary)) = untracked_counts(repo, &f.path).await {
            f.insertions = lines;
            f.binary = binary;
        }
    }
    Ok(())
}

/// Merge per-file counts onto records that arrived with only a status.
///
/// `apply_name_status_z` the other way round, and the same shape on purpose: a
/// second git read over the same paths, merged by path, leaving anything it does
/// not mention alone. `git status` names every dirty path and gives no numbers;
/// `git diff --numstat` gives numbers for a subset of them.
///
/// Merging by path is what keeps a conflict out of this. `git status` gives an
/// unmerged path its own group, and `git diff --numstat` prints TWO records for
/// one — a conflicted file's counts are the merge's question, not this one — so
/// those records find nothing here and are dropped rather than written onto some
/// other file. A record this group holds and the diff never mentions keeps its
/// zeroes, which is the honest answer rather than a gap.
pub fn apply_numstat_counts_z(files: &mut [FileChange], bytes: &[u8]) {
    for counted in parse_numstat_z(bytes) {
        if let Some(file) = files.iter_mut().find(|file| file.path == counted.path) {
            file.insertions = counted.insertions;
            file.deletions = counted.deletions;
            file.binary = counted.binary;
        }
    }
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
            "?" => wt.untracked.push(FileChange {
                path: rest.to_string(),
                status: FileStatus::Untracked,
                old_path: None,
                insertions: 0,
                deletions: 0,
                binary: false,
                submodule: false,
            }),
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
    for f in &wt.untracked {
        paths.push((&f.path, "untracked"));
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

/// What this branch is compared against, when nobody has said.
///
/// A hardcoded `main` was the first version and it was wrong in a way that
/// produced SILENCE rather than an error: a repository on `master`, or one whose
/// default branch is anything else, resolved no merge base, so every review of
/// it showed an empty diff and said nothing about why.
///
/// The repository's default branch, WITHOUT a network call.
///
/// `origin/HEAD` is the local record of what the remote calls its default, so it
/// is right whenever the clone has ever been told — and costs nothing.
pub async fn default_branch_local(repo: &Path) -> Option<String> {
    if let Ok(r) = git(repo, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]).await {
        if r.ok {
            let full = r.stdout.trim();
            if !full.is_empty() {
                // `origin/main` names a ref that resolves; keep it whole rather
                // than stripping the remote, so a repository with a local branch
                // of the same name but different tip still diffs against what the
                // remote actually has.
                return Some(full.to_string());
            }
        }
    }
    None
}

/// `origin/<name>` when that remote-tracking ref exists, and `<name>` when it
/// does not.
///
/// A default branch arrives here by two roads and only one of them was
/// remote-qualified. `default_branch_local` above reads `origin/HEAD` and hands
/// back `origin/main` whole, for the reason its comment gives. `gh repo view
/// --json defaultBranchRef` (`stack.rs:405`) answers with a BARE `main`, and
/// `Service::default_branch` (`service.rs:458`) prefers it — so on every runner
/// with a working `gh`, which is nearly all of them, the base was the LOCAL
/// `main`. A worktree branched off an up-to-date `origin/main` while the local
/// `main` sat where it was last pulled then took its merge base at that stale
/// tip, and every commit `origin/main` had gained since was counted as the
/// branch's own — commits, files and +/- all inflated, and nothing in the diff
/// looking wrong.
///
/// So both roads end here, at one call site in `Service::default_branch`, rather
/// than each qualifying its own answer and drifting apart again.
///
/// No network: `rev-parse` reads refs this clone already has. `resolve_base`
/// (`review_ops.rs:160`) forbids a fetch on the path of drawing a diff, and this
/// keeps that true. A name that is already remote-qualified passes through
/// untouched, because `refs/remotes/origin/origin/main` is not a ref.
///
/// The bare name is the right answer, not a failure, for a repository with no
/// `origin/<name>` to name — one that was `git init`ed and never pushed, or one
/// whose remote is called something else — so it is returned rather than
/// refused. Qualifying blindly there would hand `merge_base` a ref that does not
/// exist, and a diff that fails to resolve shows nothing at all, which is the
/// louder bug of the two.
pub async fn remote_qualified(repo: &Path, name: &str) -> String {
    let qualified = format!("origin/{name}");
    let full = format!("refs/remotes/{qualified}");
    if let Ok(r) = git(repo, &["rev-parse", "--verify", "--quiet", &full]).await {
        if r.ok {
            return qualified;
        }
    }
    name.to_string()
}

/// The last resort: a local `main` or `master`.
///
/// Returns `None` when neither exists, so the caller can ask the user to pick a
/// base rather than diffing against a ref that is not there — which produced an
/// empty diff and no explanation.
pub async fn guess_base(repo: &Path) -> Option<String> {
    if let Some(d) = default_branch_local(repo).await {
        return Some(d);
    }
    for candidate in ["main", "master"] {
        if let Ok(r) = git(repo, &["rev-parse", "--verify", "--quiet", candidate]).await {
            if r.ok {
                return Some(candidate.to_string());
            }
        }
    }
    None
}

/// The branch currently checked out, or `None` on a detached HEAD.
pub async fn current_branch(repo: &Path) -> Option<String> {
    let r = git(repo, &["symbolic-ref", "--short", "--quiet", "HEAD"]).await.ok()?;
    if !r.ok {
        return None;
    }
    let b = r.stdout.trim().to_string();
    if b.is_empty() { None } else { Some(b) }
}

/// Just the three numbers, for a sidebar row.
///
/// `--shortstat` rather than the full change set: a fleet-wide glance needs
/// files-changed and +/- for every worktree, and computing a whole change set
/// per row would put a `git log` and a `git status` on a timer across the fleet.
///
/// ## Against the working tree, not against `HEAD`
///
/// `git diff <base> HEAD` answers "what has been COMMITTED", which is why the
/// sidebar's `+N -M` used to sit still for the twenty minutes an agent spent
/// editing and then jump when it committed. Dropping `HEAD` compares the base
/// against what is on disk, so committed and uncommitted work arrive together —
/// out of the same single call, at the same cost. A file that has been staged
/// but not committed is in the index and therefore already in this answer;
/// nothing is counted twice.
///
/// ## Why there is a second call
///
/// `git diff` reads the index, and a file an agent has just CREATED is not in
/// it. Agents create files constantly, so a number that climbed while one
/// edited an existing file and sat still while it wrote a new one would be a
/// worse lie than the honest committed-only number it replaces — it LOOKS live.
/// `untracked_lines` is the other half. It is the expensive call `watch.rs` is
/// built to avoid, which is why nothing here runs on a timer: see
/// `Watcher::probe_change_sets`, which spends this only on a worktree something
/// free has already said may have moved.
pub async fn shortstat(repo: &Path, base_ref: &str) -> Result<(u32, u32, u32)> {
    let base = merge_base(repo, base_ref).await?;
    let r = git(repo, &["diff", "--shortstat", &base]).await?;
    if !r.ok {
        return Err(DomainError::OperationFailed);
    }
    // The same line `commits_since` reads per commit, and the same parse — see
    // `parse_shortstat`.
    let (files, insertions, deletions) = parse_shortstat(&r.stdout);
    let (new_files, new_lines) = untracked_lines(repo).await?;
    Ok((files + new_files, insertions + new_lines, deletions))
}

/// The files git has never heard of, and the lines in them.
///
/// The aggregate `shortstat` adds to its diff; `untracked_counts` is the same
/// read per file, and `apply_uncommitted_counts` spends it to give each record
/// its own number.
async fn untracked_lines(repo: &Path) -> Result<(u32, u32)> {
    let untracked = working_tree(repo).await?.untracked;

    let mut files = 0;
    let mut lines = 0;
    for f in &untracked {
        let Some((n, _binary)) = untracked_counts(repo, &f.path).await else { continue };
        files += 1;
        lines += n;
    }
    Ok((files, lines))
}

/// One untracked file's counts: the lines git would call insertions if it knew
/// about the file, and whether git would call it binary.
///
/// The half `git diff` cannot report, counted by reading the file rather than
/// made visible to diff with `git add -N`. That would write the index —
/// fighting whatever git the agent is running itself, and moving the index
/// mtime that `review::cheap_gate` reads, which is the gate every cheap thing in
/// this product is built on.
///
/// Reading each new file is the cost `worktree_digest` beside this already pays
/// over the same list, and it is bounded by the same thing: what git calls
/// untracked, which excludes everything `.gitignore` covers. So this is the size
/// of the work in progress, not the size of the repository.
///
/// `None` for a file created and deleted again between the status call and this
/// read. Not an error, and not a file to count.
async fn untracked_counts(repo: &Path, path: &str) -> Option<(u32, bool)> {
    let bytes = tokio::fs::read(repo.join(path)).await.ok()?;
    // git prints `-` rather than a count for a file it calls binary, and those
    // lines appear in no total it reports. Same test it uses: a NUL byte in the
    // first 8000.
    let binary = bytes.iter().take(8000).any(|b| *b == 0);
    Some((if binary { 0 } else { new_file_lines(&bytes) }, binary))
}

/// Lines in a file, counted the way git counts them when the whole file is an
/// addition.
///
/// A final line with no newline after it still counts — git reports it as an
/// insertion and marks the hunk `\ No newline at end of file`. An empty file is
/// zero, which is also what git says.
fn new_file_lines(bytes: &[u8]) -> u32 {
    if bytes.is_empty() {
        return 0;
    }
    let newlines = bytes.iter().filter(|b| **b == b'\n').count();
    let unterminated = usize::from(bytes.last() != Some(&b'\n'));
    (newlines + unterminated) as u32
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
    // Status first, then the counts `git status` has none of. Two reads for one
    // answer, the same way `numstat` above is two.
    let mut working_tree = working_tree(repo).await?;
    apply_uncommitted_counts(repo, &mut working_tree).await?;
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
    fn name_status_distinguishes_adds_deletes_and_type_changes() {
        let mut files = parse_numstat_z(
            b"1\t0\tadded.rs\x000\t2\tdeleted.rs\x000\t0\ttype.rs\0",
        );
        apply_name_status_z(
            &mut files,
            b"A\0added.rs\0D\0deleted.rs\0T\0type.rs\0",
        );
        assert_eq!(files[0].status, FileStatus::Added);
        assert_eq!(files[1].status, FileStatus::Deleted);
        assert_eq!(files[2].status, FileStatus::TypeChanged);
    }

    #[test]
    fn name_status_keeps_both_sides_of_a_rename() {
        let mut files = parse_numstat_z(b"1\t1\t\0before.rs\0after.rs\0");
        apply_name_status_z(&mut files, b"R100\0before.rs\0after.rs\0");
        assert_eq!(files[0].status, FileStatus::Renamed);
        assert_eq!(files[0].old_path.as_deref(), Some("before.rs"));
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
        assert_eq!(wt.untracked.len(), 1);
        assert_eq!(wt.untracked[0].path, "notes.txt");
        assert_eq!(wt.untracked[0].status, FileStatus::Untracked);
        assert_eq!(wt.conflicted, vec!["src/c.rs"]);
        assert!(wt.is_dirty());
    }

    #[test]
    fn a_status_record_arrives_with_no_counts_at_all() {
        // The whole reason `apply_uncommitted_counts` exists: `--porcelain=v2`
        // reports what changed and never how much.
        let raw = b"1 .M N... 100644 100644 100644 aaa bbb src/x.rs\0? notes.txt\0";
        let wt = parse_porcelain_v2_z(raw);
        assert_eq!((wt.unstaged[0].insertions, wt.unstaged[0].deletions), (0, 0));
        assert_eq!((wt.untracked[0].insertions, wt.untracked[0].deletions), (0, 0));
    }

    #[test]
    fn numstat_counts_merge_onto_status_records_by_path() {
        let mut wt = parse_porcelain_v2_z(
            b"1 .M N... 100644 100644 100644 aaa bbb src/x.rs\0\
              1 .M N... 100644 100644 100644 aaa bbb src/y.rs\0",
        );
        apply_numstat_counts_z(&mut wt.unstaged, b"3\t1\tsrc/x.rs\0-\t-\tsrc/y.rs\0");
        assert_eq!((wt.unstaged[0].insertions, wt.unstaged[0].deletions), (3, 1));
        assert!(!wt.unstaged[0].binary);
        assert!(wt.unstaged[1].binary, "git says `-` for a file it will not diff");
    }

    #[test]
    fn a_record_the_diff_never_mentions_keeps_its_zeroes() {
        let mut wt =
            parse_porcelain_v2_z(b"1 .M N... 100644 100644 100755 aaa bbb run.sh\0");
        apply_numstat_counts_z(&mut wt.unstaged, b"");
        assert_eq!((wt.unstaged[0].insertions, wt.unstaged[0].deletions), (0, 0));
        assert_eq!(wt.unstaged[0].status, FileStatus::Modified, "status survives");
    }

    #[test]
    fn counts_for_a_path_this_group_does_not_hold_go_nowhere() {
        // What an unmerged path looks like from here: `git status` gives it its
        // own group, and `git diff --numstat` prints TWO records for it. Merging
        // by path is what keeps a conflict's numbers from landing on some other
        // file in this list.
        let mut wt = parse_porcelain_v2_z(b"1 .M N... 100644 100644 100644 aaa bbb src/x.rs\0");
        apply_numstat_counts_z(
            &mut wt.unstaged,
            b"0\t0\tf.txt\x006\t0\tf.txt\x003\t1\tsrc/x.rs\0",
        );
        assert_eq!(wt.unstaged.len(), 1, "a merge adds nothing");
        assert_eq!((wt.unstaged[0].insertions, wt.unstaged[0].deletions), (3, 1));
    }

    #[test]
    fn merging_counts_leaves_a_records_status_and_old_path_alone() {
        let mut wt = parse_porcelain_v2_z(
            b"2 R. N... 100644 100644 100644 aaa bbb R100 new.rs\0old.rs\0",
        );
        apply_numstat_counts_z(&mut wt.staged, b"1\t1\t\0old.rs\0new.rs\0");
        assert_eq!(wt.staged[0].status, FileStatus::Renamed);
        assert_eq!(wt.staged[0].old_path.as_deref(), Some("old.rs"));
        assert_eq!((wt.staged[0].insertions, wt.staged[0].deletions), (1, 1));
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
    fn a_shortstat_line_gives_up_all_three_numbers() {
        assert_eq!(
            parse_shortstat(" 3 files changed, 12 insertions(+), 4 deletions(-)\n"),
            (3, 12, 4)
        );
    }

    #[test]
    fn a_shortstat_clause_that_is_zero_is_absent_rather_than_written() {
        // git prints no deletions clause at all here. Read positionally, the 1
        // insertion would land in `deletions` and the row would say `-1`.
        assert_eq!(parse_shortstat(" 1 file changed, 1 insertion(+)\n"), (1, 1, 0));
        assert_eq!(parse_shortstat(" 2 files changed, 5 deletions(-)\n"), (2, 0, 5));
    }

    #[test]
    fn a_commit_with_no_diff_at_all_reads_as_zeroes() {
        // What an empty commit's stat field holds, and what a merge's holds on a
        // git too old for `--diff-merges`.
        assert_eq!(parse_shortstat("\n\n"), (0, 0, 0));
    }

    #[test]
    fn a_clean_tree_is_not_dirty() {
        assert!(!parse_porcelain_v2_z(b"").is_dirty());
    }
}
