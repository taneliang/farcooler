//! Review, on the daemon side.
//!
//! Holds four things the store and the review core deliberately do not:
//!
//! - **A change-set cache**, and its invalidation. `watch.rs` samples tmux panes
//!   for agent activity and observes neither file content nor ref movement, so
//!   this brings its own: a two-syscall gate on every tick, precise invalidation
//!   when an agent writes through the daemon's own filesystem service, and an
//!   explicit Refresh because no watcher is perfect.
//! - **Prompt composition**, including the numbering that makes answers
//!   correlatable when they come back from a terminal that was already busy.
//! - **Attachment blobs**, beside the database rather than in it.
//! - **The inbox**, which must never fan out into one `git status` per workspace.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use farcooler_core::{DomainError, Result};
use farcooler_review::anchor::{Anchor, CaptureManifest, Current, content_hash, resolve};
use farcooler_review::limits;
use farcooler_store::review::Disposition;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::change_set::{BaseSource, ChangeSet, change_set, worktree_digest};

pub fn now_millis() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// A cached change set plus the cheap facts that decide whether it is still true.
#[derive(Debug, Clone)]
pub struct CachedChangeSet {
    pub set: ChangeSet,
    pub version: u64,
    /// mtime of `HEAD` and of the index, in nanoseconds. The two-syscall gate:
    /// a commit, a rebase, a checkout or a `git add` all move one of them, and
    /// checking them costs nothing on a quiet fleet.
    pub gate: (u128, u128),
    pub computed_at: i64,
}

/// The change-set cache.
///
/// Keyed by workspace and branch, because a workspace can be asked about a
/// stack link it does not have checked out.
#[derive(Default)]
pub struct ReviewCache {
    entries: Mutex<HashMap<(Uuid, String), CachedChangeSet>>,
    next_version: Mutex<u64>,
}

/// mtimes of the two files that move whenever git does something structural.
///
/// Deliberately NOT a `git status`: this runs on every tick for every workspace
/// with a review surface open, and porcelain output is the expensive call. This
/// is two `stat`s, and it is only a gate — anything it lets through is verified
/// by the real computation behind it.
pub fn cheap_gate(worktree: &Path) -> (u128, u128) {
    fn mtime(p: PathBuf) -> u128 {
        std::fs::metadata(&p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }
    // In a linked worktree `.git` is a FILE pointing at the real directory, so
    // HEAD and index live beside it there rather than here. Reading the pointer
    // is one open of a very small file, and only on the miss path.
    let dot_git = worktree.join(".git");
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        std::fs::read_to_string(&dot_git)
            .ok()
            .and_then(|s| s.strip_prefix("gitdir: ").map(|p| PathBuf::from(p.trim())))
            .unwrap_or(dot_git)
    };
    (mtime(git_dir.join("HEAD")), mtime(git_dir.join("index")))
}

impl ReviewCache {
    pub fn new() -> Self {
        Self::default()
    }

    fn bump(&self) -> u64 {
        let mut v = self.next_version.lock().unwrap_or_else(|e| e.into_inner());
        *v += 1;
        *v
    }

    /// Drop a workspace's cached sets.
    ///
    /// Called when an agent writes a file through the daemon's own filesystem
    /// service — precise, immediate, and it covers the common case, because the
    /// agents doing the work are ACP clients of this daemon.
    pub fn invalidate(&self, workspace_id: Uuid) {
        let mut e = self.entries.lock().unwrap_or_else(|x| x.into_inner());
        e.retain(|(ws, _), _| *ws != workspace_id);
    }

    /// The change set, computed only if something moved.
    pub async fn get(
        &self,
        workspace_id: Uuid,
        worktree: &Path,
        branch: &str,
        base_ref: &str,
        base_source: BaseSource,
        fresh: bool,
    ) -> Result<CachedChangeSet> {
        let key = (workspace_id, branch.to_string());
        let gate = cheap_gate(worktree);

        if !fresh {
            let cached = {
                let e = self.entries.lock().unwrap_or_else(|x| x.into_inner());
                e.get(&key).cloned()
            };
            if let Some(c) = cached {
                if c.gate == gate {
                    // The gate cannot see a file edited in place, so it is not
                    // trusted on its own for long: the digest is recomputed and
                    // compared, which reads only the files git already calls
                    // dirty.
                    let digest = worktree_digest(worktree, &c.set.head_commit).await?;
                    if digest == c.set.worktree_digest {
                        return Ok(c);
                    }
                }
            }
        }

        let set = change_set(worktree, branch, base_ref, base_source).await?;
        let cached = CachedChangeSet {
            set,
            version: self.bump(),
            gate,
            computed_at: now_millis(),
        };
        let mut e = self.entries.lock().unwrap_or_else(|x| x.into_inner());
        e.insert(key, cached.clone());
        Ok(cached)
    }
}

/// Build the manifest an entry carries, so re-read detection survives a restart.
pub async fn capture_manifest(
    worktree: &Path,
    set: &ChangeSet,
    anchor: &Anchor,
) -> CaptureManifest {
    let mut file_content_hash = None;
    let mut file_snapshot = None;

    if let Some(path) = anchor.path() {
        if let Ok(bytes) = tokio::fs::read(worktree.join(path)).await {
            if let Ok(text) = String::from_utf8(bytes) {
                file_content_hash = Some(content_hash(&text));
                // Kept only when the file is DIRTY: for committed content git
                // already holds the prior version, and storing it twice would be
                // paying for something we can already read.
                let dirty = set.working_tree.staged.iter().any(|f| f.path == path)
                    || set.working_tree.unstaged.iter().any(|f| f.path == path)
                    || set.working_tree.untracked.iter().any(|p| p == path);
                if dirty && text.len() <= limits::MAX_SNAPSHOT_BYTES {
                    file_snapshot = Some(text);
                }
            }
        }
    }

    CaptureManifest {
        base_commit: set.base_commit.clone(),
        head_commit: set.head_commit.clone(),
        worktree_digest: set.worktree_digest.clone(),
        file_content_hash,
        file_snapshot,
    }
}

/// Resolve one entry's anchor against the worktree as it is now.
pub async fn resolve_entry(
    worktree: &Path,
    set: &ChangeSet,
    anchor: &Anchor,
    manifest: &CaptureManifest,
) -> farcooler_review::anchor::Resolution {
    let mut current = Current {
        head_commit: set.head_commit.clone(),
        worktree_digest: set.worktree_digest.clone(),
        ..Default::default()
    };
    if let Some(path) = anchor.path() {
        if let Ok(bytes) = tokio::fs::read(worktree.join(path)).await {
            if let Ok(text) = String::from_utf8(bytes) {
                current.file_content_hash = Some(content_hash(&text));
                current.file_content = Some(text);
            }
        }
    }
    resolve(anchor, manifest, &current)
}

/// One entry as it goes into a prompt.
pub struct PromptEntry {
    pub body: String,
    pub anchor: Anchor,
    /// The anchored lines, when there are any. Text rather than a line number,
    /// because text is what survives the agent's own edits and what an agent can
    /// act on without a stale coordinate.
    pub anchored_text: Option<String>,
}

/// Compose one prompt from many entries.
///
/// Numbered, and the agent is asked to number its answers back. That numbering
/// is the whole correlation mechanism: the target may be a terminal that was
/// already busy on something else, so the answer cannot be identified by its
/// position in the session.
pub fn compose_prompt(branch: &str, disposition: Disposition, entries: &[PromptEntry]) -> String {
    let n = entries.len();
    let noun = if n == 1 { "comment" } else { "comments" };
    let mut out = match disposition {
        Disposition::Ask => format!("Questions about {branch} — {n} {noun}.\n\n"),
        _ => format!("Review of {branch} — {n} {noun}.\n\n"),
    };

    for (i, e) in entries.iter().enumerate() {
        let num = i + 1;
        match (&e.anchor, &e.anchored_text) {
            (Anchor::None | Anchor::Workspace, _) => {
                out.push_str(&format!("[{num}] (no specific location)\n"));
            }
            (Anchor::Branch { branch }, _) => {
                out.push_str(&format!("[{num}] branch {branch}\n"));
            }
            (Anchor::Commit { sha }, _) => {
                let short = &sha[..sha.len().min(8)];
                out.push_str(&format!("[{num}] commit {short}\n"));
            }
            (a, text) => {
                let path = a.path().unwrap_or("");
                match text {
                    Some(t) if !t.is_empty() => {
                        out.push_str(&format!("[{num}] {path}, at these lines:\n"));
                        for line in t.lines() {
                            out.push_str(&format!("    | {line}\n"));
                        }
                    }
                    _ => out.push_str(&format!("[{num}] {path}\n")),
                }
            }
        }
        for line in e.body.lines() {
            out.push_str(&format!("    {line}\n"));
        }
        out.push('\n');
    }

    match disposition {
        Disposition::Ask => out.push_str(
            "Answer each one. Start every answer with its number in square brackets, \
             like [1], so the answers can be matched back to the questions. \
             Do not modify any files — this is a question, not a task.\n",
        ),
        _ => out.push_str(
            "Address all of these. If you disagree with one, say so and skip it \
             rather than doing it badly.\n",
        ),
    }
    out
}

/// Split a numbered answer back onto its entries.
///
/// Returns one string per entry index. `None` for an entry the answer did not
/// mention, which the caller records as uncorrelated rather than inventing a
/// match. An answer that numbers nothing yields all `None`, and the caller
/// attaches the whole turn to every entry, labelled — the user sees the reply
/// and knows the daemon is not vouching for which question it belongs to.
pub fn split_numbered_answer(text: &str, count: usize) -> Vec<Option<String>> {
    let mut out: Vec<Option<String>> = vec![None; count];
    let mut current: Option<usize> = None;
    let mut buf = String::new();

    let flush = |slot: &mut Vec<Option<String>>, idx: Option<usize>, buf: &mut String| {
        if let Some(i) = idx {
            let trimmed = buf.trim().to_string();
            if !trimmed.is_empty() && i < slot.len() {
                slot[i] = Some(trimmed);
            }
        }
        buf.clear();
    };

    for line in text.lines() {
        if let Some(idx) = leading_marker(line) {
            flush(&mut out, current, &mut buf);
            if idx >= 1 && idx <= count {
                current = Some(idx - 1);
                // Keep whatever followed the marker on the same line.
                if let Some(rest) = line.split_once(']') {
                    buf.push_str(rest.1.trim_start());
                    buf.push('\n');
                }
            } else {
                // A number outside the range. Not ours; do not guess.
                current = None;
            }
            continue;
        }
        if current.is_some() {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    flush(&mut out, current, &mut buf);
    out
}

/// `[3]` or `[3] text` at the start of a line, possibly indented.
fn leading_marker(line: &str) -> Option<usize> {
    let t = line.trim_start();
    let rest = t.strip_prefix('[')?;
    let end = rest.find(']')?;
    rest[..end].trim().parse::<usize>().ok()
}

// ---------------------------------------------------------------------------
// Attachments
// ---------------------------------------------------------------------------

/// Where attachment bytes live: beside the database, never in it.
pub fn attachments_dir(root: &Path) -> PathBuf {
    root.join("review-attachments")
}

pub const MAX_ATTACHMENT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ATTACHMENTS_PER_ENTRY: usize = 4;
pub const MAX_ATTACHMENT_BYTES_PER_WORKSPACE: u64 = 64 * 1024 * 1024;

/// Store bytes content-addressed, and return the hash.
///
/// Content addressing deduplicates the same screenshot attached to several
/// entries, and makes garbage collection a sweep for unreferenced hashes rather
/// than a schema concern.
pub async fn put_attachment(root: &Path, mime: &str, content: &[u8]) -> Result<(String, u32, u32)> {
    if content.len() > MAX_ATTACHMENT_BYTES {
        return Err(DomainError::AttachmentLimit);
    }
    if !matches!(mime, "image/png" | "image/jpeg" | "image/heic") {
        return Err(DomainError::InvalidArgument { what: "attachment mime" });
    }
    let (w, h) = image_dimensions(content).unwrap_or((0, 0));

    let sha = format!("{:x}", Sha256::digest(content));
    let dir = attachments_dir(root);
    tokio::fs::create_dir_all(&dir).await.map_err(|_| DomainError::OperationFailed)?;
    let path = dir.join(&sha);
    if tokio::fs::metadata(&path).await.is_err() {
        tokio::fs::write(&path, content).await.map_err(|_| DomainError::OperationFailed)?;
    }
    Ok((sha, w, h))
}

pub async fn read_attachment(
    root: &Path,
    sha: &str,
    offset: u64,
    max_bytes: usize,
) -> Result<(Vec<u8>, u64)> {
    let path = attachments_dir(root).join(sha);
    let bytes = tokio::fs::read(&path).await.map_err(|_| DomainError::NotFound)?;
    let total = bytes.len() as u64;
    let start = (offset as usize).min(bytes.len());
    let end = (start + max_bytes).min(bytes.len());
    Ok((bytes[start..end].to_vec(), total)
    )
}

/// Width and height, read from the header only.
///
/// Enough for a client to lay out a placeholder before the bytes arrive, which
/// is the whole reason an entry carries metadata separately from content.
fn image_dimensions(b: &[u8]) -> Option<(u32, u32)> {
    // PNG: 8-byte signature, then IHDR with width and height as big-endian u32.
    if b.len() > 24 && b.starts_with(&[0x89, b'P', b'N', b'G']) {
        let w = u32::from_be_bytes([b[16], b[17], b[18], b[19]]);
        let h = u32::from_be_bytes([b[20], b[21], b[22], b[23]]);
        return Some((w, h));
    }
    // JPEG: walk the segments to SOF0/SOF2.
    if b.len() > 4 && b[0] == 0xFF && b[1] == 0xD8 {
        let mut i = 2usize;
        while i + 9 < b.len() {
            if b[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = b[i + 1];
            let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
            if (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 {
                let h = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
                let w = u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32;
                return Some((w, h));
            }
            i += 2 + len;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(body: &str, anchor: Anchor, text: Option<&str>) -> PromptEntry {
        PromptEntry {
            body: body.to_string(),
            anchor,
            anchored_text: text.map(|s| s.to_string()),
        }
    }

    #[test]
    fn an_unanchored_comment_says_so_rather_than_inventing_a_location() {
        let p = compose_prompt(
            "feat/x",
            Disposition::Fix,
            &[entry("the error copy is wrong", Anchor::None, None)],
        );
        assert!(p.contains("[1] (no specific location)"));
        assert!(p.contains("the error copy is wrong"));
        assert!(p.contains("1 comment."), "singular when there is one");
    }

    #[test]
    fn an_anchored_comment_carries_the_text_and_never_a_line_number() {
        let p = compose_prompt(
            "feat/x",
            Disposition::Fix,
            &[entry(
                "don't unwrap here",
                Anchor::File { path: "src/git.rs".into() },
                Some("let base = repo.merge_base(a, b).unwrap();"),
            )],
        );
        assert!(p.contains("src/git.rs, at these lines:"));
        assert!(p.contains("| let base = repo.merge_base(a, b).unwrap();"));
        // A number would be stale the moment the agent edits anything above it.
        assert!(!p.contains("line 42"));
    }

    #[test]
    fn a_question_prompt_asks_for_numbered_answers_and_forbids_edits() {
        let p = compose_prompt(
            "feat/x",
            Disposition::Ask,
            &[entry("why three retries?", Anchor::None, None)],
        );
        assert!(p.contains("Start every answer with its number"));
        assert!(p.contains("Do not modify any files"));
    }

    #[test]
    fn a_fix_prompt_invites_disagreement_rather_than_compliance() {
        let p = compose_prompt("feat/x", Disposition::Fix, &[entry("x", Anchor::None, None)]);
        assert!(p.contains("If you disagree with one, say so and skip it"));
    }

    #[test]
    fn numbered_answers_are_split_back_onto_their_entries() {
        let reply = "[1] Because the transport retries idempotent reads.\n\
                     [2] It is dead code, removed.\n";
        let split = split_numbered_answer(reply, 2);
        assert!(split[0].as_ref().unwrap().contains("idempotent"));
        assert!(split[1].as_ref().unwrap().contains("dead code"));
    }

    #[test]
    fn a_multi_line_answer_keeps_all_of_its_lines() {
        let reply = "[1] First line.\nStill the first answer.\n\n[2] Second.\n";
        let split = split_numbered_answer(reply, 2);
        let first = split[0].as_ref().unwrap();
        assert!(first.contains("First line."));
        assert!(first.contains("Still the first answer."));
        assert!(!first.contains("Second."));
    }

    #[test]
    fn prose_before_the_first_marker_is_not_attributed_to_anything() {
        let reply = "Sure, here are my answers:\n[1] The real answer.\n";
        let split = split_numbered_answer(reply, 1);
        assert_eq!(split[0].as_deref(), Some("The real answer."));
    }

    #[test]
    fn an_answer_that_numbers_nothing_correlates_to_nothing() {
        // The caller attaches the whole turn to every entry and labels it
        // uncorrelated. Guessing here is how an answer lands on the wrong
        // question.
        let split = split_numbered_answer("I looked at both and they seem fine.", 2);
        assert!(split.iter().all(|s| s.is_none()));
    }

    #[test]
    fn a_number_outside_the_range_is_ignored_rather_than_clamped() {
        let split = split_numbered_answer("[7] about something else entirely\n", 2);
        assert!(split.iter().all(|s| s.is_none()), "[7] of 2 is not entry 2");
    }

    #[test]
    fn an_indented_marker_still_counts() {
        let split = split_numbered_answer("  [1] indented answer\n", 1);
        assert_eq!(split[0].as_deref(), Some("indented answer"));
    }

    #[test]
    fn a_partial_answer_leaves_the_unanswered_entries_empty() {
        let split = split_numbered_answer("[2] only answered the second one\n", 2);
        assert!(split[0].is_none(), "silence about [1] is not an answer to [1]");
        assert!(split[1].is_some());
    }

    #[test]
    fn png_dimensions_are_read_from_the_header() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        png.extend_from_slice(&[0, 0, 0, 13]);
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        assert_eq!(image_dimensions(&png), Some((640, 480)));
    }

    #[tokio::test]
    async fn an_attachment_over_the_limit_is_refused_with_a_named_error() {
        let dir = tempfile::tempdir().unwrap();
        let big = vec![0u8; MAX_ATTACHMENT_BYTES + 1];
        let err = put_attachment(dir.path(), "image/png", &big).await.unwrap_err();
        assert!(matches!(err, DomainError::AttachmentLimit));
    }

    #[tokio::test]
    async fn an_attachment_of_the_wrong_type_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let err = put_attachment(dir.path(), "application/pdf", b"%PDF").await.unwrap_err();
        assert!(matches!(err, DomainError::InvalidArgument { .. }));
    }

    #[tokio::test]
    async fn the_same_bytes_stored_twice_occupy_one_file() {
        let dir = tempfile::tempdir().unwrap();
        let png = {
            let mut p = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
            p.extend_from_slice(&[0, 0, 0, 13]);
            p.extend_from_slice(b"IHDR");
            p.extend_from_slice(&2u32.to_be_bytes());
            p.extend_from_slice(&2u32.to_be_bytes());
            p.extend_from_slice(&[8, 6, 0, 0, 0]);
            p
        };
        let (a, w, h) = put_attachment(dir.path(), "image/png", &png).await.unwrap();
        let (b, _, _) = put_attachment(dir.path(), "image/png", &png).await.unwrap();
        assert_eq!(a, b, "content addressing means one file");
        assert_eq!((w, h), (2, 2));
        let count = std::fs::read_dir(attachments_dir(dir.path())).unwrap().count();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn an_attachment_is_read_back_in_chunks_with_its_total_size() {
        // Chunked because this shares a connection with live terminal output,
        // and a 4 MiB frame at full rate stalls panes mid-render.
        let dir = tempfile::tempdir().unwrap();
        let png = {
            let mut p = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
            p.extend_from_slice(&[0, 0, 0, 13]);
            p.extend_from_slice(b"IHDR");
            p.extend_from_slice(&1u32.to_be_bytes());
            p.extend_from_slice(&1u32.to_be_bytes());
            p.extend_from_slice(&[8, 6, 0, 0, 0]);
            p.extend_from_slice(&[9u8; 100]);
            p
        };
        let (sha, _, _) = put_attachment(dir.path(), "image/png", &png).await.unwrap();

        let (first, total) = read_attachment(dir.path(), &sha, 0, 16).await.unwrap();
        assert_eq!(first.len(), 16);
        assert_eq!(total as usize, png.len());

        let (tail, _) = read_attachment(dir.path(), &sha, total - 4, 64).await.unwrap();
        assert_eq!(tail.len(), 4, "a read past the end is clamped, not an error");
    }

    #[test]
    fn the_cheap_gate_moves_when_head_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let git = dir.path().join(".git");
        std::fs::create_dir_all(&git).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(git.join("index"), "x").unwrap();

        let before = cheap_gate(dir.path());
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(git.join("HEAD"), "ref: refs/heads/other\n").unwrap();
        let after = cheap_gate(dir.path());

        assert_ne!(before, after, "a checkout must be visible to the gate");
    }

    #[test]
    fn the_cheap_gate_follows_a_linked_worktrees_gitdir_pointer() {
        // A worktree's `.git` is a FILE. Without following it the gate reads two
        // missing files, returns (0, 0) forever, and every linked worktree — that
        // is, every workspace this product creates — caches a stale change set.
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real-git-dir");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(real.join("index"), "x").unwrap();

        let wt = dir.path().join("worktree");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", real.display())).unwrap();

        let g = cheap_gate(&wt);
        assert_ne!(g, (0, 0), "the pointer must be followed to the real git dir");
    }
}
