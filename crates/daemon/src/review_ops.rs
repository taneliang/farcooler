//! Review operations, one function per RPC method.
//!
//! Separate from `rpc.rs` so the dispatch table stays a dispatch table, and
//! separate from `review.rs` so that the pure pieces there stay testable without
//! a `Service`.

use std::path::Path;

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1 as pb;
use farcooler_review::anchor::{Anchor, CaptureManifest, AnchorState};
use farcooler_store::review::{Disposition, EntryStatus, ReviewEntry};
use uuid::Uuid;

use crate::change_set::{self, BaseSource};
use crate::file_diff::{self, Selector, Unsupported};
use crate::review::{self, PromptEntry, now_millis};
use crate::service::Service;
use crate::wire::id_bytes;

// ---------------------------------------------------------------------------
// wire conversions
// ---------------------------------------------------------------------------

fn pb_status(s: change_set::FileStatus) -> i32 {
    use change_set::FileStatus as F;
    (match s {
        F::Added => pb::FileStatus::Added,
        F::Modified => pb::FileStatus::Modified,
        F::Deleted => pb::FileStatus::Deleted,
        F::Renamed => pb::FileStatus::Renamed,
        F::Copied => pb::FileStatus::Copied,
        F::TypeChanged => pb::FileStatus::TypeChanged,
        F::Untracked => pb::FileStatus::Untracked,
        F::Conflicted => pb::FileStatus::Conflicted,
    }) as i32
}

fn pb_file(f: &change_set::FileChange) -> pb::FileChange {
    pb::FileChange {
        path: f.path.clone(),
        status: pb_status(f.status),
        old_path: f.old_path.clone(),
        insertions: f.insertions,
        deletions: f.deletions,
        binary: f.binary,
        submodule: f.submodule,
    }
}

fn pb_change_set(workspace_id: Uuid, c: &review::CachedChangeSet) -> pb::ChangeSet {
    let s = &c.set;
    pb::ChangeSet {
        workspace_id: id_bytes(workspace_id),
        version: c.version,
        branch: s.branch.clone(),
        base_ref: s.base_ref.clone(),
        base_source: (match s.base_source {
            BaseSource::Recorded => pb::BaseSource::Recorded,
            BaseSource::Upstream => pb::BaseSource::Upstream,
            BaseSource::Guessed => pb::BaseSource::Guessed,
        }) as i32,
        base_commit: s.base_commit.clone(),
        head_commit: s.head_commit.clone(),
        commits: s
            .commits
            .iter()
            .map(|c| pb::ReviewCommit {
                sha: c.sha.clone(),
                subject: c.subject.clone(),
                body: c.body.clone(),
                author: c.author.clone(),
                timestamp: c.timestamp,
                files_changed: c.files_changed,
                insertions: c.insertions,
                deletions: c.deletions,
            })
            .collect(),
        working_tree: Some(pb::WorkingTree {
            staged: s.working_tree.staged.iter().map(pb_file).collect(),
            unstaged: s.working_tree.unstaged.iter().map(pb_file).collect(),
            untracked: s.working_tree.untracked.clone(),
            conflicted: s.working_tree.conflicted.clone(),
        }),
        files: s.files.iter().map(pb_file).collect(),
        insertions: s.insertions,
        deletions: s.deletions,
        worktree_digest: s.worktree_digest.clone(),
        computed_at: c.computed_at,
    }
}

fn pb_hunk(h: &farcooler_review::Hunk) -> pb::Hunk {
    use farcooler_review::LineKind;
    pb::Hunk {
        index: h.index,
        header: h.header.clone(),
        old_start: h.old_start,
        old_lines: h.old_lines,
        new_start: h.new_start,
        new_lines: h.new_lines,
        lines: h
            .lines
            .iter()
            .map(|l| pb::DiffLine {
                kind: (match l.kind {
                    LineKind::Context => pb::DiffLineKind::Context,
                    LineKind::Added => pb::DiffLineKind::Added,
                    LineKind::Removed => pb::DiffLineKind::Removed,
                }) as i32,
                old_no: l.old_no,
                new_no: l.new_no,
                text: l.text.clone(),
                no_newline: l.no_newline,
            })
            .collect(),
        fingerprint: h.fingerprint.clone(),
    }
}

fn pb_disposition(d: Disposition) -> i32 {
    (match d {
        Disposition::Fix => pb::Disposition::Fix,
        Disposition::Ask => pb::Disposition::Ask,
        Disposition::Note => pb::Disposition::Note,
    }) as i32
}

fn parse_disposition(v: i32) -> Disposition {
    match pb::Disposition::try_from(v) {
        Ok(pb::Disposition::Ask) => Disposition::Ask,
        Ok(pb::Disposition::Note) => Disposition::Note,
        _ => Disposition::Fix,
    }
}

fn pb_entry_status(s: EntryStatus) -> i32 {
    (match s {
        EntryStatus::Open => pb::EntryStatus::Open,
        EntryStatus::Dispatched => pb::EntryStatus::Dispatched,
        EntryStatus::Answered => pb::EntryStatus::Answered,
        EntryStatus::Resolved => pb::EntryStatus::Resolved,
        EntryStatus::DispatchUnknown => pb::EntryStatus::DispatchUnknown,
    }) as i32
}

fn pb_anchor_state(s: AnchorState) -> i32 {
    (match s {
        AnchorState::Exact => pb::AnchorState::Exact,
        AnchorState::Moved => pb::AnchorState::Moved,
        AnchorState::Ambiguous => pb::AnchorState::Ambiguous,
        AnchorState::NeedsReread => pb::AnchorState::NeedsReread,
        AnchorState::Outdated => pb::AnchorState::Outdated,
        AnchorState::FileGone => pb::AnchorState::FileGone,
    }) as i32
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// The worktree, branch and base for a workspace.
async fn locate(svc: &Service, workspace_id: Uuid) -> Result<(String, String, String, BaseSource)> {
    let ws = svc.store.get_workspace(workspace_id)?;
    let base = svc
        .store
        .review_base(workspace_id)?
        .map(|b| (b, BaseSource::Recorded))
        .unwrap_or_else(|| (default_base(), BaseSource::Guessed));
    Ok((ws.worktree_path, ws.branch, base.0, base.1))
}

/// The base to compare against when nothing was recorded.
///
/// `main` then `master`, resolved at use. Displayed and correctable, because a
/// guess that silently produces the wrong diff is the failure mode here.
fn default_base() -> String {
    "main".to_string()
}

/// Read a stored anchor, tolerating an unparseable one rather than failing the
/// whole list: one bad row must not make a reviewer's entire buffer unreadable.
fn read_anchor(json: &str) -> Anchor {
    serde_json::from_str(json).unwrap_or(Anchor::None)
}

fn read_manifest(json: &str) -> CaptureManifest {
    serde_json::from_str(json).unwrap_or(CaptureManifest {
        base_commit: String::new(),
        head_commit: String::new(),
        worktree_digest: String::new(),
        file_content_hash: None,
        file_snapshot: None,
    })
}

/// Fill in an anchor's context fingerprint from the file as it stands.
///
/// Only when the client left it empty. A client that DID compute one is
/// describing a specific occurrence — the second `DUP` rather than the first —
/// and overwriting that would lose the very thing it was for.
async fn fingerprint_now(worktree: &Path, anchor: Anchor) -> Anchor {
    let Anchor::Lines { path, side, text, context_fingerprint } = anchor else {
        return anchor;
    };
    if !context_fingerprint.is_empty() {
        return Anchor::Lines { path, side, text, context_fingerprint };
    }

    let fp = match tokio::fs::read_to_string(worktree.join(&path)).await {
        Ok(content) => {
            let hay: Vec<&str> = content.lines().collect();
            let needle: Vec<&str> = text.lines().collect();
            // Only when the text occurs exactly once. Two occurrences means the
            // caller has not said WHICH, and picking the first here would bake a
            // guess into the entry forever — the one thing anchoring refuses to
            // do. Left empty, resolution reports Ambiguous, which is true.
            let hits: Vec<usize> = if needle.is_empty() || needle.len() > hay.len() {
                Vec::new()
            } else {
                (0..=hay.len() - needle.len())
                    .filter(|&i| hay[i..i + needle.len()] == needle[..])
                    .collect()
            };
            match hits.as_slice() {
                [at] => farcooler_review::anchor::context_fingerprint(
                    &hay,
                    *at,
                    at + needle.len(),
                ),
                _ => String::new(),
            }
        }
        Err(_) => String::new(),
    };

    Anchor::Lines { path, side, text, context_fingerprint: fp }
}

/// An entry, with its anchor resolved against the worktree as it is now.
async fn hydrate(
    svc: &Service,
    worktree: &Path,
    set: &change_set::ChangeSet,
    e: &ReviewEntry,
) -> pb::ReviewEntry {
    let anchor = read_anchor(&e.anchor_json);
    let manifest = read_manifest(&e.manifest_json);
    let r = review::resolve_entry(worktree, set, &anchor, &manifest).await;

    let attachments = svc.store.entry_attachments(e.id).unwrap_or_default();

    pb::ReviewEntry {
        id: id_bytes(e.id),
        workspace_id: id_bytes(e.workspace_id),
        resource_version: e.resource_version,
        body: e.body.clone(),
        disposition: pb_disposition(e.disposition),
        status: pb_entry_status(e.status),
        anchor_json: e.anchor_json.clone(),
        anchor_state: pb_anchor_state(r.state),
        line: r.line,
        effective_anchor_json: serde_json::to_string(&r.effective).unwrap_or_default(),
        attachments: attachments
            .iter()
            .map(|a| pb::Attachment {
                id: id_bytes(a.id),
                sha256: a.sha256.clone(),
                mime: a.mime.clone(),
                byte_size: a.byte_size,
                width: a.width,
                height: a.height,
            })
            .collect(),
        dispatch_id: e.dispatch_id.map(id_bytes),
        answer_text: e.answer_text.clone(),
        answer_terminal_id: e.answer_terminal_id.map(id_bytes),
        answer_correlation: e.answer_correlation.clone(),
        created_at: e.created_at,
        updated_at: e.updated_at,
    }
}

// ---------------------------------------------------------------------------
// operations
// ---------------------------------------------------------------------------

pub async fn change_set(svc: &Service, req: &pb::ChangeSetRequest) -> Result<pb::ChangeSet> {
    let workspace_id =
        Uuid::from_slice(&req.workspace_id).map_err(|_| DomainError::NotFound)?;
    let (worktree, branch, base, source) = locate(svc, workspace_id).await?;

    // A selector naming a branch overrides the workspace's own.
    let branch = match req.selector.as_ref().and_then(|s| s.kind.as_ref()) {
        Some(pb::change_set_selector::Kind::Branch(b)) if !b.branch.is_empty() => b.branch.clone(),
        _ => branch,
    };

    let c = svc
        .review_cache
        .get(workspace_id, Path::new(&worktree), &branch, &base, source, req.fresh)
        .await?;
    Ok(pb_change_set(workspace_id, &c))
}

pub async fn commit_files(
    svc: &Service,
    req: &pb::CommitFilesRequest,
) -> Result<pb::FileChangeList> {
    let workspace_id =
        Uuid::from_slice(&req.workspace_id).map_err(|_| DomainError::NotFound)?;
    let ws = svc.store.get_workspace(workspace_id)?;
    let files = file_diff::commit_files(Path::new(&ws.worktree_path), &req.sha).await?;
    Ok(pb::FileChangeList { items: files.iter().map(pb_file).collect() })
}

pub async fn file_diff(svc: &Service, req: &pb::FileDiffRequest) -> Result<pb::FileDiff> {
    let workspace_id =
        Uuid::from_slice(&req.workspace_id).map_err(|_| DomainError::NotFound)?;
    let (worktree, branch, base, source) = locate(svc, workspace_id).await?;
    let c = svc
        .review_cache
        .get(workspace_id, Path::new(&worktree), &branch, &base, source, false)
        .await?;

    let selector = match req.selector.as_ref().and_then(|s| s.kind.as_ref()) {
        Some(pb::diff_selector::Kind::Commit(sha)) => Selector::Commit { sha: sha.clone() },
        Some(pb::diff_selector::Kind::Staged(_)) => Selector::Staged,
        Some(pb::diff_selector::Kind::Unstaged(_)) => Selector::Unstaged,
        _ => Selector::Range { base_commit: c.set.base_commit.clone() },
    };

    let r =
        file_diff::file_diff(Path::new(&worktree), &selector, &req.path, req.from_hunk).await?;

    Ok(pb::FileDiff {
        path: r.diff.path,
        hunks: r.diff.hunks.iter().map(pb_hunk).collect(),
        truncated: r.diff.truncated.map(|t| {
            use farcooler_review::Truncation as T;
            (match t {
                T::LineCap => pb::Truncation::LineCap,
                T::HunkCap => pb::Truncation::HunkCap,
                T::ByteCap => pb::Truncation::ByteCap,
                T::Timeout => pb::Truncation::Timeout,
            }) as i32
        }),
        next_hunk: r.diff.next_hunk,
        unsupported: r.unsupported.map(|u| {
            (match u {
                Unsupported::Binary => pb::DiffUnsupported::Binary,
                Unsupported::Submodule => pb::DiffUnsupported::Submodule,
                Unsupported::CombinedDiff => pb::DiffUnsupported::CombinedDiff,
                Unsupported::Malformed => pb::DiffUnsupported::Malformed,
            }) as i32
        }),
        first_parent_of_merge: r.first_parent_of_merge,
        change_set_version: c.version,
    })
}

pub async fn set_base(svc: &Service, req: &pb::ReviewSetBase) -> Result<pb::ChangeSet> {
    let workspace_id =
        Uuid::from_slice(&req.workspace_id).map_err(|_| DomainError::NotFound)?;
    let ws = svc.store.get_workspace(workspace_id)?;

    // Validated at set time, so a typo fails here rather than silently producing
    // a wrong diff every time anybody opens the workspace.
    let ok = crate::git::git(
        Path::new(&ws.worktree_path),
        &["rev-parse", "--verify", "--quiet", &req.base_ref],
    )
    .await?;
    if !ok.ok {
        return Err(DomainError::BaseUnresolvable);
    }

    svc.store.set_review_base(workspace_id, &req.base_ref)?;
    svc.review_cache.invalidate(workspace_id);

    let c = svc
        .review_cache
        .get(
            workspace_id,
            Path::new(&ws.worktree_path),
            &ws.branch,
            &req.base_ref,
            BaseSource::Recorded,
            true,
        )
        .await?;
    Ok(pb_change_set(workspace_id, &c))
}

pub async fn capture(svc: &Service, req: &pb::ReviewCapture) -> Result<pb::ReviewEntry> {
    let workspace_id =
        Uuid::from_slice(&req.workspace_id).map_err(|_| DomainError::NotFound)?;
    let (worktree, branch, base, source) = locate(svc, workspace_id).await?;
    let c = svc
        .review_cache
        .get(workspace_id, Path::new(&worktree), &branch, &base, source, false)
        .await?;

    let anchor = read_anchor(&req.anchor_json);
    // Fingerprint the surroundings HERE rather than trusting the client to.
    //
    // A client knows the text it selected; it does not necessarily know what sits
    // three lines either side of it, and the CLI genuinely cannot. Left empty,
    // every freshly written comment resolves as `moved` the instant you look at
    // it — technically honest, and a lie in effect, because nothing moved. The
    // daemon has the file open anyway.
    let anchor = fingerprint_now(Path::new(&worktree), anchor).await;
    let manifest = review::capture_manifest(Path::new(&worktree), &c.set, &anchor).await;

    let anchor_json = serde_json::to_string(&anchor).unwrap_or_else(|_| r#"{"kind":"none"}"#.into());
    let manifest_json = serde_json::to_string(&manifest).unwrap_or_else(|_| "{}".into());

    let e = svc.store.capture_review_entry(
        workspace_id,
        &req.body,
        parse_disposition(req.disposition),
        &anchor_json,
        &manifest_json,
        now_millis(),
    )?;

    // Keep the workspace's snapshots inside their budget, oldest first.
    //
    // The evicted entries stay — they lose range precision and their state says
    // so. Dropping a comment to reclaim bytes would be the wrong trade by a mile,
    // and dropping the NEWEST snapshot instead would punish the comment you just
    // wrote for the sins of the ones before it.
    enforce_snapshot_budget(svc, workspace_id)?;

    for id in &req.attachment_ids {
        if let Ok(aid) = Uuid::from_slice(id) {
            svc.store.attach_to_entry(aid, e.id)?;
        }
    }

    Ok(hydrate(svc, Path::new(&worktree), &c.set, &e).await)
}

pub async fn update(svc: &Service, req: &pb::ReviewUpdate, expected: u64) -> Result<pb::ReviewEntry> {
    let id = Uuid::from_slice(&req.id).map_err(|_| DomainError::NotFound)?;
    let e = svc.store.update_review_entry(
        id,
        &req.body,
        parse_disposition(req.disposition),
        expected,
        now_millis(),
    )?;
    let (worktree, branch, base, source) = locate(svc, e.workspace_id).await?;
    let c = svc
        .review_cache
        .get(e.workspace_id, Path::new(&worktree), &branch, &base, source, false)
        .await?;
    Ok(hydrate(svc, Path::new(&worktree), &c.set, &e).await)
}

pub async fn delete(svc: &Service, req: &pb::ReviewDelete) -> Result<()> {
    let id = Uuid::from_slice(&req.id).map_err(|_| DomainError::NotFound)?;
    svc.store.delete_review_entry(id)
}

pub async fn list(svc: &Service, req: &pb::ReviewList) -> Result<pb::ReviewEntryList> {
    let workspace_id =
        Uuid::from_slice(&req.workspace_id).map_err(|_| DomainError::NotFound)?;
    let (worktree, branch, base, source) = locate(svc, workspace_id).await?;
    let c = svc
        .review_cache
        .get(workspace_id, Path::new(&worktree), &branch, &base, source, false)
        .await?;

    let entries = svc.store.list_review_entries(workspace_id)?;
    let mut items = Vec::with_capacity(entries.len());
    for e in &entries {
        items.push(hydrate(svc, Path::new(&worktree), &c.set, e).await);
    }
    Ok(pb::ReviewEntryList { items })
}

pub async fn dispatch(
    svc: &Service,
    req: &pb::ReviewDispatchRequest,
) -> Result<pb::ReviewDispatch> {
    let workspace_id =
        Uuid::from_slice(&req.workspace_id).map_err(|_| DomainError::NotFound)?;
    let terminal_id =
        Uuid::from_slice(&req.terminal_id).map_err(|_| DomainError::NotFound)?;
    let disposition = parse_disposition(req.disposition);
    let (worktree, branch, base, source) = locate(svc, workspace_id).await?;
    let c = svc
        .review_cache
        .get(workspace_id, Path::new(&worktree), &branch, &base, source, false)
        .await?;

    // Compose from what the client actually named, in the order it named them,
    // so the numbering the agent is asked to echo matches what the user saw.
    let mut prompt_entries = Vec::new();
    let mut versioned = Vec::new();
    let mut skipped = Vec::new();

    for de in &req.entries {
        let Ok(id) = Uuid::from_slice(&de.id) else { continue };
        let Some(e) = svc.store.review_entry(id)? else { continue };
        if matches!(e.status, EntryStatus::Dispatched) {
            skipped.push(de.id.clone());
            continue;
        }
        let anchor = read_anchor(&e.anchor_json);
        let manifest = read_manifest(&e.manifest_json);
        // The anchored TEXT, as it stands now — resolved rather than replayed,
        // so an agent is shown the code as it currently is.
        let anchored_text = match &anchor {
            Anchor::Lines { text, .. } => Some(text.clone()),
            _ => None,
        };
        let _ = manifest;
        prompt_entries.push(PromptEntry { body: e.body.clone(), anchor, anchored_text });
        versioned.push((id, de.expected_resource_version));
    }

    if versioned.is_empty() {
        return Err(DomainError::InvalidArgument { what: "no dispatchable entries" });
    }

    let prompt = review::compose_prompt(&branch, disposition, &prompt_entries);
    let d = svc.store.open_dispatch(
        workspace_id,
        terminal_id,
        disposition,
        &versioned,
        &prompt,
        now_millis(),
    )?;

    // Send AFTER the transaction committed. A send that happened with no row to
    // show for it is the one state nothing could ever recover from.
    svc.send_review_prompt(terminal_id, &prompt).await?;

    let _ = &c;
    Ok(pb::ReviewDispatch {
        id: id_bytes(d.id),
        workspace_id: id_bytes(d.workspace_id),
        terminal_id: id_bytes(d.terminal_id),
        disposition: pb_disposition(d.disposition),
        entry_ids: d.entry_ids.iter().map(|i| id_bytes(*i)).collect(),
        prompt: d.prompt,
        state: pb::DispatchState::Pending as i32,
        created_at: d.created_at,
        observed_at: d.observed_at,
        skipped_entry_ids: skipped,
    })
}

pub async fn mark_viewed(svc: &Service, req: &pb::ReviewMarkViewed) -> Result<()> {
    let workspace_id =
        Uuid::from_slice(&req.workspace_id).map_err(|_| DomainError::NotFound)?;
    svc.store.mark_viewed(
        workspace_id,
        &req.branch,
        &req.path,
        &req.content_hash,
        now_millis(),
    )
}

pub async fn mark_reviewed(svc: &Service, req: &pb::ReviewMarkReviewed) -> Result<()> {
    let workspace_id =
        Uuid::from_slice(&req.workspace_id).map_err(|_| DomainError::NotFound)?;
    let (worktree, branch, base, source) = locate(svc, workspace_id).await?;
    let c = svc
        .review_cache
        .get(workspace_id, Path::new(&worktree), &branch, &base, source, false)
        .await?;
    let _ = &req.branch;
    // The gate is stored ALONGSIDE the digest, and the inbox compares gates.
    // Storing a zero here left "changed since you looked" permanently on, because
    // the real gate never equals zero — the badge could be earned but never
    // cleared, which is worse than no badge.
    let gate = review::cheap_gate(Path::new(&worktree));
    svc.store.mark_reviewed_with_gate(
        workspace_id,
        &branch,
        &c.set.head_commit,
        &c.set.worktree_digest,
        gate.0 as i64,
        gate.1 as i64,
        now_millis(),
    )
}

pub async fn attachment_put(svc: &Service, req: &pb::AttachmentPut) -> Result<pb::Attachment> {
    let workspace_id =
        Uuid::from_slice(&req.workspace_id).map_err(|_| DomainError::NotFound)?;
    if svc.store.workspace_attachment_bytes(workspace_id)? + req.content.len() as u64
        > review::MAX_ATTACHMENT_BYTES_PER_WORKSPACE
    {
        return Err(DomainError::AttachmentLimit);
    }
    let (sha, w, h) =
        review::put_attachment(svc.root_dir(), &req.mime, &req.content).await?;
    let a = svc.store.record_attachment(
        &sha,
        &req.mime,
        req.content.len() as u64,
        w,
        h,
        now_millis(),
    )?;
    Ok(pb::Attachment {
        id: id_bytes(a.id),
        sha256: a.sha256,
        mime: a.mime,
        byte_size: a.byte_size,
        width: a.width,
        height: a.height,
    })
}

pub async fn attachment_get(svc: &Service, req: &pb::AttachmentGet) -> Result<pb::AttachmentBytes> {
    let id = Uuid::from_slice(&req.id).map_err(|_| DomainError::NotFound)?;
    let a = svc.store.attachment(id)?.ok_or(DomainError::NotFound)?;
    // Capped regardless of what the client asked for: this shares a connection
    // with live terminal output, and one huge frame stalls panes mid-render.
    let max = (req.max_bytes as usize).clamp(1, 256 * 1024);
    let (content, total) =
        review::read_attachment(svc.root_dir(), &a.sha256, req.offset, max).await?;
    Ok(pb::AttachmentBytes {
        id: req.id.clone(),
        offset: req.offset,
        content: bytes::Bytes::from(content),
        total_size: total,
    })
}

/// The fleet inbox.
///
/// Durable counts only, and the cheap gate for "changed since you looked".
/// Resolving every entry's anchor here would need a change set per workspace,
/// which is one `git status` per worktree per call — exactly what `watch.rs` was
/// built to avoid.
pub async fn inbox(svc: &Service) -> Result<pb::ReviewInbox> {
    let counts = svc.store.review_counts_by_workspace()?;
    let mut items = Vec::new();

    for c in counts {
        let Ok(ws) = svc.store.get_workspace(c.workspace_id) else { continue };
        let gate = review::cheap_gate(Path::new(&ws.worktree_path));
        let mark = svc.store.reviewed_mark(c.workspace_id, &ws.branch)?;
        // Three cheap signals, none of which runs git:
        //
        //   1. the gate, which catches commits, rebases and checkouts;
        //   2. an agent write served by this daemon, which catches the ordinary
        //      case the gate cannot see — a file edited in place;
        //   3. the digest of an already-cached change set, free when someone has
        //      been looking at this workspace anyway.
        let changed = match mark {
            Some(m) => {
                let structural = m.gate_head != gate.0 as i64 || m.gate_index != gate.1 as i64;
                let written = svc
                    .review_cache
                    .touched_at(c.workspace_id)
                    .is_some_and(|t| t > m.marked_at);
                let digest_moved = svc
                    .review_cache
                    .cached_digest(c.workspace_id, &ws.branch)
                    .is_some_and(|d| d != m.worktree_digest);
                structural || written || digest_moved
            }
            // Never marked read. Anything in the buffer wants attention.
            None => c.open + c.dispatched + c.answered + c.dispatch_unknown > 0,
        };
        let base = svc
            .store
            .review_base(c.workspace_id)?
            .unwrap_or_else(default_base);
        // Behind the cheap gate: a worktree nobody has touched costs two stats
        // and no git at all, which is what makes leaving a fleet view open
        // affordable.
        let (_, ins, del) = svc
            .review_cache
            .shortstat(c.workspace_id, Path::new(&ws.worktree_path), &base)
            .await
            .unwrap_or((0, 0, 0));

        items.push(pb::InboxWorkspace {
            workspace_id: id_bytes(c.workspace_id),
            task_name: ws.task_name,
            branch: ws.branch,
            open: c.open,
            dispatched: c.dispatched,
            answered: c.answered,
            dispatch_unknown: c.dispatch_unknown,
            changed_since_reviewed: changed,
            insertions: ins,
            deletions: del,
        });
    }

    Ok(pb::ReviewInbox { items, elsewhere: 0 })
}

/// Evict oldest-first until the workspace is under its snapshot budget.
fn enforce_snapshot_budget(svc: &Service, workspace_id: Uuid) -> Result<()> {
    let mut held = svc.store.entries_with_snapshots(workspace_id)?;
    let mut total: usize = held.iter().map(|(_, n)| *n).sum();
    if total <= review::MAX_SNAPSHOT_BYTES_PER_WORKSPACE {
        return Ok(());
    }
    // Oldest first, which is the order the query already returns.
    for (id, size) in held.drain(..) {
        if total <= review::MAX_SNAPSHOT_BYTES_PER_WORKSPACE {
            break;
        }
        svc.store.clear_snapshot(id)?;
        total = total.saturating_sub(size);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// stacks and PR state
// ---------------------------------------------------------------------------

fn pb_pr(p: &crate::stack::PrStatus) -> pb::PrStatus {
    use crate::stack as st;
    pb::PrStatus {
        number: p.number,
        url: p.url.clone(),
        state: (match p.state {
            st::PrState::Unknown => pb::PrState::Unknown,
            st::PrState::Open => pb::PrState::Open,
            st::PrState::Draft => pb::PrState::Draft,
            st::PrState::Merged => pb::PrState::Merged,
            st::PrState::Closed => pb::PrState::Closed,
        }) as i32,
        checks: (match p.checks {
            st::CheckState::Unknown => pb::CheckState::Unknown,
            st::CheckState::Passing => pb::CheckState::Passing,
            st::CheckState::Failing => pb::CheckState::Failing,
            st::CheckState::Pending => pb::CheckState::Pending,
        }) as i32,
        review_decision: (match p.review_decision {
            st::ReviewDecision::Unknown => pb::ReviewDecision::Unknown,
            st::ReviewDecision::Approved => pb::ReviewDecision::Approved,
            st::ReviewDecision::ChangesRequested => pb::ReviewDecision::ChangesRequested,
            st::ReviewDecision::ReviewRequired => pb::ReviewDecision::ReviewRequired,
        }) as i32,
        head_oid: p.head_oid.clone(),
        merged_at: p.merged_at,
        fetched_at: p.fetched_at,
        stale: p.stale,
    }
}

fn pb_link(l: &crate::stack::StackLink) -> pb::StackLink {
    use crate::stack::ParentSource as P;
    pb::StackLink {
        branch: l.branch.clone(),
        parent_branch: l.parent_branch.clone(),
        parent_source: (match l.parent_source {
            P::Recorded => pb::ParentSource::Recorded,
            P::Upstream => pb::ParentSource::Upstream,
            P::PrBase => pb::ParentSource::PrBase,
            P::Guessed => pb::ParentSource::Guessed,
        }) as i32,
        head_commit: l.head_commit.clone(),
        ahead: l.ahead,
        behind: l.behind,
        pr: l.pr.as_ref().map(pb_pr),
    }
}

/// A worktree belonging to `repository_id`, for running git and `gh` in.
async fn any_worktree(svc: &Service, repository_id: Uuid) -> Result<String> {
    svc.store
        .list_workspaces_for_repository(repository_id)?
        .into_iter()
        .find(|w| Path::new(&w.worktree_path).is_dir())
        .map(|w| w.worktree_path)
        .ok_or(DomainError::NotFound)
}

/// The repository's default branch, from `origin/HEAD` when it is known.
async fn default_branch(worktree: &Path) -> String {
    let r = crate::git::git(worktree, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]).await;
    match r {
        Ok(o) if o.ok => o
            .stdout
            .trim()
            .rsplit('/')
            .next()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "main".into()),
        _ => "main".into(),
    }
}

async fn build_stack(
    svc: &Service,
    repository_id: Uuid,
    branch: &str,
    refresh_prs: bool,
) -> Result<pb::StackLinkList> {
    let worktree_path = any_worktree(svc, repository_id).await?;
    let worktree = Path::new(&worktree_path);
    let default = default_branch(worktree).await;

    // PR state first: a PR's baseRef is the best available parent when nothing
    // was recorded and no upstream is set, which is the common case for branches
    // created by an agent.
    let prs = if refresh_prs {
        // At most `GH_MAX_CONCURRENT` of these run at once across the daemon.
        // A fleet refresh over twenty repositories would otherwise fork twenty
        // network-bound processes at the same instant.
        let _permit = svc.gh_permit().await;
        crate::stack::fetch_prs(worktree).await?
    } else {
        svc.pr_cache_get(repository_id)
    };
    if refresh_prs {
        svc.pr_cache_put(repository_id, prs.clone());
    }

    let recorded: Vec<(String, String)> = Vec::new();
    let parent_of = |b: &str| -> Option<(String, crate::stack::ParentSource)> {
        if let Some((_, p)) = recorded.iter().find(|(x, _)| x == b) {
            return Some((p.clone(), crate::stack::ParentSource::Recorded));
        }
        if let Ok(Some(p)) = svc.store.stack_parent(repository_id, b) {
            return Some((p, crate::stack::ParentSource::Recorded));
        }
        if let Some(list) = &prs {
            if let Some(pr) = list.iter().find(|p| p.head_ref == b) {
                if !pr.base_ref.is_empty() {
                    return Some((pr.base_ref.clone(), crate::stack::ParentSource::PrBase));
                }
            }
        }
        None
    };

    let (mut links, cycle) = crate::stack::walk_chain(branch, &default, &parent_of);

    for l in links.iter_mut() {
        let (ahead, behind) = crate::stack::ahead_behind(worktree, &l.branch, &l.parent_branch).await;
        l.ahead = ahead;
        l.behind = behind;
        if let Ok(o) = crate::git::git(worktree, &["rev-parse", &l.branch]).await {
            if o.ok {
                l.head_commit = o.stdout.trim().to_string();
            }
        }
        if let Some(list) = &prs {
            l.pr = list.iter().find(|p| p.head_ref == l.branch).map(|p| p.status.clone());
        }
    }

    Ok(pb::StackLinkList {
        repository_id: id_bytes(repository_id),
        items: links.iter().map(pb_link).collect(),
        cycle_detected: cycle,
    })
}

pub async fn stack_get(svc: &Service, req: &pb::StackGet) -> Result<pb::StackLinkList> {
    let repository_id =
        Uuid::from_slice(&req.repository_id).map_err(|_| DomainError::NotFound)?;
    build_stack(svc, repository_id, &req.branch, false).await
}

pub async fn stack_set_parent(
    svc: &Service,
    req: &pb::StackSetParent,
) -> Result<pb::StackLinkList> {
    let repository_id =
        Uuid::from_slice(&req.repository_id).map_err(|_| DomainError::NotFound)?;
    svc.store.set_stack_parent(repository_id, &req.branch, &req.parent_branch)?;
    build_stack(svc, repository_id, &req.branch, false).await
}

pub async fn pr_refresh(svc: &Service, req: &pb::PrRefresh) -> Result<pb::StackLinkList> {
    let repository_id =
        Uuid::from_slice(&req.repository_id).map_err(|_| DomainError::NotFound)?;
    let worktree_path = any_worktree(svc, repository_id).await?;
    let branch = svc
        .store
        .list_workspaces_for_repository(repository_id)?
        .first()
        .map(|w| w.branch.clone())
        .unwrap_or_else(|| "HEAD".into());
    let _ = worktree_path;
    build_stack(svc, repository_id, &branch, true).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_review::anchor::{Current, Side, content_hash, resolve};

    fn manifest_for(content: &str) -> CaptureManifest {
        CaptureManifest {
            base_commit: "base".into(),
            head_commit: "head".into(),
            worktree_digest: "digest".into(),
            file_content_hash: Some(content_hash(content)),
            file_snapshot: None,
        }
    }

    fn current_for(content: &str) -> Current {
        Current {
            head_commit: "head".into(),
            worktree_digest: "digest".into(),
            file_content: Some(content.into()),
            file_content_hash: Some(content_hash(content)),
            hunks: Vec::new(),
        }
    }

    #[tokio::test]
    async fn a_freshly_captured_anchor_resolves_exact_rather_than_moved() {
        // The bug this exists for: a client cannot always know what sits three
        // lines either side of the text it selected, and the CLI genuinely
        // cannot. Left empty, every comment read as `moved` the instant it was
        // written — technically true, and a lie in effect.
        let dir = tempfile::tempdir().unwrap();
        let content = "one\ntwo\nTARGET\nthree\n";
        std::fs::write(dir.path().join("x.rs"), content).unwrap();

        let bare = Anchor::Lines {
            path: "x.rs".into(),
            side: Side::New,
            text: "TARGET".into(),
            context_fingerprint: String::new(),
        };
        let filled = fingerprint_now(dir.path(), bare).await;

        let r = resolve(&filled, &manifest_for(content), &current_for(content));
        assert_eq!(r.state, AnchorState::Exact, "nothing moved, so nothing should say it did");
        assert_eq!(r.line, Some(3));
    }

    #[tokio::test]
    async fn a_fingerprint_the_client_supplied_is_left_alone() {
        // A client that DID compute one is naming a specific occurrence — the
        // second copy rather than the first — and overwriting it would throw away
        // exactly the information it was carrying.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("x.rs"), "a\nDUP\nb\nDUP\nc\n").unwrap();

        let theirs = Anchor::Lines {
            path: "x.rs".into(),
            side: Side::New,
            text: "DUP".into(),
            context_fingerprint: "the client picked the second one".into(),
        };
        let after = fingerprint_now(dir.path(), theirs.clone()).await;
        assert_eq!(after, theirs);
    }

    #[tokio::test]
    async fn text_that_appears_twice_is_left_unfingerprinted_so_it_reads_as_ambiguous() {
        // Picking the first match here would bake a guess into the entry
        // permanently. Empty is correct: resolution then reports Ambiguous.
        let dir = tempfile::tempdir().unwrap();
        let content = "a\nDUP\nb\nDUP\nc\n";
        std::fs::write(dir.path().join("x.rs"), content).unwrap();

        let bare = Anchor::Lines {
            path: "x.rs".into(),
            side: Side::New,
            text: "DUP".into(),
            context_fingerprint: String::new(),
        };
        let filled = fingerprint_now(dir.path(), bare).await;
        let Anchor::Lines { context_fingerprint, .. } = &filled else {
            panic!("still a Lines anchor");
        };
        assert!(context_fingerprint.is_empty(), "two matches means the caller has not said which");

        let r = resolve(&filled, &manifest_for(content), &current_for(content));
        assert_eq!(r.state, AnchorState::Ambiguous);
        assert_eq!(r.line, None);
    }

    #[tokio::test]
    async fn a_missing_file_leaves_the_anchor_alone_rather_than_failing_the_capture() {
        let dir = tempfile::tempdir().unwrap();
        let bare = Anchor::Lines {
            path: "not-here.rs".into(),
            side: Side::New,
            text: "x".into(),
            context_fingerprint: String::new(),
        };
        let after = fingerprint_now(dir.path(), bare).await;
        assert!(matches!(after, Anchor::Lines { .. }), "capturing still succeeds");
    }

    #[tokio::test]
    async fn an_unanchored_capture_is_untouched() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(fingerprint_now(dir.path(), Anchor::None).await, Anchor::None);
        assert_eq!(fingerprint_now(dir.path(), Anchor::Workspace).await, Anchor::Workspace);
    }
}
