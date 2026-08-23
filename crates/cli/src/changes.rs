//! `farcooler changes` — what a worktree changed.
//!
//! The Mac app drives the daemon through this CLI, so anything the app can do an
//! agent can do too. That is not incidental: a review surface only an app can
//! reach would make the one workflow this product is about the one thing it
//! cannot automate.

use clap::Subcommand;
use farcooler_client::changes_json::{change_set_json, file_change_json, file_diff_json};
use farcooler_protocol::v1::{self as pb, request, result};

use crate::{Fallible, connect_to, expect_value, req, req_for, short_bytes, uuid_of, with};

#[derive(Subcommand)]
pub enum ChangesCmd {
    /// What this worktree's branch changed.
    Status {
        workspace: String,
        /// Recompute rather than trusting the cache.
        #[arg(long)]
        fresh: bool,
    },
    /// One file's diff.
    Diff {
        workspace: String,
        path: String,
        /// A commit, instead of the whole branch.
        #[arg(long)]
        commit: Option<String>,
        /// The index against HEAD.
        #[arg(long, conflicts_with = "commit")]
        staged: bool,
        /// The worktree against the index.
        #[arg(long, conflicts_with_all = ["commit", "staged"])]
        unstaged: bool,
        /// The worktree against HEAD: everything uncommitted, staged or not.
        #[arg(long, conflicts_with_all = ["commit", "staged", "unstaged"])]
        local: bool,
        /// Lines of unchanged context around each hunk. git's default is 3.
        ///
        /// Ask for a large number to see what a diff leaves out — the lines
        /// between hunks — which is what the app's expand controls do.
        #[arg(long)]
        context: Option<u32>,
    },
    /// Which files a commit touched.
    Files { workspace: String, sha: String },
    /// Mark this worktree as read.
    Read { workspace: String },
    /// What has changed, across every worktree.
    ///
    /// The counts are everything the worktree has changed — committed work,
    /// uncommitted edits, and untracked files — which is deliberately more than
    /// `changes status` reports, whose `+N -M` is the branch's commits alone.
    /// The apps say the same thing on their own copies of this number: the
    /// Mac's sidebar in a tooltip, the phone in the row's spoken label.
    Inbox,
    /// The stack of branches containing this one, and their PRs.
    Stack {
        repo: String,
        branch: String,
        /// Ask GitHub again rather than using what was last read.
        #[arg(long)]
        refresh: bool,
    },
}

pub async fn changes(runner: Option<&str>, cmd: ChangesCmd, json: bool) -> Fallible {
    let mut link = connect_to(runner).await?;

    match cmd {
        ChangesCmd::Status { workspace, fresh } => {
            let id = crate::resolve_workspace_id(&mut link, &workspace).await?;
            let r = link
                .call(with(
                    req("changes.change_set"),
                    request::Payload::ChangeSetRequest(pb::ChangeSetRequest {
                        workspace_id: crate::id_bytes(id),
                        selector: None,
                        fresh,
                    }),
                ))
                .await?;
            let result::Value::ChangeSet(cs) = expect_value(r.value, "change_set")? else {
                return Err("the daemon returned the wrong resource".into());
            };

            if json {
                println!("{}", serde_json::to_string(&change_set_json(&cs))?);
                return Ok(());
            }

            println!("{}  vs {} ({})", cs.branch, cs.base_ref, &cs.base_commit[..8.min(cs.base_commit.len())]);
            let n = cs.files.len();
            println!(
                "  +{} -{} across {} {}",
                cs.insertions,
                cs.deletions,
                n,
                if n == 1 { "file" } else { "files" }
            );
            if !cs.commits.is_empty() {
                println!("\n  commits");
                for c in &cs.commits {
                    println!("    {}  {}", &c.sha[..8.min(c.sha.len())], c.subject);
                }
            }
            if let Some(wt) = &cs.working_tree {
                let dirty = !wt.staged.is_empty()
                    || !wt.unstaged.is_empty()
                    || !wt.untracked.is_empty()
                    || !wt.conflicted.is_empty();
                if dirty {
                    println!("\n  uncommitted");
                    for f in &wt.staged {
                        println!("    staged     {}", f.path);
                    }
                    for f in &wt.unstaged {
                        println!("    unstaged   {}", f.path);
                    }
                    for p in &wt.untracked {
                        println!("    untracked  {p}");
                    }
                    for p in &wt.conflicted {
                        println!("    CONFLICT   {p}");
                    }
                }
            }
            if !cs.files.is_empty() {
                println!("\n  files");
                for f in &cs.files {
                    let mark = if f.binary { " (binary)" } else { "" };
                    println!("    +{:<5} -{:<5} {}{}", f.insertions, f.deletions, f.path, mark);
                }
            }
        }

        ChangesCmd::Diff { workspace, path, commit, staged, unstaged, local, context } => {
            let id = crate::resolve_workspace_id(&mut link, &workspace).await?;
            let selector = pb::DiffSelector {
                kind: Some(match (&commit, staged, unstaged, local) {
                    (Some(sha), ..) => pb::diff_selector::Kind::Commit(sha.clone()),
                    (_, true, _, _) => pb::diff_selector::Kind::Staged(pb::Empty {}),
                    (_, _, true, _) => pb::diff_selector::Kind::Unstaged(pb::Empty {}),
                    (_, _, _, true) => pb::diff_selector::Kind::Local(pb::Empty {}),
                    _ => pb::diff_selector::Kind::Range(pb::Empty {}),
                }),
            };
            let r = link
                .call(with(
                    req("changes.file_diff"),
                    request::Payload::FileDiffRequest(pb::FileDiffRequest {
                        workspace_id: crate::id_bytes(id),
                        selector: Some(selector),
                        path: path.clone(),
                        from_hunk: 0,
                        context: context.unwrap_or(0),
                    }),
                ))
                .await?;
            let result::Value::FileDiff(d) = expect_value(r.value, "file_diff")? else {
                return Err("the daemon returned the wrong resource".into());
            };

            // The three things below this line that are not patch text —
            // `unsupported`, `first_parent_of_merge` and `truncated` — are
            // printed as prose a reader understands and a parser does not. The
            // Mac used to scrape the human output and so kept none of them,
            // which is worse than a missing feature: an empty hunk list with no
            // reason attached reads as "no textual changes", and it said that
            // about a submodule. `file_diff_json` is what the phones already
            // receive over the FFI, so both clients now decode one shape and
            // `AgentKit.DiffComputation` parses both.
            if json {
                println!("{}", serde_json::to_string(&file_diff_json(&d))?);
                return Ok(());
            }

            if let Some(u) = d.unsupported {
                let why = match pb::DiffUnsupported::try_from(u) {
                    Ok(pb::DiffUnsupported::Binary) => "binary file",
                    Ok(pb::DiffUnsupported::Submodule) => "submodule",
                    Ok(pb::DiffUnsupported::CombinedDiff) => {
                        "a merge commit — shown by its first parent"
                    }
                    _ => "the patch could not be read",
                };
                println!("{path}: {why}");
                return Ok(());
            }
            if d.first_parent_of_merge {
                println!("(a merge commit, shown against its first parent)\n");
            }
            for h in &d.hunks {
                println!("{}", h.header);
                for l in &h.lines {
                    let marker = match pb::DiffLineKind::try_from(l.kind) {
                        Ok(pb::DiffLineKind::Added) => '+',
                        Ok(pb::DiffLineKind::Removed) => '-',
                        _ => ' ',
                    };
                    println!("{marker}{}", l.text);
                }
            }
            if d.truncated.is_some() {
                println!("\n... truncated. More hunks remain.");
            }
        }

        ChangesCmd::Files { workspace, sha } => {
            let id = crate::resolve_workspace_id(&mut link, &workspace).await?;
            let r = link
                .call(with(
                    req("changes.commit_files"),
                    request::Payload::CommitFilesRequest(pb::CommitFilesRequest {
                        workspace_id: crate::id_bytes(id),
                        sha,
                    }),
                ))
                .await?;
            let result::Value::FileChangeList(l) = expect_value(r.value, "file_change_list")?
            else {
                return Err("the daemon returned the wrong resource".into());
            };
            if json {
                let files: Vec<_> = l.items.iter().map(file_change_json).collect();
                println!("{}", serde_json::to_string(&serde_json::json!({ "files": files }))?);
                return Ok(());
            }
            for f in &l.items {
                println!("+{:<5} -{:<5} {}", f.insertions, f.deletions, f.path);
            }
        }
        ChangesCmd::Read { workspace } => {
            let id = crate::resolve_workspace_id(&mut link, &workspace).await?;
            link.call(with(
                req("changes.mark_read"),
                request::Payload::ChangesMarkRead(pb::ChangesMarkRead {
                    workspace_id: crate::id_bytes(id),
                    branch: String::new(),
                }),
            ))
            .await?;
            println!("marked as read");
        }

        ChangesCmd::Inbox => {
            let r = link
                .call(with(req("changes.inbox"), request::Payload::ChangesInbox(pb::ChangesInboxRequest {})))
                .await?;
            let result::Value::ChangesInbox(inbox) = expect_value(r.value, "changes_inbox")? else {
                return Err("the daemon returned the wrong resource".into());
            };
            if json {
                let items: Vec<_> = inbox
                    .items
                    .iter()
                    .map(|w| {
                        serde_json::json!({
                            "workspace_id": short_bytes(&w.workspace_id),
                            "task_name": w.task_name,
                            "branch": w.branch,
                            "changed_since_reviewed": w.changed_since_reviewed,
                            "insertions": w.insertions,
                            "deletions": w.deletions,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string(&items)?);
                return Ok(());
            }
            if inbox.items.is_empty() {
                println!("nothing has changed");
                return Ok(());
            }
            for w in &inbox.items {
                let mut bits = Vec::new();
                if w.insertions > 0 || w.deletions > 0 {
                    bits.push(format!("+{} -{}", w.insertions, w.deletions));
                }
                if w.changed_since_reviewed {
                    bits.push("changed since you looked".into());
                }
                println!("{}  {}  {}", short_bytes(&w.workspace_id), w.task_name, bits.join(", "));
            }
            if inbox.elsewhere > 0 {
                println!("\n{} elsewhere, outside what this client may see", inbox.elsewhere);
            }
        }

        ChangesCmd::Stack { repo, branch, refresh } => {
            let repos = crate::list_repositories(&mut link).await?;
            let target = crate::resolve_repository(&repos, &repo)?;
            let payload = if refresh {
                request::Payload::PrRefresh(pb::PrRefresh {
                    repository_id: target.id.clone(),
                })
            } else {
                request::Payload::StackGet(pb::StackGet {
                    repository_id: target.id.clone(),
                    branch: branch.clone(),
                })
            };
            let method = if refresh { "pr.refresh" } else { "stack.get" };
            let r = link
                .call(with(req_for(method, uuid_of(&target.id)), payload))
                .await?;
            let result::Value::StackLinkList(l) = expect_value(r.value, "stack_link_list")? else {
                return Err("the daemon returned the wrong resource".into());
            };
            if l.cycle_detected {
                println!("WARNING: these branches list each other as parents. Showing what was walked.\n");
            }
            for link_ in &l.items {
                let src = match pb::ParentSource::try_from(link_.parent_source) {
                    Ok(pb::ParentSource::Guessed) => " (parent guessed)",
                    _ => "",
                };
                let pr = match &link_.pr {
                    Some(p) => {
                        let state = match pb::PrState::try_from(p.state) {
                            Ok(pb::PrState::Merged) => "merged",
                            Ok(pb::PrState::Open) => "open",
                            Ok(pb::PrState::Draft) => "draft",
                            Ok(pb::PrState::Closed) => "closed",
                            _ => "unknown",
                        };
                        format!("  #{} {}", p.number, state)
                    }
                    None => "  (no PR state — GitHub not read)".to_string(),
                };
                println!("{} <- {}{}{}", link_.branch, link_.parent_branch, src, pr);
                println!("    +{} ahead, {} behind", link_.ahead, link_.behind);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, status: pb::FileStatus, ins: u32, del: u32) -> pb::FileChange {
        pb::FileChange {
            path: path.to_string(),
            status: status as i32,
            old_path: None,
            insertions: ins,
            deletions: del,
            binary: false,
            submodule: false,
        }
    }

    /// The shape both apps decode. It is built in
    /// `farcooler_client::changes_json` and pinned here, on the side the Mac
    /// reads: this command is the Mac's whole view of a change set, so a key
    /// that stopped being printed would be a feature that stopped existing on
    /// one platform only.
    ///
    /// `changes` is what a client's Uncommitted total is a sum of. It carries
    /// counts because the daemon now has them — `change_set::apply_uncommitted_counts`
    /// — and it carries untracked files because a file an agent has just written
    /// is uncommitted work, whatever git can diff.
    #[test]
    fn the_working_tree_carries_every_dirty_path_with_its_counts() {
        let cs = pb::ChangeSet {
            working_tree: Some(pb::WorkingTree {
                staged: vec![file("staged.txt", pb::FileStatus::Added, 2, 0)],
                unstaged: vec![file("README.md", pb::FileStatus::Modified, 1, 3)],
                untracked: vec!["new.txt".to_string()],
                untracked_files: vec![file("new.txt", pb::FileStatus::Untracked, 3, 0)],
                conflicted: Vec::new(),
            }),
            ..Default::default()
        };

        let v = change_set_json(&cs);
        let changes = v["working_tree"]["changes"].as_array().expect("changes");
        assert_eq!(changes.len(), 3, "staged, unstaged, and untracked");

        let total: u64 = changes.iter().map(|c| c["insertions"].as_u64().unwrap()).sum();
        assert_eq!(total, 6);
        assert_eq!(changes[2]["path"], "new.txt");
        assert_eq!(changes[2]["status"], "untracked");
        assert_eq!(changes[2]["insertions"], 3);
        assert_eq!(changes[1]["deletions"], 3);
        assert_eq!(changes[0]["binary"], false);

        // The path lists an app in the field already decodes are unchanged.
        assert_eq!(v["working_tree"]["staged"][0], "staged.txt");
        assert_eq!(v["working_tree"]["untracked"][0], "new.txt");
    }

    /// A file that is staged and modified again is in both groups, once per
    /// diff of it. A client sums per path; dropping either row would report half
    /// the work.
    #[test]
    fn a_file_in_both_groups_appears_once_per_group() {
        let cs = pb::ChangeSet {
            working_tree: Some(pb::WorkingTree {
                staged: vec![file("a.rs", pb::FileStatus::Modified, 1, 0)],
                unstaged: vec![file("a.rs", pb::FileStatus::Modified, 4, 2)],
                ..Default::default()
            }),
            ..Default::default()
        };
        let v = change_set_json(&cs);
        let changes = v["working_tree"]["changes"].as_array().expect("changes");
        assert_eq!(changes.len(), 2);
        assert!(changes.iter().all(|c| c["path"] == "a.rs"));
    }

    /// The key that was missing for as long as there were two builders.
    ///
    /// A GUESSED base is the only source worth warning about — nothing knew
    /// what this branch came from, so a local `main` was used, and the diff
    /// that produces is wrong in a way that looks right. The daemon has said so
    /// since `BaseSource` existed; the phones were told and the Mac was not,
    /// because this command's copy of the builder never learned the key.
    #[test]
    fn the_base_says_where_it_came_from() {
        let guessed = pb::ChangeSet {
            base_ref: "main".to_string(),
            base_source: pb::BaseSource::Guessed as i32,
            ..Default::default()
        };
        assert_eq!(change_set_json(&guessed)["base_source"], "guessed");

        let pinned = pb::ChangeSet {
            base_source: pb::BaseSource::Recorded as i32,
            ..Default::default()
        };
        assert_eq!(change_set_json(&pinned)["base_source"], "recorded");

        // A runner older than the field sends the zero, and "unknown" is the
        // honest answer: not a guess, so not a warning.
        assert_eq!(change_set_json(&pb::ChangeSet::default())["base_source"], "unknown");
    }

    /// A submodule is not "no textual changes".
    ///
    /// The hunk list is empty for both, and the human output says which by
    /// printing a sentence instead of a patch — which is why scraping it lost
    /// the distinction and the Mac told people a submodule was unchanged.
    #[test]
    fn an_empty_diff_says_why_it_is_empty() {
        let d = pb::FileDiff {
            path: "vendor/thing".to_string(),
            unsupported: Some(pb::DiffUnsupported::Submodule as i32),
            ..Default::default()
        };
        let v = file_diff_json(&d);
        assert_eq!(v["path"], "vendor/thing");
        assert_eq!(v["unsupported"], "submodule");
        assert_eq!(v["hunks"].as_array().expect("hunks").len(), 0);

        // Nothing wrong with this one: empty really does mean unchanged.
        let plain = file_diff_json(&pb::FileDiff::default());
        assert!(plain["unsupported"].is_null());
        assert_eq!(plain["truncated"], false);
        assert_eq!(plain["firstParentOfMerge"], false);
    }

    /// The other two notices the pipe used to swallow.
    ///
    /// `truncated` means hunks remain and this is not the whole file;
    /// `firstParentOfMerge` means the patch is one parent's view of a merge.
    /// Both change what the diff on screen MEANS, so a client that cannot see
    /// them draws a confident half-truth.
    #[test]
    fn truncation_and_a_merges_first_parent_survive() {
        let d = pb::FileDiff {
            path: "src/main.rs".to_string(),
            truncated: Some(pb::Truncation::HunkCap as i32),
            first_parent_of_merge: true,
            hunks: vec![pb::Hunk {
                index: 0,
                header: "@@ -1,2 +1,3 @@".to_string(),
                old_start: 1,
                new_start: 1,
                lines: vec![pb::DiffLine {
                    kind: pb::DiffLineKind::Added as i32,
                    old_no: None,
                    new_no: Some(2),
                    text: "let x = 1;".to_string(),
                    no_newline: false,
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let v = file_diff_json(&d);
        assert_eq!(v["truncated"], true);
        assert_eq!(v["firstParentOfMerge"], true);

        let hunk = &v["hunks"][0];
        assert_eq!(hunk["header"], "@@ -1,2 +1,3 @@");
        assert_eq!(hunk["oldStart"], 1);
        let line = &hunk["lines"][0];
        assert_eq!(line["kind"], "added");
        assert!(line["oldNumber"].is_null());
        assert_eq!(line["newNumber"], 2);
        assert_eq!(line["text"], "let x = 1;");
    }
}
