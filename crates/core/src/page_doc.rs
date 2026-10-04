//! An orchestrator page (ov-269, ov-281): a small JSON document of typed
//! blocks that the apps draw natively.
//!
//! This is the one definition. The CLI checks a document with it before
//! publishing (`farcooler page check`), and the daemon runs it again on every
//! `page.set` and is the authority. The phones and the Mac read the normalized
//! document it writes, held to `test/fixtures/pages/`.
//!
//! # What a page is
//!
//! A flat list of blocks from a closed vocabulary of nine (`heading`, `text`,
//! `stats`, `progress`, `table`, `list`, `timeline`, `steps`, `links`). There is
//! no layout language, no nesting beyond one level, no script, no HTML and no
//! fetched image. A reference names something the app already holds (a card, a
//! lane, a theme, a page, a worktree, a terminal) and the app draws its current
//! state, so a page can't go stale about anything the board knows. The only
//! thing that leaves the app is an `https` link, drawn with its domain.
//!
//! # Additive and removable
//!
//! Nothing here knows about tasks or the plan layer. A reference to a card, a
//! lane or a theme is text; whether it names something real is the daemon's
//! question, asked of the board when a page is published, and the apps' when
//! one is drawn. Removing the plan layer leaves every page valid.
//!
//! # Parsing
//!
//! The document is read from a `serde_json::Value` by hand rather than derived,
//! because a derived `deny_unknown_fields` can't say which block or cell is
//! wrong, and the person fixing a page is a language model reading one line of
//! error. Every refusal names the JSON path and the limit.

use serde::Serialize;
use serde::ser::{SerializeMap, Serializer};

mod parse;

pub use parse::{PageError, check_value, parse};

/// The schema version this build writes and reads.
pub const VERSION: u32 = 1;

/// The nine blocks, in the order the refusals and the reference list them.
pub const BLOCK_TYPES: [&str; 9] =
    ["heading", "text", "stats", "progress", "table", "list", "timeline", "steps", "links"];

/// The limits the runner enforces. The CLI checks the same ones first.
///
/// A struct rather than constants so a test can remove one and show the
/// fixture that exceeds it is accepted: a refusal fixture that fails for some
/// other reason would be a test that can't fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caps {
    /// The document after normalizing, in bytes.
    pub document_bytes: usize,
    /// Blocks on one page.
    pub blocks: usize,
    /// Characters in a `text` block's `md`.
    pub md_chars: usize,
    /// Characters in any other string.
    pub string_chars: usize,
    /// Characters in the title.
    pub title_chars: usize,
    /// Characters in the summary.
    pub summary_chars: usize,
    /// Characters in the glance line.
    pub glance_chars: usize,
    /// Columns in a table.
    pub table_columns: usize,
    /// Rows in a table.
    pub table_rows: usize,
    /// Items in a list.
    pub list_items: usize,
    /// Entries in a timeline.
    pub timeline_entries: usize,
    /// Figures in a stats row.
    pub stats_items: usize,
    /// Steps in a pipeline.
    pub steps: usize,
    /// Chips in a links block.
    pub links_items: usize,
    /// Parts of a progress bar.
    pub progress_parts: usize,
    /// References on one page.
    pub refs: usize,
}

impl Default for Caps {
    fn default() -> Caps {
        Caps {
            document_bytes: 32 * 1024,
            blocks: 60,
            md_chars: 2000,
            string_chars: 200,
            title_chars: 60,
            summary_chars: 120,
            glance_chars: 60,
            table_columns: 8,
            table_rows: 50,
            list_items: 50,
            timeline_entries: 50,
            stats_items: 6,
            steps: 12,
            links_items: 12,
            progress_parts: 6,
            refs: 200,
        }
    }
}

impl Caps {
    /// Every cap's name, as `without` takes it.
    pub const NAMES: [&'static str; 16] = [
        "document_bytes",
        "blocks",
        "md_chars",
        "string_chars",
        "title_chars",
        "summary_chars",
        "glance_chars",
        "table_columns",
        "table_rows",
        "list_items",
        "timeline_entries",
        "stats_items",
        "steps",
        "links_items",
        "progress_parts",
        "refs",
    ];

    /// These caps with one removed. Panics on a name that isn't a cap, so a
    /// test that misspells one fails instead of passing on the full limits.
    pub fn without(mut self, name: &str) -> Caps {
        let slot = match name {
            "document_bytes" => &mut self.document_bytes,
            "blocks" => &mut self.blocks,
            "md_chars" => &mut self.md_chars,
            "string_chars" => &mut self.string_chars,
            "title_chars" => &mut self.title_chars,
            "summary_chars" => &mut self.summary_chars,
            "glance_chars" => &mut self.glance_chars,
            "table_columns" => &mut self.table_columns,
            "table_rows" => &mut self.table_rows,
            "list_items" => &mut self.list_items,
            "timeline_entries" => &mut self.timeline_entries,
            "stats_items" => &mut self.stats_items,
            "steps" => &mut self.steps,
            "links_items" => &mut self.links_items,
            "progress_parts" => &mut self.progress_parts,
            "refs" => &mut self.refs,
            other => panic!("{other} is not a page cap"),
        };
        *slot = usize::MAX;
        self
    }
}

/// How many pages a workspace holds, anchored ones included.
pub const MAX_PAGES: usize = 12;
/// How many pages one theme draws inline.
pub const MAX_ANCHORED_PER_THEME: usize = 3;
/// Changing writes to one slot in an hour, past which the runner refuses.
pub const MAX_WRITES_PER_HOUR: usize = 30;
/// The longest a slot name is.
pub const SLOT_MAX: usize = 40;
/// The most `stale_after_min` can say: seven days.
pub const STALE_AFTER_MAX_MIN: u32 = 7 * 24 * 60;

/// Whether `slot` is a slot name: `[a-z0-9][a-z0-9-]{0,39}`.
pub fn valid_slot(slot: &str) -> bool {
    let mut chars = slot.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit())
        && slot.len() <= SLOT_MAX
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A validated, normalized page.
///
/// What `Serialize` writes is the document the runner stores and the apps
/// read: times as milliseconds, text in NFC, defaults left out.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Page {
    /// The schema version, always [`VERSION`] here.
    pub v: u32,
    /// The page's name in a row and a header.
    pub title: String,
    /// One line for the page's row in the overview.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub summary: String,
    /// A reserved line for the watch (at most 60 characters). No client draws
    /// it yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glance: Option<String>,
    /// Minutes after which the page says it is stale.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_after_min: Option<u32>,
    /// The blocks, in order.
    pub blocks: Vec<Block>,
}

/// How a thing is drawn: `neutral` is the default and `attention` is the amber
/// the apps already use for "Needs you".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    /// Drawn as everything else is.
    #[default]
    Neutral,
    /// Amber, for what needs the owner.
    Attention,
}

impl Tone {
    pub(crate) fn is_neutral(&self) -> bool {
        *self == Tone::Neutral
    }
}

/// An item's state. Each has a glyph and a word, and the word is always drawn
/// or spoken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Finished.
    Done,
    /// Being worked.
    Active,
    /// Waiting on something.
    Waiting,
    /// Can't go on.
    Blocked,
    /// Went wrong.
    Failed,
    /// Not started.
    Todo,
    /// No state to show.
    #[default]
    None,
}

impl State {
    /// Every state's stored word.
    pub const WORDS: [&'static str; 7] = ["done", "active", "waiting", "blocked", "failed", "todo", "none"];

    /// The state a stored word names.
    pub fn parse(word: &str) -> Option<State> {
        Some(match word {
            "done" => State::Done,
            "active" => State::Active,
            "waiting" => State::Waiting,
            "blocked" => State::Blocked,
            "failed" => State::Failed,
            "todo" => State::Todo,
            "none" => State::None,
            _ => return None,
        })
    }

    /// The word a person reads. Empty for `none`.
    pub fn word(self) -> &'static str {
        match self {
            State::Done => "Done",
            State::Active => "Active",
            State::Waiting => "Waiting",
            State::Blocked => "Blocked",
            State::Failed => "Failed",
            State::Todo => "To do",
            State::None => "",
        }
    }

    pub(crate) fn is_none(&self) -> bool {
        *self == State::None
    }
}

/// One of the nine blocks.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    /// A section header.
    Heading {
        /// The header's words.
        text: String,
    },
    /// A paragraph, in the Markdown subset the apps draw.
    Text {
        /// The Markdown.
        md: String,
        /// Drawn amber when `attention`.
        #[serde(skip_serializing_if = "Tone::is_neutral")]
        tone: Tone,
    },
    /// A row of figures.
    Stats {
        /// One to six figures.
        items: Vec<Stat>,
    },
    /// A bar, with its count in words.
    Progress {
        /// What is counted.
        label: String,
        /// How many are done.
        done: u32,
        /// How many there are.
        total: u32,
        /// A line under the bar.
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        /// Up to six parts of the bar, each with a count.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        parts: Vec<Part>,
    },
    /// A grid, stacked on a narrow surface.
    Table {
        /// One to eight columns.
        columns: Vec<Column>,
        /// Up to fifty rows of one cell per column.
        rows: Vec<Vec<Cell>>,
    },
    /// Rows with a state glyph and its word.
    List {
        /// Up to fifty items.
        items: Vec<Item>,
    },
    /// Dated entries.
    Timeline {
        /// How the entries are drawn.
        #[serde(skip_serializing_if = "Order::is_default")]
        order: Order,
        /// Up to fifty entries.
        entries: Vec<Entry>,
    },
    /// A pipeline of chips.
    Steps {
        /// Two to twelve steps.
        steps: Vec<Step>,
    },
    /// A row of link chips.
    Links {
        /// Up to twelve references.
        items: Vec<Reference>,
    },
}

impl Block {
    /// The block's `type` word.
    pub fn kind(&self) -> &'static str {
        match self {
            Block::Heading { .. } => "heading",
            Block::Text { .. } => "text",
            Block::Stats { .. } => "stats",
            Block::Progress { .. } => "progress",
            Block::Table { .. } => "table",
            Block::List { .. } => "list",
            Block::Timeline { .. } => "timeline",
            Block::Steps { .. } => "steps",
            Block::Links { .. } => "links",
        }
    }
}

/// One figure in a stats row.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Stat {
    /// What it measures.
    pub label: String,
    /// The figure, as written.
    pub value: String,
    /// A line under it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Amber when `attention`.
    #[serde(skip_serializing_if = "Tone::is_neutral")]
    pub tone: Tone,
}

/// One part of a progress bar.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Part {
    /// What the part is.
    pub label: String,
    /// How many are in it.
    pub count: u32,
}

/// A table column's side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    /// Where reading starts.
    #[default]
    Start,
    /// The middle.
    Center,
    /// Where reading ends: figures.
    End,
}

impl Align {
    pub(crate) fn is_default(&self) -> bool {
        *self == Align::Start
    }
}

/// A table column.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Column {
    /// Its heading, which a stacked row says before its value.
    pub title: String,
    /// Which side its cells align to.
    #[serde(skip_serializing_if = "Align::is_default")]
    pub align: Align,
    /// Whether it takes the room the others leave.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub grow: bool,
}

/// What a reference's live value is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Show {
    /// A lane's state words ("Fixing, round 1").
    State,
    /// A lane's spend.
    Spend,
}

/// A table cell: a string, or an object that can link and draw live.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    /// The text drawn. With none, the app draws the reference's live value.
    pub text: Option<String>,
    /// What the cell links to.
    pub reference: Option<Reference>,
    /// Which live value to draw, with no `text`.
    pub show: Option<Show>,
    /// Amber when `attention`.
    pub tone: Tone,
    /// Drawn in the monospaced face.
    pub mono: bool,
}

impl Serialize for Cell {
    /// A cell with only text is the string; anything else is an object.
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if let (Some(text), None, None, true, false) = (&self.text, &self.reference, &self.show, self.tone.is_neutral(), self.mono) {
            return s.serialize_str(text);
        }
        let mut map = s.serialize_map(None)?;
        if let Some(text) = &self.text {
            map.serialize_entry("text", text)?;
        }
        if let Some(reference) = &self.reference {
            map.serialize_entry("ref", reference)?;
        }
        if let Some(show) = &self.show {
            map.serialize_entry("show", show)?;
        }
        if !self.tone.is_neutral() {
            map.serialize_entry("tone", &self.tone)?;
        }
        if self.mono {
            map.serialize_entry("mono", &true)?;
        }
        map.end()
    }
}

/// One row of a list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Item {
    /// The row's words.
    pub text: String,
    /// Its state glyph and word.
    #[serde(skip_serializing_if = "State::is_none")]
    pub state: State,
    /// A line under it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// What it links to.
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<Reference>,
    /// Amber when `attention`.
    #[serde(skip_serializing_if = "Tone::is_neutral")]
    pub tone: Tone,
}

/// The order a timeline is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    /// Latest first.
    #[default]
    Newest,
    /// As written.
    Given,
}

impl Order {
    pub(crate) fn is_default(&self) -> bool {
        *self == Order::Newest
    }
}

/// One entry of a timeline.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Entry {
    /// When, in Unix milliseconds. Written as RFC 3339 or milliseconds.
    pub at: i64,
    /// What happened.
    pub text: String,
    /// What it links to.
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<Reference>,
}

/// One step of a pipeline.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Step {
    /// The step's name.
    pub label: String,
    /// Where it stands.
    pub state: State,
}

/// What a reference points at.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// A card, by key.
    Task(String),
    /// A card's open question.
    Ask(String),
    /// A lane, by name.
    Lane(String),
    /// A theme, by name.
    Theme(String),
    /// Another page, by slot.
    Page(String),
    /// A worktree, by name.
    Worktree(String),
    /// A terminal in a worktree.
    Terminal {
        /// The worktree's name.
        worktree: String,
        /// The terminal's name.
        name: String,
    },
    /// An `https` link.
    Url(String),
}

impl Target {
    /// Every target's key, in the order the refusals list them.
    pub const KINDS: [&'static str; 8] = ["task", "ask", "lane", "theme", "page", "worktree", "terminal", "url"];

    /// The key this target is written under.
    pub fn kind(&self) -> &'static str {
        match self {
            Target::Task(_) => "task",
            Target::Ask(_) => "ask",
            Target::Lane(_) => "lane",
            Target::Theme(_) => "theme",
            Target::Page(_) => "page",
            Target::Worktree(_) => "worktree",
            Target::Terminal { .. } => "terminal",
            Target::Url(_) => "url",
        }
    }

    /// The name this target is written with, for a message.
    pub fn name(&self) -> &str {
        match self {
            Target::Task(s)
            | Target::Ask(s)
            | Target::Lane(s)
            | Target::Theme(s)
            | Target::Page(s)
            | Target::Worktree(s)
            | Target::Url(s) => s,
            Target::Terminal { name, .. } => name,
        }
    }
}

/// A reference: one target and an optional label.
///
/// In a `links` block the target key sits in the item itself; everywhere else
/// it is under `ref`. Either way it is this.
#[derive(Debug, Clone, PartialEq)]
pub struct Reference {
    /// What it points at.
    pub target: Target,
    /// What to say instead of the live value, or beside a link.
    pub label: Option<String>,
}

impl Serialize for Reference {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(None)?;
        match &self.target {
            Target::Terminal { worktree, name } => {
                #[derive(Serialize)]
                struct Terminal<'a> {
                    worktree: &'a str,
                    name: &'a str,
                }
                map.serialize_entry("terminal", &Terminal { worktree, name })?;
            }
            other => map.serialize_entry(other.kind(), other.name())?,
        }
        if let Some(label) = &self.label {
            map.serialize_entry("label", label)?;
        }
        map.end()
    }
}

/// A reference and where it sits on the page, for checking against the board.
#[derive(Debug, Clone, PartialEq)]
pub struct RefAt<'a> {
    /// The JSON path of the cell, item or entry that holds it.
    pub path: String,
    /// The reference.
    pub reference: &'a Reference,
}

impl Page {
    /// The canonical document: what the runner stores and the apps read.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("a page is plain data")
    }

    /// Every reference on the page, in reading order.
    pub fn references(&self) -> Vec<RefAt<'_>> {
        let mut out = Vec::new();
        for (b, block) in self.blocks.iter().enumerate() {
            match block {
                Block::Table { rows, .. } => {
                    for (r, row) in rows.iter().enumerate() {
                        for (c, cell) in row.iter().enumerate() {
                            if let Some(reference) = &cell.reference {
                                out.push(RefAt { path: format!("blocks[{b}].rows[{r}][{c}]"), reference });
                            }
                        }
                    }
                }
                Block::List { items } => {
                    for (i, item) in items.iter().enumerate() {
                        if let Some(reference) = &item.reference {
                            out.push(RefAt { path: format!("blocks[{b}].items[{i}]"), reference });
                        }
                    }
                }
                Block::Timeline { entries, .. } => {
                    for (i, entry) in entries.iter().enumerate() {
                        if let Some(reference) = &entry.reference {
                            out.push(RefAt { path: format!("blocks[{b}].entries[{i}]"), reference });
                        }
                    }
                }
                Block::Links { items } => {
                    for (i, reference) in items.iter().enumerate() {
                        out.push(RefAt { path: format!("blocks[{b}].items[{i}]"), reference });
                    }
                }
                Block::Heading { .. }
                | Block::Text { .. }
                | Block::Stats { .. }
                | Block::Progress { .. }
                | Block::Steps { .. } => {}
            }
        }
        out
    }

    /// The page's shape as text, with no content: block counts in the
    /// vocabulary's order, then the reference kinds used with a `ref-` prefix
    /// (`heading:2 stats:1 table:1 list:1 ref-lane:3 ref-task:2`). What
    /// `page_events` keeps, so which shapes recur can be measured without
    /// reading anyone's page (design section 8).
    pub fn shape(&self) -> String {
        let mut parts = Vec::new();
        for kind in BLOCK_TYPES {
            let n = self.blocks.iter().filter(|b| b.kind() == kind).count();
            if n > 0 {
                parts.push(format!("{kind}:{n}"));
            }
        }
        let refs = self.references();
        for kind in Target::KINDS {
            let n = refs.iter().filter(|r| r.reference.target.kind() == kind).count();
            if n > 0 {
                parts.push(format!("ref-{kind}:{n}"));
            }
        }
        parts.join(" ")
    }
}

/// The host of an `https` link, or `None` when it isn't a plain one.
///
/// What an app draws beside a link's label, so the label can't hide where the
/// link goes. The validator refuses a link this can't read, and one with a
/// user name in front of the host (`https://github.com@elsewhere.example/`).
pub fn url_host(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => host,
        Some(_) => return None,
        None => authority,
    };
    let ok = !host.is_empty()
        && host.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    ok.then_some(host)
}

#[cfg(test)]
mod tests;
