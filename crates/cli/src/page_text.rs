//! A page as text, the way the apps draw it (ov-269, ov-283).
//!
//! `farcooler page show` prints this so the orchestrator reads what the owner
//! sees: tables as aligned columns, lists with their state words, and each live
//! reference resolved against the board it was read from. A reference the board
//! can't resolve (a lane that's gone, a runner with no plan) is plain text, as
//! it is in the apps.

use std::collections::HashMap;

use farcooler_core::local_time;
use farcooler_core::page_doc::{Block, Cell, Entry, Item, Order, Page, Reference, Show, Stat, State, Target, Tone, url_host};
use farcooler_core::usage_words::{NOT_REPORTED, dollars, tokens};
use farcooler_protocol::v1 as pb;

use crate::ci_words;

/// What a page's references are drawn from: the board as the app holds it.
#[derive(Default)]
pub(crate) struct Live {
    /// Cards by lowercase key: title and status.
    pub(crate) cards: HashMap<String, (String, i32)>,
    /// The plan, when the runner has one and the page names a lane, a theme,
    /// CI or a card count: CI reads and the board's counts come with it
    /// (ov-306).
    pub(crate) plan: Option<pb::Plan>,
    /// Pages by slot: title.
    pub(crate) pages: HashMap<String, String>,
}

const ATTENTION: &str = " (attention)";

fn tone(t: Tone) -> &'static str {
    if t == Tone::Attention { ATTENTION } else { "" }
}

/// A lane's state, as a person reads it.
fn lane_state(l: &pb::Lane) -> String {
    let word = match pb::LaneState::try_from(l.state) {
        Ok(pb::LaneState::Queued) => "Queued",
        Ok(pb::LaneState::Building) => "Building",
        Ok(pb::LaneState::Review) => "In review",
        Ok(pb::LaneState::Fixing) => "Fixing",
        Ok(pb::LaneState::Landing) => "Landing",
        Ok(pb::LaneState::Landed) => "Landed",
        Ok(pb::LaneState::Dropped) => "Dropped",
        _ => "Unknown",
    };
    if l.state == pb::LaneState::Fixing as i32 && l.fix_rounds > 0 {
        format!("{word} · round {}", l.fix_rounds)
    } else {
        word.to_string()
    }
}

fn task_status(status: i32) -> &'static str {
    match pb::TaskStatus::try_from(status) {
        Ok(pb::TaskStatus::Backlog) => "Backlog",
        Ok(pb::TaskStatus::Todo) => "To Do",
        Ok(pb::TaskStatus::NeedsDecision) => "Needs Decision",
        Ok(pb::TaskStatus::InProgress) => "In Progress",
        Ok(pb::TaskStatus::InReview) => "In Review",
        Ok(pb::TaskStatus::Done) => "Done",
        Ok(pb::TaskStatus::Cancelled) => "Cancelled",
        _ => "Unknown",
    }
}

fn spend(s: &pb::LaneSpend) -> String {
    let total = s.input_tokens + s.output_tokens + s.cache_read_tokens + s.cache_write_tokens;
    if total == 0 {
        return NOT_REPORTED.to_string();
    }
    let mut said = format!("{} tokens", tokens(total));
    if let Some(micros) = s.cost_micros {
        said.push_str(&format!(" · {} API-equivalent", dollars(micros)));
    }
    said
}

/// "1 of 2 done", the cancelled cards left out, as the apps say it.
fn theme_progress(v: &pb::BoardThemeView) -> String {
    let c = v.counts.unwrap_or_default();
    let total = c.backlog + c.todo + c.needs_decision + c.in_progress + c.in_review + c.done;
    format!("{} of {total} done", c.done)
}

fn short(text: &str, max: usize) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() { format!("{head}...") } else { head }
}

/// What a reference draws, and whether it needs the owner.
fn live_value(reference: &Reference, show: Option<Show>, live: &Live) -> (String, bool) {
    let label = reference.label.as_deref();
    match &reference.target {
        Target::Task(key) => match live.cards.get(&key.to_ascii_lowercase()) {
            Some((title, status)) => (format!("{key} {} ({})", short(title, 40), task_status(*status)), false),
            None => (label.unwrap_or(key).to_string(), false),
        },
        Target::Ask(key) => match live.cards.get(&key.to_ascii_lowercase()) {
            Some((_, status)) if *status == pb::TaskStatus::NeedsDecision as i32 => ("Needs you".to_string(), true),
            Some(_) => ("Answered".to_string(), false),
            None => (label.unwrap_or(key).to_string(), false),
        },
        Target::Lane(name) => {
            let lane = live
                .plan
                .as_ref()
                .and_then(|p| p.lanes.iter().find(|l| l.name.eq_ignore_ascii_case(name)));
            match (lane, show) {
                (Some(l), Some(Show::State)) => (lane_state(l), false),
                (Some(l), Some(Show::Spend)) => (spend(&l.spend.unwrap_or_default()), false),
                (Some(l), None) => (label.unwrap_or(&l.name).to_string(), false),
                (None, _) => (label.unwrap_or(name).to_string(), false),
            }
        }
        Target::Theme(name) => {
            let theme = live.plan.as_ref().and_then(|p| {
                p.themes.iter().find(|v| v.theme.as_ref().is_some_and(|t| t.name.eq_ignore_ascii_case(name)))
            });
            match (theme, show) {
                (Some(v), Some(Show::Spend)) => (spend(&v.spend.unwrap_or_default()), false),
                (Some(v), _) => (format!("{}, {}", label.unwrap_or(&v.theme.clone().unwrap_or_default().name), theme_progress(v)), false),
                (None, _) => (label.unwrap_or(name).to_string(), false),
            }
        }
        Target::Ci(_) => match ci_read(reference, live) {
            Some(read) => {
                let now = live.plan.as_ref().map_or(0, |p| p.now_ms);
                let mut said = format!("{}: {}", ci_name(reference), ci_words::summary(read));
                if let Some(stale) = ci_words::stale(read, now) {
                    said.push_str(&format!(" · {stale}"));
                }
                (said, ci_words::needs_attention(read))
            }
            None => (ci_name(reference), false),
        },
        Target::Cards(status) => match card_count(status, live) {
            Some(n) => (n.to_string(), false),
            None => (label.unwrap_or(status).to_string(), false),
        },
        Target::Page(slot) => (label.map(str::to_string).or_else(|| live.pages.get(slot).cloned()).unwrap_or_else(|| slot.clone()), false),
        Target::Worktree(name) => (label.unwrap_or(name).to_string(), false),
        Target::Terminal { name, .. } => (label.unwrap_or(name).to_string(), false),
        Target::Url(url) => {
            let domain = url_host(url).unwrap_or("");
            (match label {
                Some(label) => format!("{label} ({domain})"),
                None => domain.to_string(),
            }, false)
        }
    }
}

/// What a CI reference is called: its label, else "Main", "Run 812" or the
/// commit's first eight digits.
fn ci_name(reference: &Reference) -> String {
    if let Some(label) = &reference.label {
        return label.clone();
    }
    match &reference.target {
        Target::Ci(s) if s == "main" => "Main".to_string(),
        Target::Ci(s) => match s.strip_prefix("run:") {
            Some(id) => format!("Run {id}"),
            None => s.chars().take(8).collect(),
        },
        other => other.name().to_string(),
    }
}

/// The runner's last read of a CI reference, when the plan carries one.
fn ci_read<'a>(reference: &Reference, live: &'a Live) -> Option<&'a pb::BoardCiRead> {
    let subject = reference.target.ci_subject()?;
    ci_words::read_for(&live.plan.as_ref()?.ci, &subject)
}

/// How many of the board's cards are in `status` (`open`: not done or
/// canceled), when the plan carries the board's counts.
fn card_count(status: &str, live: &Live) -> Option<u32> {
    let c = live.plan.as_ref()?.board_counts?;
    Some(match status {
        "backlog" => c.backlog,
        "todo" => c.todo,
        "needs_decision" => c.needs_decision,
        "in_progress" => c.in_progress,
        "in_review" => c.in_review,
        "done" => c.done,
        "cancelled" => c.cancelled,
        "open" => c.backlog + c.todo + c.needs_decision + c.in_progress + c.in_review,
        _ => return None,
    })
}

/// A figure's value, its detail and whether it needs the owner: as written,
/// or drawn live from its reference (ov-306). A CI figure is its status word,
/// with how its jobs stand as the detail unless the page gave one.
fn stat_value(s: &Stat, live: &Live) -> (String, Option<String>, bool) {
    let Some(reference) = &s.reference else { return (s.value.clone(), s.detail.clone(), false) };
    if let (Target::Ci(_), Some(read)) = (&reference.target, ci_read(reference, live)) {
        let summary = ci_words::summary(read);
        let (word, jobs) = summary.split_once(" · ").map_or((summary.as_str(), None), |(w, j)| (w, Some(j.to_string())));
        // A stale read says how old it is before anything else under it.
        let stale = ci_words::stale(read, live.plan.as_ref().map_or(0, |p| p.now_ms));
        return (word.to_string(), stale.or(s.detail.clone()).or(jobs), ci_words::needs_attention(read));
    }
    let (value, attention) = match (&reference.target, s.show) {
        (Target::Theme(name), None) => {
            let theme = live.plan.as_ref().and_then(|p| {
                p.themes.iter().find(|v| v.theme.as_ref().is_some_and(|t| t.name.eq_ignore_ascii_case(name)))
            });
            theme.map_or_else(|| (reference.label.clone().unwrap_or_else(|| name.clone()), false), |v| (theme_progress(v), false))
        }
        (Target::Lane(_), None) => live_value(reference, Some(Show::State), live),
        _ => live_value(reference, s.show, live),
    };
    (value, s.detail.clone(), attention)
}

/// Where a reference goes, for a person who can't tap it: `(ask ov-274)`.
fn goes_to(reference: &Reference) -> String {
    match &reference.target {
        Target::Terminal { worktree, name } => format!("terminal {worktree}/{name}"),
        Target::Url(url) => url.clone(),
        other => format!("{} {}", other.kind(), other.name()),
    }
}

fn cell_text(cell: &Cell, live: &Live) -> String {
    let mut said = match (&cell.text, &cell.reference) {
        (Some(text), _) => text.clone(),
        (None, Some(reference)) => {
            let (value, attention) = live_value(reference, cell.show, live);
            if attention { format!("{value}{ATTENTION}") } else { value }
        }
        (None, None) => String::new(),
    };
    said.push_str(tone(cell.tone));
    said
}

fn item_line(item: &Item, live: &Live) -> String {
    let mut line = match item.state {
        State::None => "- ".to_string(),
        state => format!("- [{}] ", state.word()),
    };
    line.push_str(&item.text);
    line.push_str(tone(item.tone));
    if let Some(detail) = &item.detail {
        line.push_str(&format!(" · {detail}"));
    }
    if let Some(reference) = &item.reference {
        let (value, attention) = live_value(reference, None, live);
        line.push_str(&format!(" -> {value}{} ({})", if attention { ATTENTION } else { "" }, goes_to(reference)));
    }
    line
}

fn entry_line(entry: &Entry, live: &Live) -> String {
    let mut line = format!("{}  {}", local_time::moment(entry.at), entry.text);
    if let Some(reference) = &entry.reference {
        let (value, attention) = live_value(reference, None, live);
        line.push_str(&format!(" -> {value}{}", if attention { ATTENTION } else { "" }));
    }
    line
}

fn bar(done: u32, total: u32) -> String {
    let filled = ((u64::from(done) * 10) / u64::from(total.max(1))) as usize;
    format!("[{}{}]", "#".repeat(filled), "-".repeat(10 - filled))
}

/// "5 min ago", "3 h ago": how long since `at`, coarsely.
fn ago(ms: i64) -> String {
    let minutes = ms.max(0) / 60_000;
    match minutes {
        0 => "just now".to_string(),
        1..=59 => format!("{minutes} min ago"),
        60..=1439 => format!("{} h ago", minutes / 60),
        _ => format!("{} d ago", minutes / 1440),
    }
}

fn block_text(block: &Block, live: &Live, out: &mut Vec<String>) {
    match block {
        Block::Heading { text } => out.push(format!("## {text}")),
        Block::Text { md, tone: t } => out.push(format!("{md}{}", tone(*t))),
        Block::Stats { items } => {
            let figures: Vec<String> = items
                .iter()
                .map(|s| {
                    let (value, detail, attention) = stat_value(s, live);
                    let mut f = format!("{}: {value}", s.label);
                    if let Some(detail) = &detail {
                        f.push_str(&format!(" ({detail})"));
                    }
                    f.push_str(if attention { ATTENTION } else { tone(s.tone) });
                    f
                })
                .collect();
            out.push(figures.join(" · "));
        }
        Block::Progress { label, done, total, detail, parts } => {
            let mut line = format!("{label}: {done} of {total} {}", bar(*done, *total));
            if let Some(detail) = detail {
                line.push_str(&format!(" · {detail}"));
            }
            out.push(line);
            if !parts.is_empty() {
                out.push(format!("  {}", parts.iter().map(|p| format!("{} {}", p.label, p.count)).collect::<Vec<_>>().join(" · ")));
            }
        }
        Block::Table { columns, rows } => {
            let cells: Vec<Vec<String>> = rows.iter().map(|r| r.iter().map(|c| cell_text(c, live)).collect()).collect();
            let widths: Vec<usize> = (0..columns.len())
                .map(|c| {
                    cells.iter().map(|r| r[c].chars().count()).chain([columns[c].title.chars().count()]).max().unwrap_or(0)
                })
                .collect();
            let line = |texts: Vec<&str>| -> String {
                texts
                    .iter()
                    .enumerate()
                    .map(|(c, t)| {
                        let pad = widths[c].saturating_sub(t.chars().count());
                        match columns[c].align {
                            farcooler_core::page_doc::Align::End => format!("{}{t}", " ".repeat(pad)),
                            _ => format!("{t}{}", " ".repeat(pad)),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("  ")
                    .trim_end()
                    .to_string()
            };
            out.push(line(columns.iter().map(|c| c.title.as_str()).collect()));
            for row in &cells {
                out.push(line(row.iter().map(String::as_str).collect()));
            }
        }
        Block::List { items } => out.extend(items.iter().map(|i| item_line(i, live))),
        Block::Timeline { order, entries } => {
            let mut sorted: Vec<&Entry> = entries.iter().collect();
            if *order == Order::Newest {
                sorted.sort_by_key(|e| std::cmp::Reverse(e.at));
            }
            out.extend(sorted.iter().map(|e| entry_line(e, live)));
        }
        Block::Steps { steps } => {
            out.push(steps.iter().map(|s| format!("{} ({})", s.label, s.state.word())).collect::<Vec<_>>().join(" > "));
        }
        Block::Links { items } => {
            let links: Vec<String> = items
                .iter()
                .map(|r| {
                    let (value, _) = live_value(r, None, live);
                    format!("{value} [{}]", goes_to(r))
                })
                .collect();
            out.push(format!("Links: {}", links.join("; ")));
        }
    }
}

/// The page, as `show` prints it. `now` is the runner's clock in milliseconds.
pub(crate) fn render(stored: &pb::BoardPage, page: &Page, theme: Option<&str>, live: &Live, now: i64) -> String {
    let mut out = vec![page.title.clone()];
    if !page.summary.is_empty() {
        out.push(page.summary.clone());
    }
    let mut meta = format!("Updated {} by {} · revision {}", ago(now - stored.updated_at_ms), stored.actor, stored.revision);
    if let Some(theme) = theme {
        meta.push_str(&format!(" · in theme {theme}"));
    }
    if let Some(stale) = page.stale_after_min
        && now - stored.updated_at_ms > i64::from(stale) * 60_000
    {
        meta.push_str(&format!(" · not updated for {}", ago(now - stored.updated_at_ms).trim_end_matches(" ago")));
    }
    out.push(meta);
    for block in &page.blocks {
        out.push(String::new());
        block_text(block, live, &mut out);
    }
    out.join("\n")
}
