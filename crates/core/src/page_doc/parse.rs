//! Reading a page from JSON: validate, normalize, and say where it's wrong.

use std::fmt;

use icu_normalizer::ComposingNormalizerBorrowed;
use serde_json::{Map, Value};

use super::{
    Align, BLOCK_TYPES, Block, Caps, Cell, Column, Entry, Item, Order, Page, Part, Reference, Show, State, Stat, Step,
    STALE_AFTER_MAX_MIN, Target, Tone, VERSION, url_host, valid_slot,
};

/// Why a document isn't a page: the JSON path of what's wrong and what is.
///
/// The path is empty when it's the whole document. A sentence a person, or a
/// model, can act on without opening the schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageError {
    /// `blocks[3].rows[51]`, or empty for the document.
    pub path: String,
    /// What's wrong, as a sentence.
    pub message: String,
}

impl fmt::Display for PageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            f.write_str(&self.message)
        } else {
            write!(f, "{}: {}", self.path, self.message)
        }
    }
}

impl std::error::Error for PageError {}

type R<T> = Result<T, PageError>;

fn err<T>(path: &str, message: impl Into<String>) -> R<T> {
    Err(PageError { path: path.to_string(), message: message.into() })
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() { key.to_string() } else { format!("{path}.{key}") }
}

/// `a, b and c`.
fn words(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// A name from the caller, cut short enough to sit in a sentence.
fn quoted(s: &str) -> String {
    let mut it = s.chars();
    let head: String = it.by_ref().take(40).collect();
    if it.next().is_some() { format!("{head}...") } else { head }
}

struct Ctx<'a> {
    caps: &'a Caps,
    refs: usize,
}

/// Read a document from its JSON text.
pub fn parse(json: &str, caps: &Caps) -> Result<Page, PageError> {
    match serde_json::from_str::<Value>(json) {
        Ok(value) => check_value(&value, caps),
        Err(e) => err("", format!("That isn't valid JSON (line {}, column {}).", e.line(), e.column())),
    }
}

/// Validate and normalize a document that is already JSON.
pub fn check_value(value: &Value, caps: &Caps) -> Result<Page, PageError> {
    let m = object(value, "", &["v", "title", "summary", "glance", "stale_after_min", "blocks"], true)?;
    match m.get("v") {
        Some(Value::Number(n)) if n.as_u64() == Some(u64::from(VERSION)) => {}
        Some(Value::Number(n)) if n.as_u64().is_some() => {
            return err(
                "v",
                format!("This runner draws pages up to version {VERSION}. Update the runner, or write version {VERSION}."),
            );
        }
        _ => return err("v", format!("needs the version, a whole number. Write version {VERSION}.")),
    }
    // The fields, now that the version is one this build knows: a newer
    // version's new fields are a version problem, not a misspelling.
    object(value, "", &["v", "title", "summary", "glance", "stale_after_min", "blocks"], false)?;
    let mut ctx = Ctx { caps, refs: 0 };
    let title = text(required(m, "title", "")?, "title", caps.title_chars, false, false)?;
    let summary = match m.get("summary") {
        Some(v) => text(v, "summary", caps.summary_chars, false, true)?,
        None => String::new(),
    };
    let glance = match m.get("glance") {
        Some(v) => Some(text(v, "glance", caps.glance_chars, false, false)?),
        None => None,
    };
    let stale_after_min = match m.get("stale_after_min") {
        None => None,
        Some(v) => match v.as_u64() {
            Some(n) if (1..=u64::from(STALE_AFTER_MAX_MIN)).contains(&n) => Some(n as u32),
            _ => return err("stale_after_min", format!("minutes from 1 to {STALE_AFTER_MAX_MIN} (seven days).")),
        },
    };
    let blocks = list(required(m, "blocks", "")?, "blocks", 1, caps.blocks, "a page", "blocks")?
        .iter()
        .enumerate()
        .map(|(i, b)| block(b, &format!("blocks[{i}]"), &mut ctx))
        .collect::<R<Vec<_>>>()?;
    let page = Page { v: VERSION, title, summary, glance, stale_after_min, blocks };
    let bytes = page.to_json().len();
    if bytes > caps.document_bytes {
        return err("", format!("The page is {bytes} bytes once written out, and the most is {}.", caps.document_bytes));
    }
    Ok(page)
}

fn block(v: &Value, path: &str, ctx: &mut Ctx) -> R<Block> {
    let kind = match v.get("type") {
        Some(Value::String(s)) => s.as_str(),
        _ => return err(&join(path, "type"), format!("needs a block type. The blocks are {}.", words(&BLOCK_TYPES))),
    };
    let caps = ctx.caps;
    Ok(match kind {
        "heading" => {
            let m = object(v, path, &["type", "text"], false)?;
            Block::Heading { text: text(required(m, "text", path)?, &join(path, "text"), caps.string_chars, false, false)? }
        }
        "text" => {
            let m = object(v, path, &["type", "md", "tone"], false)?;
            Block::Text {
                md: text(required(m, "md", path)?, &join(path, "md"), caps.md_chars, true, false)?,
                tone: tone(m, path)?,
            }
        }
        "stats" => {
            let m = object(v, path, &["type", "items"], false)?;
            let p = join(path, "items");
            let items = list(required(m, "items", path)?, &p, 1, caps.stats_items, "a stats row", "figures")?
                .iter()
                .enumerate()
                .map(|(i, item)| stat(item, &format!("{p}[{i}]"), caps))
                .collect::<R<Vec<_>>>()?;
            Block::Stats { items }
        }
        "progress" => progress(v, path, caps)?,
        "table" => table(v, path, ctx)?,
        "list" => {
            let m = object(v, path, &["type", "items"], false)?;
            let p = join(path, "items");
            let items = list(required(m, "items", path)?, &p, 1, caps.list_items, "a list", "items")?
                .iter()
                .enumerate()
                .map(|(i, item)| list_item(item, &format!("{p}[{i}]"), ctx))
                .collect::<R<Vec<_>>>()?;
            Block::List { items }
        }
        "timeline" => {
            let m = object(v, path, &["type", "order", "entries"], false)?;
            let order = match m.get("order") {
                None => Order::Newest,
                Some(Value::String(s)) if s == "newest" => Order::Newest,
                Some(Value::String(s)) if s == "given" => Order::Given,
                Some(_) => return err(&join(path, "order"), "newest or given."),
            };
            let p = join(path, "entries");
            let entries = list(required(m, "entries", path)?, &p, 1, caps.timeline_entries, "a timeline", "entries")?
                .iter()
                .enumerate()
                .map(|(i, entry)| timeline_entry(entry, &format!("{p}[{i}]"), ctx))
                .collect::<R<Vec<_>>>()?;
            Block::Timeline { order, entries }
        }
        "steps" => {
            let m = object(v, path, &["type", "steps"], false)?;
            let p = join(path, "steps");
            let steps = list(required(m, "steps", path)?, &p, 2, caps.steps, "a pipeline", "steps")?
                .iter()
                .enumerate()
                .map(|(i, s)| step(s, &format!("{p}[{i}]"), caps))
                .collect::<R<Vec<_>>>()?;
            Block::Steps { steps }
        }
        "links" => {
            let m = object(v, path, &["type", "items"], false)?;
            let p = join(path, "items");
            let items = list(required(m, "items", path)?, &p, 1, caps.links_items, "a links block", "links")?
                .iter()
                .enumerate()
                .map(|(i, r)| reference(r, &format!("{p}[{i}]"), ctx))
                .collect::<R<Vec<_>>>()?;
            Block::Links { items }
        }
        other => {
            return err(
                &join(path, "type"),
                format!("there's no block called {}. The blocks are {}.", quoted(other), words(&BLOCK_TYPES)),
            );
        }
    })
}

fn stat(v: &Value, path: &str, caps: &Caps) -> R<Stat> {
    let m = object(v, path, &["label", "value", "detail", "tone"], false)?;
    Ok(Stat {
        label: text(required(m, "label", path)?, &join(path, "label"), caps.string_chars, false, false)?,
        value: text(required(m, "value", path)?, &join(path, "value"), caps.string_chars, false, false)?,
        detail: optional_text(m, "detail", path, caps.string_chars)?,
        tone: tone(m, path)?,
    })
}

fn progress(v: &Value, path: &str, caps: &Caps) -> R<Block> {
    let m = object(v, path, &["type", "label", "done", "total", "detail", "parts"], false)?;
    let label = text(required(m, "label", path)?, &join(path, "label"), caps.string_chars, false, false)?;
    let done = count(required(m, "done", path)?, &join(path, "done"))?;
    let total = count(required(m, "total", path)?, &join(path, "total"))?;
    if total == 0 {
        return err(&join(path, "total"), "needs at least 1.");
    }
    if done > total {
        return err(&join(path, "done"), format!("is {done}, and there are only {total}."));
    }
    let parts = match m.get("parts") {
        None => Vec::new(),
        Some(v) => {
            let p = join(path, "parts");
            list(v, &p, 1, caps.progress_parts, "a bar", "parts")?
                .iter()
                .enumerate()
                .map(|(i, part)| {
                    let pp = format!("{p}[{i}]");
                    let m = object(part, &pp, &["label", "count"], false)?;
                    Ok(Part {
                        label: text(required(m, "label", &pp)?, &join(&pp, "label"), caps.string_chars, false, false)?,
                        count: count(required(m, "count", &pp)?, &join(&pp, "count"))?,
                    })
                })
                .collect::<R<Vec<_>>>()?
        }
    };
    let sum: u64 = parts.iter().map(|p| u64::from(p.count)).sum();
    if sum > u64::from(total) {
        return err(&join(path, "parts"), format!("add up to {sum}, and the bar's total is {total}."));
    }
    Ok(Block::Progress { label, done, total, detail: optional_text(m, "detail", path, caps.string_chars)?, parts })
}

fn table(v: &Value, path: &str, ctx: &mut Ctx) -> R<Block> {
    let caps = ctx.caps;
    let m = object(v, path, &["type", "columns", "rows"], false)?;
    let cp = join(path, "columns");
    let columns = list(required(m, "columns", path)?, &cp, 1, caps.table_columns, "a table", "columns")?
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let p = format!("{cp}[{i}]");
            let m = object(c, &p, &["title", "align", "grow"], false)?;
            let align = match m.get("align") {
                None => Align::Start,
                Some(Value::String(s)) if s == "start" => Align::Start,
                Some(Value::String(s)) if s == "center" => Align::Center,
                Some(Value::String(s)) if s == "end" => Align::End,
                Some(_) => return err(&join(&p, "align"), "start, center or end."),
            };
            let grow = match m.get("grow") {
                None => false,
                Some(Value::Bool(b)) => *b,
                Some(_) => return err(&join(&p, "grow"), "true or false."),
            };
            Ok(Column {
                title: text(required(m, "title", &p)?, &join(&p, "title"), caps.string_chars, false, false)?,
                align,
                grow,
            })
        })
        .collect::<R<Vec<_>>>()?;
    let rp = join(path, "rows");
    let rows = list(required(m, "rows", path)?, &rp, 0, caps.table_rows, "a table", "rows")?
        .iter()
        .enumerate()
        .map(|(r, row)| {
            let p = format!("{rp}[{r}]");
            let cells = list(row, &p, 0, usize::MAX, "a row", "cells")?;
            if cells.len() != columns.len() {
                return err(
                    &p,
                    format!(
                        "this row has {} {} and the table has {} {}.",
                        cells.len(),
                        if cells.len() == 1 { "cell" } else { "cells" },
                        columns.len(),
                        if columns.len() == 1 { "column" } else { "columns" },
                    ),
                );
            }
            cells.iter().enumerate().map(|(c, cell)| self::cell(cell, &format!("{p}[{c}]"), ctx)).collect::<R<Vec<_>>>()
        })
        .collect::<R<Vec<_>>>()?;
    Ok(Block::Table { columns, rows })
}

fn cell(v: &Value, path: &str, ctx: &mut Ctx) -> R<Cell> {
    let caps = ctx.caps;
    if let Value::String(_) = v {
        let t = text(v, path, caps.string_chars, false, true)?;
        return Ok(Cell { text: Some(t), reference: None, show: None, tone: Tone::Neutral, mono: false });
    }
    let m = object(v, path, &["text", "ref", "show", "tone", "mono"], false)?;
    let text_v = match m.get("text") {
        Some(t) => Some(text(t, &join(path, "text"), caps.string_chars, false, true)?),
        None => None,
    };
    let reference = match m.get("ref") {
        Some(r) => Some(self::reference(r, &join(path, "ref"), ctx)?),
        None => None,
    };
    if text_v.is_none() && reference.is_none() {
        return err(path, "a cell needs text, or a ref to draw live.");
    }
    let show = match m.get("show") {
        None => None,
        Some(s) => {
            let show = match s.as_str() {
                Some("state") => Show::State,
                Some("spend") => Show::Spend,
                _ => return err(&join(path, "show"), "state or spend."),
            };
            if !matches!(reference, Some(Reference { target: Target::Lane(_), .. })) {
                return err(&join(path, "show"), format!("a lane's {} can only be drawn for a ref to a lane.", match show {
                    Show::State => "state",
                    Show::Spend => "spend",
                }));
            }
            if text_v.is_some() {
                return err(&join(path, "show"), "draws the live value, which text would hide. Take out the text, or take out show.");
            }
            Some(show)
        }
    };
    let mono = match m.get("mono") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return err(&join(path, "mono"), "true or false."),
    };
    Ok(Cell { text: text_v, reference, show, tone: tone(m, path)?, mono })
}

fn list_item(v: &Value, path: &str, ctx: &mut Ctx) -> R<Item> {
    let caps = ctx.caps;
    let m = object(v, path, &["text", "state", "detail", "ref", "tone"], false)?;
    Ok(Item {
        text: text(required(m, "text", path)?, &join(path, "text"), caps.string_chars, false, false)?,
        state: state(m, "state", path, State::None)?,
        detail: optional_text(m, "detail", path, caps.string_chars)?,
        reference: match m.get("ref") {
            Some(r) => Some(reference(r, &join(path, "ref"), ctx)?),
            None => None,
        },
        tone: tone(m, path)?,
    })
}

fn timeline_entry(v: &Value, path: &str, ctx: &mut Ctx) -> R<Entry> {
    let caps = ctx.caps;
    let m = object(v, path, &["at", "text", "ref"], false)?;
    let at = match required(m, "at", path)? {
        Value::Number(n) => match n.as_i64() {
            Some(ms) if (0..=MAX_MILLIS).contains(&ms) => ms,
            _ => return err(&join(path, "at"), "milliseconds since 1970, up to the year 2100."),
        },
        Value::String(s) => match rfc3339_millis(s) {
            Some(ms) => ms,
            None => {
                return err(
                    &join(path, "at"),
                    "a time as RFC 3339, like 2026-10-04T15:02:00-07:00, or as milliseconds since 1970.",
                );
            }
        },
        _ => return err(&join(path, "at"), "a time as RFC 3339, or as milliseconds since 1970."),
    };
    Ok(Entry {
        at,
        text: text(required(m, "text", path)?, &join(path, "text"), caps.string_chars, false, false)?,
        reference: match m.get("ref") {
            Some(r) => Some(reference(r, &join(path, "ref"), ctx)?),
            None => None,
        },
    })
}

fn step(v: &Value, path: &str, caps: &Caps) -> R<Step> {
    let m = object(v, path, &["label", "state"], false)?;
    Ok(Step {
        label: text(required(m, "label", path)?, &join(path, "label"), caps.string_chars, false, false)?,
        state: match m.get("state") {
            None => return err(&join(path, "state"), format!("is required: {}.", words(&State::WORDS))),
            Some(_) => state(m, "state", path, State::None)?,
        },
    })
}

fn reference(v: &Value, path: &str, ctx: &mut Ctx) -> R<Reference> {
    let caps = ctx.caps;
    let Value::Object(m) = v else {
        return err(path, format!("expected a reference: an object naming one of {}.", words(&Target::KINDS)));
    };
    for key in m.keys() {
        if key != "label" && !Target::KINDS.contains(&key.as_str()) {
            return err(
                &join(path, key),
                format!("there's no field called {}. A reference names one of {}, and can have a label.", quoted(key), words(&Target::KINDS)),
            );
        }
    }
    let named: Vec<&str> = Target::KINDS.iter().copied().filter(|k| m.contains_key(*k)).collect();
    let [kind] = named[..] else {
        return err(path, format!("a reference names exactly one thing: {}.", words(&Target::KINDS)));
    };
    let at = join(path, kind);
    let value = &m[kind];
    let target = match kind {
        "task" | "ask" => {
            let key = text(value, &at, caps.string_chars, false, false)?;
            if key.len() > 40 || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                return err(&at, "a task key looks like ov-113.");
            }
            if kind == "task" { Target::Task(key) } else { Target::Ask(key) }
        }
        "lane" => Target::Lane(text(value, &at, caps.string_chars, false, false)?),
        "theme" => Target::Theme(text(value, &at, caps.string_chars, false, false)?),
        "worktree" => Target::Worktree(text(value, &at, caps.string_chars, false, false)?),
        "page" => {
            let slot = text(value, &at, caps.string_chars, false, false)?;
            if !valid_slot(&slot) {
                return err(&at, "a page is named by its slot: lowercase letters, digits and hyphens, at most 40.");
            }
            Target::Page(slot)
        }
        "terminal" => {
            let tm = object(value, &at, &["worktree", "name"], false)?;
            Target::Terminal {
                worktree: text(required(tm, "worktree", &at)?, &join(&at, "worktree"), caps.string_chars, false, false)?,
                name: text(required(tm, "name", &at)?, &join(&at, "name"), caps.string_chars, false, false)?,
            }
        }
        _ => {
            let url = text(value, &at, caps.string_chars, false, false)?;
            if !url.starts_with("https://") {
                return err(&at, "a link has to start with https://. Pages open no other kind.");
            }
            if url.chars().any(char::is_whitespace) {
                return err(&at, "a link can't hold spaces.");
            }
            if url_host(&url).is_none() {
                return err(&at, "this link's domain isn't one a page can show. Write the domain in plain letters, with no user name before it.");
            }
            Target::Url(url)
        }
    };
    ctx.refs += 1;
    if ctx.refs > caps.refs {
        return err(path, format!("a page has at most {} references.", caps.refs));
    }
    Ok(Reference { target, label: optional_text(m, "label", path, caps.string_chars)? })
}

// ---- the small readers ----

/// `v` as an object whose keys are all in `fields`. With `first_pass` the
/// field check is skipped, for the one caller that wants the version first.
fn object<'a>(v: &'a Value, path: &str, fields: &[&str], first_pass: bool) -> R<&'a Map<String, Value>> {
    let Value::Object(m) = v else {
        return err(path, "expected an object.");
    };
    if !first_pass {
        for key in m.keys() {
            if !fields.contains(&key.as_str()) {
                return err(
                    &join(path, key),
                    format!("there's no field called {}. The fields are {}.", quoted(key), words(fields)),
                );
            }
        }
    }
    Ok(m)
}

fn required<'a>(m: &'a Map<String, Value>, key: &str, path: &str) -> R<&'a Value> {
    match m.get(key) {
        Some(v) => Ok(v),
        None => err(&join(path, key), "is required."),
    }
}

fn list<'a>(v: &'a Value, path: &str, min: usize, max: usize, subject: &str, what: &str) -> R<&'a Vec<Value>> {
    let Value::Array(items) = v else {
        return err(path, "expected a list.");
    };
    if items.len() < min {
        return err(path, format!("{subject} needs {min} or more {what}."));
    }
    if items.len() > max {
        // The first one past the limit, so the path points at what to cut.
        return err(&format!("{path}[{max}]"), format!("{subject} has at most {max} {what}."));
    }
    Ok(items)
}

fn count(v: &Value, path: &str) -> R<u32> {
    match v.as_u64().and_then(|n| u32::try_from(n).ok()) {
        Some(n) => Ok(n),
        None => err(path, "a whole number, 0 or more."),
    }
}

fn tone(m: &Map<String, Value>, path: &str) -> R<Tone> {
    match m.get("tone") {
        None => Ok(Tone::Neutral),
        Some(Value::String(s)) if s == "neutral" => Ok(Tone::Neutral),
        Some(Value::String(s)) if s == "attention" => Ok(Tone::Attention),
        Some(_) => err(&join(path, "tone"), "neutral or attention."),
    }
}

fn state(m: &Map<String, Value>, key: &str, path: &str, default: State) -> R<State> {
    match m.get(key) {
        None => Ok(default),
        Some(Value::String(s)) => match State::parse(s) {
            Some(state) => Ok(state),
            None => err(&join(path, key), format!("there's no state called {}. The states are {}.", quoted(s), words(&State::WORDS))),
        },
        Some(_) => err(&join(path, key), format!("one of {}.", words(&State::WORDS))),
    }
}

fn optional_text(m: &Map<String, Value>, key: &str, path: &str, max: usize) -> R<Option<String>> {
    match m.get(key) {
        None => Ok(None),
        Some(v) => Ok(Some(text(v, &join(path, key), max, false, false)?)),
    }
}

/// A string, NFC-normalized, with no control characters (a newline is allowed
/// when `multiline`) and no text-direction overrides, and at most `max`
/// characters. Empty only when `allow_empty`.
fn text(v: &Value, path: &str, max: usize, multiline: bool, allow_empty: bool) -> R<String> {
    let Value::String(raw) = v else {
        return err(path, "expected text.");
    };
    for ch in raw.chars() {
        if ch.is_control() && !(multiline && ch == '\n') {
            return err(path, format!("holds a control character (U+{:04X}), which a page can't have.", ch as u32));
        }
        if matches!(ch, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}') {
            return err(
                path,
                format!("holds a text-direction override (U+{:04X}), which a page can't have, since it can reorder a link's domain.", ch as u32),
            );
        }
    }
    let normalized = ComposingNormalizerBorrowed::new_nfc().normalize(raw).into_owned();
    if !allow_empty && normalized.trim().is_empty() {
        return err(path, "can't be empty.");
    }
    let n = normalized.chars().count();
    if n > max {
        return err(path, format!("at most {max} characters, and this is {n}."));
    }
    Ok(normalized)
}

// ---- times ----

/// The year 2100, in milliseconds: a timeline time past it is a typo.
const MAX_MILLIS: i64 = 4_102_444_800_000;

/// An RFC 3339 time as Unix milliseconds: `2026-10-04T15:02:00-07:00`, with
/// optional fractional seconds, and `Z` or an offset.
fn rfc3339_millis(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let digits = |from: usize, len: usize| -> Option<i64> {
        let part = b.get(from..from + len)?;
        part.iter().all(u8::is_ascii_digit).then(|| part.iter().fold(0i64, |n, d| n * 10 + i64::from(d - b'0')))
    };
    let (year, month, day) = (digits(0, 4)?, digits(5, 2)?, digits(8, 2)?);
    if b.get(4) != Some(&b'-') || b.get(7) != Some(&b'-') || !matches!(b.get(10), Some(b'T' | b't')) {
        return None;
    }
    let (hour, minute, second) = (digits(11, 2)?, digits(14, 2)?, digits(17, 2)?);
    if b.get(13) != Some(&b':') || b.get(16) != Some(&b':') {
        return None;
    }
    let mut i = 19;
    let mut millis = 0;
    if b.get(i) == Some(&b'.') {
        let start = i + 1;
        let mut end = start;
        while b.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        if end == start {
            return None;
        }
        for k in 0..3 {
            millis = millis * 10 + b.get(start + k).filter(|_| start + k < end).map_or(0, |d| i64::from(d - b'0'));
        }
        i = end;
    }
    let offset = match b.get(i)? {
        b'Z' | b'z' if i + 1 == b.len() => 0,
        sign @ (b'+' | b'-') if i + 6 == b.len() && b.get(i + 3) == Some(&b':') => {
            let (oh, om) = (digits(i + 1, 2)?, digits(i + 4, 2)?);
            if oh > 23 || om > 59 {
                return None;
            }
            let minutes = oh * 60 + om;
            if *sign == b'+' { minutes } else { -minutes }
        }
        _ => return None,
    };
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if !(1..=12).contains(&month) || day < 1 || day > month_days[(month - 1) as usize] || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    let ms = ((days * 24 + hour) * 60 + minute - offset) * 60_000 + second * 1000 + millis;
    (0..=MAX_MILLIS).contains(&ms).then_some(ms)
}

#[cfg(test)]
pub(super) fn rfc3339_for_test(s: &str) -> Option<i64> {
    rfc3339_millis(s)
}
