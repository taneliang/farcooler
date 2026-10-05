//! Where a pull request stands (ov-312, ov-305 section 4.1): the stage a lane
//! or a card shows, derived on read from what GitHub last said and never
//! written anywhere.
//!
//! One table, here, so the Mac, the phones and `farcooler plan` say the same
//! words and none of them re-derives a stage (the way `stack::is_stale` keeps
//! three apps from holding three opinions about "a while ago"):
//!
//! | PR facts | Stage |
//! |---|---|
//! | no PR (a read that found none), lane building | Building |
//! | draft, lane in review or landing | Agent review |
//! | draft, lane fixing | Fixing |
//! | ready, no decision yet | Waiting on a reviewer, by login when asked of one |
//! | changes requested | Changes requested |
//! | approved, merge conflicts | Approved · conflicts |
//! | approved, checks pending | Approved · checks running |
//! | approved, checks failing | Approved · checks failing |
//! | approved, checks passing or none configured | Approved |
//! | in the merge queue | Queued, position N |
//! | merged / closed unmerged | Merged / Closed |
//! | GitHub could not be asked, lane in review, fixing or landing | PR state unknown |
//!
//! The labels are the runner's own words, in sentence case, for the CLI and as
//! a fallback. The apps word a stage in their own style from its kind and
//! facts (`Stage`).
//!
//! "Unknown" is never "no PR": the first is a runner that could not ask and
//! must not claim the lane has no pull request (an app would offer to open
//! one over a PR that exists); the second is a runner that asked and heard
//! none. A lane that has not started, and one whose read could not be made
//! while it is still building, have no stage at all, and an app falls back to
//! the lane's own state word.

use crate::stack::{CheckState, PrInfo, PrState, ReviewDecision};

/// Where the lane is, as far as the stage cares. A copy of the plan layer's
/// lane state with its own name, so this table names nothing of the layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Queued,
    Building,
    Review,
    Fixing,
    Landing,
    Landed,
    Dropped,
}

impl Phase {
    /// Whether the lane's pull request is being worked on or waited for now:
    /// the lanes whose PRs are re-read every minute (`pr_watch`).
    pub fn is_active(self) -> bool {
        matches!(self, Phase::Review | Phase::Fixing | Phase::Landing)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Building,
    AgentReview,
    Fixing,
    WaitingOnReviewer,
    ChangesRequested,
    ApprovedChecksRunning,
    ApprovedChecksFailing,
    Approved,
    ApprovedConflicts,
    Queued,
    Merged,
    Closed,
    Unknown,
}

impl Kind {
    /// How far along: what a lane's rolled-up stage takes the least of. Unknown
    /// is below everything, because a runner that cannot see one of a lane's
    /// pull requests cannot claim the lane is further along than that.
    fn progress(self) -> u8 {
        match self {
            Kind::Unknown => 0,
            Kind::Building => 1,
            Kind::AgentReview => 2,
            Kind::Fixing => 3,
            Kind::ChangesRequested => 4,
            Kind::WaitingOnReviewer => 5,
            Kind::ApprovedChecksFailing => 6,
            Kind::ApprovedChecksRunning => 7,
            Kind::ApprovedConflicts => 6,
            Kind::Approved => 8,
            Kind::Queued => 9,
            Kind::Merged => 10,
            // Finished without landing: it holds nothing back, and a lane
            // whose only pull requests were closed says so (`rollup`).
            Kind::Closed => 11,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stage {
    pub kind: Kind,
    /// Whom a review waits on: logins and team slugs, in GitHub's order.
    pub reviewers: Vec<String>,
    /// From 1 as GitHub counts it.
    pub queue_position: u32,
    pub checks: CheckState,
    /// The pull request read, 0 for none.
    pub pr_number: u32,
    pub pr_url: String,
    pub unresolved_threads: Option<u32>,
    /// When GitHub was read, in unix milliseconds; 0 when it has not been.
    pub read_at: i64,
}

impl Stage {
    fn bare(kind: Kind) -> Stage {
        Stage {
            kind,
            reviewers: Vec::new(),
            queue_position: 0,
            checks: CheckState::Unknown,
            pr_number: 0,
            pr_url: String::new(),
            unresolved_threads: None,
            read_at: 0,
        }
    }

    /// What a runner that could not ask GitHub says.
    pub fn unknown() -> Stage {
        Stage::bare(Kind::Unknown)
    }

    /// The runner's own words, in sentence case: for the CLI, and as a
    /// fallback for an app that does not know the kind.
    pub fn label(&self) -> String {
        match self.kind {
            Kind::Building => "Building".into(),
            Kind::AgentReview => "Agent review".into(),
            Kind::Fixing => "Fixing".into(),
            Kind::WaitingOnReviewer if self.reviewers.is_empty() => "Waiting on a reviewer".into(),
            Kind::WaitingOnReviewer => format!("Waiting on {}", names(&self.reviewers)),
            Kind::ChangesRequested => "Changes requested".into(),
            Kind::ApprovedChecksRunning => "Approved \u{b7} checks running".into(),
            Kind::ApprovedChecksFailing => "Approved \u{b7} checks failing".into(),
            Kind::ApprovedConflicts => "Approved \u{b7} conflicts".into(),
            Kind::Approved => "Approved".into(),
            Kind::Queued => format!("Queued, position {}", self.queue_position),
            Kind::Merged => "Merged".into(),
            Kind::Closed => "Closed".into(),
            Kind::Unknown => "PR state unknown".into(),
        }
    }
}

/// The stage of a card's pull request, or of no pull request, as GitHub's last
/// answer said.
///
/// `pr` is `None` only when GitHub answered and the card has none. A runner
/// that could not ask never gets here: `Stage::unknown` is its answer.
pub fn derive(pr: Option<&PrInfo>, phase: Phase) -> Option<Stage> {
    let Some(pr) = pr else {
        // No pull request is "Building" for a lane that is building. For a
        // lane already in review or landing it contradicts the lane's own
        // state, so the runner says nothing and the app shows that state.
        return (phase == Phase::Building).then(|| Stage::bare(Kind::Building));
    };
    let status = &pr.status;
    let review = &pr.review;
    let kind = match status.state {
        PrState::Merged => Kind::Merged,
        PrState::Closed => Kind::Closed,
        PrState::Unknown => Kind::Unknown,
        PrState::Open if review.queue_position.is_some() => Kind::Queued,
        PrState::Draft => match phase {
            Phase::Fixing => Kind::Fixing,
            Phase::Queued | Phase::Building => Kind::Building,
            _ => Kind::AgentReview,
        },
        PrState::Open => match status.review_decision {
            ReviewDecision::ChangesRequested => Kind::ChangesRequested,
            ReviewDecision::Approved if review.merge_state == "DIRTY" => Kind::ApprovedConflicts,
            ReviewDecision::Approved => match status.checks {
                CheckState::Failing => Kind::ApprovedChecksFailing,
                CheckState::Pending => Kind::ApprovedChecksRunning,
                CheckState::Passing | CheckState::Unknown => Kind::Approved,
            },
            ReviewDecision::ReviewRequired | ReviewDecision::Unknown => Kind::WaitingOnReviewer,
        },
    };
    let reviewers = if kind == Kind::WaitingOnReviewer { review.requested.clone() } else { Vec::new() };
    Some(Stage {
        kind,
        reviewers,
        queue_position: if kind == Kind::Queued { review.queue_position.unwrap_or(0) } else { 0 },
        checks: status.checks,
        pr_number: status.number,
        pr_url: status.url.clone(),
        unresolved_threads: review.unresolved_threads,
        read_at: status.fetched_at,
    })
}

/// "alice", "alice and bob", "alice and 2 others".
fn names(requested: &[String]) -> String {
    match requested {
        [] => String::new(),
        [one] => one.clone(),
        [a, b] => format!("{a} and {b}"),
        [a, rest @ ..] => format!("{a} and {} others", rest.len()),
    }
}

/// A lane's stage: the least advanced of its cards' (ov-312, the owner's
/// ruling: one PR per card, so a lane with three cards has three PRs and is
/// only as far along as the slowest). A pull request closed without merging
/// holds nothing back unless every one of them was.
pub fn rollup(stages: &[Stage]) -> Option<Stage> {
    let live = stages.iter().filter(|s| s.kind != Kind::Closed);
    live.min_by_key(|s| s.kind.progress()).or_else(|| stages.first()).cloned()
}

/// The fixture's pull requests, the card of PR `100 + n` called `ov-n`.
#[cfg(test)]
pub(crate) fn fixture_prs() -> Vec<PrInfo> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/pr-stage/pr-list.json");
    let mut text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    for n in 1..=25 {
        text = text.replace(&format!("@{n}@"), &format!("ov-{n}"));
    }
    crate::stack::parse_prs(text.as_bytes()).expect("the recorded list parses")
}

#[cfg(test)]
#[path = "pr_stage_tests.rs"]
mod tests;
