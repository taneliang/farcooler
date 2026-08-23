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
//! The shapes are pinned by tests on the side that decodes them — see
//! `crates/cli/src/changes.rs`.

use farcooler_protocol::v1 as pb;
use serde_json::json;

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
            "changes": w.staged.iter().chain(w.unstaged.iter())
                .chain(w.untracked_files.iter()).map(file_change_json)
                .collect::<Vec<_>>(),
        })),
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
