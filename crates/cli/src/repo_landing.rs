//! `farcooler repo landing` (ov-313, ov-305 section 6.2): how this repository
//! can land work, read by the runner with its own `gh`.
//!
//! ```text
//! farcooler repo landing [--repo NAME]
//! ```
//!
//! Prints the base branch, the mode the runner suggests with its reasons, the
//! merge-queue warning when there is one, and every fact that went into it. A
//! fact the runner could not read is said to be unread, never "no". **It
//! suggests and never switches:** the board's mode is `workspace set --landing`
//! and nothing here writes it. When the board chose `direct` and the base
//! refuses pushes, that is said, and left as it is.
//!
//! Only the runner's `gh` knows the owner's login, so the read is the runner's
//! (`repository.landing`), and `--runner` reaches another runner's.

use clap::Args;
use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, result};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::tasks::{DispatchLink, Refused};
use crate::workspaces::{WORKSPACE_ENV, repository_to_create_in, workspaces_on};
use crate::{Fallible, Link, expect_value, req_for, uuid_of};

const NEEDS_UPDATE: &str = "This runner needs an update to read how it lands work.";

#[derive(Debug, Clone, Args)]
pub struct LandingArgs {
    /// Which repository. Defaults to the one the pane's own workspace is in,
    /// then to the only one there is.
    #[arg(long)]
    pub repo: Option<String>,
}

pub async fn landing(link: &mut Link, args: LandingArgs, json: bool) -> Fallible {
    let env = std::env::var(WORKSPACE_ENV).ok().and_then(|v| v.parse::<Uuid>().ok());
    println!("{}", read(link, args.repo.as_deref(), env, json).await?);
    Ok(())
}

/// What `repo landing` prints.
pub(crate) async fn read<L: DispatchLink>(
    link: &mut L,
    repo: Option<&str>,
    env: Option<Uuid>,
    json: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    if !link.capabilities().iter().any(|c| c == capability::LANDING) {
        return Err(Box::new(Refused::new(NEEDS_UPDATE.to_string(), Some(pb::ErrorCode::CapabilityUnsupported as i32))));
    }
    let repositories = crate::list_repositories(link).await?;
    let all = workspaces_on(link, None).await?;
    let repository = repository_to_create_in(&repositories, &all, repo, env)?;
    let mut call = req_for("repository.landing", repository);
    call.required_capabilities.push(capability::LANDING.to_string());
    let answer = link.call(call).await.map_err(|e| crate::tasks::refusal(e, "the runner couldn't read that repository's rules"))?;
    let result::Value::RepositoryLanding(landing) = expect_value(answer.value)? else {
        return Err(crate::daemon_link::UNREADABLE.into());
    };
    let main = all.iter().find(|w| w.is_main && uuid_of(&w.repository_id) == repository);
    Ok(if json { landing_json(&landing, main).to_string() } else { render(&landing, main) })
}

/// `--json`: the suggestion, its reasons and every fact, with the facts the
/// runner could not read as `null`.
pub(crate) fn landing_json(l: &pb::RepositoryLanding, board: Option<&pb::Workspace>) -> Value {
    let f = l.facts.clone().unwrap_or_default();
    json!({
        "base": l.base,
        "suggested": farcooler_client::workspaces_json::landing_word(l.suggested),
        "direct_impossible": l.direct_impossible,
        "reasons": l.reasons,
        "warnings": l.warnings,
        "facts": {
            "pull_request_rule": f.pull_request_rule,
            "required_approvals": f.required_approvals,
            "merge_queue": f.merge_queue,
            "required_checks": f.required_checks,
            "branch_protected": f.branch_protected,
            "squash_allowed": f.squash_allowed,
            "rebase_allowed": f.rebase_allowed,
            "merge_commit_allowed": f.merge_commit_allowed,
            "merge_method": f.merge_method,
            "codeowners": f.codeowners,
            "viewer_permission": f.viewer_permission,
            "merge_group_workflow": f.merge_group_workflow,
        },
        "read_at": l.read_at,
        "chosen": board.and_then(|w| w.landing).and_then(farcooler_client::workspaces_json::landing_word),
    })
}

fn yes_no(fact: Option<bool>) -> &'static str {
    match fact {
        Some(true) => "yes",
        Some(false) => "no",
        None => "not read",
    }
}

/// What `repo landing` says, for a person.
pub(crate) fn render(l: &pb::RepositoryLanding, board: Option<&pb::Workspace>) -> String {
    let f = l.facts.clone().unwrap_or_default();
    let mut out = vec![format!("Base branch: {}", l.base)];
    out.push(match pb::LandingMode::try_from(l.suggested) {
        Ok(pb::LandingMode::Direct) => "Suggested: land directly on the base branch".to_string(),
        Ok(pb::LandingMode::PullRequests) => "Suggested: land through pull requests".to_string(),
        _ => "Suggested: nothing, because not enough could be read".to_string(),
    });
    if l.direct_impossible {
        out.push("Landing directly can't work here.".into());
    }
    out.push(String::new());
    out.push("Why:".into());
    out.extend(l.reasons.iter().map(|r| format!("  {r}")));
    if !l.warnings.is_empty() {
        out.push(String::new());
        out.push("Warnings:".into());
        out.extend(l.warnings.iter().map(|w| format!("  {w}")));
    }

    out.push(String::new());
    out.push("What was read:".into());
    out.push(format!(
        "  Pull request rule: {}",
        match (f.pull_request_rule, f.required_approvals) {
            (Some(true), Some(n)) if n > 0 => format!("yes, with {n} approving review{}", if n == 1 { "" } else { "s" }),
            (other, _) => yes_no(other).to_string(),
        }
    ));
    out.push(format!("  Merge queue: {}", yes_no(f.merge_queue)));
    // The checks come from the rules read, which is the same read that says
    // whether there is a pull request rule.
    out.push(format!(
        "  Required checks: {}",
        match (&f.pull_request_rule, f.required_checks.as_slice()) {
            (None, _) => "not read".to_string(),
            (_, []) => "none".to_string(),
            (_, checks) => checks.join(", "),
        }
    ));
    out.push(format!(
        "  Branch protection: {}",
        match f.branch_protected {
            Some(true) => "protected (GitHub shows its details only to an admin)",
            Some(false) => "not protected",
            None => "not read",
        }
    ));
    let none_allowed = [f.squash_allowed, f.rebase_allowed, f.merge_commit_allowed].iter().all(Option::is_some);
    out.push(format!(
        "  Merge method: {}",
        f.merge_method.as_deref().unwrap_or(if none_allowed { "none allowed" } else { "not read" })
    ));
    out.push(format!("  CODEOWNERS: {}", yes_no(f.codeowners)));
    out.push(format!("  This login's permission: {}", f.viewer_permission.as_deref().unwrap_or("not read")));
    out.push(format!("  A workflow runs on merge_group: {}", yes_no(f.merge_group_workflow)));

    out.push(String::new());
    out.push(match board {
        Some(w) => match (pb::LandingMode::try_from(w.landing.unwrap_or(0)), l.direct_impossible) {
            (Ok(pb::LandingMode::Direct), true) => format!(
                "{} is set to land directly, and that can't work here. Nothing was changed. Choose pull requests with `farcooler workspace set {} --landing pull-requests`.",
                w.name, w.name
            ),
            (Ok(pb::LandingMode::Direct), false) => format!("{} is set to land directly.", w.name),
            (Ok(pb::LandingMode::PullRequests), _) => format!("{} is set to land through pull requests.", w.name),
            _ => format!(
                "{} hasn't chosen yet. Choose with `farcooler workspace set {} --landing direct` or `--landing pull-requests`.",
                w.name, w.name
            ),
        },
        None => "No workspace was found to compare with.".into(),
    });
    out.join("\n")
}

#[cfg(test)]
#[path = "repo_landing_tests.rs"]
mod tests;
