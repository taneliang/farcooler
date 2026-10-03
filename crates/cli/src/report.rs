//! `farcooler report`: what got done in a period, from the runner's board.
//!
//! The runner computes it (`report.get`, `farcooler_daemon::report`); this
//! works out the period in the local time zone, names the repository or
//! workspace, and prints. `--json` prints the runner's JSON untouched, the
//! shape an app reads; otherwise a summary in plain words, built from the
//! same structs.

use std::error::Error;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Args;
use farcooler_core::usage_words::tokens;
use farcooler_daemon::report::spend::{SpendLine, SpendReport};
use farcooler_daemon::report::{Group, Report, Spread, Tally};
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::ClientError;
use uuid::Uuid;

use crate::tasks::{DispatchLink, Refused, refusal};
use crate::{Fallible, connect_to, expect_value, req, with};

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

/// `farcooler report`'s flags.
#[derive(Debug, Clone, Args)]
pub struct ReportArgs {
    /// Where the period starts: `24h`, `7d`, `2w` or `90m` ago, `today`,
    /// `yesterday`, or a date and time like `2026-09-28` or
    /// `2026-09-28 14:00`, in this machine's time zone. The last 7 days if
    /// left out.
    #[arg(long)]
    since: Option<String>,
    /// Where it ends, in the same words as `--since`. Now if left out.
    #[arg(long)]
    until: Option<String>,
    /// Only this repository, by name. Every repository on the runner if
    /// left out.
    #[arg(long)]
    repo: Option<String>,
    /// Only this workspace, by name or task prefix.
    #[arg(long)]
    workspace: Option<String>,
}

/// What a runner too old for reports is told.
const NO_REPORT: &str = "this runner's Far Cooler is older than reports. update it and try again";

/// What a refused report is told, whatever the runner's reason.
const REPORT_UNREAD: &str = "the runner couldn't make the report. try again";

pub async fn report(runner: Option<&str>, args: ReportArgs, json: bool) -> Fallible {
    let now = now_millis();
    let period = period(&args, now)?;
    let mut link = connect_to(runner).await?;
    println!("{}", report_read(&mut link, &args, period, json).await?);
    Ok(())
}

/// The report, read and printed as `--json` or the summary.
async fn report_read<L: DispatchLink>(
    link: &mut L,
    args: &ReportArgs,
    period: Period,
    json: bool,
) -> Result<String, Box<dyn Error>> {
    if !link.capabilities().iter().any(|c| c == farcooler_protocol::capability::REPORT) {
        return Err(Box::new(Refused::new(
            NO_REPORT.to_string(),
            Some(pb::ErrorCode::CapabilityUnsupported as i32),
        )));
    }
    let (repository_id, workspace_id) = narrowing(link, args).await?;
    let ask = pb::ReportRequest {
        since: period.since,
        until: period.until,
        repository_id: repository_id.map(|id| bytes::Bytes::copy_from_slice(id.as_bytes())),
        workspace_id: workspace_id.map(|id| bytes::Bytes::copy_from_slice(id.as_bytes())),
        utc_offset_minutes: utc_offset_minutes(period.until),
    };
    let r = link
        .call(with(req("report.get"), request::Payload::ReportRequest(ask)))
        .await
        .map_err(|e| Box::new(refused(e)) as Box<dyn Error>)?;
    let result::Value::Report(r) = expect_value(r.value)? else {
        return Err(crate::daemon_link::UNREADABLE.into());
    };
    if json {
        return Ok(r.report_json);
    }
    let report: Report =
        serde_json::from_str(&r.report_json).map_err(|_| crate::daemon_link::UNREADABLE)?;
    Ok(render(&report, period.label.as_deref()))
}

fn refused(e: ClientError) -> Refused {
    match &e {
        ClientError::Daemon { code, .. } if farcooler_core::error::word_for(*code) == "capability-unsupported" => {
            Refused::new(NO_REPORT.to_string(), Some(*code))
        }
        ClientError::Daemon { code, what, .. } if farcooler_core::error::word_for(*code) == "not-found" => {
            Refused::naming("that repository or workspace isn't on this runner".to_string(), *code, what.clone())
        }
        ClientError::Daemon { code, what, .. } => Refused::naming(REPORT_UNREAD.to_string(), *code, what.clone()),
        _ => refusal(e, REPORT_UNREAD),
    }
}

/// The repository or workspace `--repo` and `--workspace` name, as ids.
async fn narrowing<L: DispatchLink>(
    link: &mut L,
    args: &ReportArgs,
) -> Result<(Option<Uuid>, Option<Uuid>), Box<dyn Error>> {
    if args.repo.is_none() && args.workspace.is_none() {
        return Ok((None, None));
    }
    let repositories = crate::list_repositories(link).await?;
    let Some(given) = args.workspace.as_deref() else {
        let name = args.repo.as_deref().unwrap_or_default();
        let repository = crate::resolve_repository(&repositories, name)?;
        return Ok((Some(uuid_of(&repository.id)), None));
    };
    let all = crate::workspaces::workspaces_on(link, None).await?;
    let scoped = crate::workspaces::scope(&all, &repositories, args.repo.as_deref(), None)?;
    let workspace = crate::workspaces::resolve_workspace(&scoped, &repositories, given)?;
    Ok((None, Some(uuid_of(&workspace.id))))
}

fn uuid_of(bytes: &[u8]) -> Uuid {
    Uuid::from_slice(bytes).unwrap_or(Uuid::nil())
}

fn now_millis() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

// ---- the period ----

/// `[since, until)`, and how to say it when it's "the last N".
#[derive(Debug, Clone, PartialEq, Eq)]
struct Period {
    since: i64,
    until: i64,
    label: Option<String>,
}

fn period(args: &ReportArgs, now: i64) -> Result<Period, String> {
    let since_text = args.since.as_deref().unwrap_or("7d");
    let since = when(since_text, now)?;
    let until = match args.until.as_deref() {
        Some(text) => when(text, now)?,
        None => now,
    };
    if until <= since {
        return Err("the period ends before it starts. give --until a later time than --since".into());
    }
    let label = args.until.is_none().then(|| last(since_text)).flatten();
    Ok(Period { since, until, label })
}

/// "Last 7 days" for `7d`, when the period runs to now.
fn last(text: &str) -> Option<String> {
    let (n, unit) = relative(text)?;
    let word = match unit {
        MINUTE => "minute",
        HOUR => "hour",
        DAY => "day",
        _ => "week",
    };
    Some(if n == 1 { format!("Last {word}") } else { format!("Last {n} {word}s") })
}

/// `7d` as `(7, DAY)`.
fn relative(text: &str) -> Option<(i64, i64)> {
    let text = text.trim();
    let split = text.find(|c: char| !c.is_ascii_digit())?;
    let (n, unit) = text.split_at(split);
    let n: i64 = n.parse().ok().filter(|n| *n > 0)?;
    let unit = match unit {
        "m" | "min" => MINUTE,
        "h" => HOUR,
        "d" => DAY,
        "w" => 7 * DAY,
        _ => return None,
    };
    Some((n, unit))
}

/// One end of the period, in Unix milliseconds.
fn when(text: &str, now: i64) -> Result<i64, String> {
    let trimmed = text.trim();
    if let Some((n, unit)) = relative(trimmed) {
        return Ok(now - n * unit);
    }
    match trimmed {
        "now" => return Ok(now),
        "today" => return midnight(now, 0),
        "yesterday" => return midnight(now, -1),
        _ => {}
    }
    local(trimmed).ok_or_else(|| {
        format!("\"{text}\" isn't a time this understands. try 24h, 7d, yesterday, or a date like 2026-09-28")
    })
}

/// Local midnight `days` from the day `now` is in.
fn midnight(now: i64, days: i32) -> Result<i64, String> {
    let tm = broken_down(now);
    to_millis(tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday + days, 0, 0)
        .ok_or_else(|| "couldn't work out the local date".to_string())
}

/// `local`, for `task wait --until`.
pub(crate) fn local_time(text: &str) -> Option<i64> {
    local(text)
}

/// `2026-09-28`, `2026-09-28 14:00` or `2026-09-28T14:00`, local time.
fn local(text: &str) -> Option<i64> {
    let (date, clock) = match text.split_once(['T', ' ']) {
        Some((d, c)) => (d, Some(c)),
        None => (text, None),
    };
    let mut parts = date.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: i32 = parts.next()?.parse().ok()?;
    let day: i32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1970..=9999).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let (hour, minute) = match clock {
        None => (0, 0),
        Some(c) => {
            let (h, m) = c.split_once(':')?;
            let (h, m): (i32, i32) = (h.parse().ok()?, m.parse().ok()?);
            if !(0..=23).contains(&h) || !(0..=59).contains(&m) {
                return None;
            }
            (h, m)
        }
    };
    to_millis(year, month, day, hour, minute)
}

/// A local wall-clock time as Unix milliseconds. `day` may run past the
/// month's end or below 1: `mktime` carries it, which is what "yesterday"
/// on the first of a month needs.
fn to_millis(year: i32, month: i32, day: i32, hour: i32, minute: i32) -> Option<i64> {
    // SAFETY: `mktime` reads and writes only the `tm` it is given, which is
    // fully initialized here. `tm_isdst` of -1 asks it to work out daylight
    // saving for that date, which is the only right answer for a wall-clock
    // time with no offset.
    let seconds = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        tm.tm_year = year - 1900;
        tm.tm_mon = month - 1;
        tm.tm_mday = day;
        tm.tm_hour = hour;
        tm.tm_min = minute;
        tm.tm_isdst = -1;
        libc::mktime(&mut tm)
    };
    (seconds >= 0).then_some(seconds as i64 * 1000)
}

/// This machine's offset from UTC at `millis`, in minutes: where the
/// report's days begin.
fn utc_offset_minutes(millis: i64) -> i32 {
    (broken_down(millis).tm_gmtoff / 60) as i32
}

// The libc crate marks `time_t` deprecated on musl only, warning that it will
// follow musl 1.2's move to 64 bits. On the 64-bit targets we ship it is
// `c_long`, already 64 bits, so there is nothing to act on.
#[cfg_attr(target_env = "musl", allow(deprecated))]
fn broken_down(millis: i64) -> libc::tm {
    let seconds: libc::time_t = (millis / 1000) as libc::time_t;
    // SAFETY: `localtime_r` writes only the `tm` it is given and reads only
    // `seconds`; both outlive the call.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&seconds, &mut tm);
        tm
    }
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// `Sep 26, 3:12 PM`, local time.
fn moment(millis: i64) -> String {
    let tm = broken_down(millis);
    let hour = match tm.tm_hour % 12 {
        0 => 12,
        h => h,
    };
    let half = if tm.tm_hour < 12 { "AM" } else { "PM" };
    format!("{} {}, {hour}:{:02} {half}", MONTHS[tm.tm_mon as usize % 12], tm.tm_mday, tm.tm_min)
}

// ---- the summary ----

/// `3 h 10 min`, `6 min`, `2 d 5 h`.
fn span(ms: i64) -> String {
    let minutes = ms / MINUTE;
    match minutes {
        m if m < 1 => "under a minute".into(),
        m if m < 60 => format!("{m} min"),
        m if m < 24 * 60 => match m % 60 {
            0 => format!("{} h", m / 60),
            rest => format!("{} h {rest} min", m / 60),
        },
        m => match (m / 60) % 24 {
            0 => format!("{} d", m / (24 * 60)),
            hours => format!("{} d {hours} h", m / (24 * 60)),
        },
    }
}

/// A total of time, in hours once it is an hour or more: "41 h", not
/// "1 d 17 h", because a sum of agent time is not a stretch of calendar.
fn hours(ms: i64) -> String {
    match ms {
        ms if ms < HOUR => span(ms),
        ms if ms < 10 * HOUR => span(ms - ms % MINUTE),
        ms => format!("{} h", ms / HOUR),
    }
}

fn status_name(word: &str) -> &str {
    match word {
        "backlog" => "Backlog",
        "todo" => "To Do",
        "needs_decision" => "Needs Decision",
        "in_progress" => "In Progress",
        "in_review" => "In Review",
        other => other,
    }
}

fn plural(n: u32, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

fn median_line(spread: &Option<Spread>) -> Option<String> {
    spread.map(|s| span(s.median_ms))
}

/// One line per thing that happened, in plain words. Lines about nothing are
/// left out.
fn headline(t: &Tally, spent: bool) -> Vec<String> {
    let mut lines = Vec::new();
    let mut moved = vec![format!("{} done", t.completed)];
    if t.canceled > 0 {
        moved.push(format!("{} canceled", t.canceled));
    }
    moved.push(format!("{} new", t.created));
    lines.push(moved.join(", "));
    if t.filed_done > 0 {
        lines.push(format!("{} filed already done, left out of the times below", t.filed_done));
    }
    if let Some(s) = t.time_to_done {
        lines.push(format!("Median time to done {}; 90% within {}", span(s.median_ms), span(s.p90_ms)));
    }
    if let Some(s) = t.work_time {
        lines.push(format!("Median work time {}, from In Progress to Done; 90% within {}", span(s.median_ms), span(s.p90_ms)));
    }
    let mut came_back = Vec::new();
    if t.reopened > 0 {
        came_back.push(format!("{} reopened", plural(t.reopened, "task", "tasks")));
    }
    if t.fix_rounds > 0 {
        came_back.push(plural(t.fix_rounds, "fix round", "fix rounds"));
    }
    if !came_back.is_empty() {
        lines.push(came_back.join(", "));
    }
    let a = &t.acceptance;
    if a.total > 0 {
        lines.push(format!(
            "Acceptance: {} of {} lines met; {} of {} finished every line",
            a.met,
            a.total,
            a.tasks_fully_met,
            plural(t.completed - a.tasks_without_lines, "task", "tasks")
        ));
    }
    // With the spend section below, it says this and more.
    if let Some(u) = t.usage.filter(|_| !spent) {
        let mut used = Vec::new();
        if let Some(ms) = u.agent_ms {
            used.push(format!("Agent time {}", hours(ms)));
        }
        let total: u64 = [u.input_tokens, u.output_tokens, u.cache_read_tokens, u.cache_write_tokens]
            .into_iter()
            .flatten()
            .sum();
        if total > 0 {
            used.push(format!("{} tokens", tokens(total)));
        }
        if !used.is_empty() {
            lines.push(used.join(" · "));
        }
    }
    lines
}

fn people(t: &Tally) -> Vec<String> {
    let d = &t.decisions;
    let n = &t.needs_you;
    let mut lines = Vec::new();
    if d.asked + d.answered + d.unanswered + d.closed_unanswered > 0 {
        let mut q = format!("Questions: {} asked, {} answered", d.asked, d.answered);
        if d.unanswered > 0 {
            q.push_str(&format!(", {} still open", d.unanswered));
        }
        if d.closed_unanswered > 0 {
            q.push_str(&format!(", {} closed without an answer", d.closed_unanswered));
        }
        lines.push(q);
        if let Some(m) = median_line(&d.latency_you) {
            lines.push(format!("  You answered {}, in a median of {m}", d.answered_by_you));
        }
        if d.answered_by_orchestrator > 0 {
            lines.push(format!("  The orchestrator answered {}", d.answered_by_orchestrator));
        }
        if let Some(m) = median_line(&d.latency).filter(|_| d.answered_by_you != d.answered) {
            lines.push(format!("  Median wait for an answer {m}"));
        }
    }
    if n.times + n.cleared + n.waiting > 0 {
        let mut line = format!("Needs you: {}", plural(n.times, "time", "times"));
        if let Some(m) = median_line(&n.time_to_clear) {
            line.push_str(&format!("; cleared in a median of {m}"));
        }
        if n.waiting > 0 {
            line.push_str(&format!("; {} waiting", n.waiting));
        }
        lines.push(line);
    }
    if d.recorded > 0 {
        lines.push(format!("Decisions recorded: {}", d.recorded));
    }
    lines
}

fn group_line(g: &Group, width: usize) -> String {
    let t = &g.tally;
    let name = match &g.repository {
        Some(repository) => format!("{} ({repository})", g.name),
        None => g.name.clone(),
    };
    let mut parts = vec![format!("{} done", t.completed)];
    if t.canceled > 0 {
        parts.push(format!("{} canceled", t.canceled));
    }
    parts.push(format!("{} new", t.created));
    if let Some(s) = t.time_to_done {
        parts.push(format!("median {}", span(s.median_ms)));
    }
    if t.decisions.answered > 0 {
        parts.push(plural(t.decisions.answered, "answer", "answers"));
    }
    format!("  {name:width$}  {}", parts.join(" · "))
}

fn groups(out: &mut Vec<String>, heading: &str, groups: &[Group], limit: usize) {
    if groups.is_empty() {
        return;
    }
    out.push(String::new());
    out.push(heading.to_string());
    let shown = &groups[..groups.len().min(limit)];
    let width = shown
        .iter()
        .map(|g| g.name.chars().count() + g.repository.as_ref().map_or(0, |r| r.chars().count() + 3))
        .max()
        .unwrap_or(0);
    out.extend(shown.iter().map(|g| group_line(g, width)));
    if groups.len() > limit {
        out.push(format!("  and {} more", groups.len() - limit));
    }
}

fn shorten(title: &str, max: usize) -> String {
    if title.chars().count() <= max {
        return title.to_string();
    }
    format!("{}…", title.chars().take(max - 1).collect::<String>().trim_end())
}

/// What agents spent: the whole, then by harness, model, period and task,
/// in `usage_words`' wording, as each task's Usage section has it.
fn spend_lines(s: &SpendReport) -> Vec<String> {
    let t = &s.total;
    let mut out = vec!["Agent spend".to_string()];
    out.push(match t.token_detail() {
        Some(detail) => format!("  {} ({detail})", t.tokens_line()),
        None => format!("  {}", t.tokens_line()),
    });
    out.push(format!("  {}", t.cost_line()));
    out.extend(t.time_line().map(|l| format!("  {l}")));

    let section = |out: &mut Vec<String>, heading: &str, lines: &[SpendLine], name: &dyn Fn(&SpendLine) -> String| {
        if lines.is_empty() {
            return;
        }
        out.push(format!("  {heading}"));
        let names: Vec<String> = lines.iter().map(name).collect();
        let width = names.iter().map(|n| n.chars().count()).max().unwrap_or(0);
        for (line, name) in lines.iter().zip(names) {
            out.push(format!("    {name:width$}  {}", line.spend.line_detail()));
        }
    };
    section(&mut out, "By harness", &s.by_harness, &|l| l.name.clone());
    section(&mut out, "By model", &s.by_model, &|l| if l.name.is_empty() { "Unnamed model".into() } else { l.name.clone() });
    let by_period = match s.period_unit.as_str() {
        "week" => "By week, from each Monday",
        "month" => "By month",
        _ => "By day",
    };
    section(&mut out, by_period, &s.by_period, &|l| l.name.clone());
    section(&mut out, "By task", &s.by_task, &|l| match &l.title {
        Some(title) => format!("{:8}  {}", l.name, shorten(title, 44)),
        None => l.name.clone(),
    });
    if s.other_tasks > 0 {
        out.push(format!("    and {} more", plural(s.other_tasks, "task", "tasks")));
    }
    out
}

/// The summary `farcooler report` prints.
fn render(r: &Report, label: Option<&str>) -> String {
    let mut out = Vec::new();
    let scope = match (r.scope.kind.as_str(), &r.scope.name, &r.scope.repository) {
        ("workspace", Some(name), Some(repository)) => format!("{name}, in {repository}"),
        (_, Some(name), _) => name.clone(),
        _ => "Every repository on this runner".to_string(),
    };
    let range = format!("{} to {}", moment(r.since), moment(r.until));
    out.push(match label {
        Some(label) => format!("{label} · {scope}"),
        None => scope,
    });
    out.push(range);
    out.push(String::new());

    let t = &r.totals;
    let nothing = r.spend.is_none()
        && t.created + t.completed + t.filed_done + t.canceled + t.reopened + t.decisions.asked + t.decisions.answered == 0
        && t.needs_you.times + t.needs_you.waiting == 0
        && t.time_in_status.is_empty();
    if nothing {
        out.push("Nothing happened on the board in this period.".into());
        return out.join("\n");
    }

    out.extend(headline(t, r.spend.is_some()));
    let asked = people(t);
    if !asked.is_empty() {
        out.push(String::new());
        out.extend(asked);
    }

    if !t.time_in_status.is_empty() {
        out.push(String::new());
        out.push("Time in each status".into());
        for s in &t.time_in_status {
            out.push(format!(
                "  {:14}  {} across {}",
                status_name(&s.status),
                hours(s.total_ms),
                plural(s.tasks, "task", "tasks")
            ));
        }
    }

    if let Some(spend) = &r.spend {
        out.push(String::new());
        out.extend(spend_lines(spend));
    }

    if r.by_repository.len() > 1 {
        groups(&mut out, "By repository", &r.by_repository, 10);
    }
    if r.by_workspace.len() > 1 {
        groups(&mut out, "By workspace", &r.by_workspace, 10);
    }
    groups(&mut out, "By area", &r.by_area, 12);
    groups(&mut out, "By label", &r.by_label, 8);

    let n = &r.notable;
    let mut notable = Vec::new();
    let mut list = |heading: &str, rows: Vec<(String, String, String)>| {
        if rows.is_empty() {
            return;
        }
        notable.push(format!("  {heading}"));
        for (key, title, detail) in rows {
            notable.push(format!("    {key:8}  {:44}  {detail}", shorten(&title, 44)));
        }
    };
    list("Slowest to finish", n.slowest.iter().map(|t| (t.key.clone(), t.title.clone(), t.ms.map(span).unwrap_or_default())).collect());
    list(
        "Longest waits on a person",
        n.longest_waits
            .iter()
            .map(|w| {
                let still = if w.open { ", still waiting" } else { "" };
                (w.key.clone(), w.title.clone(), format!("{}, {}{still}", w.kind, span(w.ms)))
            })
            .collect(),
    );
    list(
        "Reopened",
        n.reopened.iter().map(|t| (t.key.clone(), t.title.clone(), plural(t.count.unwrap_or(1), "time", "times"))).collect(),
    );
    list(
        "Most fix rounds",
        n.most_fix_rounds.iter().map(|t| (t.key.clone(), t.title.clone(), plural(t.count.unwrap_or(1), "round", "rounds"))).collect(),
    );
    list("Canceled", n.canceled.iter().map(|t| (t.key.clone(), t.title.clone(), String::new())).collect());
    if !notable.is_empty() {
        out.push(String::new());
        out.push("Notable".into());
        out.extend(notable);
    }
    out.iter().map(|l| l.trim_end()).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
