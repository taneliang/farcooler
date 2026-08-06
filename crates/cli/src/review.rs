//! `farcooler review` — what changed, what you said about it, and sending it on.
//!
//! The Mac app drives the daemon through this CLI, so anything the app can do an
//! agent can do too. That is not incidental: a review surface only an app can
//! reach would make the one workflow this product is about the one thing it
//! cannot automate.

use clap::Subcommand;
use farcooler_protocol::v1::{self as pb, request, result};

use crate::{Fallible, connect_to, expect_value, req, req_for, short_bytes, uuid_of, with};

#[derive(Subcommand)]
pub enum ReviewCmd {
    /// What this workspace's branch changed.
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
    },
    /// Which files a commit touched.
    Files { workspace: String, sha: String },
    /// Write a comment into the buffer.
    ///
    /// With no `--file`, the comment is about the workspace rather than any
    /// particular place — which is what most review comments actually are.
    Note {
        workspace: String,
        body: String,
        /// Attach it to a file.
        #[arg(long)]
        file: Option<String>,
        /// Attach it to exact lines, given as their text.
        ///
        /// Text, not a line number: an agent is editing underneath you, and a
        /// number is stale by the time you have finished typing.
        #[arg(long, requires = "file")]
        lines: Option<String>,
        /// A question rather than an instruction.
        #[arg(long)]
        ask: bool,
    },
    /// Everything in this workspace's buffer.
    List { workspace: String },
    /// Drop a comment.
    Drop { workspace: String, entry: String },
    /// Send comments to a terminal.
    ///
    /// The terminal is yours to pick, including one already working: there is no
    /// such thing as an ask terminal, because the terminal that just answered a
    /// question is very often the one you then tell to fix the thing.
    Send {
        workspace: String,
        terminal: String,
        /// Only these entries. Defaults to everything still open.
        #[arg(long)]
        entry: Vec<String>,
        /// Send them as questions.
        #[arg(long)]
        ask: bool,
    },
    /// Mark this workspace as read.
    Seen { workspace: String },
    /// What needs you, across every workspace.
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

fn anchor_json(file: Option<&str>, lines: Option<&str>) -> String {
    match (file, lines) {
        (Some(path), Some(text)) => serde_json::json!({
            "kind": "lines",
            "path": path,
            "side": "new",
            "text": text,
            // Left empty: the daemon fingerprints the surroundings at capture,
            // and a caller inventing one here would only be able to get it wrong.
            "context_fingerprint": ""
        })
        .to_string(),
        (Some(path), None) => {
            serde_json::json!({ "kind": "file", "path": path }).to_string()
        }
        _ => serde_json::json!({ "kind": "none" }).to_string(),
    }
}

fn state_word(s: i32) -> &'static str {
    match pb::AnchorState::try_from(s) {
        Ok(pb::AnchorState::Exact) => "",
        Ok(pb::AnchorState::Moved) => " (moved)",
        Ok(pb::AnchorState::Ambiguous) => " (ambiguous — degraded to the file)",
        Ok(pb::AnchorState::NeedsReread) => " (needs re-read)",
        Ok(pb::AnchorState::Outdated) => " (outdated)",
        Ok(pb::AnchorState::FileGone) => " (file gone)",
        _ => "",
    }
}

fn status_word(s: i32) -> &'static str {
    match pb::EntryStatus::try_from(s) {
        Ok(pb::EntryStatus::Open) => "open",
        Ok(pb::EntryStatus::Dispatched) => "sent",
        Ok(pb::EntryStatus::Answered) => "answered",
        Ok(pb::EntryStatus::Resolved) => "done",
        Ok(pb::EntryStatus::DispatchUnknown) => "UNKNOWN",
        _ => "?",
    }
}

pub async fn review(host: Option<&str>, cmd: ReviewCmd, json: bool) -> Fallible {
    let mut link = connect_to(host).await?;

    match cmd {
        ReviewCmd::Status { workspace, fresh } => {
            let id = crate::resolve_workspace_id(&mut link, &workspace).await?;
            let r = link
                .call(with(
                    req("review.change_set"),
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
            println!("  +{} -{} across {} files", cs.insertions, cs.deletions, cs.files.len());
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

        ReviewCmd::Diff { workspace, path, commit, staged, unstaged } => {
            let id = crate::resolve_workspace_id(&mut link, &workspace).await?;
            let selector = pb::DiffSelector {
                kind: Some(match (&commit, staged, unstaged) {
                    (Some(sha), _, _) => pb::diff_selector::Kind::Commit(sha.clone()),
                    (_, true, _) => pb::diff_selector::Kind::Staged(pb::Empty {}),
                    (_, _, true) => pb::diff_selector::Kind::Unstaged(pb::Empty {}),
                    _ => pb::diff_selector::Kind::Range(pb::Empty {}),
                }),
            };
            let r = link
                .call(with(
                    req("review.file_diff"),
                    request::Payload::FileDiffRequest(pb::FileDiffRequest {
                        workspace_id: crate::id_bytes(id),
                        selector: Some(selector),
                        path: path.clone(),
                        from_hunk: 0,
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

        ReviewCmd::Files { workspace, sha } => {
            let id = crate::resolve_workspace_id(&mut link, &workspace).await?;
            let r = link
                .call(with(
                    req("review.commit_files"),
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

        ReviewCmd::Note { workspace, body, file, lines, ask } => {
            let id = crate::resolve_workspace_id(&mut link, &workspace).await?;
            let r = link
                .call(with(
                    req("review.capture"),
                    request::Payload::ReviewCapture(pb::ReviewCapture {
                        workspace_id: crate::id_bytes(id),
                        body,
                        disposition: if ask {
                            pb::Disposition::Ask as i32
                        } else {
                            pb::Disposition::Fix as i32
                        },
                        anchor_json: anchor_json(file.as_deref(), lines.as_deref()),
                        attachment_ids: Vec::new(),
                    }),
                ))
                .await?;
            let result::Value::ReviewEntry(e) = expect_value(r.value, "review_entry")? else {
                return Err("the daemon returned the wrong resource".into());
            };
            println!("noted {}", short_bytes(&e.id));
        }

        ReviewCmd::List { workspace } => {
            let id = crate::resolve_workspace_id(&mut link, &workspace).await?;
            let entries = list_entries(&mut link, id).await?;

            if json {
                let items: Vec<_> = entries.iter().map(entry_json).collect();
                println!("{}", serde_json::to_string(&items)?);
                return Ok(());
            }

            if entries.is_empty() {
                println!("nothing in the buffer yet");
                return Ok(());
            }
            for e in &entries {
                let anchor: serde_json::Value =
                    serde_json::from_str(&e.effective_anchor_json).unwrap_or_default();
                let where_ = match anchor.get("kind").and_then(|k| k.as_str()) {
                    Some("file") | Some("lines") | Some("hunk") => {
                        let p = anchor.get("path").and_then(|p| p.as_str()).unwrap_or("");
                        match e.line {
                            Some(n) => format!("{p}:{n}"),
                            None => p.to_string(),
                        }
                    }
                    Some("commit") => "a commit".to_string(),
                    _ => "no specific location".to_string(),
                };
                let kind = match pb::Disposition::try_from(e.disposition) {
                    Ok(pb::Disposition::Ask) => "ask",
                    _ => "fix",
                };
                println!(
                    "{}  [{}] {} — {}{}",
                    short_bytes(&e.id),
                    status_word(e.status),
                    kind,
                    where_,
                    state_word(e.anchor_state)
                );
                for line in e.body.lines() {
                    println!("      {line}");
                }
                if let Some(a) = &e.answer_text {
                    let label = match e.answer_correlation.as_deref() {
                        Some("uncorrelated") => "  answer (could not be matched to this question)",
                        _ => "  answer",
                    };
                    println!("  {label}");
                    for line in a.lines() {
                        println!("      {line}");
                    }
                }
            }
        }

        ReviewCmd::Drop { workspace, entry } => {
            // Resolved by prefix against the workspace's own buffer, because a
            // short id is what `review list` prints and therefore what anyone
            // will type. Ambiguity is refused rather than resolved to the first
            // match — deleting the wrong comment is silent.
            let ws = crate::resolve_workspace_id(&mut link, &workspace).await?;
            let entries = list_entries(&mut link, ws).await?;
            let matches: Vec<&pb::ReviewEntry> = entries
                .iter()
                .filter(|e| short_bytes(&e.id).starts_with(&entry))
                .collect();
            let target = match matches.as_slice() {
                [one] => *one,
                [] => return Err(format!("no comment matching {entry:?}").into()),
                many => {
                    return Err(
                        format!("{entry:?} matches {} comments, be more specific", many.len())
                            .into(),
                    );
                }
            };
            link.call(with(
                req("review.delete"),
                request::Payload::ReviewDelete(pb::ReviewDelete { id: target.id.clone() }),
            ))
            .await?;
            println!("dropped {}", short_bytes(&target.id));
        }

        ReviewCmd::Send { workspace, terminal, entry, ask } => {
            let ws = crate::resolve_workspace_id(&mut link, &workspace).await?;
            let term = crate::resolve_terminal_id(&mut link, &terminal).await?;
            let all = list_entries(&mut link, ws).await?;

            let wanted: Vec<&pb::ReviewEntry> = if entry.is_empty() {
                all.iter()
                    .filter(|e| {
                        e.status == pb::EntryStatus::Open as i32
                            || e.status == pb::EntryStatus::Answered as i32
                    })
                    .collect()
            } else {
                all.iter()
                    .filter(|e| entry.iter().any(|w| short_bytes(&e.id).starts_with(w)))
                    .collect()
            };

            if wanted.is_empty() {
                return Err("no entries to send".into());
            }

            let entries: Vec<pb::DispatchEntry> = wanted
                .iter()
                .map(|e| pb::DispatchEntry {
                    id: e.id.clone(),
                    expected_resource_version: e.resource_version,
                })
                .collect();

            let r = link
                .call(with(
                    req("review.dispatch"),
                    request::Payload::ReviewDispatch(pb::ReviewDispatchRequest {
                        workspace_id: crate::id_bytes(ws),
                        terminal_id: crate::id_bytes(term),
                        disposition: if ask {
                            pb::Disposition::Ask as i32
                        } else {
                            pb::Disposition::Fix as i32
                        },
                        entries,
                    }),
                ))
                .await?;
            let result::Value::ReviewDispatchResult(d) =
                expect_value(r.value, "review_dispatch")?
            else {
                return Err("the daemon returned the wrong resource".into());
            };
            println!("sent {} to {}", d.entry_ids.len(), short_bytes(&d.terminal_id));
            if !d.skipped_entry_ids.is_empty() {
                println!("  skipped {} already sent", d.skipped_entry_ids.len());
            }
            println!("\n{}", d.prompt);
        }

        ReviewCmd::Seen { workspace } => {
            let id = crate::resolve_workspace_id(&mut link, &workspace).await?;
            link.call(with(
                req("review.mark_reviewed"),
                request::Payload::ReviewMarkReviewed(pb::ReviewMarkReviewed {
                    workspace_id: crate::id_bytes(id),
                    branch: String::new(),
                }),
            ))
            .await?;
            println!("marked as read");
        }

        ReviewCmd::Inbox => {
            let r = link
                .call(with(req("review.inbox"), request::Payload::ReviewInbox(pb::ReviewInboxRequest {})))
                .await?;
            let result::Value::ReviewInbox(inbox) = expect_value(r.value, "review_inbox")? else {
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
                            "open": w.open,
                            "dispatched": w.dispatched,
                            "answered": w.answered,
                            "dispatch_unknown": w.dispatch_unknown,
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
                println!("nothing needs you");
                return Ok(());
            }
            for w in &inbox.items {
                let mut bits = Vec::new();
                if w.open > 0 {
                    bits.push(format!("{} open", w.open));
                }
                if w.answered > 0 {
                    bits.push(format!("{} answered", w.answered));
                }
                if w.dispatch_unknown > 0 {
                    bits.push(format!("{} UNKNOWN", w.dispatch_unknown));
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

        ReviewCmd::Stack { repo, branch, refresh } => {
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

async fn list_entries(
    link: &mut crate::Link,
    workspace: uuid::Uuid,
) -> Result<Vec<pb::ReviewEntry>, Box<dyn std::error::Error>> {
    let r = link
        .call(with(
            req("review.list"),
            request::Payload::ReviewList(pb::ReviewList {
                workspace_id: crate::id_bytes(workspace),
            }),
        ))
        .await?;
    let result::Value::ReviewEntryList(l) = expect_value(r.value, "review_entry_list")? else {
        return Err("the daemon returned the wrong resource".into());
    };
    Ok(l.items)
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

fn entry_json(e: &pb::ReviewEntry) -> serde_json::Value {
    serde_json::json!({
        "id": short_bytes(&e.id),
        "body": e.body,
        "status": status_word(e.status),
        "disposition": match pb::Disposition::try_from(e.disposition) {
            Ok(pb::Disposition::Ask) => "ask",
            Ok(pb::Disposition::Note) => "note",
            _ => "fix",
        },
        "anchor": serde_json::from_str::<serde_json::Value>(&e.effective_anchor_json)
            .unwrap_or_default(),
        "anchor_state": state_word(e.anchor_state).trim(),
        "line": e.line,
        "answer": e.answer_text,
        "answer_correlation": e.answer_correlation,
        "resource_version": e.resource_version,
    })
}
