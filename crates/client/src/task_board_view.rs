//! The task board's projection of a runner's task list and its card labels.
//! The adapters supply the current clock and render the resulting columns and cards.

use std::collections::{HashMap, HashSet};
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;
use serde::Serialize;
use serde_json::{Value, json};

const DAY: f64 = 86_400.0;
const DONE_FLOOR: usize = 3;
const SECTION_LIMIT: usize = 10;
const LIVE_STATES: [&str; 3] = ["running", "starting", "unknown"];

#[derive(Clone, Copy)]
struct Status {
    wire: &'static str,
    apple: &'static str,
    android: &'static str,
    finished: bool,
    active: bool,
    holds_blocks: bool,
}

const STATUSES: [Status; 7] = [
    Status { wire: "needs_decision", apple: "Needs Decision", android: "Needs decision", finished: false, active: false, holds_blocks: false },
    Status { wire: "backlog", apple: "Backlog", android: "Backlog", finished: false, active: false, holds_blocks: true },
    Status { wire: "todo", apple: "To Do", android: "To do", finished: false, active: false, holds_blocks: true },
    Status { wire: "in_progress", apple: "In Progress", android: "In progress", finished: false, active: true, holds_blocks: true },
    Status { wire: "in_review", apple: "In Review", android: "In review", finished: false, active: true, holds_blocks: false },
    Status { wire: "done", apple: "Done", android: "Done", finished: true, active: false, holds_blocks: false },
    Status { wire: "cancelled", apple: "Canceled", android: "Canceled", finished: true, active: false, holds_blocks: false },
];

#[derive(Serialize)]
struct StatusView {
    wire: &'static str,
    title: &'static str,
    finished: bool,
    holds_blocks: bool,
    action: Option<&'static str>,
}

impl Status {
    fn view(self, android: bool) -> StatusView {
        StatusView {
            wire: self.wire,
            title: if android { self.android } else { self.apple },
            finished: self.finished,
            holds_blocks: self.holds_blocks,
            action: (self.wire == "needs_decision").then_some("Answer to unblock this"),
        }
    }
}

fn age(seconds: f64) -> String {
    match seconds.max(0.0) {
        s if s < 60.0 => "just now".into(),
        s if s < 3600.0 => format!("{}m ago", (s / 60.0) as u64),
        s if s < DAY => format!("{}h ago", (s / 3600.0) as u64),
        s if s < 30.0 * DAY => format!("{}d ago", (s / DAY) as u64),
        s if s < 365.0 * DAY => format!("{}mo ago", ((s / (30.0 * DAY)) as u64).min(11)),
        s => format!("{}y ago", (s / (365.0 * DAY)) as u64),
    }
}

fn waiting(count: i64) -> Option<String> {
    match count {
        ..=0 => None,
        1 => Some("1 task is waiting on you".into()),
        n => Some(format!("{n} tasks are waiting on you")),
    }
}

#[derive(Serialize)]
struct ClockView {
    last_moved: f64,
    stopped_for: f64,
    stale_note: Option<String>,
    time_note: Option<String>,
}

fn clock(status: Status, now: f64, since: f64, created: Option<f64>, updated: Option<f64>) -> ClockView {
    let last_moved = since.max(updated.unwrap_or(since));
    let stopped_for = (now - last_moved).max(0.0);
    let stale_note = (status.active && stopped_for >= DAY).then(|| match (stopped_for / DAY) as u64 {
        1 => "Hasn’t moved in a day".into(),
        days => format!("Hasn’t moved in {days} days"),
    });
    let time_note = if status.finished {
        Some(format!("{} {}", status.apple, age(now - since)))
    } else if stale_note.is_none() {
        created.map(|created| match updated.filter(|updated| *updated > created) {
            Some(updated) => format!("Updated {}", age(now - updated)),
            None => format!("Added {}", age(now - created)),
        })
    } else {
        None
    };
    ClockView { last_moved, stopped_for, stale_note, time_note }
}

#[derive(Serialize)]
struct Column {
    status: &'static str,
    indices: Vec<usize>,
}

#[derive(Serialize)]
struct Board {
    columns: Vec<Column>,
    unreadable: Vec<usize>,
}

/// Project by index so the adapters retain the original task payload and decode
/// only native data types (dates, worker handles and wait types). Repeated IDs
/// are separate rows, and order within a column is the runner's order.
fn board(statuses: &[Value]) -> Board {
    let mut columns: Vec<_> = STATUSES.iter().map(|status| Column {
        status: status.wire, indices: Vec::new(),
    }).collect();
    let mut unreadable = Vec::new();
    for (index, status) in statuses.iter().enumerate() {
        let Some(status) = status.as_str() else { continue };
        if let Some(column) = columns.iter_mut().find(|column| column.status == status) {
            column.indices.push(index);
        } else {
            unreadable.push(index);
        }
    }
    Board { columns, unreadable }
}

#[derive(Serialize)]
struct Acceptance {
    met: usize,
    total: usize,
}

fn acceptance(lines: &[Value]) -> Option<Acceptance> {
    let total = lines.len();
    if total == 0 {
        return None;
    }
    let met = lines.iter().filter(|line| line.as_bool() == Some(true)).count();
    Some(Acceptance { met, total })
}

fn acceptance_words(met: i64, total: i64) -> String {
    match (total > 0 && met == total, total) {
        (true, 1) => "Met".into(),
        (true, _) => format!("All {total} met"),
        _ => format!("{met} of {total}"),
    }
}

#[derive(serde::Deserialize)]
struct CutRow {
    id: String,
    since: f64,
    #[serde(default)]
    recent: bool,
}

#[derive(Serialize)]
struct Cut {
    indices: Vec<usize>,
    hidden: usize,
    history: Option<usize>,
}

fn cut(rows: &[CutRow], finished: bool, all: bool, filtering: bool, keeping: Option<&str>) -> Cut {
    let total = rows.len();
    let mut indices: Vec<_> = (0..total).collect();
    if finished {
        indices.sort_by(|&a, &b| rows[b].since.total_cmp(&rows[a].since));
        if !filtering {
            indices = indices.into_iter().enumerate().filter_map(|(rank, index)| {
                (rank < DONE_FLOOR || rows[index].recent || keeping == Some(&rows[index].id)).then_some(index)
            }).collect();
        }
        let history = (indices.len() < total).then_some(total);
        return Cut { indices, hidden: 0, history };
    }
    if !all && !filtering && total > SECTION_LIMIT {
        indices.truncate(SECTION_LIMIT);
        if let Some(kept) = keeping.and_then(|id| rows.iter().position(|row| row.id == id))
            && kept >= indices.len()
        {
            indices.push(kept);
        }
    }
    let hidden = total - indices.len();
    Cut { indices, hidden, history: None }
}

// The renderers supply boundaries from their local calendar and time zone.
#[derive(serde::Deserialize)]
struct HistoryCalendar {
    today: f64,
    tomorrow: Option<f64>,
    yesterday: Option<f64>,
    week_start: Option<f64>,
    week_end: Option<f64>,
}

impl HistoryCalendar {
    fn period(&self, at: f64, android: bool) -> usize {
        if self.tomorrow.is_some_and(|end| (self.today..end).contains(&at)) {
            0
        } else if self.yesterday.is_some_and(|start| (start..self.today).contains(&at)) {
            1
        } else if self.week_start.zip(self.week_end)
            .is_some_and(|(start, end)| at >= start && (at < end || (!android && at == end))) {
            2
        } else {
            3
        }
    }

    fn groups(&self, stamps: &[f64], android: bool) -> [Vec<usize>; 4] {
        let mut indices: Vec<_> = (0..stamps.len()).collect();
        indices.sort_by(|&a, &b| stamps[b].total_cmp(&stamps[a]));
        let mut groups: [Vec<usize>; 4] = Default::default();
        for index in indices {
            groups[self.period(stamps[index], android)].push(index);
        }
        groups
    }
}

fn area(title: &str, android: bool) -> Option<&str> {
    let (prefix, _) = title.split_once(": ")?;
    // Foundation's whitespaces set excludes line breaks. Kotlin trims those too.
    let area = if android {
        prefix.trim()
    } else {
        prefix.trim_matches(|c: char| c == '\t' || (c.is_whitespace() && !matches!(c, '\n'..='\r' | '\u{85}' | '\u{2028}' | '\u{2029}')))
    };
    let length = if android { area.encode_utf16().count() } else { area.graphemes(true).count() };
    (length > 0 && length <= 16 && !area.contains(['.', ',', ';', '!', '?', '(', ')'])).then_some(area)
}

fn areas(titles: &[&str], android: bool) -> Vec<String> {
    let mut counts = HashMap::new();
    for title in titles {
        if let Some(area) = area(title, android) {
            let (count, _) = counts.entry(area_key(area, android)).or_insert((0, area));
            *count += 1;
        }
    }
    let mut counts: Vec<_> = counts.into_iter().collect();
    counts.sort_by(|(a, (n, _)), (b, (m, _))| m.cmp(n).then_with(|| {
        if android { a.encode_utf16().cmp(b.encode_utf16()) } else { a.cmp(b) }
    }));
    counts.into_iter().map(|(_, (_, area))| area.to_owned()).collect()
}

fn area_key(area: &str, android: bool) -> String {
    if android { area.to_owned() } else { area.nfc().collect() }
}

#[derive(serde::Deserialize)]
struct SearchRow<'a> {
    #[serde(default)]
    id: &'a str,
    key: &'a str,
    title: &'a str,
}

fn search_terms(query: &str, android: bool) -> Vec<&str> {
    // Kotlin's Regex("\\s+") uses the ASCII whitespace class.
    query.split(|c: char| if android { matches!(c, ' ' | '\t'..='\r') } else { c.is_whitespace() })
        .filter(|part| !part.is_empty()).collect()
}

fn android_search_text(text: &str) -> String {
    // Java's case-insensitive substring comparison uses simple upper then lower case.
    text.chars().map(|c| {
        let mut upper = c.to_uppercase();
        let first = upper.next().unwrap_or(c);
        let simple = if upper.next().is_some() { c } else { first };
        simple.to_lowercase().next().unwrap_or(simple)
    }).collect()
}

#[cfg(target_vendor = "apple")]
fn apple_matches(text: &str, terms: &[&str]) -> bool {
    use core_foundation::base::TCFType;
    use core_foundation::string::{CFString, CFStringFind, kCFCompareCaseInsensitive, kCFCompareDiacriticInsensitive};

    // Use the same comparison as Foundation's range(of:options:) on Apple.
    let text = CFString::new(text);
    terms.iter().all(|term| {
        let term = CFString::new(term);
        // Both CFStrings are retained for the duration of the call.
        let range = unsafe {
            CFStringFind(text.as_concrete_TypeRef(), term.as_concrete_TypeRef(),
                kCFCompareCaseInsensitive | kCFCompareDiacriticInsensitive)
        };
        range.location >= 0
    })
}

#[cfg(not(target_vendor = "apple"))]
fn apple_matches(text: &str, terms: &[&str]) -> bool {
    // The Apple presenter runs on Darwin. Keep the projection usable in host tests elsewhere.
    fn fold(text: &str) -> String {
        text.nfd().filter(|c| !unicode_normalization::char::is_combining_mark(*c))
            .flat_map(char::to_lowercase).collect()
    }
    let text = fold(text);
    terms.iter().all(|term| text.contains(&fold(term)))
}

fn search(
    rows: &[SearchRow<'_>], query: &str, selected: Option<&str>, note_hits: &[Value], android: bool,
) -> Vec<usize> {
    let terms = search_terms(query, android);
    let android_terms: Vec<_> = if android { terms.iter().map(|word| android_search_text(word)).collect() } else { Vec::new() };
    let note_hits: HashSet<_> = note_hits.iter().filter_map(Value::as_str).collect();
    let selected = selected.map(|area| area_key(area, android));
    rows.iter().enumerate().filter_map(|(index, row)| {
        if let Some(selected) = &selected
            && area(row.title, android).is_none_or(|area| area_key(area, android) != *selected)
        {
            return None;
        }
        if terms.is_empty() || note_hits.contains(row.id) {
            return Some(index);
        }
        let text = format!("{} {}", row.key, row.title);
        let matches = if android {
            let text = android_search_text(&text);
            android_terms.iter().all(|term| text.contains(term))
        } else {
            apple_matches(&text, &terms)
        };
        matches.then_some(index)
    }).collect()
}

fn runs_agent(preset: &str, changes: bool) -> bool {
    if changes {
        return false;
    }
    let Some(name) = preset.split(':').find(|part| !part.is_empty()) else { return false };
    name != "shell" && !matches!(name.to_lowercase().as_str(), "sh" | "zsh" | "bash" | "fish" | "dash" | "ksh" | "-zsh")
}

fn agent_presence(asked: &Value) -> Option<Value> {
    let live = asked["live"].as_i64()?;
    if asked["recorded"] != true {
        return Some(json!({"kind": "unsaid"}));
    }
    let subagents = asked["subagents"].as_i64().unwrap_or(0);
    let state = &asked["state"];
    if live > 0 {
        return Some(json!({"kind": "agents", "count": live + if state == "working" { subagents } else { 0 }}));
    }
    if subagents > 0 {
        return Some(json!({"kind": "subagents", "count": subagents}));
    }
    let kind = if asked["status"] == "in_progress" && asked["ranked"] != true { "no_agent" } else { "unsaid" };
    Some(json!({"kind": kind}))
}

fn live_task(pane: &Value) -> Option<&str> {
    let task = pane["task"].as_str()?;
    let state = pane["state"].as_str()?;
    if task.is_empty() || !LIVE_STATES.contains(&state) {
        return None;
    }
    let agent = pane["agent"].as_bool().or_else(|| {
        Some(runs_agent(pane["preset"].as_str()?, pane["changes"] == true))
    })?;
    agent.then_some(task)
}

fn counted_words(count: i64, one: &str, many: impl FnOnce(i64) -> String) -> Option<String> {
    match count {
        ..=0 => None,
        1 => Some(one.into()),
        n => Some(many(n)),
    }
}

fn agent_title(kind: &str, count: i64, android: bool) -> Option<String> {
    match (kind, count) {
        ("no_agent", _) => Some(if android { "No agent" } else { "No Agent" }.into()),
        ("agents", 1) => Some("Agent".into()),
        ("subagents", 1) => Some("Subagent".into()),
        ("agents", n) => Some(format!("{n} {}", if android { "agents" } else { "Agents" })),
        ("subagents", n) => Some(format!("{n} {}", if android { "subagents" } else { "Subagents" })),
        _ => None,
    }
}

fn decision_count(items: &[Value], workspace: &str, implicit: bool) -> usize {
    items.iter().filter(|item| {
        item["decision"] == true && if implicit {
            item["workspace"].is_null() && item["repository"] == workspace
        } else {
            item["workspace"] == workspace
        }
    }).count()
}

fn repository_groups(repositories: &[Value], workspace_repositories: &[Value]) -> Vec<Value> {
    let mut order = repositories.to_vec();
    for repository in workspace_repositories.iter().filter(|repository| repository.is_string()) {
        if !order.contains(repository) {
            order.push(repository.clone());
        }
    }
    order.into_iter().map(|repository| {
        let indices: Vec<_> = workspace_repositories.iter().enumerate()
            .filter_map(|(index, candidate)| (*candidate == repository).then_some(index))
            .collect();
        json!({ "repository": repository, "indices": indices })
    }).collect()
}

fn missing_boards(found: &[Value], workspace: &Value, from: &Value) -> Vec<Value> {
    let mut missing = Vec::new();
    for name in [workspace, from].into_iter().filter(|name| name.is_string()) {
        if !found.contains(name) && !missing.contains(name) {
            missing.push(name.clone());
        }
    }
    missing
}

// Return the original JSON number to preserve millisecond integers and Date's
// fractional seconds through the bridge.
fn compare_stamps(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a.as_i64(), b.as_i64()) {
        (Some(a), Some(b)) => a.cmp(&b),
        _ => a.as_f64().unwrap_or(f64::NEG_INFINITY)
            .total_cmp(&b.as_f64().unwrap_or(f64::NEG_INFINITY)),
    }
}

fn newest_stamp(stamps: &[Value]) -> Option<Value> {
    stamps.iter().filter(|stamp| stamp.is_number()).max_by(|a, b| compare_stamps(a, b)).cloned()
}

pub(crate) fn project(request: &Value) -> Option<Value> {
    if request["schema"].as_u64() != Some(1) {
        return None;
    }
    present(request.get("task_board")?)
}

fn present(asked: &Value) -> Option<Value> {
    let result = match asked["op"].as_str()? {
        "settings" => json!({
            "day": DAY, "done_floor": DONE_FLOOR, "section_limit": SECTION_LIMIT, "live_states": LIVE_STATES,
        }),
        "statuses" => json!(STATUSES.map(|status| status.view(asked["android"] == true))),
        "columns" => json!(board(asked["statuses"].as_array()?)),
        "clock" => {
            let status = STATUSES.iter().find(|status| asked["status"] == status.wire)?;
            json!(clock(*status, asked["now"].as_f64()?, asked["since"].as_f64()?,
                asked["created"].as_f64(), asked["updated"].as_f64()))
        }
        "age" => json!(age(asked["seconds"].as_f64()?)),
        "first_look" => {
            if asked["android"] == true {
                json!(asked["now"].as_i64()?.wrapping_sub(DAY as i64 * 1000))
            } else {
                json!(asked["now"].as_f64()? - DAY)
            }
        }
        "read_mark" => newest_stamp(asked["stamps"].as_array()?)?,
        "prune_marks" => {
            let floor = &asked["floor"];
            let mut opened = asked["opened"].as_object()?.clone();
            opened.retain(|_, mark| compare_stamps(mark, floor).is_gt());
            json!(opened)
        }
        "unread" => json!(compare_stamps(&asked["at"], &asked["mark"]).is_gt()),
        "waiting" => json!(waiting(asked["count"].as_i64()?)),
        "waiting_count" => {
            let key = if asked["read"] == true && asked["served"] == true { "decisions" } else { "column" };
            json!(asked[key].as_i64()?)
        }
        "decision_count" => json!(decision_count(
            asked["items"].as_array()?, asked["workspace"].as_str()?, asked["implicit"] == true,
        )),
        "repository_groups" => json!(repository_groups(
            asked["repositories"].as_array()?, asked["workspace_repositories"].as_array()?,
        )),
        "missing_boards" => json!(missing_boards(asked["found"].as_array()?, &asked["workspace"], &asked["from"])),
        "acceptance" => json!(acceptance(asked["lines"].as_array()?)),
        "acceptance_words" => json!(acceptance_words(asked["met"].as_i64()?, asked["total"].as_i64()?)),
        "cut" => {
            let rows: Vec<CutRow> = serde_json::from_value(asked["rows"].clone()).ok()?;
            json!(cut(&rows, asked["finished"] == true, asked["all"] == true, asked["filtering"] == true, asked["keeping"].as_str()))
        }
        "show_more" => {
            let word = if asked["android"] == true { "more" } else { "More" };
            json!(format!("Show {} {word}", asked["count"].as_i64()?))
        }
        "history_title" => {
            let status = STATUSES.iter().find(|status| asked["status"] == status.wire)?;
            json!(format!("All {}", status.view(asked["android"] == true).title))
        }
        "history_period" | "history_groups" => {
            let calendar: HistoryCalendar = serde_json::from_value(asked["calendar"].clone()).ok()?;
            let android = asked["android"] == true;
            if let Some(at) = asked["at"].as_f64() {
                json!(calendar.period(at, android))
            } else {
                let stamps: Vec<f64> = serde_json::from_value(asked["stamps"].clone()).ok()?;
                json!(calendar.groups(&stamps, android))
            }
        }
        "history_area" => json!(area(asked["title"].as_str()?, asked["android"] == true)),
        "history_areas" => {
            let titles: Vec<&str> = serde::Deserialize::deserialize(&asked["titles"]).ok()?;
            json!(areas(&titles, asked["android"] == true))
        }
        "search_empty" => json!(search_terms(asked["query"].as_str()?, asked["android"] == true).is_empty()),
        "search" => {
            let rows: Vec<SearchRow<'_>> = serde::Deserialize::deserialize(&asked["rows"]).ok()?;
            json!(search(&rows, asked["query"].as_str()?, asked["area"].as_str(),
                asked["note_hits"].as_array().map_or(&[], Vec::as_slice), asked["android"] == true))
        }
        "period_titles" => if asked["android"] == true {
            json!(["Today", "Yesterday", "This week", "Earlier"])
        } else {
            json!(["Today", "Yesterday", "This Week", "Earlier"])
        },
        "menu_titles" => {
            let titles = asked["titles"].as_array()?;
            let shorts = asked["shorts"].as_array()?;
            let mut counts = std::collections::HashMap::new();
            for title in titles {
                *counts.entry(title.as_str()?).or_insert(0) += 1;
            }
            let rendered: Option<Vec<_>> = titles.iter().enumerate().map(|(index, title)| {
                let title = title.as_str()?;
                let short = shorts.get(index).and_then(Value::as_str);
                Some(match short.filter(|_| counts[title] > 1) {
                    Some(short) => format!("{title} ({short})"),
                    None => title.to_owned(),
                })
            }).collect();
            json!(rendered?)
        }
        "runs_agent" => json!(runs_agent(asked["preset"].as_str()?, asked["changes"] == true)),
        "live_panes" => {
            let task = asked["task"].as_str()?;
            let panes = asked["panes"].as_array()?;
            json!(panes.iter().enumerate().filter_map(|(index, pane)| {
                (live_task(pane) == Some(task)).then_some(index)
            }).collect::<Vec<_>>())
        }
        "live_task_count" => {
            let live: HashSet<_> = asked["panes"].as_array()?.iter().filter_map(live_task).collect();
            json!(asked["tasks"].as_array()?.iter().filter_map(Value::as_str).filter(|task| live.contains(task)).count())
        }
        "show_agents" => json!(asked["connected"] == true && asked["recorded"] == true),
        "agent_presence" => agent_presence(asked)?,
        "agent_title" => json!(agent_title(asked["kind"].as_str()?, asked["count"].as_i64()?, asked["android"] == true)),
        "closed_pane" => json!("That agent’s pane has closed."),
        "decisions" => json!(counted_words(asked["count"].as_i64()?, "1 task needs a decision", |n| format!("{n} tasks need a decision"))),
        "agents" => json!(counted_words(asked["count"].as_i64()?, "An agent is on 1 task", |n| format!("Agents are on {n} tasks"))),
        _ => return None,
    };
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_groups_sort_newest_first_and_keep_input_order_for_ties() {
        // A 23-hour day across a local clock change, and a week ending tomorrow.
        let calendar = json!({"today": 100, "tomorrow": 123, "yesterday": 76, "week_start": -45, "week_end": 123});
        let request = json!({"op": "history_groups", "calendar": calendar,
            "stamps": [-46, 101, -45, 76, 122, 101, 123, 100, 75]});
        assert_eq!(present(&request), Some(json!([[4, 1, 5, 7], [3], [6, 8, 2], [0]])));
        let mut android = request;
        android["android"] = json!(true);
        assert_eq!(present(&android), Some(json!([[4, 1, 5, 7], [3], [8, 2], [6, 0]])));
    }

    #[test]
    fn history_areas_order_by_frequency_and_ignore_invalid_prefixes() {
        let request = json!({"op": "history_areas", "titles": [
            "Mac: Title", "Rust: Parser", "Mac: History", "Android: UI",
            "Which area?: Unknown", "Longer than sixteen: Words", " : Empty", "No prefix",
        ]});
        assert_eq!(present(&request), Some(json!(["Mac", "Android", "Rust"])));
        assert_eq!(area("  Café: Fix", false), Some("Café"));
        assert_eq!(area("👋👋👋👋👋👋👋👋👋: Fix", false), Some("👋👋👋👋👋👋👋👋👋"));
        assert_eq!(area("👋👋👋👋👋👋👋👋👋: Fix", true), None);
    }

    #[test]
    fn history_search_combines_area_words_and_note_hits() {
        let rows = json!([
            {"id": "a", "key": "ov-1", "title": "Mac: Café board"},
            {"id": "b", "key": "ov-2", "title": "Mac: Menu"},
            {"id": "c", "key": "ov-3", "title": "Linux: Café board"},
        ]);
        let request = json!({"op": "search", "query": "CAFE board", "area": "Mac", "rows": rows, "note_hits": ["b", "c"]});
        assert_eq!(present(&request), Some(json!([0, 1])));
        let mut android = request;
        android["android"] = json!(true);
        assert_eq!(present(&android), Some(json!([1])));
        android["query"] = json!("CAFÉ ov-1");
        assert_eq!(present(&android), Some(json!([0, 1])));
        android["query"] = json!("  ");
        assert_eq!(present(&android), Some(json!([0, 1])));
    }

    #[cfg(target_vendor = "apple")]
    #[test]
    fn apple_search_matches_folded_words_at_character_boundaries() {
        let rows = json!([{"key": "ov-1", "title": "Straße Σ"}, {"key": "ov-2", "title": "ß"}]);
        let mut request = json!({"op": "search", "query": "STRASSE ς", "rows": rows});
        assert_eq!(present(&request), Some(json!([0])));
        request["query"] = json!("s");
        assert_eq!(present(&request), Some(json!([0])));
    }

    #[test]
    fn live_panes_and_task_counts_exclude_idle_shells_and_unlinked_panes() {
        let panes = json!([
            {"task": "a", "state": "unknown", "agent": true},
            {"task": "a", "state": "exited", "agent": true},
            {"task": "a", "state": "running", "agent": false},
            {"task": "", "state": "running", "agent": true},
            {"task": "a", "state": "starting", "agent": true},
            {"task": "b", "state": "running", "agent": true},
        ]);
        assert_eq!(present(&json!({"op": "live_panes", "task": "a", "panes": panes})), Some(json!([0, 4])));
        assert_eq!(present(&json!({"op": "live_task_count", "tasks": ["a", "b", "", "c"], "panes": panes})), Some(json!(2)));
    }

    #[test]
    fn columns_keep_empty_columns_duplicates_and_unreadable_rows() {
        let view = board(&[json!("done"), json!("new"), Value::Null, json!("done"), json!("needs_decision")]);
        assert_eq!(view.columns.iter().map(|c| c.status).collect::<Vec<_>>(),
            ["needs_decision", "backlog", "todo", "in_progress", "in_review", "done", "cancelled"]);
        assert_eq!(view.columns[0].indices, [4]);
        assert!(view.columns[1].indices.is_empty());
        assert_eq!(view.columns[5].indices, [0, 3]);
        assert_eq!(view.unreadable, [1]);
    }

    #[test]
    fn clock_preserves_the_exact_status_date_across_the_json_bridge() {
        let request: Value = serde_json::from_str(r#"{
            "schema": 1,
            "task_board": {
                "op": "clock", "status": "in_review", "now": 816863112.913055,
                "since": 816859512.913055, "updated": 816600000
            }
        }"#).unwrap();
        let view = project(&request).unwrap();
        assert_eq!(view["last_moved"].as_f64(), Some(816859512.913055));
        assert_eq!(view["stopped_for"].as_f64(), Some(3600.0));
    }

    #[test]
    fn movement_resets_staleness_and_only_active_work_goes_stale() {
        let active = STATUSES[3];
        assert_eq!(clock(active, DAY, 0.0, Some(0.0), None).stale_note.as_deref(), Some("Hasn’t moved in a day"));
        let updated = clock(active, DAY, 0.0, Some(0.0), Some(DAY - 60.0));
        assert_eq!(updated.stopped_for, 60.0);
        assert_eq!(updated.stale_note, None);
        assert_eq!(updated.time_note.as_deref(), Some("Updated 1m ago"));
        assert_eq!(clock(STATUSES[5], 2.0 * DAY, 0.0, None, None).time_note.as_deref(), Some("Done 2d ago"));
        assert_eq!(clock(STATUSES[0], DAY, 0.0, None, None).stale_note, None);
        assert_eq!(clock(active, 0.0, 60.0, None, Some(0.0)).stopped_for, 0.0);
    }

    #[test]
    fn cuts_keep_unread_today_and_selection_in_stable_history_order() {
        let mut rows: Vec<_> = (0..12).map(|index| CutRow {
            id: index.to_string(), since: (index / 2) as f64, recent: index == 1,
        }).collect();
        assert_eq!(cut(&rows, true, false, false, Some("0")).indices, [10, 11, 8, 0, 1]);
        assert_eq!(cut(&rows, true, false, false, None).history, Some(12));
        let in_progress = cut(&rows, false, false, false, Some("11"));
        assert_eq!(in_progress.indices.last(), Some(&11));
        assert_eq!(in_progress.hidden, 1);
        assert_eq!(cut(&rows, false, true, false, None).hidden, 0);
        rows.truncate(2);
        assert_eq!(cut(&rows, true, false, false, None).history, None);
    }

    #[test]
    fn agent_presence_preserves_waits_older_runners_and_working_subagents() {
        let mut facts = json!({"live": 0, "recorded": true, "status": "in_progress", "ranked": true});
        assert_eq!(agent_presence(&facts).unwrap()["kind"], "unsaid");
        facts["ranked"] = json!(false);
        assert_eq!(agent_presence(&facts).unwrap()["kind"], "no_agent");
        facts["subagents"] = json!(2);
        facts["state"] = json!("working");
        facts["live"] = json!(1);
        assert_eq!(agent_presence(&facts).unwrap()["count"], 3);
        facts["recorded"] = json!(false);
        assert_eq!(agent_presence(&facts).unwrap()["kind"], "unsaid");
        assert!(runs_agent(":codex:default", false));
        assert!(!runs_agent("ZSH:login", false));
        assert!(!runs_agent("codex", true));
    }

    #[test]
    fn overview_keeps_repository_order_and_counts_one_decision_per_item() {
        let groups = repository_groups(
            &[json!("empty"), json!("first")],
            &[json!("first"), json!("later"), Value::Null, json!("first")],
        );
        assert_eq!(groups, [
            json!({"repository": "empty", "indices": []}),
            json!({"repository": "first", "indices": [0, 3]}),
            json!({"repository": "later", "indices": [1]}),
        ]);
        assert_eq!(missing_boards(&[json!("seen")], &json!("new"), &json!("new")), [json!("new")]);
        let items = [
            json!({"decision": true, "workspace": "main", "repository": "repo"}),
            json!({"decision": true, "repository": "repo"}),
            json!({"decision": false, "workspace": "main"}),
        ];
        assert_eq!(decision_count(&items, "main", false), 1);
        assert_eq!(decision_count(&items, "repo", true), 1);
    }

    #[test]
    fn read_marks_keep_millisecond_precision_and_skip_missing_stamps() {
        let now = json!(i64::MAX);
        let older = json!(i64::MAX - 1);
        assert_eq!(newest_stamp(&[Value::Null, now.clone(), older.clone()]), Some(now.clone()));
        assert!(compare_stamps(&now, &older).is_gt());
        let request = json!({"schema": 1, "task_board": {
            "op": "prune_marks", "floor": older, "opened": { "old": older, "new": now },
        }});
        assert_eq!(project(&request), Some(json!({"new": now})));
    }

    #[test]
    fn board_words_preserve_boundaries_and_platform_case() {
        for (seconds, expected) in [(-1.0, "just now"), (60.0, "1m ago"), (360.0 * DAY, "11mo ago"), (365.0 * DAY, "1y ago")] {
            assert_eq!(age(seconds), expected);
        }
        assert_eq!(waiting(0), None);
        assert_eq!(waiting(1).as_deref(), Some("1 task is waiting on you"));
        assert_eq!(waiting(2).as_deref(), Some("2 tasks are waiting on you"));
        assert_eq!(STATUSES[0].view(false).title, "Needs Decision");
        assert_eq!(STATUSES[0].view(true).title, "Needs decision");
        assert!(acceptance(&[]).is_none());
        let progress = acceptance(&[json!(true), json!(false)]).unwrap();
        assert_eq!((progress.met, progress.total), (1, 2));
        assert_eq!(acceptance_words(1, 1), "Met");
        assert_eq!(acceptance_words(1, 2), "1 of 2");
    }
}
