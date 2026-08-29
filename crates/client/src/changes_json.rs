//! What a `changes` answer looks like once it leaves the wire.
//!
//! One implementation, two callers. The phones reach these shapes through the
//! FFI (`session.rs`); the Mac reaches the same shapes by shelling out to
//! `farcooler changes … --json` (`crates/cli/src/changes.rs`). Until this module
//! existed each side held its own copy, and each copy carried a comment saying
//! it was deliberately identical to the other. They drifted anyway:
//! `base_source` was added for the phones and never reached the Mac, so the Mac
//! could not draw the guessed-base warning even though the daemon had been
//! sending it the whole time, and `changes diff` had no `--json` at all, so the
//! Mac scraped the human output and lost the three notices that are not lines of
//! patch. Both of those are the same bug: a rule that two files were asked to
//! remember. A shared module makes it a thing the compiler holds instead.
//!
//! The inbox joined them later and was the worst of the three: not a missing
//! key but a different envelope, a bare array against `{"items": …}`, with the
//! same key meaning a short id on one side and a full UUID on the other.
//! `c2f1117` found it while moving the change-set builders here and left it,
//! because unifying it breaks the Mac's decoder and that wanted saying out loud
//! rather than riding along.
//!
//! The shapes are pinned by tests on the side that decodes them — see
//! `crates/cli/src/changes.rs`.

use farcooler_protocol::v1 as pb;
use serde_json::json;

use crate::session::{short, uuid_of};

/// A change set, in the shape `farcooler changes status --json` prints and the
/// FFI's `changes.change_set` returns.
pub fn change_set_json(cs: &pb::ChangeSet) -> serde_json::Value {
    json!({
        "branch": cs.branch,
        "base_ref": cs.base_ref,
        "base_source": base_source_name(cs.base_source),
        "base_commit": cs.base_commit,
        "head_commit": cs.head_commit,
        "insertions": cs.insertions,
        "deletions": cs.deletions,
        // The three counts are the commit's OWN, against its first parent, and
        // a merge's are its first-parent counts rather than a combined diff.
        // They were zeroes in the daemon until `commits_since` learned to read
        // `--shortstat`, and were left off the wire because of it; a client that
        // sums a selected commit's file list to get `+N -M` is now summing to
        // reach a number it was sent.
        "commits": cs.commits.iter().map(|c| json!({
            "sha": c.sha,
            "subject": c.subject,
            // The message under the subject. `commits_since` has always parsed
            // `%b` into it and nothing ever sent it on, so a phone reading a
            // branch saw only first lines — which is where an agent puts what
            // it did, and never why.
            "body": c.body,
            "author": c.author,
            "timestamp": c.timestamp,
            "files_changed": c.files_changed,
            "insertions": c.insertions,
            "deletions": c.deletions,
        })).collect::<Vec<_>>(),
        "files": cs.files.iter().map(file_change_json).collect::<Vec<_>>(),
        "working_tree": cs.working_tree.as_ref().map(|w| json!({
            "staged": w.staged.iter().map(|f| &f.path).collect::<Vec<_>>(),
            "unstaged": w.unstaged.iter().map(|f| &f.path).collect::<Vec<_>>(),
            "untracked": w.untracked,
            "conflicted": w.conflicted,
            // Every dirty path with its own counts, which is what an
            // "uncommitted" total is a sum of. A file that is staged AND
            // modified again appears twice, once per group, because those are
            // two different diffs of it — a reader summing per path gets what
            // `git diff HEAD` would say, and `change_set::apply_uncommitted_counts`
            // says where that is exact and where it is an upper bound. Untracked
            // files are here too: they are uncommitted work, and a client
            // counting only what git can diff misses the file an agent just
            // wrote.
            //
            // **The third `.chain()` is load-bearing and it is the whole of a
            // number two apps draw.** `24f2c1d` added it and said so: on the
            // worked example the Uncommitted header moved from +1 to +4, a
            // deliberate deviation from that spec's own gate, because a header
            // that excludes a row drawn directly beneath it is the same species
            // of wrong as one that climbs while you scroll. Deleting
            // `untracked_files` here reverts it silently — the counts stay
            // whole, the untracked rows keep appearing, and only the total is
            // short. `the_uncommitted_total_counts_the_file_git_has_never_seen`
            // in `crates/cli/src/changes.rs` is what catches that.
            "changes": w.staged.iter().chain(w.unstaged.iter())
                .chain(w.untracked_files.iter()).map(file_change_json)
                .collect::<Vec<_>>(),
        })),
    })
}

/// The fleet inbox, in the shape `farcooler changes inbox --json` prints and
/// the FFI's `changes.inbox` returns.
///
/// The last of the `changes` answers to have two emitters, and the one they
/// disagreed about hardest. `c2f1117` moved the change-set builders here and
/// recorded this pair as "not the same shape at all — a bare array with a short
/// id against `{"items": …, "elsewhere": n}` with a full UUID", then left it
/// alone because unifying it breaks the Mac's decoder. It wanted its own change
/// and this is it.
///
/// The FFI's shape is the one that survived, on three counts and not because it
/// is older. It carries `elsewhere`, which the CLI printed to a person and
/// dropped from `--json` — a machine format strictly poorer than the human one
/// it sits beside, and the count exists precisely to admit that a triage list is
/// incomplete. It sends the full UUID under `workspace_id` and the short
/// separately, which is what every other `--json` in the CLI does (`id` plus
/// `short` on repositories, workspaces, terminals, layouts and the `watch`
/// event stream) — the inbox was the one place that broke the CLI's own
/// convention. And `watch --json` tells a script "this worktree's diff moved,
/// go re-read `changes inbox`" while handing it a full UUID, so the two
/// surfaces designed to be used together could not be joined on an id.
///
/// One key with two meanings was the `Workspace.repository` trap, and that one
/// cost a release (`07e75e8`). This is the same trap closed rather than
/// documented, because unlike `repository` neither meaning was load-bearing:
/// both halves fit in one object.
pub fn inbox_json(inbox: &pb::ChangesInbox) -> serde_json::Value {
    json!({
        "items": inbox.items.iter().map(inbox_row_json).collect::<Vec<_>>(),
        // Hard-coded to zero in `review_ops::inbox` today, with no path that
        // sets it, and on the wire anyway — the proto's argument is that a
        // triage surface hiding half the fleet is worse than one that says so.
        // Emitted rather than skipped so that the day the daemon computes it,
        // every reader already has the key. Android records the same waiting
        // reader on its side in `InboxReply`.
        "elsewhere": inbox.elsewhere,
    })
}

/// One worktree's line in that inbox.
///
/// `workspace_id` is the FULL UUID and `short` is the eight characters a person
/// types — the CLI used to send the short under `workspace_id` and no `short`
/// at all, so a reader could not tell which it had been given without measuring
/// the string.
pub fn inbox_row_json(w: &pb::InboxWorkspace) -> serde_json::Value {
    json!({
        "workspace_id": uuid_of(&w.workspace_id).to_string(),
        "short": short(&w.workspace_id),
        "task_name": w.task_name,
        "branch": w.branch,
        "changed_since_reviewed": w.changed_since_reviewed,
        "insertions": w.insertions,
        "deletions": w.deletions,
    })
}

/// One changed file, in the shape both clients decode.
///
/// Shared by `changes status --json`, `changes files --json` and the FFI's
/// `changes.commit_files`.
pub fn file_change_json(f: &pb::FileChange) -> serde_json::Value {
    json!({
        "path": f.path,
        "status": file_status_name(f.status),
        "old_path": f.old_path,
        "insertions": f.insertions,
        "deletions": f.deletions,
        "binary": f.binary,
    })
}

/// One file's patch, flattened out of its hunks.
///
/// The hunk boundaries survive as `gap` markers rather than as nesting: a
/// client draws a diff as one list and needs to know where the unchanged lines
/// were left out, which is exactly what the jump between two hunks' line
/// numbers says. `AgentKit.DiffComputation` already parses this shape.
pub fn file_diff_json(d: &pb::FileDiff) -> serde_json::Value {
    json!({
        "path": d.path,
        // Why there are no hunks, when the reason is not "nothing changed". An
        // empty list on its own reads as unchanged, which for a binary file is
        // a lie — see `DiffUnsupported` in the protocol.
        "unsupported": d.unsupported.and_then(|u| match pb::DiffUnsupported::try_from(u) {
            Ok(pb::DiffUnsupported::Binary) => Some("binary"),
            Ok(pb::DiffUnsupported::Submodule) => Some("submodule"),
            Ok(pb::DiffUnsupported::CombinedDiff) => Some("combined_diff"),
            Ok(pb::DiffUnsupported::Malformed) => Some("malformed"),
            _ => None,
        }),
        "truncated": d.truncated.is_some(),
        "firstParentOfMerge": d.first_parent_of_merge,
        "hunks": d.hunks.iter().map(|h| json!({
            "index": h.index,
            "header": h.header,
            "oldStart": h.old_start,
            "newStart": h.new_start,
            "lines": h.lines.iter().map(|l| json!({
                "kind": match pb::DiffLineKind::try_from(l.kind) {
                    Ok(pb::DiffLineKind::Added) => "added",
                    Ok(pb::DiffLineKind::Removed) => "removed",
                    _ => "context",
                },
                "oldNumber": l.old_no,
                "newNumber": l.new_no,
                "text": l.text,
                "noNewline": l.no_newline,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn file_status_name(status: i32) -> &'static str {
    match pb::FileStatus::try_from(status).unwrap_or(pb::FileStatus::Unspecified) {
        pb::FileStatus::Added => "added",
        pb::FileStatus::Modified => "modified",
        pb::FileStatus::Deleted => "deleted",
        pb::FileStatus::Renamed => "renamed",
        pb::FileStatus::Copied => "copied",
        pb::FileStatus::TypeChanged => "type_changed",
        pb::FileStatus::Untracked => "untracked",
        pb::FileStatus::Conflicted => "conflicted",
        pb::FileStatus::Unspecified => "modified",
    }
}

/// Where the base came from, because only one of these is a guess and only a
/// guess is worth warning about.
fn base_source_name(source: i32) -> &'static str {
    match pb::BaseSource::try_from(source).unwrap_or(pb::BaseSource::Unspecified) {
        pb::BaseSource::Recorded => "recorded",
        pb::BaseSource::Upstream => "upstream",
        pb::BaseSource::Guessed => "guessed",
        pb::BaseSource::PrBase => "pr_base",
        pb::BaseSource::DefaultBranch => "default_branch",
        pb::BaseSource::Unspecified => "unknown",
    }
}

/// A branch's place in its stack, and what GitHub says about it.
///
/// Here rather than in `session.rs` for the reason at the top of this file, and
/// because `changes stack --json` was silently printing the human form: the flag
/// parsed, the arm ignored it, and the Mac had no shape to read. That is the
/// same defect this module's header records about `changes diff`.
pub fn stack_json(l: &farcooler_protocol::v1::StackLinkList) -> serde_json::Value {
    use farcooler_protocol::v1::{CheckState, ParentSource, PrState, ReviewDecision};
    json!({
        // Reported rather than followed: a parent chain that loops is walked as
        // far as it was walked, and a client that silently drew it would draw a
        // stack that does not exist.
        "cycleDetected": l.cycle_detected,
        // Whether `gh` answered at all. "There is no pull request for this
        // branch" and "we could not ask GitHub" both arrive as an absent `pr`
        // on every link, and only this separates them.
        //
        // It decides whether an app may offer to CREATE a pull request. Offering
        // that while a PR exists behind a logged-out `gh` is the app being
        // confidently wrong about the one action on the row. False means show
        // neither PR state nor an offer: show nothing, say nothing.
        "prKnown": l.pr_known,
        // The repository's page on GitHub, for building a compare link for a
        // branch that has no PR yet. Null rather than "" when `gh` has not
        // answered for this repository yet — a client must get nothing rather
        // than a link to nowhere.
        "repoUrl": if l.repo_url.is_empty() { serde_json::Value::Null } else { json!(l.repo_url) },
        "links": l.items.iter().map(|k| json!({
            "branch": k.branch,
            "parentBranch": k.parent_branch,
            // Only a GUESS is worth labeling. The others are recorded facts;
            // a guessed parent produces a wrong diff and looks like a right one.
            "parentGuessed": ParentSource::try_from(k.parent_source) == Ok(ParentSource::Guessed),
            "ahead": k.ahead,
            "behind": k.behind,
            "pr": k.pr.as_ref().map(|p| json!({
                "number": p.number,
                "url": p.url,
                "state": match PrState::try_from(p.state) {
                    Ok(PrState::Open) => "open",
                    Ok(PrState::Draft) => "draft",
                    Ok(PrState::Merged) => "merged",
                    Ok(PrState::Closed) => "closed",
                    _ => "unknown",
                },
                "checks": match CheckState::try_from(p.checks) {
                    Ok(CheckState::Passing) => "passing",
                    Ok(CheckState::Failing) => "failing",
                    Ok(CheckState::Pending) => "pending",
                    _ => "unknown",
                },
                "review": match ReviewDecision::try_from(p.review_decision) {
                    Ok(ReviewDecision::Approved) => "approved",
                    Ok(ReviewDecision::ChangesRequested) => "changes_requested",
                    Ok(ReviewDecision::ReviewRequired) => "review_required",
                    _ => "unknown",
                },
                // The PR head as GitHub last saw it. The only thing that can
                // establish that a SQUASH-merged branch holds nothing unmerged:
                // its commits never appear in main with these SHAs, so local
                // ancestry can never prove it.
                "headOid": p.head_oid,
                // When it landed, in unix milliseconds, or null if it has not.
                "mergedAt": p.merged_at,
                // When the daemon last read this from GitHub, in unix
                // milliseconds. `stale` below is derived from it — by the
                // DAEMON, so three platforms cannot disagree about what "a
                // while ago" means — and this is here so a client can say how
                // long ago rather than only that it was a while.
                "fetchedAt": p.fetched_at,
                // Whether this was read from GitHub long enough ago to doubt.
                "stale": p.stale,
            })),
        })).collect::<Vec<_>>(),
    })
}
