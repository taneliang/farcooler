//! `farcooler changes` — what a worktree changed.
//!
//! The Mac app drives the daemon through this CLI, so anything the app can do an
//! agent can do too. That is not incidental: a review surface only an app can
//! reach would make the one workflow this product is about the one thing it
//! cannot automate.

use clap::Subcommand;
use farcooler_client::changes_json::{
    change_set_json, file_change_json, file_diff_json, inbox_json, stack_json,
};
use farcooler_protocol::v1::{self as pb, request, result};

use farcooler_transport::ClientError;

use crate::tasks::{DispatchLink, Refused};
use crate::{Fallible, connect_to, expect_value, req, req_for, short_bytes, uuid_of, with};

#[derive(Subcommand)]
pub enum ChangesCmd {
    /// What this worktree's branch changed.
    Status {
        worktree: String,
        /// Recompute rather than trusting the cache.
        #[arg(long)]
        fresh: bool,
    },
    /// One file's diff.
    Diff {
        worktree: String,
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
    Files { worktree: String, sha: String },
    /// Mark this worktree as read.
    Read { worktree: String },
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
    changes_over(&mut link, cmd, json).await
}

/// `changes`, over a link already made, with every refusal in this CLI's words.
///
/// One place, rather than a `map_err` at each call: every arm here makes two
/// or three calls (the worktree list, then the one it came for), and a call
/// site that kept its bare `?` would print the runner's log line again. That
/// is what each of these did, all of them: "resource not found", "invalid
/// argument: base_ref", "operation failed", "Broken pipe (os error 32)".
async fn changes_over<L: DispatchLink>(link: &mut L, cmd: ChangesCmd, json: bool) -> Fallible {
    let about = About::of(&cmd);
    answer(link, cmd, json).await.map_err(|e| plainly(e, about))
}

async fn answer<L: DispatchLink>(link: &mut L, cmd: ChangesCmd, json: bool) -> Fallible {
    match cmd {
        ChangesCmd::Status { worktree, fresh } => {
            let id = crate::resolve_worktree_id(link, &worktree).await?;
            let r = link
                .call(with(
                    req("changes.change_set"),
                    request::Payload::ChangeSetRequest(pb::ChangeSetRequest {
                        worktree_id: crate::id_bytes(id),
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

        ChangesCmd::Diff { worktree, path, commit, staged, unstaged, local, context } => {
            let id = crate::resolve_worktree_id(link, &worktree).await?;
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
                        worktree_id: crate::id_bytes(id),
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

        ChangesCmd::Files { worktree, sha } => {
            let id = crate::resolve_worktree_id(link, &worktree).await?;
            let r = link
                .call(with(
                    req("changes.commit_files"),
                    request::Payload::CommitFilesRequest(pb::CommitFilesRequest {
                        worktree_id: crate::id_bytes(id),
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
        ChangesCmd::Read { worktree } => {
            let id = crate::resolve_worktree_id(link, &worktree).await?;
            link.call(with(
                req("changes.mark_read"),
                request::Payload::ChangesMarkRead(pb::ChangesMarkRead {
                    worktree_id: crate::id_bytes(id),
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
            // The same object the FFI's `changes.inbox` returns, out of one
            // builder, since the two stopped being different shapes. This
            // printed a bare array whose `worktree_id` was the eight-character
            // short, with no `short` key and no `elsewhere` — the one `--json`
            // in this CLI that did not send an id alongside its short, and the
            // only one anywhere that told a person something it withheld from a
            // script: `elsewhere` is printed to a person further down this arm
            // and used to be dropped here. See `changes_json::inbox_json`.
            if json {
                println!("{}", serde_json::to_string(&inbox_json(&inbox))?);
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
                println!("{}  {}  {}", short_bytes(&w.worktree_id), w.task_name, bits.join(", "));
            }
            if inbox.elsewhere > 0 {
                println!("\n{} elsewhere, outside what this client may see", inbox.elsewhere);
            }
        }

        ChangesCmd::Stack { repo, branch, refresh } => {
            let repos = crate::list_repositories(link).await?;
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
            // `--json` parsed here and then did nothing: the flag was accepted,
            // this arm printed the human form regardless, and anything reading
            // it had to scrape sentences. The shape comes from
            // `changes_json::stack_json`, the same one the phones decode, so
            // the two cannot drift — which is what that module exists for.
            if json {
                println!("{}", serde_json::to_string(&stack_json(&l))?);
                return Ok(());
            }
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
                        let stale = if p.stale { " (read a while ago)" } else { "" };
                        format!("  #{} {}{}", p.number, state, stale)
                    }
                    // Two different facts, and they used to print as one. A
                    // branch GitHub says has no pull request is not a branch we
                    // failed to ask about, and only the first is a branch it
                    // would help to open a PR for.
                    None if l.pr_known => "  (GitHub has no PR for this branch)".to_string(),
                    None => "  (no PR state — GitHub could not be read)".to_string(),
                };
                println!("{} <- {}{}{}", link_.branch, link_.parent_branch, src, pr);
                println!("    +{} ahead, {} behind", link_.ahead, link_.behind);
            }
        }
    }
    Ok(())
}

/// What a `changes` command was asking about, so that a refusal can say
/// which thing it refused.
///
/// The runner's code alone can't: `not-found` from `changes status` is a
/// worktree that has gone, and from `changes stack` it is a repository with
/// no worktree left on disk to run git in (`review_ops::any_worktree`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum About {
    /// One worktree's changes: `status`, `diff` and `read`.
    Worktree,
    /// A commit in a worktree: `files`, and `diff --commit`.
    Commit,
    /// Every worktree at once: `inbox`.
    Fleet,
    /// A repository's branches: `stack`.
    Repository,
}

impl About {
    fn of(cmd: &ChangesCmd) -> About {
        match cmd {
            ChangesCmd::Status { .. } | ChangesCmd::Read { .. } => About::Worktree,
            ChangesCmd::Diff { commit: Some(_), .. } | ChangesCmd::Files { .. } => About::Commit,
            ChangesCmd::Diff { .. } => About::Worktree,
            ChangesCmd::Inbox => About::Fleet,
            ChangesCmd::Stack { .. } => About::Repository,
        }
    }
}

/// A failure from `answer`, with the runner's refusals said in this CLI's
/// words and everything else left as it was.
///
/// "Everything else" is this CLI's own sentences ("no worktree matching
/// \"x\"", from `resolve`), which are already written for a person.
fn plainly(error: Box<dyn std::error::Error>, about: About) -> Box<dyn std::error::Error> {
    match error.downcast::<ClientError>() {
        Ok(err) => Box::new(refusal(*err, about)),
        Err(other) => other,
    }
}

/// A refusal from the runner, in this CLI's words, keeping the runner's code.
///
/// The board's `tasks::refusal` for these commands, and for the same reasons:
/// the runner's `message` is its log line ("resource not found", "invalid
/// argument: base_ref", "operation failed") and is never printed; the code
/// is kept, so `--json` still ends with `code: <word>` for a script, and the
/// Mac's changes pane, which shows this line under its own heading, shows a
/// sentence. The board's sentences are about tasks ("that task is not on
/// this runner"), so these commands have their own.
///
/// In clap's style, after its `error: `: lowercase, no closing full stop.
fn refusal(err: ClientError, about: About) -> Refused {
    let (code, what) = match err {
        ClientError::Daemon { code, what, .. } => (code, what),
        // A closed socket or a garbled frame reads the same whatever was
        // asked, so the board's sentences answer it, uncoded.
        other => return crate::tasks::refusal(other, ""),
    };
    let word = farcooler_core::error::word_for(code);
    let said: String = match (word, about) {
        ("not-found", About::Worktree | About::Commit) => {
            "that worktree is no longer on this runner".into()
        }
        ("not-found", About::Repository) => {
            "none of that repository's worktrees is on this runner's disk, so its branches can't be read"
                .into()
        }
        ("operation-failed", About::Worktree) => "git couldn't read this worktree's changes".into(),
        ("operation-failed", About::Commit) => "git couldn't read that commit in this worktree".into(),
        ("operation-failed", About::Fleet) => "the runner couldn't read what its worktrees changed".into(),
        ("operation-failed", About::Repository) => "git couldn't read this repository's branches".into(),
        ("base-unresolvable", _) => {
            "couldn't work out which branch this worktree is compared against".into()
        }
        // This command's own words first, then the board's (which knows
        // `worktree_id` and friends), then the runner's sentence for a word
        // newer than this build, as the board does. Never the word itself.
        ("invalid-argument", _) => said_about(&what)
            .or_else(|| crate::tasks::said_about(&what))
            .or_else(|| farcooler_core::error::sentence(&what))
            .map(str::to_string)
            .unwrap_or_else(|| format!("the runner refused that ({word})")),
        ("diff-too-large", _) => "that diff is too large to send at once".into(),
        ("diff-unsupported", _) => "that file has no diff to show".into(),
        ("pr-state-unavailable", _) => {
            "couldn't read pull requests from GitHub. try again in a moment".into()
        }
        ("scope-denied", _) => "this client isn't allowed to do that on this runner".into(),
        ("auth-required", _) => "this client isn't paired with the runner".into(),
        ("capability-unsupported", _) => {
            "this runner's Far Cooler is older than this command. update it and try again".into()
        }
        // The machine word rather than the runner's prose, as the board says
        // it: a stable vocabulary, and what a bug report needs.
        (other, _) => format!("the runner refused that ({other})"),
    };
    Refused::new(said, Some(code))
}

/// This command's sentence for an argument the runner named.
///
/// `base_ref` is `change_set::merge_base` failing: the base the runner chose
/// (pinned, a pull request's base, or the default branch) either isn't in
/// this worktree's checkout — a pull request's base that was never fetched —
/// or shares no history with the branch.
fn said_about(what: &str) -> Option<&'static str> {
    Some(match what {
        "base_ref" => {
            "the branch this worktree is compared against isn't in its checkout, or shares no history with it"
        }
        _ => return None,
    })
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

    /// The worked example's Uncommitted total, at the layer that decides it.
    ///
    /// Base `main`, one commit adding two lines, one further uncommitted line
    /// in that same file, one new untracked file of three lines. Uncommitted is
    /// **+4** — "what is not committed yet", one tracked line plus the whole of
    /// a file git has never seen — and that is a number `24f2c1d` deliberately
    /// moved from +1, against its own spec's gate, then wrote down as reverting
    /// by dropping one `.chain()`.
    ///
    /// `the_sidebar_and_the_panel_keep_answering_their_own_questions` pins the
    /// same example on the daemon's records and `UncommittedCountsTests` pins
    /// the Mac's sum of it. This is the link between them: the `.chain()` lives
    /// in `changes_json` and nothing else asserts that an untracked file's
    /// lines survive it into the total. A reader dropping it sees rows that
    /// still list the file and a header that quietly stops counting it.
    #[test]
    fn the_uncommitted_total_counts_the_file_git_has_never_seen() {
        let cs = pb::ChangeSet {
            insertions: 2,
            working_tree: Some(pb::WorkingTree {
                unstaged: vec![file("a.txt", pb::FileStatus::Modified, 1, 0)],
                untracked: vec!["new.txt".to_string()],
                untracked_files: vec![file("new.txt", pb::FileStatus::Untracked, 3, 0)],
                ..Default::default()
            }),
            ..Default::default()
        };

        let v = change_set_json(&cs);
        let uncommitted: u64 = v["working_tree"]["changes"]
            .as_array()
            .expect("changes")
            .iter()
            .map(|c| c["insertions"].as_u64().unwrap())
            .sum();
        assert_eq!(uncommitted, 4, "one tracked line and the whole of new.txt");

        // The other two totals are NOT equalized with it. The panel's Branch
        // segment is committed work and stays +2; telling the three apart is
        // the property, not making them agree.
        assert_eq!(v["insertions"], 2, "the branch's own commits, and nothing else");
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

    /// The inbox, transcribed key for key, on both sides of the id.
    ///
    /// This command and the FFI's `changes.inbox` are one builder now, and
    /// three clients decode it: the Mac shells out to this, iOS and Android call
    /// the FFI. So a key renamed here is a front door emptied there, which is
    /// the failure `NeedsYouTest` pins on the Kotlin side and this pins on the
    /// producer's.
    ///
    /// The two that moved, and the reason the test names them: `worktree_id`
    /// was the SHORT id in this command and the full UUID in the FFI — the same
    /// key with two meanings, which is the shape that cost `07e75e8` a release
    /// under the name `Worktree.repository`. And `elsewhere` was printed to a
    /// person by this very command and dropped from its `--json`.
    #[test]
    fn the_inbox_is_one_shape_with_the_id_both_ways_round() {
        let id = uuid::Uuid::parse_str("018f7c1e-0000-7000-8000-0123456789ab").expect("uuid");
        let inbox = pb::ChangesInbox {
            items: vec![pb::InboxWorktree {
                worktree_id: bytes::Bytes::copy_from_slice(id.as_bytes()),
                task_name: "ship the thing".to_string(),
                branch: "feature".to_string(),
                changed_since_reviewed: true,
                insertions: 12,
                deletions: 3,
            }],
            elsewhere: 2,
        };

        let v = inbox_json(&inbox);
        let row = &v["items"][0];
        assert_eq!(row["worktree_id"], id.to_string(), "the full UUID, hyphens and all");
        // The last eight hex of the simple form: a UUIDv7 leads with a
        // timestamp, so the tail is the half that tells two of them apart.
        assert_eq!(row["short"], "456789ab");
        assert_eq!(row["task_name"], "ship the thing");
        assert_eq!(row["branch"], "feature");
        assert_eq!(row["changed_since_reviewed"], true);
        assert_eq!(row["insertions"], 12);
        assert_eq!(row["deletions"], 3);
        assert_eq!(row.as_object().expect("row").len(), 7, "seven keys, no more");

        // The count that says the list is not the whole fleet. Zero out of the
        // daemon today, and a key a script can read either way.
        assert_eq!(v["elsewhere"], 2);
        assert_eq!(v.as_object().expect("envelope").len(), 2);
    }

    /// An empty inbox is an empty LIST, not an absent one.
    ///
    /// A client that reads `items` off a bare `{}` gets nothing and a client
    /// that reads it off `{"items": []}` gets nothing, but only one of them can
    /// tell "nothing has changed" from "the call failed".
    #[test]
    fn an_empty_inbox_still_has_both_keys() {
        let v = inbox_json(&pb::ChangesInbox::default());
        assert_eq!(v["items"].as_array().expect("items").len(), 0);
        assert_eq!(v["elsewhere"], 0);
    }

    // -----------------------------------------------------------------------
    // Refusals
    // -----------------------------------------------------------------------

    use uuid::Uuid;

    /// What the runner logs for each of these, which is what `changes`
    /// printed after `error: ` until it had sentences of its own.
    const LOG_LINE: &str = "LOG LINE: resource version is stale";

    fn daemon(code: pb::ErrorCode, what: &str) -> ClientError {
        ClientError::Daemon {
            code: code as i32,
            retryable: false,
            message: LOG_LINE.into(),
            what: what.into(),
        }
    }

    /// Every refusal a `changes` command can meet, pinned sentence by
    /// sentence, each with the runner's code kept for `--json`.
    ///
    /// The producers, read off the runner: `rpc.rs` for scope, pairing and
    /// an unknown method; `review_ops.rs` for not-found (the worktree, or a
    /// repository with none on disk) and base-unresolvable; `change_set.rs`,
    /// `file_diff.rs` and `stack.rs` for operation-failed and `base_ref`. The
    /// review family's other three are defined for this surface and produced
    /// by nothing today, so a runner that starts sending them is already
    /// answered.
    #[test]
    fn each_refusal_a_changes_command_can_meet_has_its_own_line() {
        use pb::ErrorCode as C;
        let table: &[(About, C, &str, &str)] = &[
            (About::Worktree, C::NotFound, "", "that worktree is no longer on this runner"),
            (About::Commit, C::NotFound, "", "that worktree is no longer on this runner"),
            (
                About::Repository,
                C::NotFound,
                "",
                "none of that repository's worktrees is on this runner's disk, so its branches can't be read",
            ),
            (About::Fleet, C::NotFound, "", "the runner refused that (not-found)"),
            (About::Worktree, C::OperationFailed, "", "git couldn't read this worktree's changes"),
            (About::Commit, C::OperationFailed, "", "git couldn't read that commit in this worktree"),
            (About::Fleet, C::OperationFailed, "", "the runner couldn't read what its worktrees changed"),
            (About::Repository, C::OperationFailed, "", "git couldn't read this repository's branches"),
            (
                About::Worktree,
                C::BaseUnresolvable,
                "",
                "couldn't work out which branch this worktree is compared against",
            ),
            (
                About::Worktree,
                C::InvalidArgument,
                "base_ref",
                "the branch this worktree is compared against isn't in its checkout, or shares no history with it",
            ),
            (About::Commit, C::DiffTooLarge, "", "that diff is too large to send at once"),
            (About::Worktree, C::DiffUnsupported, "", "that file has no diff to show"),
            (
                About::Repository,
                C::PrStateUnavailable,
                "",
                "couldn't read pull requests from GitHub. try again in a moment",
            ),
            (About::Worktree, C::ScopeDenied, "", "this client isn't allowed to do that on this runner"),
            (About::Fleet, C::AuthRequired, "", "this client isn't paired with the runner"),
            (
                About::Fleet,
                C::CapabilityUnsupported,
                "",
                "this runner's Far Cooler is older than this command. update it and try again",
            ),
            (About::Worktree, C::ResourceConflict, "", "the runner refused that (resource-conflict)"),
        ];
        for &(about, code, what, expected) in table {
            let refused = refusal(daemon(code, what), about);
            let said = refused.to_string();
            let word = farcooler_core::error::word(code);
            assert_eq!(said, expected, "{about:?} {word} {what}");
            assert_eq!(refused.word(), Some(word), "{about:?} {word} keeps its code for --json");
            assert!(!said.contains("LOG LINE"), "{word} printed the runner's log line");
            assert!(!said.contains('_'), "{word} put a machine word on a screen: {said}");
            assert!(said.starts_with(char::is_lowercase), "{word}: {said}");
            assert!(!said.ends_with('.'), "{word}: {said}");
        }
        // Each code its own line, within what one command can meet.
        for about in [About::Worktree, About::Commit, About::Fleet, About::Repository] {
            let mut seen = std::collections::HashMap::new();
            for code in table.iter().map(|row| (row.1, row.2)) {
                let said = refusal(daemon(code.0, code.1), about).to_string();
                if let Some(earlier) = seen.insert(said.clone(), code.0) {
                    assert_eq!(earlier, code.0, "{about:?}: two codes read alike: {said}");
                }
            }
        }
    }

    /// An argument the runner names gets a sentence, and a word this build
    /// has never heard of gets the stable code word, never itself.
    #[test]
    fn an_argument_the_runner_names_is_said_and_never_printed() {
        let said = |what: &str| {
            refusal(daemon(pb::ErrorCode::InvalidArgument, what), About::Worktree).to_string()
        };
        // The board's sentence, in this CLI's style, not the runner's.
        assert_eq!(said("worktree_id"), "that is not a worktree on this runner");
        assert_eq!(said("task_prefix"), "a prefix is a letter followed by up to seven letters or digits");
        let unheard = said("columns");
        assert_eq!(unheard, "the runner refused that (invalid-argument)");
    }

    /// Not the runner's, and not the operating system's either.
    #[test]
    fn a_runner_that_stops_answering_says_so() {
        let refused = refusal(ClientError::Closed, About::Worktree);
        assert_eq!(refused.to_string(), "the runner stopped answering");
        assert_eq!(refused.word(), None);
    }

    /// One method, and the refusal it gets.
    type Refusal = Option<(&'static str, fn() -> ClientError)>;

    /// A link that answers the lists and refuses one method.
    struct FakeLink {
        refuse: Refusal,
    }

    const LANE: Uuid = Uuid::from_u128(0x11);
    const REPO: Uuid = Uuid::from_u128(0x22);

    impl DispatchLink for FakeLink {
        fn capabilities(&self) -> Vec<String> {
            Vec::new()
        }
        async fn pause(&mut self, _wait: std::time::Duration) {}
        async fn call(&mut self, req: pb::Request) -> Result<pb::Result, ClientError> {
            if let Some((method, refuse)) = self.refuse
                && method == req.method
            {
                return Err(refuse());
            }
            let value = match req.method.as_str() {
                "worktree.list" => result::Value::WorktreeList(pb::WorktreeList {
                    items: vec![pb::Worktree {
                        id: crate::id_bytes(LANE),
                        task_name: "lane".into(),
                        repository_id: crate::id_bytes(REPO),
                        ..Default::default()
                    }],
                }),
                "repository.list" => result::Value::RepositoryList(pb::RepositoryList {
                    items: vec![pb::Repository {
                        id: crate::id_bytes(REPO),
                        display_name: "overnight".into(),
                        ..Default::default()
                    }],
                }),
                other => panic!("this fake doesn't answer {other}"),
            };
            Ok(pb::Result { value: Some(value) })
        }
    }

    async fn failing(cmd: ChangesCmd, refuse: Refusal) -> Box<dyn std::error::Error> {
        let mut link = FakeLink { refuse };
        changes_over(&mut link, cmd, true).await.expect_err("refused")
    }

    fn status() -> ChangesCmd {
        ChangesCmd::Status { worktree: "lane".into(), fresh: false }
    }

    /// Through the command itself, so an arm that went back to a bare `?`
    /// would print the runner's line here and fail.
    #[tokio::test]
    async fn a_refused_changes_command_prints_a_sentence_and_keeps_its_code() {
        let err = failing(
            status(),
            Some(("changes.change_set", || daemon(pb::ErrorCode::BaseUnresolvable, ""))),
        )
        .await;
        assert_eq!(err.to_string(), "couldn't work out which branch this worktree is compared against");
        let refused = err.downcast_ref::<Refused>().expect("a refusal, not the runner's error");
        assert_eq!(refused.word(), Some("base-unresolvable"));

        // The worktree list it reads first, before the call it came for.
        let err = failing(status(), Some(("worktree.list", || ClientError::Closed))).await;
        assert_eq!(err.to_string(), "the runner stopped answering");

        // `stack` says a repository, not a worktree.
        let err = failing(
            ChangesCmd::Stack { repo: "overnight".into(), branch: "main".into(), refresh: false },
            Some(("stack.get", || daemon(pb::ErrorCode::NotFound, ""))),
        )
        .await;
        assert_eq!(
            err.to_string(),
            "none of that repository's worktrees is on this runner's disk, so its branches can't be read"
        );

        // `diff --commit` is about the commit.
        let err = failing(
            ChangesCmd::Diff {
                worktree: "lane".into(),
                path: "a.rs".into(),
                commit: Some("deadbeef".into()),
                staged: false,
                unstaged: false,
                local: false,
                context: None,
            },
            Some(("changes.file_diff", || daemon(pb::ErrorCode::OperationFailed, ""))),
        )
        .await;
        assert_eq!(err.to_string(), "git couldn't read that commit in this worktree");

        // This CLI's own sentence passes through untouched.
        let err = failing(ChangesCmd::Read { worktree: "nope".into() }, None).await;
        assert_eq!(err.to_string(), "no worktree matching \"nope\"");
    }
}
