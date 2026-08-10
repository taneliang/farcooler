//! `farcooler changes` — what a worktree changed.
//!
//! The Mac app drives the daemon through this CLI, so anything the app can do an
//! agent can do too. That is not incidental: a review surface only an app can
//! reach would make the one workflow this product is about the one thing it
//! cannot automate.

use clap::Subcommand;
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

pub async fn changes(host: Option<&str>, cmd: ChangesCmd, json: bool) -> Fallible {
    let mut link = connect_to(host).await?;

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

fn change_set_json(cs: &pb::ChangeSet) -> serde_json::Value {
    serde_json::json!({
        "branch": cs.branch,
        "base_ref": cs.base_ref,
        "base_commit": cs.base_commit,
        "head_commit": cs.head_commit,
        "insertions": cs.insertions,
        "deletions": cs.deletions,
        "commits": cs.commits.iter().map(|c| serde_json::json!({
            "sha": c.sha, "subject": c.subject, "author": c.author, "timestamp": c.timestamp,
        })).collect::<Vec<_>>(),
        "files": cs.files.iter().map(|f| serde_json::json!({
            "path": f.path, "insertions": f.insertions, "deletions": f.deletions,
            "binary": f.binary,
        })).collect::<Vec<_>>(),
        "working_tree": cs.working_tree.as_ref().map(|w| serde_json::json!({
            "staged": w.staged.iter().map(|f| &f.path).collect::<Vec<_>>(),
            "unstaged": w.unstaged.iter().map(|f| &f.path).collect::<Vec<_>>(),
            "untracked": w.untracked,
            "conflicted": w.conflicted,
        })),
    })
}

