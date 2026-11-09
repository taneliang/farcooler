//! What a ticket's page says, and the Themes summary (ov-1038): the words the page used to
//! work out natively, from the same plan the cards read, so the apps only draw them.

use serde::{Deserialize, Serialize};
use typeshare::typeshare;

use super::*;

/// How long a theme sits with no ticket and nothing moving before it reads as quiet: a day
/// (the owner's ruling D3).
const QUIET_AFTER_MS: i64 = 24 * 3_600_000;

/// A pull request's stage on a ticket's page.
#[typeshare]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailStage {
    /// The stage in the Plan's words, leading with the problem when a person has to act.
    pub words: String,
    /// The stage alone in one phrase, state first ("Approved, Checks Failing"), for a line with no
    /// room for two.
    pub full: String,
    /// What a card row under the detail draws: the problem alone when a person has to act, else the
    /// stage in one phrase, state first.
    pub row: String,
    /// "PR 31 · 2 threads open", when the ticket has a pull request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Changes requested, checks failing or conflicts: drawn in color.
    pub attention: bool,
}

/// What a ticket's page says that its card does not.
#[typeshare]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TicketDetail {
    /// The ticket's task id.
    pub id: String,
    /// "Fixing · round 1", "Landed · ac840108": its stage with what it needs said beside it.
    pub status: String,
    /// `status` when the stage is drawn on a line of its own: only what that stage leaves to the ticket.
    pub status_beside_stage: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<DetailStage>,
    /// "No move in 2 h", for a stale ticket.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale: Option<String>,
}

/// Where a theme stands in motion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackKind {
    OverBudget,
    Stuck,
    Moving,
    Queued,
    Quiet,
    Idle,
    AllDone,
    Paused,
    Done,
}

/// The ticket details of a plan, and the one line the Themes section says when it is folded.
#[typeshare]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TicketDetails {
    /// One per ticket the plan lists, a dropped line only where its task has no other.
    pub tickets: Vec<TicketDetail>,
    /// Where each theme the plan shows stands (active, then paused, then done): what the summary counts.
    /// Never sent; the tests read it.
    #[serde(skip)]
    #[typeshare(skip)]
    pub tracks: Vec<TrackKind>,
    /// "3 waiting on you · 4 moving · 1 quiet", for the Themes section when it is folded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_summary: Option<String>,
}

impl Reader<'_> {
    fn detail(&self, line: &Line) -> TicketDetail {
        let shown = self.shown_stage(line);
        TicketDetail {
            id: line.task.clone(),
            status: self.status(line, true),
            status_beside_stage: self.status(line, shown.is_none()),
            stage: shown.and_then(|s| {
                let words = pr_words(s, self.style)?;
                let attention = pr_needs_attention(s);
                let full = pr_full_words(s, self.style).unwrap_or_else(|| words.clone());
                let row = if attention { words.clone() } else { full.clone() };
                Some(DetailStage { words, full, row, detail: pr_detail(s), attention })
            }),
            stale: self.stale(line),
        }
    }

    /// When a theme last moved, in ms: the newest of its story, the runner's word, the board's own
    /// word on its cards, its tickets (a dropped ticket is gone) and its rulings, made or settled.
    fn last_moved(&self, theme: &Theme) -> i64 {
        let tasks: BTreeSet<&str> = theme.cards.iter().map(|c| c.task.as_str()).collect();
        let mut at = theme.story_at.max(theme.last_moved_at.unwrap_or(0));
        for task in &tasks {
            at = at.max(self.context.activity.get(*task).copied().unwrap_or(0));
        }
        for line in &self.plan.lines {
            if self.stage(line) != Stage::Dropped && tasks.contains(line.task.as_str()) {
                at = at.max(line.stage_since);
            }
        }
        for ruling in self.plan.rulings.iter().filter(|r| r.theme_id.as_deref() == Some(theme.id.as_str())) {
            at = at.max(ruling.created_at).max(ruling.settled_at.unwrap_or(0));
        }
        at
    }

    /// Where a theme stands in motion, as of the runner's clock: the first rule that matches.
    fn track(&self, theme: &Theme) -> TrackKind {
        match theme.state.as_str() {
            "paused" => return TrackKind::Paused,
            "done" => return TrackKind::Done,
            _ => {}
        }
        if theme.budget_tokens.is_some_and(|b| theme.spend.total() > b) {
            return TrackKind::OverBudget;
        }
        let tasks: BTreeSet<&str> = theme.cards.iter().map(|c| c.task.as_str()).collect();
        let mine: Vec<&Line> = self.plan.lines.iter().filter(|l| tasks.contains(l.task.as_str())).collect();
        let working: Vec<&Line> =
            mine.iter().copied().filter(|l| self.stage(l).is_live() && self.stage(l) != Stage::Waiting).collect();
        // Stuck only when nothing else is moving (an hour is routine for a build).
        if !working.is_empty() {
            return if working.iter().all(|l| l.stale) { TrackKind::Stuck } else { TrackKind::Moving };
        }
        if mine.iter().any(|l| self.stage(l) == Stage::Waiting) {
            return TrackKind::Queued;
        }
        let total = theme.counts.total();
        if total > 0 && theme.counts.done == total {
            return TrackKind::AllDone;
        }
        let moved = self.last_moved(theme);
        if total > theme.counts.done && moved > 0 && self.plan.now_ms - moved >= QUIET_AFTER_MS {
            return TrackKind::Quiet;
        }
        TrackKind::Idle
    }
}

pub(super) fn details(r: &Reader) -> TicketDetails {
    let mut shown: Vec<&Theme> = r.plan.themes.iter().filter(|t| t.state != "dropped").collect();
    let rank = |t: &Theme| match t.state.as_str() {
        "active" => 0,
        "paused" => 1,
        "done" => 2,
        _ => 3,
    };
    shown.sort_by_key(|t| (rank(t), t.ordinal));
    let tracks: Vec<TrackKind> = shown.iter().map(|t| r.track(t)).collect();
    // Only active themes: a paused theme's ask is inside the closed fold, and Needs You lists it.
    let active: Vec<(&&Theme, &TrackKind)> = shown.iter().zip(&tracks).filter(|(t, _)| t.state == "active").collect();
    let counts = [
        (active.iter().filter(|(t, _)| !t.owner_ask.is_empty()).count(), "waiting on you"),
        (active.iter().filter(|(_, k)| matches!(k, TrackKind::Moving | TrackKind::Stuck)).count(), "moving"),
        (active.iter().filter(|(_, k)| **k == TrackKind::Quiet).count(), "quiet"),
    ];
    let parts: Vec<String> = counts.iter().filter(|(n, _)| *n > 0).map(|(n, w)| format!("{n} {w}")).collect();
    TicketDetails {
        tickets: r
            .plan
            .lines
            .iter()
            .filter(|l| r.stage(l) != Stage::Dropped || !r.plan.lines.iter().any(|o| o.task == l.task && r.stage(o) != Stage::Dropped))
            .map(|l| r.detail(l))
            .collect(),
        tracks,
        track_summary: (!parts.is_empty()).then(|| parts.join(DOT)),
    }
}
