//! `farcooler plan ticket pr-body` (ov-314, ov-305 section 4.3, ov-872): a
//! pull request's description, drawn from the card.
//!
//! ```text
//! farcooler plan ticket pr-body KEY [--base REF]  print the section
//! farcooler plan ticket pr-body KEY --apply PR    write it into PR
//! ```
//!
//! One pull request is one ticket's (owner, 2026-10-05), and a ticket is on
//! its own branch. The section is rendered by `plan_pr_section` (what is in
//! it, and why those words); this file reads the board, and with `--apply`
//! reads the description, merges, and writes it back with the owner's own
//! `gh`, run in the directory the command is run in, so the pull request is
//! looked up in that directory's repository (a URL names any other).
//! Everything outside the markers stays as the reviewers left it; `--apply`
//! with nothing to change writes nothing.
//!
//! **`--apply` only writes to the ticket's own pull request.** Its head branch
//! must be the ticket's branch, because a typo or the wrong checkout would
//! otherwise append this card's section to somebody else's pull request, under
//! the owner's login, in public. `--force` is the explicit override. The
//! description is read again just before the write and nothing is written if
//! it moved (GitHub has no compare-and-set, so this narrows the window to a
//! moment): a reviewer's edit is never lost to a slow merge.

use clap::Args;
use farcooler_core::reviewers::{self, Evidence, Suggestion};
use farcooler_core::reviewers_git::{self, Gathered};
use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, request, result};
use serde_json::json;

#[path = "plan_pr_section.rs"]
pub(super) mod section;
#[path = "plan_pr_diff.rs"]
pub(super) mod diff;

use super::{Failed, expect_value, find_line, refused_here, unreadable};
use crate::tasks::{Board, DispatchLink};
use crate::{req_for, with};

#[derive(Debug, Clone, Args)]
pub(super) struct PrBodyArgs {
    /// The ticket: its card's key.
    key: String,
    /// Write it into this pull request's description: a number, a branch or a
    /// URL. Only the text between the Far Cooler markers changes.
    #[arg(long, value_name = "PR")]
    apply: Option<String>,
    /// With --apply: write even when the pull request's branch isn't the
    /// ticket's, or the ticket has no branch recorded.
    #[arg(long, requires = "apply")]
    force: bool,
    /// What the ticket's branch is compared against for the diff stat and the
    /// files to review first.
    #[arg(long, value_name = "REF", default_value = "origin/main")]
    base: String,
}

/// A pull request as `gh` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Pr {
    pub body: String,
    /// The branch it is from.
    pub head: String,
    /// From a fork rather than a branch of this repository.
    pub cross_repository: bool,
}

/// What reads and writes a pull request's description: `gh`, or a test's own.
pub(super) trait Description {
    /// The pull request as it stands now, its description exactly.
    fn read(&mut self, pr: &str) -> Result<Pr, String>;
    fn write(&mut self, pr: &str, body: &str) -> Result<(), String>;
    /// What `branch` changed since `base`, from git in this directory; a test
    /// answers for itself.
    fn changes(&mut self, base: &str, branch: &str) -> Result<diff::Changes, String> {
        diff::changes(base, branch)
    }
    /// The owner's GitHub login, who opens the pull request. `None` when it
    /// can't be said, and then nobody is suggested: without it the author
    /// could be.
    fn login(&mut self) -> Option<String> {
        None
    }
    /// What whom-to-ask is drawn from, read by git in this directory.
    fn gathered(&mut self, base: &str, branch: &str, paths: &[String]) -> Gathered {
        reviewers_git::gather(None, base, branch, paths)
    }
}

/// The owner's `gh`, found on the path.
pub(super) struct Gh {
    program: std::ffi::OsString,
}

impl Gh {
    pub(super) fn new() -> Gh {
        Gh { program: "gh".into() }
    }

    fn run(&self, args: &[&std::ffi::OsStr]) -> Result<Vec<u8>, String> {
        let out = std::process::Command::new(&self.program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => "gh isn't installed here, so Far Cooler can't reach GitHub.".to_string(),
                _ => "gh couldn't be started.".to_string(),
            })?;
        if out.status.success() {
            return Ok(out.stdout);
        }
        Err("GitHub wouldn't do that. Check that gh is signed in and that the pull request is in this directory's repository.".into())
    }
}

/// A pull request as `gh` takes it: never something it could read as a flag.
fn named(pr: &str) -> Result<&str, String> {
    let pr = pr.trim();
    if pr.is_empty() || pr.starts_with('-') {
        return Err("Name the pull request by its number, its branch or its URL.".into());
    }
    Ok(pr)
}

impl Description for Gh {
    fn login(&mut self) -> Option<String> {
        let out = self.run(&["api".as_ref(), "user".as_ref(), "--jq".as_ref(), ".login".as_ref()]).ok()?;
        let login = String::from_utf8_lossy(&out).trim().to_string();
        reviewers::login_ok(&login).then_some(login)
    }

    fn read(&mut self, pr: &str) -> Result<Pr, String> {
        let pr = named(pr)?;
        let fields = ["pr".as_ref(), "view".as_ref(), pr.as_ref(), "--json".as_ref(), "body,headRefName,isCrossRepository".as_ref()];
        let out = self.run(&fields)?;
        let json: serde_json::Value =
            serde_json::from_slice(&out).map_err(|_| "GitHub's answer couldn't be read.".to_string())?;
        let text = |key: &str| json.get(key).and_then(|b| b.as_str()).map(str::to_string);
        match (text("body"), text("headRefName")) {
            (Some(body), Some(head)) => {
                let cross_repository = json.get("isCrossRepository").and_then(|b| b.as_bool()).unwrap_or(false);
                Ok(Pr { body, head, cross_repository })
            }
            _ => Err("GitHub's answer had no description in it.".to_string()),
        }
    }

    fn write(&mut self, pr: &str, body: &str) -> Result<(), String> {
        let pr = named(pr)?;
        // Created exclusively and private to this user, and removed on drop.
        let mut file = tempfile::Builder::new()
            .prefix("farcooler-pr-body-")
            .suffix(".md")
            .tempfile()
            .map_err(|_| "The description couldn't be written to a temporary file.".to_string())?;
        std::io::Write::write_all(&mut file, body.as_bytes())
            .map_err(|_| "The description couldn't be written to a temporary file.".to_string())?;
        let done = self.run(&["pr".as_ref(), "edit".as_ref(), pr.as_ref(), "--body-file".as_ref(), file.path().as_os_str()]);
        done.map(drop)
    }
}

/// The one line on stderr when git can't say what the branch changed: that the
/// review list and the stat are affected, and that a base which isn't fetched is
/// the usual reason.
fn diff_notice(base: &str, why: &str, applying: bool) -> String {
    let after = if applying { "stay as the description has them" } else { "are left out" };
    format!(
        "farcooler: couldn't read the diff against {base} ({why}); the review list and diff stat {after}. \
         If {base} isn't fetched here, run git fetch."
    )
}

/// Whom to ask to review what `changes` touches (ov-580): CODEOWNERS, then
/// history, then the reviewer map, never the author or the branch's own
/// committers. Nobody when the owner's login can't be told, because the one
/// person who must not be suggested is the one that can't then be excluded.
fn suggested(gh: &mut impl Description, base: &str, branch: &str, changes: &diff::Changes) -> Vec<Suggestion> {
    let paths: Vec<String> = changes.files.iter().map(|f| f.path.clone()).collect();
    if paths.is_empty() {
        return Vec::new();
    }
    // The owner's own login must be known: the branch's committers alone don't say who opens the PR.
    let Some(owner) = gh.login() else { return Vec::new() };
    let found = gh.gathered(base, branch, &paths);
    let mut authors = vec![owner];
    authors.extend(found.committers.iter().cloned());
    reviewers::suggest(&Evidence {
        authors: &authors,
        paths: &paths,
        codeowners: found.codeowners.as_deref(),
        history: &found.history,
        map: found.map.as_deref(),
    })
}

/// Who worked the ticket and in what role, for the cost line: each agent the
/// plan still lists, by its model (or harness), in the order it started. An
/// agent the plan no longer lists is left out, since its model isn't known.
fn workers(plan: &pb::Plan, line: &pb::TicketLine) -> Vec<section::Worker> {
    let mut out = Vec::new();
    for w in &line.work {
        let role = match pb::WorkRole::try_from(w.role) {
            Ok(pb::WorkRole::Build) => "build",
            Ok(pb::WorkRole::Fix) => "fix",
            Ok(pb::WorkRole::Review) => "review",
            _ => continue,
        };
        let Some(agent) = plan.agents.iter().find(|a| a.id == w.agent_id) else { continue };
        let model = if agent.model.is_empty() { agent.harness.clone() } else { agent.model.clone() };
        out.push(section::Worker { model, role });
    }
    out
}

async fn detail<L: DispatchLink>(link: &mut L, task: &bytes::Bytes) -> Result<pb::TaskDetail, Failed> {
    let mut r = with(
        req_for("task.get", crate::uuid_of(task)),
        request::Payload::TaskGet(pb::TaskGetRequest { task_id: task.clone(), note_kind: 0 }),
    );
    r.required_capabilities.push(capability::TASKS.to_string());
    let answer = link.call(r).await.map_err(|e| refused_here(e, "That card couldn't be read."))?;
    match expect_value(answer.value)? {
        result::Value::TaskDetail(d) => Ok(d),
        _ => Err(unreadable()),
    }
}

/// `plan ticket pr-body`, with the real `gh`.
pub(super) async fn pr_body<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    plan: &pb::Plan,
    args: PrBodyArgs,
    json: bool,
) -> Result<String, Failed> {
    pr_body_with(link, board, plan, args, json, &mut Gh::new()).await
}

/// `plan ticket pr-body`, reading and writing descriptions through `gh`.
pub(super) async fn pr_body_with<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    plan: &pb::Plan,
    args: PrBodyArgs,
    json: bool,
    gh: &mut impl Description,
) -> Result<String, Failed> {
    let (card, line) = find_line(plan, &args.key)?;
    let key = card.key.clone();
    let detail = detail(link, &line.task_id).await?;
    let theme = plan
        .themes
        .iter()
        .find(|v| v.task_ids.contains(&line.task_id))
        .and_then(|v| v.theme.as_ref())
        .map(|t| t.name.as_str());
    // The calls that touch this ticket: one pull request is one ticket's.
    let rulings: Vec<&pb::BoardRuling> = plan.rulings.iter().filter(|r| r.task_ids.contains(&line.task_id)).collect();
    let cost_line = board.workspace.as_ref().is_some_and(|w| w.pr_cost_line == Some(true));
    let changes = gh.changes(&args.base, &line.branch);
    if let Err(why) = &changes {
        eprintln!("{}", diff_notice(&args.base, why, args.apply.is_some()));
    }
    let asked = match &changes {
        Ok(c) => suggested(gh, &args.base, &line.branch, c),
        Err(_) => Vec::new(),
    };
    let workers = workers(plan, line);
    let render = |carried: Option<&section::Carried>| {
        section::render(&section::Input {
            detail: &detail,
            spend: line.spend.unwrap_or_default(),
            workers: &workers,
            theme,
            rulings: rulings.clone(),
            cost_line,
            changes: changes.as_ref().ok(),
            carried,
            reviewers: &asked,
        })
    };
    let text = render(None);

    let Some(pr) = args.apply else {
        return Ok(if json { json!({ "card": key, "section": text }).to_string() } else { text });
    };
    let existing = gh.read(&pr)?;
    if !args.force {
        if existing.cross_repository {
            return Err(format!("{pr} comes from a fork, not from a branch of this repository. Add --force if it is the right pull request.").into());
        }
        match line.branch.as_str() {
            "" => {
                return Err(format!(
                    "{key} has no branch recorded, so Far Cooler can't tell that {pr} is its pull request. \
                     Check the number, then add --force."
                )
                .into());
            }
            branch if branch != existing.head => {
                return Err(format!(
                    "{pr} is from the branch {}, and {key} is on {branch}. Check the number, or add --force if it is the right pull request.",
                    existing.head
                )
                .into());
            }
            _ => {}
        }
    }
    // A diff git couldn't read keeps what the description says about it.
    let kept = changes.is_err().then(|| section::Carried::of(&existing.body));
    let text = if kept.is_some() { render(kept.as_ref()) } else { text };
    let merged = section::merge(&existing.body, &text).map_err(|d| d.0)?;
    let changed = merged != existing.body;
    if changed {
        if gh.read(&pr)? != existing {
            return Err("The description changed while Far Cooler was reading it, so nothing was written. Run this again."
                .to_string()
                .into());
        }
        gh.write(&pr, &merged)?;
    }
    let had_section = existing.body.contains(section::BEGIN);
    Ok(if json {
        json!({ "card": key, "pr": pr, "changed": changed, "section": text }).to_string()
    } else if !changed {
        format!("The description of {pr} already says this.")
    } else if had_section {
        format!("Updated the Far Cooler section of {pr}. Everything outside it was left alone.")
    } else {
        format!("Added the Far Cooler section to the end of {pr}. Everything else was left alone.")
    })
}

#[cfg(test)]
#[path = "plan_pr_body_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "plan_pr_reviewers_tests.rs"]
mod reviewers_tests;
