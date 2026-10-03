//! The report as a pure function of the board, a period and a clock.
//!
//! Each task's record is read once, into a [`Reading`]: the spells it spent
//! in each status, the moves that matter, its questions and their answers.
//! Every tally after that is arithmetic over readings, so the totals, a
//! workspace, an area and a label are all one function applied to different
//! sets of tasks, and cannot disagree about what a number means.

use std::collections::{BTreeMap, HashSet};

use farcooler_store::models::{Actor, NoteKind, TaskStatus};

use super::{
    Group, Inputs, NOTABLE_LIMIT, Notable, NotableTask, NotableWait, Period, Report, SCHEMA, Spread, StatusTime, Tally, TaskFacts, Usage,
};

/// The statuses a task waits in, in board order. Done and Canceled end a
/// task; time "in" them is time since it ended, which says nothing.
const OPEN: [TaskStatus; 5] = [
    TaskStatus::Backlog,
    TaskStatus::Todo,
    TaskStatus::NeedsDecision,
    TaskStatus::InProgress,
    TaskStatus::InReview,
];

/// A stretch of time one task spent in one status. `end` is `None` while the
/// task is still there.
#[derive(Debug, Clone, Copy)]
struct Spell {
    status: TaskStatus,
    start: i64,
    end: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
struct Question {
    asked: i64,
    answer: Option<(i64, Actor)>,
    /// When the task was done or canceled with this still unanswered: the
    /// question went away without an answer note, and nobody waits on it.
    closed: Option<i64>,
}

impl Question {
    /// Still waiting at `at`: asked before it, and neither answered nor
    /// closed by then.
    fn open_at(&self, at: i64) -> bool {
        self.asked < at && self.answer.is_none_or(|(a, _)| a >= at) && self.closed.is_none_or(|c| c >= at)
    }
}

/// One task's record, read once.
#[derive(Debug)]
struct Reading<'a> {
    facts: &'a TaskFacts,
    created_in: bool,
    /// The last move to Done inside the period.
    done_at: Option<i64>,
    canceled_in: bool,
    reopened: u32,
    fix_rounds: u32,
    spells: Vec<Spell>,
    questions: Vec<Question>,
    /// `Decision` notes inside the period.
    recorded: u32,
}

impl Reading<'_> {
    fn time_to_done(&self) -> Option<i64> {
        self.done_at.map(|at| at - self.facts.task.created_at)
    }
}

/// A group's name, and the repository a workspace is in.
type GroupKey = (String, Option<String>);

/// The report for `inputs` over `period`, as of `now`.
pub fn compute(inputs: &Inputs, period: Period, now: i64) -> Report {
    let horizon = period.until.min(now);
    let readings: Vec<Reading> = inputs.tasks.iter().map(|t| read(t, period)).collect();
    let all: Vec<&Reading> = readings.iter().collect();

    let grouped = |key: &dyn Fn(&Reading) -> Vec<GroupKey>| -> Vec<Group> {
        let mut groups: BTreeMap<GroupKey, Vec<&Reading>> = BTreeMap::new();
        for r in &readings {
            for k in key(r) {
                groups.entry(k).or_default().push(r);
            }
        }
        let mut out: Vec<Group> = groups
            .into_iter()
            .map(|((name, repository), members)| Group {
                name,
                repository,
                tally: tally(&members, inputs, period, horizon),
            })
            .filter(|g| !quiet(&g.tally))
            .collect();
        // Stable, and the map was ordered by name: most completed first, then
        // by name.
        out.sort_by_key(|g| std::cmp::Reverse(g.tally.completed));
        out
    };

    Report {
        schema: SCHEMA,
        since: period.since,
        until: period.until,
        generated_at: now,
        scope: inputs.scope.clone(),
        totals: tally(&all, inputs, period, horizon),
        by_repository: grouped(&|r| vec![(r.facts.repository.clone(), None)]),
        by_workspace: grouped(&|r| vec![(r.facts.workspace.clone(), Some(r.facts.repository.clone()))]),
        by_area: grouped(&|r| {
            vec![(area_of(&r.facts.task.title).unwrap_or("Other").to_string(), None)]
        }),
        by_label: grouped(&|r| r.facts.task.labels.iter().map(|l| (l.clone(), None)).collect()),
        notable: notable(&all, period, horizon),
    }
}

/// The `Area` of an `Area: outcome` title, when it has one: one or two words
/// of letters before the first colon, with something after.
/// "Mac: the jumpbar jumps anywhere" is `Mac`; "Fix: a: b" is `Fix`; a title
/// with no colon, or a sentence that happens to hold one, has none.
pub fn area_of(title: &str) -> Option<&str> {
    let (head, rest) = title.split_once(':')?;
    let head = head.trim();
    let words = head.split_whitespace().count();
    let starts_with_letter = head.chars().next().is_some_and(char::is_alphabetic);
    let plain = head.chars().all(|c| c.is_alphabetic() || c == ' ');
    (starts_with_letter && plain && (1..=2).contains(&words) && head.len() <= 24 && !rest.trim().is_empty())
        .then_some(head)
}

fn read(facts: &TaskFacts, period: Period) -> Reading<'_> {
    let task = &facts.task;
    let mut r = Reading {
        facts,
        created_in: period.contains(task.created_at),
        done_at: None,
        canceled_in: false,
        reopened: 0,
        fix_rounds: 0,
        spells: vec![Spell { status: TaskStatus::Backlog, start: task.created_at, end: None }],
        questions: Vec::new(),
        recorded: 0,
    };

    // A question another question replaces was withdrawn.
    let withdrawn: HashSet<_> = facts
        .notes
        .iter()
        .filter(|n| n.kind == NoteKind::Question)
        .filter_map(|n| n.supersedes)
        .collect();
    let mut open_questions: Vec<usize> = Vec::new();

    for note in &facts.notes {
        match note.kind {
            NoteKind::StatusChange => {
                let Some(to) = note.extra.get("to").and_then(|v| v.as_str()).and_then(TaskStatus::parse) else {
                    continue;
                };
                let at = note.at.max(task.created_at);
                let current = r.spells.last_mut().expect("a task starts in a spell");
                let from = current.status;
                current.end = Some(at);
                r.spells.push(Spell { status: to, start: at, end: None });
                if matches!(to, TaskStatus::Done | TaskStatus::Cancelled) {
                    for i in open_questions.drain(..) {
                        r.questions[i].closed = Some(at);
                    }
                }
                if !period.contains(at) {
                    continue;
                }
                match (from, to) {
                    (_, TaskStatus::Done) => r.done_at = Some(at),
                    (_, TaskStatus::Cancelled) => r.canceled_in = true,
                    (TaskStatus::InReview, TaskStatus::InProgress) => r.fix_rounds += 1,
                    _ => {}
                }
                if from == TaskStatus::Done {
                    r.reopened += 1;
                }
            }
            NoteKind::Question if !withdrawn.contains(&note.id) => {
                open_questions.push(r.questions.len());
                r.questions.push(Question { asked: note.at, answer: None, closed: None });
            }
            NoteKind::Answer => {
                for i in open_questions.drain(..) {
                    r.questions[i].answer = Some((note.at, note.actor));
                }
            }
            NoteKind::Decision if period.contains(note.at) => r.recorded += 1,
            _ => {}
        }
    }
    // A task done inside the period and then moved on again inside it is
    // not done. One moved on after the period ended still was, then.
    if let Some(done) = r.done_at {
        let undone_in_period = r.spells.iter().any(|s| {
            s.status == TaskStatus::Done && s.start == done && s.end.is_some_and(|e| period.contains(e))
        });
        if undone_in_period {
            r.done_at = None;
        }
    }
    r
}

/// The part of `[start, end)` inside `[since, horizon)`.
fn overlap(start: i64, end: Option<i64>, since: i64, horizon: i64) -> i64 {
    (end.unwrap_or(horizon).min(horizon) - start.max(since)).max(0)
}

fn tally(readings: &[&Reading], inputs: &Inputs, period: Period, horizon: i64) -> Tally {
    let mut t = Tally::default();
    let mut to_done = Vec::new();
    let (mut latency, mut latency_you, mut to_clear) = (Vec::new(), Vec::new(), Vec::new());
    let mut in_status: [Vec<i64>; 5] = Default::default();
    let mut usage: Option<Usage> = None;

    for r in readings {
        t.created += u32::from(r.created_in);
        t.canceled += u32::from(r.canceled_in);
        t.reopened += r.reopened;
        t.fix_rounds += r.fix_rounds;
        t.decisions.recorded += r.recorded;

        if let Some(ms) = r.time_to_done() {
            t.completed += 1;
            to_done.push(ms);
            let acceptance = &r.facts.task.acceptance;
            let met = acceptance.iter().filter(|a| a.met).count() as u32;
            t.acceptance.met += met;
            t.acceptance.total += acceptance.len() as u32;
            t.acceptance.tasks_fully_met += u32::from(!acceptance.is_empty() && met as usize == acceptance.len());
            t.acceptance.tasks_without_lines += u32::from(acceptance.is_empty());
        }

        for (slot, status) in in_status.iter_mut().zip(OPEN) {
            let ms: i64 = r
                .spells
                .iter()
                .filter(|s| s.status == status)
                .map(|s| overlap(s.start, s.end, period.since, horizon))
                .sum();
            if ms > 0 {
                slot.push(ms);
            }
        }

        for q in &r.questions {
            t.decisions.asked += u32::from(period.contains(q.asked));
            match q.answer {
                Some((at, actor)) if period.contains(at) => {
                    t.decisions.answered += 1;
                    latency.push(at - q.asked);
                    if actor == Actor::User {
                        t.decisions.answered_by_you += 1;
                        latency_you.push(at - q.asked);
                    }
                    t.decisions.answered_by_orchestrator += u32::from(actor == Actor::Manager);
                }
                _ => {}
            }
            t.decisions.unanswered += u32::from(q.open_at(period.until));
            t.decisions.closed_unanswered += u32::from(q.answer.is_none() && q.closed.is_some_and(|c| period.contains(c)));
        }

        for s in r.spells.iter().filter(|s| waits_on_you(s.status)) {
            if period.contains(s.start) {
                t.needs_you.times += 1;
                t.needs_you.decisions += u32::from(s.status == TaskStatus::NeedsDecision);
                t.needs_you.reviews += u32::from(s.status == TaskStatus::InReview);
            }
            match s.end {
                Some(end) if period.contains(end) => {
                    t.needs_you.cleared += 1;
                    to_clear.push(end - s.start);
                }
                Some(end) if end < period.until => {}
                _ => t.needs_you.waiting += u32::from(s.start < period.until),
            }
        }

        if let Some(u) = inputs.usage.get(&r.facts.task.id) {
            usage = Some(usage.unwrap_or_default().plus(*u));
        }
    }

    t.time_to_done = spread(to_done);
    t.decisions.latency = spread(latency);
    t.decisions.latency_you = spread(latency_you);
    t.needs_you.time_to_clear = spread(to_clear);
    t.time_in_status = OPEN
        .iter()
        .zip(in_status)
        .filter(|(_, v)| !v.is_empty())
        .map(|(status, v)| StatusTime {
            status: status.as_str().to_string(),
            total_ms: v.iter().sum(),
            tasks: v.len() as u32,
            median_ms: median(&v),
        })
        .collect();
    t.usage = usage;
    t
}

/// Needs Decision and In Review: the two statuses the needs-you rollup
/// counts as a person's to act on.
fn waits_on_you(status: TaskStatus) -> bool {
    matches!(status, TaskStatus::NeedsDecision | TaskStatus::InReview)
}

/// Nothing happened and nothing was worked on: no event in the period, and
/// no time in any status but Backlog and To Do. A label every parked task
/// carries is not news.
fn quiet(t: &Tally) -> bool {
    let events = t.created + t.completed + t.canceled + t.reopened + t.fix_rounds
        + t.decisions.asked + t.decisions.answered + t.decisions.unanswered + t.decisions.recorded
        + t.decisions.closed_unanswered
        + t.needs_you.times + t.needs_you.cleared + t.needs_you.waiting;
    let worked = t.time_in_status.iter().any(|s| s.status != "backlog" && s.status != "todo");
    events == 0 && !worked && t.usage.is_none()
}

fn spread(values: Vec<i64>) -> Option<Spread> {
    (!values.is_empty()).then(|| Spread { count: values.len() as u32, median_ms: median(&values), p90_ms: p90(&values) })
}

/// The middle value, or the mean of the two middle values (rounded down).
/// Zero for no values.
pub fn median(values: &[i64]) -> i64 {
    let mut v = values.to_vec();
    v.sort_unstable();
    match v.len() {
        0 => 0,
        n if n % 2 == 1 => v[n / 2],
        n => (v[n / 2 - 1] + v[n / 2]).div_euclid(2),
    }
}

/// Nearest rank: the value at position ceil(0.9 n), counting from one.
/// Zero for no values.
pub fn p90(values: &[i64]) -> i64 {
    let mut v = values.to_vec();
    v.sort_unstable();
    if v.is_empty() {
        return 0;
    }
    let rank = (9 * v.len()).div_ceil(10);
    v[rank - 1]
}

fn notable(readings: &[&Reading], period: Period, horizon: i64) -> Notable {
    let named = |r: &Reading, ms: Option<i64>, count: Option<u32>| NotableTask {
        key: r.facts.task.key.clone(),
        title: r.facts.task.title.clone(),
        workspace: r.facts.workspace.clone(),
        ms,
        count,
    };
    let top = |mut v: Vec<NotableTask>| {
        // Biggest first; ties in the order the tasks were filed.
        v.sort_by(|a, b| b.ms.cmp(&a.ms).then(b.count.cmp(&a.count)));
        v.truncate(NOTABLE_LIMIT);
        v
    };

    let mut waits = Vec::new();
    for r in readings {
        let wait = |kind: &str, ms: i64, open: bool| NotableWait {
            key: r.facts.task.key.clone(),
            title: r.facts.task.title.clone(),
            workspace: r.facts.workspace.clone(),
            kind: kind.to_string(),
            ms,
            open,
        };
        for q in &r.questions {
            match q.answer {
                Some((at, _)) if period.contains(at) => waits.push(wait("question", at - q.asked, false)),
                _ if q.open_at(period.until) => waits.push(wait("question", horizon - q.asked, true)),
                _ => {}
            }
        }
        for s in r.spells.iter().filter(|s| s.status == TaskStatus::InReview) {
            match s.end {
                Some(end) if period.contains(end) => waits.push(wait("review", end - s.start, false)),
                Some(end) if end < period.until => {}
                _ if s.start < period.until => waits.push(wait("review", horizon - s.start, true)),
                _ => {}
            }
        }
    }
    waits.sort_by_key(|w| std::cmp::Reverse(w.ms));
    waits.truncate(NOTABLE_LIMIT);

    Notable {
        slowest: top(readings.iter().filter_map(|r| r.time_to_done().map(|ms| named(r, Some(ms), None))).collect()),
        longest_waits: waits,
        reopened: top(readings.iter().filter(|r| r.reopened > 0).map(|r| named(r, None, Some(r.reopened))).collect()),
        most_fix_rounds: top(
            readings.iter().filter(|r| r.fix_rounds > 0).map(|r| named(r, None, Some(r.fix_rounds))).collect(),
        ),
        canceled: top(readings.iter().filter(|r| r.canceled_in).map(|r| named(r, None, None)).collect()),
    }
}
