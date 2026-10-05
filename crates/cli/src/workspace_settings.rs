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
    fn wire(self) -> pb::LandingMode {
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
            }),
        ));
        if self.wake_on_answer.is_some() {
            r.required_capabilities.push(capability::WAKE_ON_ANSWER.to_string());
        }
        if self.touches_landing() {
            r.required_capabilities.push(capability::LANDING.to_string());
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
        if self.pr_cost_line.is_some() {
            let state = if w.pr_cost_line == Some(true) { "show" } else { "leave out" };
            lines.push(format!("{}'s pull request descriptions {state} a cost line now", w.name));
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

#[cfg(test)]
#[path = "workspace_settings_tests.rs"]
mod tests;
