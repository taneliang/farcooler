//! How long a pull request waited on its reviewers (ov-579, ov-318 rank 6):
//! the moments GitHub dates in each PR, kept on the runner beside the PR
//! cache, and the median wait of each reviewer drawn from them.
//!
//! **Recorded.** Per lane PR: opened, ready, first review, approval, changes
//! requested, and the diff size. Read apart from the PR list, by
//! `pr_timing_read`, with one GraphQL call per repository every few minutes, so
//! a stage, a lane's status and Needs You never wait on it (and a failed read
//! leaves the last timing and nothing else changed). Nothing is written to the
//! store: the cache is in memory (`Service::pr_cache`), and a restart refills
//! it from GitHub, which dates everything below. So no migration, and the
//! runner's clock plays no part: every time here is GitHub's.
//!
//! **Unknown is not none.** A moment is `At`, `NotYet` (GitHub was asked and it
//! has not happened: a draft, a PR nobody has reviewed) or `Unknown` (the runner
//! has not read it, or GitHub did not say). Counts and sizes are `Option` for
//! the same reason: `None` is unknown, and `Some(0)` is a PR nobody sent back.
//!
//! **Ready.** The time of the PR's latest `ReadyForReviewEvent`. With none, a
//! PR that was never a draft was ready when it was opened, and one that still
//! is a draft is `NotYet`. A PR that was marked ready and is a draft again is
//! `NotYet` too.
//!
//! **A reviewer's wait.** For each PR whose ready moment is known, the time
//! from ready to that reviewer's first review at or after it: a review made
//! while the PR was a draft answered no request, and is skipped without
//! discarding the PR. The median of that reviewer's latest `LATEST` such
//! waits. Ten: the median of ten ignores the two or three outliers a weekend or
//! a holiday leaves, and for someone who reviews a PR a day it reaches back two
//! weeks, which is what "how long does alice take" means now. Fewer than ten is
//! all there is, and none is "no history yet", never zero minutes. Only lane
//! PRs are read, so the history is the lanes' own. This is a fact to show a
//! person deciding whom to ask. It throttles nothing (owner, Oct 5).

use crate::stack::{PrInfo, PrState};

/// How many of a reviewer's latest reviews their median is drawn from.
pub const LATEST: usize = 10;

/// When something happened, as far as the runner can say.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Moment {
    /// The runner cannot say.
    #[default]
    Unknown,
    /// GitHub was asked, and it has not happened.
    NotYet,
    /// Unix milliseconds.
    At(i64),
}

impl Moment {
    pub fn at(self) -> Option<i64> {
        match self {
            Moment::At(t) => Some(t),
            _ => None,
        }
    }
}

/// A reviewer's first review of a PR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstReview {
    pub reviewer: String,
    pub at: i64,
}

/// The moments of one pull request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrTiming {
    pub opened: Moment,
    /// Marked ready for review (see the module note on how it is dated).
    pub ready: Moment,
    /// The first review by anyone but the author.
    pub first_review: Moment,
    /// The first approval.
    pub approval: Moment,
    /// How many reviews asked for changes and still stand (a dismissed one
    /// reads `DISMISSED`, not `CHANGES_REQUESTED`, and is not counted). A
    /// review count, not a count of rounds: one reviewer asking twice is two.
    pub changes_requested: Option<u32>,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    /// Each reviewer's first review at or after the PR was ready, earliest first.
    pub first_by: Vec<FirstReview>,
    /// When the runner read this (Unix milliseconds); 0 for never. What a
    /// nudge asks, so that a timing read that stopped answering is not taken
    /// for a PR nobody reviewed (ov-580).
    pub read_at: i64,
}

/// One review as gh prints it, already read.
pub struct ReviewFact<'a> {
    pub reviewer: &'a str,
    pub state: &'a str,
    pub at: Option<i64>,
}

/// What GitHub says of one PR.
pub struct Read<'a> {
    pub opened: Option<i64>,
    pub draft: bool,
    /// The latest `ReadyForReviewEvent`, when it has one.
    pub ready_event: Option<i64>,
    pub author: &'a str,
    /// `None` when GitHub did not list reviews at all.
    pub reviews: Option<Vec<ReviewFact<'a>>>,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
}

impl PrTiming {
    pub fn of(r: Read<'_>) -> PrTiming {
        let ready = match (r.draft, r.ready_event, r.opened) {
            (true, ..) => Moment::NotYet,
            (false, Some(at), _) => Moment::At(at),
            // Never a draft: ready when opened.
            (false, None, Some(opened)) => Moment::At(opened),
            (false, None, None) => Moment::Unknown,
        };
        let mut t = PrTiming {
            opened: r.opened.map_or(Moment::Unknown, Moment::At),
            ready,
            additions: r.additions,
            deletions: r.deletions,
            ..PrTiming::default()
        };
        let Some(reviews) = r.reviews else { return t };
        // A review still pending is the reviewer's draft, and says nothing to the author.
        let mut seen: Vec<&ReviewFact<'_>> = reviews
            .iter()
            .filter(|v| v.state != "PENDING" && !v.reviewer.is_empty() && !v.reviewer.eq_ignore_ascii_case(r.author))
            .collect();
        seen.sort_by_key(|v| v.at.unwrap_or(i64::MAX));
        t.changes_requested = Some(seen.iter().filter(|v| v.state == "CHANGES_REQUESTED").count() as u32);
        let when = |v: Option<&&ReviewFact<'_>>| match v {
            None => Moment::NotYet,
            Some(v) => v.at.map_or(Moment::Unknown, Moment::At),
        };
        t.first_review = when(seen.first());
        t.approval = when(seen.iter().find(|v| v.state == "APPROVED"));
        // A reviewer's wait counts from ready: a draft-time review is skipped, their next one counts.
        if let Moment::At(ready) = t.ready {
            for v in seen {
                let Some(at) = v.at.filter(|at| *at >= ready) else { continue };
                if !t.first_by.iter().any(|f| f.reviewer.eq_ignore_ascii_case(v.reviewer)) {
                    t.first_by.push(FirstReview { reviewer: v.reviewer.to_string(), at });
                }
            }
        }
        t
    }
}

/// One reviewer's wait, as `plan.get` carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wait {
    pub reviewer: String,
    /// Pull requests of live lanes that are ready and ask this reviewer.
    pub prs: u32,
    /// The median wait over the reviewer's latest reviews, in milliseconds;
    /// `None` with no history, which is not zero.
    pub median_ms: Option<i64>,
    /// How many reviews the median is drawn from, at most `LATEST`.
    pub samples: u32,
}

/// The median of `waits`, which may be in any order: the middle one, or the
/// mean of the middle two. `None` for none.
pub fn median(waits: &[i64]) -> Option<i64> {
    let mut sorted = waits.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => Some(sorted[n / 2]),
        _ => Some((sorted[n / 2 - 1] + sorted[n / 2]) / 2),
    }
}

/// How long `reviewer` took to first review the PRs in `prs`, newest review
/// first, at most `LATEST` of them.
pub fn waits_of(prs: &[PrInfo], reviewer: &str) -> Vec<i64> {
    let mut found: Vec<(i64, i64)> = prs
        .iter()
        .filter_map(|p| {
            let ready = p.timing.ready.at()?;
            let first = p.timing.first_by.iter().find(|f| f.reviewer.eq_ignore_ascii_case(reviewer))?;
            Some((first.at, (first.at - ready).max(0)))
        })
        .collect();
    found.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    found.into_iter().take(LATEST).map(|(_, wait)| wait).collect()
}

/// For each reviewer asked by one of `waiting` (the open PRs of live lanes),
/// how many PRs wait on them and how long they usually take, drawn from
/// `history` (every PR the runner has read). Most PRs first, then by name.
pub fn waits(waiting: &[&PrInfo], history: &[PrInfo]) -> Vec<Wait> {
    let mut out: Vec<Wait> = Vec::new();
    for p in waiting.iter().filter(|p| p.status.state == PrState::Open) {
        for who in &p.review.requested {
            match out.iter_mut().find(|w| w.reviewer.eq_ignore_ascii_case(who)) {
                Some(w) => w.prs += 1,
                None => out.push(Wait { reviewer: who.clone(), prs: 1, median_ms: None, samples: 0 }),
            }
        }
    }
    for w in &mut out {
        let found = waits_of(history, &w.reviewer);
        w.median_ms = median(&found);
        w.samples = found.len() as u32;
    }
    out.sort_by(|a, b| b.prs.cmp(&a.prs).then_with(|| a.reviewer.to_lowercase().cmp(&b.reviewer.to_lowercase())));
    out
}

#[cfg(test)]
#[path = "pr_timing_tests.rs"]
mod tests;
