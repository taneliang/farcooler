//! `workspace set`'s settings (ov-313): the wake switch and how the board
//! lands its work. A child of `workspaces.rs`, which owns the command; this
//! owns what the flags mean, the request they make and what is said back.

use clap::{Args, ValueEnum};
use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, request};
use uuid::Uuid;

use crate::workspaces::OnOff;
use crate::{req_for, with};

/// How a board lands its work, as `--landing` spells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum LandingArg {
    /// Push to the base branch.
    Direct,
    /// One pull request per card, merged there.
    PullRequests,
}

impl LandingArg {
    pub(crate) fn wire(self) -> pb::LandingMode {
        match self {
            LandingArg::Direct => pb::LandingMode::Direct,
            LandingArg::PullRequests => pb::LandingMode::PullRequests,
        }
    }
}

/// The settings `workspace set` takes. Each one given is set; the rest stay.
#[derive(Args, Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsArgs {
    /// Whether answering one of this board's decisions types the answer
    /// into the agent waiting on it (the task's agent, or else the
    /// orchestrator) once it's idle. On unless turned off.
    #[arg(long, value_name = "on|off")]
    pub wake_on_answer: Option<OnOff>,
    /// How this board lands its work: straight on its base branch, or one
    /// pull request per card. A runner only suggests one (`farcooler repo
    /// landing`); it never switches this for you.
    #[arg(long, value_name = "direct|pull-requests")]
    pub landing: Option<LandingArg>,
    /// The branch work lands on, when it isn't the repository's default. An
    /// empty value takes it away.
    #[arg(long, value_name = "BRANCH")]
    pub base: Option<String>,
    /// How many changed lines a pull request should stay under. 0 takes it away.
    #[arg(long, value_name = "LINES")]
    pub budget_lines: Option<u32>,
    /// Whether a pull request's description carries a cost line. Off unless turned on.
    #[arg(long, value_name = "on|off")]
    pub pr_cost_line: Option<OnOff>,
    /// Whether Far Cooler may post one polite comment on a pull request that
    /// has waited for its first review, as you, on GitHub. Off unless turned
    /// on; never between 22:00 and 08:00; at most once per pull request.
    #[arg(long, value_name = "on|off")]
    pub pr_nudge: Option<OnOff>,
    /// Hours a ready pull request waits for its first review before that
    /// comment. 24 unless set; 0 goes back to 24.
    #[arg(long, value_name = "HOURS")]
    pub pr_nudge_hours: Option<u32>,
    /// Who leads this workspace: the one person who talks to its
    /// orchestrator directly, by handle (`farcooler member list`), or
    /// `owner` for the runner's owner, who leads until it's handed on.
    /// Runner admins only.
    #[arg(long, value_name = "HANDLE|owner")]
    pub lead: Option<String>,
}

impl SettingsArgs {
    /// Whether none was given.
    pub fn is_empty(&self) -> bool {
        *self == SettingsArgs::default()
    }

    /// Whether any of the landing settings was given: those need the `landing` capability.
    pub fn touches_landing(&self) -> bool {
        self.landing.is_some() || self.base.is_some() || self.budget_lines.is_some() || self.pr_cost_line.is_some()
    }

    /// Whether either nudge setting was given: those need the `pr_nudge` capability.
    pub fn touches_nudge(&self) -> bool {
        self.pr_nudge.is_some() || self.pr_nudge_hours.is_some()
    }

    /// `workspace.set_settings` for one workspace, at the version it was read.
    /// Names the capability of each setting it carries, so an older runner
    /// refuses rather than dropping a field it doesn't know.
    pub fn request(&self, workspace: Uuid, version: u64) -> pb::Request {
        let mut r = crate::workspaces::needs_workstreams(with(
            req_for("workspace.set_settings", workspace),
            request::Payload::WorkspaceSetSettings(pb::WorkspaceSetSettings {
                wake_on_answer: self.wake_on_answer.map(|s| s == OnOff::On),
                expected_version: Some(version),
                landing: self.landing.map(|m| m.wire() as i32),
                base: self.base.as_ref().map(|b| b.trim().to_string()),
                pr_max_lines: self.budget_lines,
                pr_cost_line: self.pr_cost_line.map(|s| s == OnOff::On),
                pr_nudge: self.pr_nudge.map(|s| s == OnOff::On),
                pr_nudge_hours: self.pr_nudge_hours,
                lead: self.lead.as_ref().map(|l| l.trim().to_string()),
            }),
        ));
        if self.wake_on_answer.is_some() {
            r.required_capabilities.push(capability::WAKE_ON_ANSWER.to_string());
        }
        if self.touches_landing() {
            r.required_capabilities.push(capability::LANDING.to_string());
        }
        if self.touches_nudge() {
            r.required_capabilities.push(capability::PR_NUDGE.to_string());
        }
        if self.lead.is_some() {
            r.required_capabilities.push(capability::REPRESENTATIVES.to_string());
        }
        r
    }

    /// What was set, one line each, from what the runner answered.
    pub fn said(&self, w: &pb::Workspace) -> String {
        let mut lines = Vec::new();
        if self.wake_on_answer.is_some() {
            let state = if w.wake_on_answer == Some(false) { "off" } else { "on" };
            lines.push(format!("waking the agent when you answer is {state} for {}", w.name));
        }
        if self.landing.is_some() {
            lines.push(match pb::LandingMode::try_from(w.landing.unwrap_or(0)) {
                Ok(pb::LandingMode::Direct) => format!("{} lands straight on its base branch now", w.name),
                Ok(pb::LandingMode::PullRequests) => format!("{} lands through pull requests now", w.name),
                _ => format!("{} has not chosen how it lands work", w.name),
            });
        }
        if self.base.is_some() {
            lines.push(match w.base.as_deref() {
                Some(base) => format!("{} lands on {base} now", w.name),
                None => format!("{} lands on the repository's default branch now", w.name),
            });
        }
        if self.budget_lines.is_some() {
            lines.push(match w.pr_max_lines {
                Some(n) => format!("a pull request from {} should stay under {n} changed lines", w.name),
                None => format!("{} has no line budget now", w.name),
            });
        }
        if self.lead.is_some() {
            lines.push(match w.lead_name.as_deref().filter(|n| !n.is_empty()) {
                Some(name) => format!("{name} leads {} now: only they talk to its orchestrator, and everyone else through their rep", w.name),
                None => format!("the runner's owner leads {} now: only they talk to its orchestrator", w.name),
            });
            if let Some(word) = w.orchestrator_not_restarted.as_deref() {
                lines.push(not_restarted(w, word));
            }
        }
        if self.pr_cost_line.is_some() {
            let state = if w.pr_cost_line == Some(true) { "show" } else { "leave out" };
            lines.push(format!("{}'s pull request descriptions {state} a cost line now", w.name));
        }
        if self.pr_nudge.is_some() {
            lines.push(if w.pr_nudge == Some(true) {
                format!("{} may nudge a pull request that has waited {} hours for a review, once, and not at night", w.name, w.pr_nudge_hours.unwrap_or(24))
            } else {
                format!("{} no longer nudges pull requests", w.name)
            });
        } else if self.pr_nudge_hours.is_some() {
            lines.push(format!("a pull request from {} waits {} hours before a nudge", w.name, w.pr_nudge_hours.unwrap_or(24)));
        }
        // What was chosen can still be a way that can't work here; say so.
        if w.landing == Some(pb::LandingMode::Direct as i32)
            && let Some(why) = &w.direct_refused
        {
            lines.push(format!("{why} Nothing was switched for you."));
        }
        lines.join("\n")
    }
}

/// What a lead change says when the runner stopped the orchestrator and
/// couldn't start it again on the new lead's login (ov-585): why, in the
/// runner's own sentence for the word where it has one, and how to start it.
fn not_restarted(w: &pb::Workspace, word: &str) -> String {
    let whose = match w.lead_name.as_deref().filter(|n| !n.is_empty()) {
        Some(name) => format!("{name}'s"),
        None => "the runner's own".to_string(),
    };
    let why = farcooler_core::error::sentence(word).map(|s| format!(" {s}")).unwrap_or_default();
    let name = if w.name.contains(char::is_whitespace) { format!("{:?}", w.name) } else { w.name.clone() };
    format!(
        "its orchestrator was stopped and couldn't start again on {whose} Claude login.{why} Start it again with `farcooler workspace start-orchestrator {name}`"
    )
}

/// A refused `workspace set`, in words: handing on the lead has its own two
/// refusals, and everything else is a workspace refusal.
pub fn refused(e: farcooler_transport::ClientError, lead: bool) -> crate::tasks::Refused {
    if lead && let farcooler_transport::ClientError::Daemon { code, what, .. } = &e {
        let said = match (farcooler_core::error::word_for(*code), what.as_str()) {
            ("scope-denied", _) => Some("only a runner admin can hand on a workspace's lead"),
            ("invalid-argument", "lead") => Some("there's nobody on this runner by that handle. `farcooler member list` names them"),
            _ => None,
        };
        if let Some(said) = said {
            return crate::tasks::Refused::naming(said.to_string(), *code, what.clone());
        }
    }
    crate::workspaces::workspace_refused(e, crate::workspaces::GONE, "that setting could not be changed")
}

#[cfg(test)]
#[path = "workspace_settings_tests.rs"]
mod tests;
