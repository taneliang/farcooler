//! What a ticket's page and a theme's track line say (ov-1038): the words the pages used to
//! work out natively, from the same plan the cards read, so the apps only draw them.

use serde::{Deserialize, Serialize};

use super::*;

/// How long a theme sits with no ticket and nothing moving before it reads as quiet: a day
/// (the owner's ruling D3).
const QUIET_AFTER_MS: i64 = 24 * 3_600_000;

/// A pull request's stage on a ticket's page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageStage {
    /// The stage in the Plan's words, leading with the problem when a person has to act.
    pub words: String,
    /// The stage alone in one phrase, state first ("Approved, Checks Failing"), for a line with no
    /// room for two.
    pub full: String,
    /// What a card row under the page draws: the problem alone when a person has to act, else the
    /// stage in one phrase, state first.
    pub row: String,
    /// "PR 31 · 2 threads open", when the ticket has a pull request.
    pub detail: Option<String>,
    /// Changes requested, checks failing or conflicts: drawn in color.
    pub attention: bool,
}

/// What a ticket's page says that its card does not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketPage {
    /// The ticket's task id.
    pub id: String,
    /// "Fixing · round 1", "Landed · ac840108": its stage with what it needs said beside it.
    pub status: String,
    /// `status` when the stage is drawn on a line of its own: only what that stage leaves to the ticket.
    pub status_beside_stage: String,
    pub stage: Option<PageStage>,
    /// "No move in 2 h", for a stale ticket.
    pub stale: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

/// One plain-words line per theme that says whether the work is moving. Computed from facts the
/// plan carries, never from a stored health flag.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeTrack {
    /// The theme's id.
    pub id: String,
    pub kind: TrackKind,
    /// "2 tickets moving", "Fix gestures is fixing, round 1".
    pub words: String,
    /// The same for a screen reader: a middle dot isn't read well.
    pub spoken: String,
    /// Amber, and only for a budget gone over.
    pub attention: bool,
}

/// The pages and tracks of a plan.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pages {
    /// One per ticket the plan lists.
    pub tickets: Vec<TicketPage>,
    /// One per theme the plan shows: active, then paused, then done.
    pub tracks: Vec<ThemeTrack>,
    /// "3 waiting on you · 4 moving · 1 quiet", for the Themes section when it is folded.
    pub track_summary: Option<String>,
}

impl Reader<'_> {
    fn page(&self, line: &Line) -> TicketPage {
        let shown = self.shown_stage(line);
        TicketPage {
            id: line.task.clone(),
            status: self.status(line, true),
            status_beside_stage: self.status(line, shown.is_none()),
            stage: shown.and_then(|s| {
                let words = pr_words(s, self.style)?;
                let attention = pr_needs_attention(s);
                let full = pr_full_words(s, self.style).unwrap_or_else(|| words.clone());
                let row = if attention { words.clone() } else { full.clone() };
                Some(PageStage { words, full, row, detail: pr_detail(s), attention })
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

    fn track(&self, theme: &Theme) -> ThemeTrack {
        let (kind, words, spoken) = self.track_words(theme);
        let spoken = spoken.unwrap_or_else(|| words.replace(" · ", ", "));
        ThemeTrack { id: theme.id.clone(), kind, attention: kind == TrackKind::OverBudget, words, spoken }
    }

    /// Where a theme stands in motion, as of the runner's clock: the first rule that matches.
    fn track_words(&self, theme: &Theme) -> (TrackKind, String, Option<String>) {
        let plain = |kind: TrackKind, words: &str| (kind, words.to_string(), None);
        match theme.state.as_str() {
            "paused" => return plain(TrackKind::Paused, "Paused"),
            "done" => return plain(TrackKind::Done, "Done"),
            _ => {}
        }
        let used = theme.spend.total();
        if let Some(budget) = theme.budget_tokens.filter(|b| used > *b) {
            return (
                TrackKind::OverBudget,
                format!("Over budget: {} of {} tokens", tokens(used), tokens(budget)),
                Some(format!("Over budget. {} of {} tokens used.", words::spoken_tokens(used), words::spoken_tokens(budget))),
            );
        }
        let now = self.plan.now_ms;
        let tasks: BTreeSet<&str> = theme.cards.iter().map(|c| c.task.as_str()).collect();
        let mine: Vec<&Line> = self.plan.lines.iter().filter(|l| tasks.contains(l.task.as_str())).collect();
        let working: Vec<&Line> =
            mine.iter().copied().filter(|l| self.stage(l).is_live() && self.stage(l) != Stage::Waiting).collect();
        let no_move = |since: i64| {
            let minutes = (now - since).max(0) / 60_000;
            if minutes < 120 { "no move in an hour".to_string() } else { format!("no move in {} h", minutes / 60) }
        };
        // The ticket stalled longest, not the first in plan order; it stands alone only when nothing
        // else is moving (an hour is routine for a build, so it never hides the tickets that are).
        let stalled = working.iter().copied().filter(|l| l.stale).min_by_key(|l| l.stage_since);
        if let Some(stalled) = stalled
            && working.iter().all(|l| l.stale)
        {
            return plain(TrackKind::Stuck, &format!("{}: {}", self.heading(stalled), no_move(stalled.stage_since)));
        }
        if !working.is_empty() {
            let tail = stalled.map(|l| format!("{DOT}{}: {}", self.heading(l), no_move(l.stage_since))).unwrap_or_default();
            let head = match working.as_slice() {
                [one] => {
                    let verb = if self.is_fixing(one) {
                        if one.fix_rounds > 0 { format!("fixing, round {}", one.fix_rounds) } else { "fixing".to_string() }
                    } else {
                        match self.stage(one) {
                            Stage::Building => "building",
                            Stage::Reviewing => "in review",
                            Stage::NeedsYou => "waiting on you",
                            Stage::Landing => "landing",
                            _ => "moving",
                        }
                        .to_string()
                    };
                    format!("{} is {verb}", self.heading(one))
                }
                many => format!("{} tickets moving", many.len()),
            };
            return plain(TrackKind::Moving, &format!("{head}{tail}"));
        }
        let queued: Vec<&&Line> = mine.iter().filter(|l| self.stage(l) == Stage::Waiting).collect();
        if !queued.is_empty() {
            let words = match queued.iter().filter_map(|l| l.plan_rank).min() {
                Some(rank) => format!("Queued, {} up", words::ordinal(rank)),
                None => "Queued".to_string(),
            };
            return plain(TrackKind::Queued, &words);
        }
        let total = theme.counts.total();
        if total > 0 && theme.counts.done == total {
            return plain(TrackKind::AllDone, "Every card is done");
        }
        let moved = self.last_moved(theme);
        if total > theme.counts.done && moved > 0 && now - moved >= QUIET_AFTER_MS {
            let days = ((now - moved).max(0) / 86_400_000).max(1);
            return plain(
                TrackKind::Quiet,
                &if days == 1 { "No ticket · quiet for 1 day".to_string() } else { format!("No ticket · quiet for {days} days") },
            );
        }
        plain(TrackKind::Idle, "No ticket yet")
    }
}

pub(super) fn pages(r: &Reader) -> Pages {
    let mut shown: Vec<&Theme> = r.plan.themes.iter().filter(|t| t.state != "dropped").collect();
    let rank = |t: &Theme| match t.state.as_str() {
        "active" => 0,
        "paused" => 1,
        "done" => 2,
        _ => 3,
    };
    shown.sort_by_key(|t| (rank(t), t.ordinal));
    let tracks: Vec<ThemeTrack> = shown.iter().map(|t| r.track(t)).collect();
    // Only active themes: a paused theme's ask is inside the closed fold, and Needs You lists it.
    let active: Vec<(&&Theme, &ThemeTrack)> = shown.iter().zip(&tracks).filter(|(t, _)| t.state == "active").collect();
    let counts = [
        (active.iter().filter(|(t, _)| !t.owner_ask.is_empty()).count(), "waiting on you"),
        (active.iter().filter(|(_, k)| matches!(k.kind, TrackKind::Moving | TrackKind::Stuck)).count(), "moving"),
        (active.iter().filter(|(_, k)| k.kind == TrackKind::Quiet).count(), "quiet"),
    ];
    let parts: Vec<String> = counts.iter().filter(|(n, _)| *n > 0).map(|(n, w)| format!("{n} {w}")).collect();
    Pages {
        tickets: r.plan.lines.iter().map(|l| r.page(l)).collect(),
        tracks,
        track_summary: (!parts.is_empty()).then(|| parts.join(DOT)),
    }
}
