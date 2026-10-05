//! `farcooler plan lane pr-body` (ov-314, ov-305 section 4.3): a pull
//! request's description, drawn from the card.
//!
//! ```text
//! farcooler plan lane pr-body LANE [--card KEY]            print the section
//! farcooler plan lane pr-body LANE [--card KEY] --apply PR  write it into PR
//! ```
//!
//! One pull request is one card's (owner, 2026-10-05), so a lane that works
//! several cards is asked which: `--card`. The section is rendered by
//! `plan_pr_section` (what is in it, and why those words); this file reads the
//! board, and with `--apply` reads the description, merges, and writes it back
//! with the owner's own `gh`, run in the directory the command is run in, so
//! the pull request is looked up in that directory's repository (a URL names
//! any other). Everything outside the markers stays as the reviewers left it;
//! `--apply` with nothing to change writes nothing.
//!
//! **`--apply` only writes to the lane's own pull request.** Its head branch
//! must be the lane's branch, because a typo or the wrong checkout would
//! otherwise append this card's section to somebody else's pull request, under
//! the owner's login, in public. `--force` is the explicit override. The
//! description is read again just before the write and nothing is written if
//! it moved (GitHub has no compare-and-set, so this narrows the window to a
//! moment): a reviewer's edit is never lost to a slow merge.

use clap::Args;
use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, request, result};
use serde_json::json;

#[path = "plan_pr_section.rs"]
pub(super) mod section;

use super::{Failed, Keys, expect_value, refused_here, unreadable};
use crate::tasks::{Board, DispatchLink};
use crate::{req_for, short_bytes, with};

#[derive(Debug, Clone, Args)]
pub(super) struct PrBodyArgs {
    /// The lane's name.
    name: String,
    /// The card, when the lane works several: a pull request is one card's.
    #[arg(long, value_name = "KEY")]
    card: Option<String>,
    /// Write it into this pull request's description: a number, a branch or a
    /// URL. Only the text between the Far Cooler markers changes.
    #[arg(long, value_name = "PR")]
    apply: Option<String>,
    /// With --apply: write even when the pull request's branch isn't the
    /// lane's, or the lane has no branch recorded.
    #[arg(long, requires = "apply")]
    force: bool,
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

/// The card this pull request is for: `--card`, or the lane's only one.
fn card_of<'a>(lane: &'a pb::Lane, keys: &Keys, wanted: Option<&str>) -> Result<&'a pb::LaneCard, Failed> {
    let mut cards: Vec<&pb::LaneCard> = Vec::new();
    for c in &lane.cards {
        if !cards.iter().any(|seen| seen.task_id == c.task_id) {
            cards.push(c);
        }
    }
    let names = || cards.iter().map(|c| keys.of(&c.task_id)).collect::<Vec<_>>().join(", ");
    match (wanted, cards.as_slice()) {
        (_, []) => Err(format!("{} has no cards yet.", lane.name).into()),
        (Some(key), _) => {
            let key = key.trim().to_lowercase();
            cards
                .iter()
                .find(|c| keys.of(&c.task_id).to_lowercase() == key || short_bytes(&c.task_id) == key)
                .copied()
                .ok_or_else(|| format!("{} doesn't work {key}. Its cards: {}.", lane.name, names()).into())
        }
        (None, [only]) => Ok(only),
        (None, many) => Err(format!(
            "{} works {} cards, and a pull request is one card's. Name one with --card: {}.",
            lane.name,
            many.len(),
            names()
        )
        .into()),
    }
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

/// `plan lane pr-body`, with the real `gh`.
pub(super) async fn pr_body<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    plan: &pb::Plan,
    keys: &Keys,
    args: PrBodyArgs,
    json: bool,
) -> Result<String, Failed> {
    pr_body_with(link, board, plan, keys, args, json, &mut Gh::new()).await
}

/// `plan lane pr-body`, reading and writing descriptions through `gh`.
pub(super) async fn pr_body_with<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    plan: &pb::Plan,
    keys: &Keys,
    args: PrBodyArgs,
    json: bool,
    gh: &mut impl Description,
) -> Result<String, Failed> {
    let lane = super::find_lane(plan, &args.name)?;
    let card = card_of(lane, keys, args.card.as_deref())?;
    let detail = detail(link, &card.task_id).await?;
    let theme = plan
        .themes
        .iter()
        .find(|v| v.task_ids.contains(&card.task_id))
        .and_then(|v| v.theme.as_ref())
        .map(|t| t.name.as_str());
    // The calls that touch this card: one pull request is one card's.
    let rulings = plan.rulings.iter().filter(|r| r.task_ids.contains(&card.task_id)).collect();
    let cost_line = board.workspace.as_ref().is_some_and(|w| w.pr_cost_line == Some(true));
    let text = section::render(&section::Input { detail: &detail, lane, theme, rulings, cost_line });
    let key = keys.of(&card.task_id);

    let Some(pr) = args.apply else {
        return Ok(if json { json!({ "card": key, "section": text }).to_string() } else { text });
    };
    let existing = gh.read(&pr)?;
    if !args.force {
        if existing.cross_repository {
            return Err(format!("{pr} comes from a fork, not from a branch of this repository. Add --force if it is the right pull request.").into());
        }
        match lane.branch.as_str() {
            "" => {
                return Err(format!(
                    "{} has no branch recorded, so Far Cooler can't tell that {pr} is its pull request. \
                     Check the number, then add --force.",
                    lane.name
                )
                .into());
            }
            branch if branch != existing.head => {
                return Err(format!(
                    "{pr} is from the branch {}, and {} works {branch}. Check the number, or add --force if it is the right pull request.",
                    existing.head, lane.name
                )
                .into());
            }
            _ => {}
        }
    }
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
