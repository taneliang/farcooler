//! Review operations, one function per RPC method.
//!
//! Separate from `rpc.rs` so the dispatch table stays a dispatch table, and
//! separate from `review.rs` so that the pure pieces there stay testable without
//! a `Service`.

use std::path::Path;

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1 as pb;
use uuid::Uuid;

use crate::change_set::{self, BaseSource};
use crate::file_diff::{self, Selector, Unsupported};
use crate::review::{self, now_millis};
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

fn pb_base_source(s: BaseSource) -> i32 {
    (match s {
        BaseSource::Recorded => pb::BaseSource::Recorded,
        BaseSource::Upstream => pb::BaseSource::Upstream,
        BaseSource::PrBase => pb::BaseSource::PrBase,
        BaseSource::DefaultBranch => pb::BaseSource::DefaultBranch,
        BaseSource::Guessed => pb::BaseSource::Guessed,
    }) as i32
}

fn pb_change_set(workspace_id: Uuid, c: &review::CachedChangeSet) -> pb::ChangeSet {
    let s = &c.set;
    pb::ChangeSet {
        workspace_id: id_bytes(workspace_id),
        version: c.version,
        branch: s.branch.clone(),
        base_ref: s.base_ref.clone(),
        base_source: pb_base_source(s.base_source),
        base_commit: s.base_commit.clone(),
        head_commit: s.head_commit.clone(),
        commits: s
            .commits
            .iter()
            .map(|c| pb::ChangeCommit {
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
            // One list, written twice: `untracked` is what an app already in
            // the field decodes, `untracked_files` is the same files with the
            // counts a bare path cannot carry.
            untracked: s.working_tree.untracked.iter().map(|f| f.path.clone()).collect(),
            untracked_files: s.working_tree.untracked.iter().map(pb_file).collect(),
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

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// The worktree, branch and base for a workspace.
async fn locate(svc: &Service, workspace_id: Uuid) -> Result<(String, String, String, BaseSource)> {
    let ws = svc.store.get_workspace(workspace_id)?;
    let (base, source) = resolve_base(svc, &ws).await?;
    Ok((ws.worktree_path, ws.branch, base, source))
}

/// What this branch is compared against, asked in order of what actually knows.
///
/// `main` was hardcoded here once, and it failed in the worst possible way: a
/// repository on `master` resolved no merge base, so review showed an empty diff
/// and said nothing about why. Nothing about "no changes" and "I could not work
/// out what to compare against" looked different.
///
/// The order is by how much each source really knows about THIS branch:
///
/// 1. **What the user pinned.** Nothing overrules a person who said so.
/// 2. **The branch's pull request.** GitHub records exactly what it is based on,
///    and for a stacked branch that is its parent slice — where the repository
///    default would be flatly wrong and would show the whole stack's diff.
/// 3. **The repository's default branch**, from `gh` when it is there.
/// 4. **`origin/HEAD`**, the same fact recorded locally, no network.
/// 5. **A local `main` or `master`** — the only one labeled a guess, because it
///    is the only one that can quietly be the wrong answer.
///
/// Every step degrades to the next, so a runner with no `gh`, no network and no
/// remote still reviews.
pub(crate) async fn resolve_base(
    svc: &Service,
    ws: &farcooler_store::models::Workspace,
) -> Result<(String, BaseSource)> {
    if let Some(recorded) = svc.store.review_base(ws.id)? {
        return Ok((recorded, BaseSource::Recorded));
    }

    let worktree = Path::new(&ws.worktree_path);

    // Whatever PR state was last read. Never fetched here: resolving a base must
    // not put a network call on the path of drawing a diff.
    if let Some(prs) = svc.pr_cache_get(ws.repository_id) {
        if let Some(pr) = prs.iter().find(|p| p.head_ref == ws.branch) {
            if !pr.base_ref.is_empty() && pr.base_ref != ws.branch {
                return Ok((pr.base_ref.clone(), BaseSource::PrBase));
            }
        }
    }

    if let Some(d) = svc.default_branch(ws.repository_id, worktree).await {
        return Ok((d, BaseSource::DefaultBranch));
    }

    change_set::guess_base(worktree)
        .await
        .map(|b| (b, BaseSource::Guessed))
        .ok_or(DomainError::BaseUnresolvable)
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
        Some(pb::diff_selector::Kind::Local(_)) => Selector::Local,
        _ => Selector::Range { base_commit: c.set.base_commit.clone() },
    };

    let r =
        file_diff::file_diff(Path::new(&worktree), &selector, &req.path, req.from_hunk, req.context)
            .await?;

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

pub async fn set_base(svc: &Service, req: &pb::ChangesSetBase) -> Result<pb::ChangeSet> {
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

pub async fn mark_read(svc: &Service, req: &pb::ChangesMarkRead) -> Result<()> {
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

/// The fleet inbox.
///
/// Durable counts only, and the cheap gate for "changed since you looked".
/// Resolving every entry's anchor here would need a change set per workspace,
/// which is one `git status` per worktree per call — exactly what `watch.rs` was
/// built to avoid.
///
/// Nothing here runs git, which is the whole reason a client may poll it. The
/// `+N -M` used to be computed in this loop, behind a gate that made it free for
/// a worktree nobody had committed in; once the numbers included uncommitted
/// work that gate stopped holding, and a fleet-wide `git diff` on every client's
/// three-second timer is not a thing to build. `Watcher::probe_change_sets` owns
/// the probe now and this reads what it left.
pub async fn inbox(svc: &Service) -> Result<pb::ChangesInbox> {
    let mut items = Vec::new();

    for ws in svc.store.list_all_workspaces()? {
        let worktree = Path::new(&ws.worktree_path);
        if !worktree.is_dir() {
            continue;
        }
        let gate = review::cheap_gate(worktree);
        let mark = svc.store.reviewed_mark(ws.id, &ws.branch)?;

        // Three cheap signals, none of which runs git: the gate, which catches
        // commits, rebases and checkouts; an agent write this daemon served
        // itself, which catches the ordinary case the gate cannot see, a file
        // edited in place; and the digest of an already-cached change set, free
        // when somebody has been looking at this worktree anyway.
        let changed = match &mark {
            Some(m) => {
                m.gate_head != gate.0 as i64
                    || m.gate_index != gate.1 as i64
                    || svc.review_cache.touched_at(ws.id).is_some_and(|t| t > m.marked_at)
                    || svc
                        .review_cache
                        .cached_digest(ws.id, &ws.branch)
                        .is_some_and(|d| d != m.worktree_digest)
            }
            // Never marked read, so anything it has to say is new to you.
            None => true,
        };

        let (ins, del) = match svc.review_cache.counts(ws.id) {
            review::Counts::Known(_, ins, del) => (ins, del),
            // A worktree with no base to compare against has nothing to report,
            // which is not the same as a worktree that changed nothing.
            review::Counts::NoBase => continue,
            // The watch loop has not reached it yet, which lasts one tick after
            // the daemon starts. Zero rather than a guess, on the same terms a
            // failed `shortstat` was always treated: the row can still be worth
            // showing because it is unread, and no number beats an invented one.
            review::Counts::Unknown => (0, 0),
        };

        // A worktree with nothing changed and nothing moved has nothing to say.
        if ins == 0 && del == 0 && !changed {
            continue;
        }

        items.push(pb::InboxWorkspace {
            workspace_id: id_bytes(ws.id),
            task_name: ws.name(),
            branch: ws.branch,
            changed_since_reviewed: changed,
            insertions: ins,
            deletions: del,
        });
    }

    Ok(pb::ChangesInbox { items, elsewhere: 0 })
}

// ---------------------------------------------------------------------------
// stacks and PR state
// ---------------------------------------------------------------------------

/// One PR's state on the wire.
///
/// `stale` is DERIVED here, at the moment of answering, and there is no longer a
/// field on the record to copy it from. It has to be derived: it is a fact about
/// how long ago the read happened, and that changes while the value sits in the
/// cache. The field it used to be copied from was written `false` at both of
/// `stack.rs`'s construction sites and set true by nothing, so "Last read from
/// GitHub a while ago" — which iOS and Android both already render — was a
/// sentence that could not appear.
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
        stale: crate::stack::is_stale(p.fetched_at, now_millis()),
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
        // A forced refresh is already spending somebody's rate limit and is
        // already `Scope::Control`, so it is the right moment to learn the
        // repository's web URL as well. Cached for the daemon's lifetime, so
        // this is one subprocess the first time and nothing afterwards.
        let _ = svc.default_branch(repository_id, worktree).await;
    }
    // Whether `gh` ANSWERED, which is not whether there are any PRs.
    //
    // Read back from the cache rather than from `prs` above, because `prs` has
    // already been flattened: a failed `gh` and a `gh` nobody ran are both
    // `None` there, and telling them apart is the entire point of this field.
    // See `Service::pr_answer_is_known`.
    let pr_known = svc.pr_answer_is_known(repository_id);

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
        pr_known,
        // Cache-only. Nothing on this path may wait on `gh`, so a repository
        // nobody has asked about yet reports no URL and gets one on the
        // `stack_changed` event the fill below pushes.
        repo_url: svc.repo_web_url(repository_id).unwrap_or_default(),
    })
}

/// Fill an empty PR cache for `repository_id`, off the caller's thread, and
/// announce the result.
///
/// Decision 2 of `pr-status-at-a-glance.md`, and its first constraint is what
/// this shape is for: `stack.get` must return what is cached — empty — at local
/// speed, and the answer must arrive later as an event. Awaiting `gh` inside the
/// read would put GitHub's latency on the path of drawing a fleet, which is the
/// same thing `resolve_base` refuses to do and for the same reason.
///
/// Detached rather than awaited, and claimed rather than merely spawned:
/// `claim_pr_fill` is what stops five clients opening the same repository from
/// forking five `gh` processes, and what bounds a repository `gh` can never
/// answer for to one attempt per `GH_MIN_INTERVAL`.
///
/// It announces even when `gh` failed. A client that got an empty first read is
/// otherwise waiting on an event that will never come, with no way to tell that
/// from an answer still in flight — and `pr_known: false` on the event is a
/// different statement from `pr_known: false` on the read: the first says
/// GitHub was asked and could not answer, the second says nobody has asked yet.
pub fn fill_prs_in_background(
    service: std::sync::Arc<Service>,
    watcher: std::sync::Arc<crate::watch::Watcher>,
    repository_id: Uuid,
    branch: String,
) {
    if !service.claim_pr_fill(repository_id) {
        return;
    }
    tokio::spawn(async move {
        let Ok(worktree_path) = any_worktree(&service, repository_id).await else { return };
        let worktree = Path::new(&worktree_path);

        // The repository's default branch and web URL, from one cached
        // `gh repo view`. Before the PR list rather than after, so a client that
        // acts on this event has the compare link's other half in the same
        // message it learns there is no PR from.
        let _ = service.default_branch(repository_id, worktree).await;

        let prs = {
            let _permit = service.gh_permit().await;
            crate::stack::fetch_prs(worktree).await
        };
        // An `Err` here is not a `gh` that failed — those are `Ok(None)` on
        // purpose — so there is nothing to record and nothing to say.
        let Ok(prs) = prs else { return };
        service.pr_cache_put(repository_id, prs);

        match build_stack(&service, repository_id, &branch, false).await {
            Ok(list) => watcher.announce_stack_changed(list),
            Err(e) => tracing::warn!(error = %e, "could not rebuild a stack after filling it"),
        }
    });
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

    fn a_pr_read_at(fetched_at: i64) -> crate::stack::PrStatus {
        crate::stack::PrStatus {
            number: 335,
            url: "https://github.com/o/r/pull/335".into(),
            state: crate::stack::PrState::Open,
            checks: crate::stack::CheckState::Passing,
            review_decision: crate::stack::ReviewDecision::Approved,
            head_oid: "deadbeef".into(),
            merged_at: None,
            fetched_at,
        }
    }

    /// `stale` is answered from the clock, not copied from the record.
    ///
    /// It used to be copied, from a field written `false` at both construction
    /// sites in `stack.rs` and set true by nothing — so "Last read from GitHub a
    /// while ago", which iOS and Android both render, was a sentence that could
    /// not appear. The threshold lives in the daemon so three platforms cannot
    /// hold three opinions about what "a while" is.
    #[test]
    fn how_stale_a_pr_is_comes_from_when_it_was_read() {
        let now = now_millis();
        let threshold = crate::stack::PR_STALE_AFTER.as_millis() as i64;

        assert!(!pb_pr(&a_pr_read_at(now)).stale, "a read that just happened is current");
        assert!(
            pb_pr(&a_pr_read_at(now - threshold - 60_000)).stale,
            "a read past the threshold must say so, or the warning can never appear"
        );
    }

    /// And the timestamp it is derived from crosses the wire too, so a client
    /// can say how long ago rather than only that it was a while.
    #[test]
    fn the_timestamp_stale_is_derived_from_crosses_the_wire_as_well() {
        let read_at = 1_785_925_800_000;
        assert_eq!(pb_pr(&a_pr_read_at(read_at)).fetched_at, read_at);
    }
}
