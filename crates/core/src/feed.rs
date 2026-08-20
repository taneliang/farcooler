//! What an agent is saying, and what it is doing, short enough to put in a row.
//!
//! A row that says "Working" reports that something is happening and refuses to
//! say what. The session logs know exactly what, and this is where that becomes
//! two things a person can read at a glance, kept apart on purpose:
//!
//! * **The transcript** — a [`Feed`], which is a WINDOW onto what the agent
//!   has been saying, in its own words, with no verb in front of it. Only
//!   prose reaches it. Tool calls used to, prefixed with the tool's own
//!   lowercased name, which is how a lock screen came to read `taskupdate`
//!   and `says Done.`
//! * **The signal line** — one line saying where the agent IS: the question it
//!   is blocked on, or its position in its own task list, or what it is doing
//!   right now. See [`signal`] for the priority and [`line`] for the rung that
//!   renders it.
//!
//! Four decisions live here rather than in each client, and each one is a
//! disagreement waiting to happen if it moves:
//!
//! * **Three LINES, not three messages.** A sidebar row is about forty
//!   characters wide and a Live Activity is smaller, so three lines is what
//!   there is room for. What fills them is the tail of the prose, wrapped —
//!   new text pushes old text up, the way watching a terminal does. It used to
//!   be the last three MESSAGES, each cut to its first forty characters, which
//!   is a different thing wearing the same shape: three fragments, none of
//!   them a sentence, and the complaint that named this stage.
//! * **Redaction, on the way in.** These strings are tool arguments and
//!   agent-authored prose — a shell command, a file path, a task's phrase — and
//!   a shell command is exactly where a token lives. They reach a phone's lock
//!   screen through the relay, so they pass through [`crate::redact::redact`]
//!   BEFORE they are stored, not before they are displayed. [`Step`] and
//!   [`Phrase`] have private fields, and [`Feed::append`] and [`cleaned`] are
//!   the only ways to make one — which is what makes that unbypassable rather
//!   than merely conventional.
//! * **Paths narrowed to their last segment, in that same place and for that
//!   same reason.** See [`narrow_paths`].
//! * **Wrapping and truncation, here in the daemon.** A Mac, a phone and a
//!   watch must not each decide where the line breaks or where the ellipsis
//!   goes, so the strings that go on the wire are already the strings that go
//!   in the rows.
//!
//! Nothing here clears a feed. A finished agent KEEPS what it said: "what did
//! it do while I was away" is exactly when the summary is worth most.
//!
//! ## The ladder
//!
//! The feed is one rung of a four-rung ladder, and the other three live here
//! too: [`glyph`], [`headline`] and [`line`], plus [`rank`] beside them. The
//! destination for all four is a Live Activity — a lock screen, a Dynamic
//! Island, a watch face — where the job is overseeing a fleet from wherever
//! you are, in a space that may be one line tall. A Dynamic Island cannot
//! truncate a forty-character string well, and a watch complication cannot
//! re-derive which of six agents matters most, so both are decided ONCE,
//! here, rather than by every client that renders them.
//!
//! Each rung is a STRICT NARROWING of the one below: whatever [`headline`]
//! says must be derivable from [`line`], and [`glyph`] from both. A row that
//! says `?` on a watch must be a row whose headline says `needs you`, must be
//! a row whose line is a question — never three surfaces disagreeing about
//! the same pane because each truncated a different string in its own way.
//!
//! [`Subject`] is the one argument every rung takes, so they cannot be handed
//! mismatched facts about two different panes, and a fifth rung added later
//! needs no new argument list.

use std::time::Duration;

use farcooler_protocol::v1::AgentActivity;

use crate::activity::exit_wants_attention;
use crate::redact;

/// How many display lines the window shows.
///
/// Three. A row is one line of a sidebar and a Live Activity is smaller; a
/// fourth would either shrink the other three or push one off a screen that
/// never had room for it.
pub const CAPACITY: usize = 3;

/// How wide one display line is, in CHARACTERS.
///
/// Roughly the width of a sidebar row. Counted in characters rather than bytes
/// because that is what a person reads and what a layout is measured in — and
/// because slicing a byte count would land inside a multi-byte character.
pub const WIDTH: usize = 40;

/// The most of one message that can reach the window, in characters.
///
/// The bound on everything here, and it is worth being explicit about why it
/// is enough. Only [`CAPACITY`] lines are ever kept, each holding at most
/// [`WIDTH`] characters plus the space a break eats, so no more than
/// `CAPACITY * (WIDTH + 1)` characters of a message can ever be on screen —
/// twice that is generous by a factor of two, which is what makes the wrap
/// fall where it would have fallen if the whole message had been wrapped, and
/// leaves a part-word at the cut with nowhere to land.
///
/// This matters because it runs per pane on a loop, and an agent writes a lot
/// of prose: a `Feed` holds three short strings whatever it is fed, and the
/// work per message is linear in the message rather than in the turn.
const WINDOW: usize = 2 * CAPACITY * (WIDTH + 1);

/// `text` fit for a row: redacted, flattened, narrowed and cut to `max`.
///
/// The one choke point. Redaction runs first and truncation last, and that
/// order matters: redaction works on whitespace-separated tokens, so a string
/// already cut mid-token hides the shape it was looking for. A newline-bearing
/// string — an agent's prose runs to several lines — is flattened to one line
/// only AFTER redaction, because `redact` reads a line at a time and joining
/// first would hand it one very long line instead. [`narrow_paths`] comes after
/// that flattening, because it reads the tokens of exactly one line.
///
/// Every user-visible string this module produces goes through these three
/// passes, and the types that hold one ([`Step`], [`Phrase`]) keep their
/// fields private so there is no other way in. A [`Phrase`] is one row, so it
/// takes them here and ends in a cut; a [`Step`] is one line of several, so it
/// takes them in [`Feed::append`] and ends in a wrap. Two callers, one order,
/// no third way. That is what makes the guarantee unbypassable rather than
/// conventional — and the guarantee is about what those types CONTAIN, not
/// about which argument a caller happened to put a string in.
fn cleaned(text: &str, max: usize) -> String {
    clipped(&narrow_paths(&one_line(&redact::redact(text))), max)
}

/// One display line of the window, already redacted and already a row wide.
///
/// A line of the agent's prose, verbatim, with no verb in front of it — the
/// transcript is only what the agent SENT. It is a LINE and not a message:
/// long prose spans several of these, and a short sentence is one. A tool call
/// is neither: it is a [`Phrase`] on the signal line, which is where "what it
/// is doing" belongs.
///
/// The field is private on purpose. Every string in here is bound for a lock
/// screen, and a public field would let a caller construct a `Step` that never
/// passed through redaction — which is the failure this module exists to
/// prevent, not a style preference. [`Feed::append`] is the only constructor
/// there is, and it is private too: [`Feed::push`] and [`Feed::conclude`] both
/// go through it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    text: String,
}

impl Step {
    /// What the agent said, as one line.
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// One agent-authored phrase, already redacted and already short.
///
/// A task's `activeForm` ("Designing test matrix"), a subagent's description,
/// or a tool call rendered as an action ("Writing fruit.txt"). All three are
/// written by the agent and can carry a path or a credential exactly as a
/// transcript line can, so all three take the same treatment — see [`cleaned`],
/// and note the private field for the same reason [`Step`] has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phrase {
    text: String,
}

impl Phrase {
    /// A phrase the agent wrote, cleaned for a row.
    pub fn new(text: &str) -> Self {
        Phrase { text: cleaned(text, WIDTH) }
    }

    /// A tool call as the action it is: `Writing fruit.txt`.
    ///
    /// The two halves are budgeted exactly as a transcript line's used to be:
    /// the verb takes what it needs and the object takes what is left, because
    /// `Writing` with no room for the file it is writing says less than
    /// nothing. Either half can be empty — claude's `step_object` yields
    /// nothing for a tool whose input names no file, command, pattern or
    /// description — so the join never produces a leading or trailing space.
    pub fn action(verb: &str, object: &str) -> Self {
        Self::phrased(&humanized(verb), object)
    }

    /// The same tool call, once the turn that made it is over: `Wrote
    /// fruit.txt`.
    ///
    /// The action outlives its turn on purpose — a finished row is read to
    /// find out what the agent DID, which is why `Signals` clears this at the
    /// start of the next turn rather than at the end of this one. But it was
    /// rendered in the present tense either way, so a pane that had finished
    /// half an hour ago still said `Running cd …` as though it were running
    /// something. Same fact, correct tense.
    pub fn action_done(verb: &str, object: &str) -> Self {
        Self::phrased(&past(verb), object)
    }

    fn phrased(verb: &str, object: &str) -> Self {
        let verb = cleaned(verb, WIDTH);
        // `saturating_sub` because a pathologically long verb can claim the
        // whole width, which leaves the object nothing rather than a negative
        // budget.
        let room = WIDTH.saturating_sub(verb.chars().count() + 1);
        let object = cleaned(object, room);
        let text = match (verb.is_empty(), object.is_empty()) {
            (false, false) => format!("{verb} {object}"),
            (false, true) => verb,
            (true, false) => object,
            (true, true) => String::new(),
        };
        Phrase { text }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

/// A tool name said in English, and a rule for the ones nobody predicted.
///
/// The table holds the tools that actually turn up, and deliberately does not
/// try to be complete: a table that claimed to cover every tool claude ships
/// would be wrong the first time it ships another one, and wrong in the worst
/// way — a match arm nothing falls through leaves a row blank. So the fallback
/// is TOTAL rather than a last arm that gives up: any name at all comes back
/// as itself, capitalized, with the separators an MCP tool spells its name
/// with turned into spaces. `mcp__linear__create_issue` reads as
/// `Mcp linear create issue`, which is not beautiful and is legible — tool
/// names are written for people to read, which is exactly why the lowercased
/// ones read so badly.
///
/// Matching is on the lowercased name the parsers emit (`session_log::claude`
/// and friends), so the table is written that way too.
fn humanized(verb: &str) -> String {
    let known = match verb {
        "write" => Some("Writing"),
        "read" => Some("Reading"),
        "edit" | "multiedit" | "notebookedit" | "apply_patch" => Some("Editing"),
        "bash" | "shell" | "run" | "bashoutput" => Some("Running"),
        "grep" | "glob" | "search" | "websearch" | "codebase_search" => Some("Searching"),
        "webfetch" | "fetch" => Some("Fetching"),
        "agent" | "task" => Some("Delegating"),
        "taskcreate" | "taskupdate" | "tasklist" | "todowrite" | "update_plan" => Some("Planning"),
        // Not "Askuserquestion", which is what the fallback below made of it:
        // a lowercased tool name with its first letter pushed back up, on the
        // one row a person is most likely to be reading. The agent is not doing
        // this to the user, it is stopped in front of them.
        "askuserquestion" | "exitplanmode" => Some("Asking"),
        _ => None,
    };
    if let Some(known) = known {
        return known.to_string();
    }
    let spaced: String = verb.chars().map(|c| if c == '_' { ' ' } else { c }).collect();
    let mut words = spaced.split_whitespace();
    let Some(first) = words.next() else { return String::new() };
    let rest = words.collect::<Vec<&str>>().join(" ");
    let mut head = first.chars();
    let capitalized = match head.next() {
        Some(c) => c.to_uppercase().collect::<String>() + head.as_str(),
        None => String::new(),
    };
    if rest.is_empty() { capitalized } else { format!("{capitalized} {rest}") }
}

/// The same tool name, said as something that already happened.
///
/// Only the table has a past tense. The fallback deliberately does not try to
/// invent one: `humanized`'s unknown-verb path produces `Mcp linear create
/// issue`, which is a tool's NAME rather than an English verb, and bolting
/// `-ed` onto whatever an MCP server happens to call itself would be a
/// guess that reads worse than the name it mangled. An unknown tool therefore
/// reads the same in both tenses, which is the honest answer — it never
/// claimed to be a verb in the first place.
fn past(verb: &str) -> String {
    let known = match verb {
        "write" => Some("Wrote"),
        "read" => Some("Read"),
        "edit" | "multiedit" | "notebookedit" | "apply_patch" => Some("Edited"),
        "bash" | "shell" | "run" | "bashoutput" => Some("Ran"),
        "grep" | "glob" | "search" | "websearch" | "codebase_search" => Some("Searched"),
        "webfetch" | "fetch" => Some("Fetched"),
        "agent" | "task" => Some("Delegated"),
        "askuserquestion" | "exitplanmode" => Some("Asked"),
        "taskcreate" | "taskupdate" | "tasklist" | "todowrite" | "update_plan" => Some("Planned"),
        _ => None,
    };
    known.map_or_else(|| humanized(verb), str::to_string)
}

/// Where an agent is in its own task list, folded from its log.
///
/// `done` and `total` are counts and cannot leak anything, so they are plain
/// numbers. `active_form` is the agent's own present-tense phrase for the task
/// it is on ("Designing test matrix"), which is agent-authored text and
/// therefore a [`Phrase`].
///
/// `active_form` is `None` whenever the phrase could not be joined to the task
/// that moved — see the daemon's fold. That asymmetry is deliberate: the COUNT
/// is what a person reads a progress indicator for, and it must survive a join
/// that broke. A plan with no phrase reads `3/7`, which is still worth the row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub done: u32,
    pub total: u32,
    pub active_form: Option<Phrase>,
}

/// The signal line for an agent that is NOT blocked: where it is, in one line.
///
/// Strictly prioritized, first match wins, and the question that outranks both
/// of these lives in [`line`] because only the rungs know the agent's state:
///
/// 1. **Plan position** — `3/7 · Designing test matrix`, plus a subagent count
///    when any are running. An agent that has written itself a task list has
///    said, in its own words, both how far along it is and what it is on.
/// 2. **The current action** — `Writing fruit.txt` — for an agent with no task
///    list, which is codex, cursor, and most claude sessions.
///
/// `None` when there is neither, which is a pane that has done nothing yet
/// rather than a gap to paper over; [`line`] falls back to the headline.
pub fn signal(plan: Option<&Plan>, subagents: usize, action: Option<&Phrase>) -> Option<String> {
    if let Some(plan) = plan.filter(|plan| plan.total > 0) {
        let mut parts = vec![format!("{}/{}", plan.done, plan.total)];
        if let Some(form) = plan.active_form.as_ref().filter(|form| !form.is_empty()) {
            parts.push(form.as_str().to_string());
        }
        if subagents > 0 {
            parts.push(format!("{subagents} {}", if subagents == 1 { "agent" } else { "agents" }));
        }
        return Some(clipped(&parts.join(" · "), WIDTH));
    }
    action.filter(|action| !action.is_empty()).map(|action| action.as_str().to_string())
}

/// The last [`CAPACITY`] display lines of what the agent has been saying.
///
/// A window, not a list. Prose arrives, is wrapped to [`WIDTH`], and the lines
/// join the end of the window; the fourth line from the bottom falls off the
/// top. Watching it is watching a terminal scroll.
///
/// Nothing clears it. A finished agent KEEPS what it said, because "what did
/// it do while I was away" is exactly when the summary is worth most — which
/// is also why a row never changes HEIGHT when a turn ends: three lines stay
/// three lines, and a sidebar that rearranges itself while it is being read is
/// worse than a long one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Feed {
    /// A `Vec` rather than a `VecDeque`: it holds three elements, so shifting
    /// on eviction costs nothing, and a slice is what callers want to read.
    steps: Vec<Step>,
}

/// Which end of a message survives when it is longer than the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Keep {
    /// Its opening. An answer is a summary, and a summary says what it did in
    /// its first sentence; what follows is the detail and the offer to do more.
    Head,
    /// Its ending. Narration is a stream being watched, and the newest thing
    /// in it is the whole reason to look.
    Tail,
}

impl Feed {
    /// Record narration — prose the agent wrote on its way through a turn.
    ///
    /// The window takes its tail, so a long paragraph reads as the sentence it
    /// is on now rather than as the sentence it opened with.
    pub fn push(&mut self, text: &str) {
        self.append(text, Keep::Tail);
    }

    /// Record the turn's closing answer.
    ///
    /// Same window, one difference: a long answer enters from its START. It is
    /// a summary rather than a stream, and an agent's summary opens with what
    /// it did — "I've added the tail window and fixed the stale action" — then
    /// runs on into detail and into offers of further work, which is exactly
    /// what three lines have no room for. This is the only thing the
    /// narration/conclusion distinction is kept for; see
    /// `session_log::TurnEvent::Said`.
    ///
    /// Nothing is cleared first. The narration above an answer is what the
    /// agent was doing to reach it, and it reads as a small transcript rather
    /// than as clutter — and clearing would shrink the row at the moment the
    /// turn ended.
    pub fn conclude(&mut self, text: &str) {
        self.append(text, Keep::Head);
    }

    /// The one door in, and the only place a [`Step`] is made.
    ///
    /// Redaction runs on the WHOLE message, before anything is thrown away:
    /// `redact` reads whitespace-separated tokens, so cutting first could
    /// present it half a token and hide the shape it was looking for. Only
    /// then is the message cut to [`WINDOW`] — from whichever end will be
    /// shown — and wrapped.
    ///
    /// See [`cleaned`] for why the passes run in the order they do; this is
    /// the same order with the final cut replaced by a wrap.
    fn append(&mut self, text: &str, keep: Keep) {
        let text = narrow_paths(&one_line(&redact::redact(text)));
        if text.is_empty() {
            return;
        }
        let mut lines = wrapped(&windowed(&text, keep), WIDTH);
        if keep == Keep::Head && lines.len() > CAPACITY {
            // The answer ran past what a row can hold, and a reader who is not
            // told that reads the third line as the end of the sentence.
            lines.truncate(CAPACITY);
            if let Some(last) = lines.last_mut() {
                *last = clipped(&format!("{last}…"), WIDTH);
            }
        }
        self.steps.extend(lines.into_iter().map(|text| Step { text }));
        if self.steps.len() > CAPACITY {
            self.steps.drain(..self.steps.len() - CAPACITY);
        }
    }

    /// The lines, oldest first.
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// The window as the lines a client renders, oldest first.
    ///
    /// What goes on the wire. A client receives finished lines and decides
    /// nothing about them, which is the whole point of wrapping here.
    pub fn lines(&self) -> Vec<String> {
        self.steps.iter().map(|step| step.text.clone()).collect()
    }

    /// The most recent thing the agent said, or nothing when it has said
    /// nothing.
    ///
    /// One step is one line — see [`lines`](Self::lines) — so this is a whole
    /// message rather than a fragment of one, already redacted and already cut
    /// to the window by whichever of [`push`](Self::push) or
    /// [`conclude`](Self::conclude) recorded it. Which end it was cut from is
    /// the point: a turn that ended with an answer went through `conclude`,
    /// which keeps the OPENING, and an agent's summary opens with what it did.
    ///
    /// That is what makes this worth putting in a notification. "claude
    /// finished" is a sentence about Far Cooler; "I've added the tail window
    /// and fixed the stale action" is a sentence about the work, and it is
    /// already sitting here by the time the turn is over.
    pub fn latest(&self) -> Option<&str> {
        self.steps.last().map(|step| step.text.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

/// The end of `text` the window could possibly show, and no more.
///
/// A cut lands mid-word, and that is fine at both ends: the mangled word is
/// beyond the [`CAPACITY`] lines that survive the wrap either way, because
/// [`WINDOW`] is twice what those lines can hold. See [`WINDOW`].
fn windowed(text: &str, keep: Keep) -> String {
    let length = text.chars().count();
    if length <= WINDOW {
        return text.to_string();
    }
    match keep {
        Keep::Head => text.chars().take(WINDOW).collect(),
        Keep::Tail => text.chars().skip(length - WINDOW).collect(),
    }
}

/// `text` broken into lines of at most `width` CHARACTERS, at the spaces.
///
/// Greedy, which is what a terminal does and what a reader expects: a word
/// goes on the current line if it fits and starts the next one if it does not.
///
/// A word LONGER than the width is broken rather than allowed to overhang. A
/// token with no space in it that runs past forty characters is a URL, a hash
/// or a base64 blob, and a line that refuses to break it is a line that either
/// overflows the row or vanishes into the truncation whole.
fn wrapped(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut len = 0usize;
    // Split on a single space rather than on whitespace because `one_line` has
    // already run: every separator left is exactly one space.
    for word in text.split(' ').filter(|word| !word.is_empty()) {
        if len > 0 && len + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
            len = 0;
        }
        if len > 0 {
            line.push(' ');
            len += 1;
        }
        for c in word.chars() {
            if len == width {
                lines.push(std::mem::take(&mut line));
                len = 0;
            }
            line.push(c);
            len += 1;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// `text` with every run of whitespace — newlines included — as one space.
///
/// A row is one line. An agent's prose and a heredoc-bearing shell command both
/// arrive with newlines in them, and a client that rendered them raw would draw
/// one row over three.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<&str>>().join(" ")
}

/// `text` with every ABSOLUTE FILESYSTEM PATH in it cut down to its last
/// segment.
///
/// Two reasons, and either alone would be enough.
///
/// The protocol's rule, stated in `crates/daemon/src/wire.rs`: paths are
/// `host_admin` only, every path-bearing field is optional in the proto and is
/// populated only for a client that holds it, because a phone on someone
/// else's network has no business learning the directory layout of the machine
/// it is driving. A feed step is not one of those optional fields — it is
/// unconditional, at every scope — and its object is a raw tool argument. So
/// `bash cd /Users/someone/Dev/acme-client-confidential && cargo test` was
/// naming a client, a home directory and a machine's layout on a lock screen
/// that is not allowed to know any of them.
///
/// And forty characters spent on a path prefix are forty characters not spent
/// on what the agent did. `write feed.rs` is both the private rendering and
/// the legible one — which is why claude's `step_object` was already taking
/// the basename of `file_path` before this existed. This does the same for the
/// arguments that were still passing through whole: a `command`, a `pattern`,
/// a `description`.
///
/// Conservative for the same reason [`redact`](crate::redact::redact) is: a
/// false positive costs a few characters of context, and a token is narrowed
/// only when it is unmistakably a path — it begins `/` or `~/` and has a
/// second segment after that. `cargo test -p farcooler-core`,
/// `https://example.com/a/b`, `and/or`, `src/feed.rs` and a bare `/tmp` all
/// come back exactly as they went in.
///
/// Splits on a single space rather than on whitespace because [`one_line`] has
/// already run: every separator left is exactly one space, so the join puts
/// back what the split took.
fn narrow_paths(text: &str) -> String {
    text.split(' ').map(narrow_token).collect::<Vec<String>>().join(" ")
}

/// One whitespace-free token, narrowed if it is or carries a path.
fn narrow_token(token: &str) -> String {
    // `--out=/a/b/c.txt` names a flag and then a path. Narrowing only what is
    // after the '=' leaves the command still reading as the command it was —
    // "do not mangle flags" is about the flag, not about the path bolted to it.
    if let Some((name, value)) = token.split_once('=') {
        if let Some(last) = last_segment_of_a_path(value) {
            return format!("{name}={last}");
        }
    }
    last_segment_of_a_path(token).unwrap_or_else(|| token.to_string())
}

/// The last segment of `token` when `token` is unmistakably an absolute path,
/// and `None` for everything else — which is most things.
///
/// Each rejection below is a shape that is not this machine's directory
/// layout: a relative path (`src/feed.rs`) says nothing about where the
/// machine keeps things; `//host/share` opens a protocol-relative URL or a UNC
/// name; and a single-segment `/tmp` or `~/Dev` reveals nothing a person could
/// not have guessed, while reading worse as `tmp`.
fn last_segment_of_a_path(token: &str) -> Option<String> {
    let quotes = |c: char| c == '"' || c == '\'';
    let path = token.trim_matches(quotes);
    let rest = match path.strip_prefix("~/") {
        Some(rest) => rest,
        None => path.strip_prefix('/').filter(|rest| !rest.starts_with('/'))?,
    };
    if !rest.contains('/') {
        return None;
    }
    // A trailing slash is a directory's, not a segment of its own, so the
    // LAST NON-EMPTY segment is what `/a/b/` is named by.
    Some(rest.rsplit('/').find(|segment| !segment.is_empty())?.to_string())
}

/// `text` cut to at most `max` characters, ending in an ellipsis when it was.
///
/// Characters, not bytes: see [`WIDTH`]. The trailing space of a cut that
/// happened to land after a word is trimmed, so the result reads `cargo test…`
/// rather than `cargo test …` — the latter looks like the ellipsis is a word of
/// its own.
fn clipped(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    // The ellipsis costs a character, so a budget of one cannot hold any text
    // at all — and a budget of zero cannot hold the ellipsis either.
    if max == 0 {
        return String::new();
    }
    let kept: String = text.chars().take(max - 1).collect();
    format!("{}…", kept.trim_end())
}

/// How wide a [`headline`] may be, in characters.
///
/// The design's own budget is "~16" — approximate, and its own worked
/// examples prove it: `codex needs you` is fifteen characters but `cargo test
/// failed` is seventeen. Eighteen fits both worked examples whole, with one
/// character of headroom, rather than truncating the design doc's own
/// example the first time this is checked against it.
pub const HEADLINE_WIDTH: usize = 18;

/// The shapes an agent's activity is worth reporting at this size.
///
/// Not [`AgentActivity`] itself: `Idle`, `None`, `Unknown` and the zero value
/// all read the same at a glyph's one character — there is nothing to rank
/// and nothing to say beyond "nothing is happening" — so collapsing them here
/// is what keeps [`glyph`] and [`rank`] from needing an opinion about a state
/// the ladder never named. See the design doc's "Built for a lock screen,
/// rendered in a sidebar", whose table names exactly `working`, `blocked` and
/// `done` for an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Working,
    Blocked,
    Done,
    /// Finished, and finished BADLY: the agent's own log recorded that the
    /// turn died.
    ///
    /// A rung of this ladder rather than an [`AgentActivity`] on purpose. The
    /// wire's activity vocabulary is what `activity::advance` folds and what
    /// notifications key off, and a whole new state there would have to be
    /// created, destroyed and acknowledged by that fold; whether the turn that
    /// just ended came back an error is a reading of the same `Done`, which is
    /// exactly the kind of narrowing this enum exists to do. Every rung below
    /// must decide what to make of it, which is why it is a variant here and
    /// not a boolean threaded past three matches that could each forget it.
    Failed,
    Idle,
}

impl From<AgentActivity> for AgentState {
    /// Never [`AgentState::Failed`], and that is not an omission: the wire's
    /// activity says only that a turn ended, and how it ended is a separate
    /// fact carried beside it (`Terminal.turn_failed`). The caller holding both
    /// is the one that gets to narrow a `Done` to a `Failed` — see
    /// `daemon::wire::rung_subject`.
    fn from(activity: AgentActivity) -> Self {
        match activity {
            AgentActivity::Working => AgentState::Working,
            AgentActivity::Blocked => AgentState::Blocked,
            AgentActivity::Done => AgentState::Done,
            AgentActivity::Idle
            | AgentActivity::None
            | AgentActivity::Unknown
            | AgentActivity::Unspecified => AgentState::Idle,
        }
    }
}

/// How a non-agent pane's foreground command ended, or is still running.
///
/// Mirrors the wire's `ExitStatus` field for field: the daemon already has
/// one of these for every terminal that has exited, and inventing a second
/// shape here would be a second thing to keep in sync with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exit {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

/// One pane's compact state, in facts the daemon already knows — never text a
/// client would have to interpret. [`glyph`], [`headline`], [`line`] and
/// [`rank`] all take this and nothing else.
///
/// Two shapes because a fleet is not only agents: a `pnpm dev`, a `cargo
/// build`, a plain shell sit in the same sidebar and have to fill the same
/// rungs, from different facts — the command, what it serves, whether it
/// exited and how, rather than a session log it does not have. See the
/// design doc's "Panes that are not agents".
#[derive(Debug, Clone)]
pub enum Subject {
    Agent {
        /// What to call this pane: the agent's own preset name (`claude`,
        /// `codex`) when nothing better is known, or the task title once the
        /// agent has set one. Whichever it is, it is the same string a
        /// sidebar row already shows as its label — reused here rather than
        /// invented a second time.
        name: String,
        state: AgentState,
        /// Time since the current TURN began, held across a `Blocked`
        /// question being answered. What [`headline`]'s `claude 4m` reports:
        /// "how long has this task been running", not "how long has this
        /// screen looked like this". `None` between turns.
        turn_elapsed: Option<Duration>,
        /// Time since `state` last changed. What [`rank`]'s "the oldest
        /// blocked first" is measured against: for `Blocked` this is exactly
        /// how long the person has been kept waiting, which `turn_elapsed` is
        /// NOT — it is held across the very prompt this is timing.
        state_age: Duration,
        /// What the agent is asking, when `state` is `Blocked` and the
        /// question was legible. Already redacted.
        question: Option<String>,
        /// Where the agent is, in one line: its plan position or its current
        /// action, already composed by [`signal`] and already cleaned.
        ///
        /// Composed rather than assembled here, so the daemon that folds the
        /// task list and the rung that renders it cannot come to two different
        /// answers about the same pane.
        signal: Option<String>,
    },
    Command {
        /// What is running, or last ran: `cargo build`, `pnpm dev`.
        command: String,
        /// `None` while the command is still running.
        exit: Option<Exit>,
        /// Time since the command started (still running) or exited — the
        /// same purpose `state_age` serves for an agent, and [`line`]'s
        /// trailing duration.
        state_age: Duration,
    },
}

/// `duration` as at most a few characters: `0s`, `4m`, `1m 40s`, `2h 5m`.
///
/// The unit that would read as zero is dropped rather than shown as `0`, so a
/// fresh command reads `0s` and not `0h 0m 0s` — a row this size has no room
/// for a duration that spends most of its budget saying nothing happened yet.
fn short_duration(duration: Duration) -> String {
    let total = duration.as_secs();
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        if s > 0 { format!("{m}m {s}s") } else { format!("{m}m") }
    } else {
        format!("{s}s")
    }
}

/// How an ENDED command is named in [`line`], when it wanted attention.
///
/// A signal takes precedence over a code: a process killed by one usually has
/// no meaningful exit code at all, and the signal is the more honest answer
/// to "what happened".
fn exit_detail(exit: &Exit) -> String {
    match (exit.signal, exit.code) {
        (Some(signal), _) => format!("signal {signal}"),
        (None, Some(code)) => format!("exit {code}"),
        (None, None) => "exit".to_string(),
    }
}

/// The state, and nothing else: `working` / `blocked` / `done` / `failed` for
/// an agent, `running` / `ok` / `failed` for a command — one character, for a
/// watch complication or an Island's minimal presentation.
pub fn glyph(subject: &Subject) -> char {
    match subject {
        Subject::Agent { state, .. } => match state {
            AgentState::Working => '●',
            AgentState::Blocked => '?',
            AgentState::Done => '✓',
            // The same mark a failed command already gets, for the same
            // reading: something ended and it ended badly. A second failure
            // glyph would mean two vocabularies for one column.
            AgentState::Failed => '✗',
            AgentState::Idle => '·',
        },
        Subject::Command { exit, .. } => match exit {
            None => '●',
            Some(e) if exit_wants_attention(e.code, e.signal) => '✗',
            Some(_) => '✓',
        },
    }
}

/// The state plus just enough to say whose, cut to [`HEADLINE_WIDTH`]: an
/// Island's compact presentation, a notification's title.
///
/// Never the question itself — sixteen characters cannot hold `Run: cargo
/// test with the release flag?`, only that there is one. [`line`] is where
/// the question lives.
pub fn headline(subject: &Subject) -> String {
    let raw = match subject {
        Subject::Agent { name, state, turn_elapsed, .. } => match state {
            AgentState::Blocked => format!("{name} needs you"),
            AgentState::Done => format!("{name} done"),
            // The word a failed command already uses, so one headline reads
            // the same whether the pane held an agent or a `cargo test`.
            AgentState::Failed => format!("{name} failed"),
            AgentState::Idle => format!("{name} idle"),
            AgentState::Working => match turn_elapsed {
                Some(d) => format!("{name} {}", short_duration(*d)),
                None => format!("{name} working"),
            },
        },
        Subject::Command { command, exit, .. } => match exit {
            None => format!("{command} running"),
            Some(e) if exit_wants_attention(e.code, e.signal) => format!("{command} failed"),
            Some(_) => format!("{command} ok"),
        },
    };
    clipped(&raw, HEADLINE_WIDTH)
}

/// The signal line, cut to [`WIDTH`]: a lock screen, a sidebar row.
///
/// The single most valuable string on any compact surface, and strictly
/// prioritized — first match wins:
///
/// 1. **The question**, when blocked. It outranks everything, including a task
///    count: a fleet where one agent needs an answer and the rest are churning
///    is precisely the case this product exists for, and a row that answered
///    `3/7` while an agent sat waiting would be spending its one line on the
///    thing nobody has to act on.
/// 2. **Where the agent is** — its plan position, or what it is doing. See
///    [`signal`], which decides between those two.
///
/// A non-agent command falls back to the outcome of the last command, rather
/// than repeating `headline` verbatim, so a client that only has room for one
/// rung still learns something `headline` did not have the width to say.
pub fn line(subject: &Subject) -> String {
    let raw = match subject {
        Subject::Agent { name, state, question, signal, .. } => match state {
            AgentState::Blocked => {
                question.clone().unwrap_or_else(|| format!("{name} needs you"))
            }
            _ => signal.clone().unwrap_or_else(|| headline(subject)),
        },
        Subject::Command { command, exit, state_age } => {
            let status = match exit {
                None => "running".to_string(),
                Some(e) if exit_wants_attention(e.code, e.signal) => exit_detail(e),
                Some(_) => "ok".to_string(),
            };
            format!("{command} · {status} · {}", short_duration(*state_age))
        }
    };
    clipped(&raw, WIDTH)
}

/// How many seconds of "how long has this state been true" fit inside one
/// tier's share of [`rank`]'s range before the next tier's numbers begin.
///
/// Comfortably larger than any real state age — a pane stuck for a full year
/// (about 31.5 million seconds) is still deep inside it — and comfortably
/// smaller than `u32::MAX`, so four tiers fit with room to spare.
const TIER_SPAN: u32 = 100_000_000;

/// Which tier a subject sorts into, smallest first: blocked (or a command
/// that has failed) outranks done (or a failed command — see below) outranks
/// working (or running) outranks idle (or a command that exited clean).
///
/// A failed command shares `Done`'s tier rather than getting the top one:
/// both are "already happened and unseen", the fact `wants_attention`
/// already treats the same way, and neither is an open question the way a
/// permission prompt is — only `Blocked` gets to outrank everything.
fn tier(subject: &Subject) -> u32 {
    match subject {
        Subject::Agent { state, .. } => match state {
            AgentState::Blocked => 0,
            // A failed turn shares `Done`'s tier for the reason a failed
            // command already does: both are "already happened and unseen",
            // and neither is an open question the way a permission prompt is.
            // Within the tier the older one sorts first, which is the same
            // answer for a failure as for a success — the news that has been
            // waiting longest is the news costing the most.
            AgentState::Done | AgentState::Failed => 1,
            AgentState::Working => 2,
            AgentState::Idle => 3,
        },
        Subject::Command { exit, .. } => match exit {
            Some(e) if exit_wants_attention(e.code, e.signal) => 1,
            None => 2,
            Some(_) => 3,
        },
    }
}

fn state_age(subject: &Subject) -> Duration {
    match subject {
        Subject::Agent { state_age, .. } | Subject::Command { state_age, .. } => *state_age,
    }
}

/// Where this subject sorts in a fleet view. SMALLER sorts FIRST.
///
/// Blocked outranks done outranks working, and within a tier the OLDEST
/// first — the agent that has been stuck longest is the one costing the
/// most. Computed here, beside the activity, so an Island showing one agent
/// and a sidebar showing twelve cannot disagree about which one that is.
///
/// The tier dominates (each is a whole [`TIER_SPAN`] apart) and the age
/// breaks ties within it: age is subtracted from the tier's span so that a
/// LARGER age — an older, more-stuck state — produces a SMALLER number,
/// which is what sorts it first.
pub fn rank(subject: &Subject) -> u32 {
    let age_secs = state_age(subject).as_secs().min(u64::from(TIER_SPAN) - 1) as u32;
    tier(subject) * TIER_SPAN + (TIER_SPAN - 1 - age_secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_log::{claude, cursor, TurnEvent};

    const CURSOR_TURN: &str = include_str!("../fixtures/session-logs/cursor-complete-turn.jsonl");
    const CLAUDE_TURN: &str = include_str!("../fixtures/session-logs/claude-complete-turn.jsonl");

    /// The transcript a fixture's lines produce: what the agent SAID, in order.
    ///
    /// Only `Said`. A tool call is not transcript any more — it is the signal
    /// line's fallback, which is what `Phrase::action` builds. The routing of
    /// a conclusion is the daemon's, and this mirrors it deliberately: the
    /// fixtures are what prove the two ends meet.
    fn feed_of(fixture: &str, parse: fn(&str) -> Vec<TurnEvent>) -> Feed {
        let mut feed = Feed::default();
        for event in fixture.lines().flat_map(parse) {
            match event {
                TurnEvent::Said { text, conclusion: true } => feed.conclude(&text),
                TurnEvent::Said { text, conclusion: false } => feed.push(&text),
                _ => {}
            }
        }
        feed
    }

    #[test]
    fn three_steps_in_three_steps_out() {
        let mut feed = Feed::default();
        feed.push("Reading watch.rs.");
        feed.push("Editing feed.rs.");
        feed.push("Both tests pass.");
        assert_eq!(
            feed.lines(),
            vec!["Reading watch.rs.", "Editing feed.rs.", "Both tests pass."]
        );
    }

    #[test]
    fn the_fourth_step_evicts_the_first() {
        let mut feed = Feed::default();
        for step in ["one", "two", "three", "four"] {
            feed.push(step);
        }
        assert_eq!(feed.lines(), vec!["two", "three", "four"]);
        assert_eq!(feed.steps().len(), CAPACITY, "a feed never grows past its capacity");
    }

    /// The daemon decides where every line breaks, once, for every client.
    ///
    /// A token with no space in it — a hash, a URL, a base64 blob — is broken
    /// rather than left to overhang a row that is forty characters wide.
    #[test]
    fn a_word_too_long_for_a_row_is_broken_rather_than_left_to_overhang() {
        let mut feed = Feed::default();
        feed.push(&"x".repeat(200));
        let lines = feed.lines();
        assert_eq!(lines.len(), CAPACITY, "a window is a window: {lines:?}");
        for line in &lines {
            assert_eq!(line.chars().count(), WIDTH, "a line wider than the row: {line}");
        }
    }

    /// The window is a window: text arrives, and what does not fit scrolls off
    /// the top.
    ///
    /// This is the complaint that named the stage, stated as an assertion. The
    /// old feed kept the last three MESSAGES and cut each to forty characters,
    /// so one long sentence read as one fragment with two blank rows under it.
    #[test]
    fn one_long_sentence_fills_the_window_and_scrolls_it() {
        let mut feed = Feed::default();
        feed.push(
            "I'm going to read the watcher first, then the parser it calls, \
             and only then change anything, because the fold is the part that \
             has been wrong.",
        );
        assert_eq!(
            feed.lines(),
            vec![
                "then the parser it calls, and only then",
                "change anything, because the fold is the",
                "part that has been wrong.",
            ],
            "the tail of the sentence, wrapped, with the start of it pushed off the top"
        );
    }

    /// Narration is watched at its tail; an answer is read from its head.
    ///
    /// The only thing the narration/conclusion distinction is kept for, and
    /// the reason it survived stage 4 rather than being erased with the gate
    /// that used to drop narration entirely.
    #[test]
    fn an_answer_enters_from_its_start_and_narration_from_its_end() {
        let long = "Alpha bravo charlie delta echo foxtrot golf hotel india \
                    juliet kilo lima mike november oscar papa quebec romeo \
                    sierra tango";

        let mut narrating = Feed::default();
        narrating.push(long);
        assert!(narrating.lines()[2].ends_with("tango"), "{:?}", narrating.lines());

        let mut concluding = Feed::default();
        concluding.conclude(long);
        assert!(concluding.lines()[0].starts_with("Alpha bravo"), "{:?}", concluding.lines());
        assert!(
            concluding.lines()[2].ends_with('…'),
            "an answer cut short says so: {:?}",
            concluding.lines()
        );
    }

    /// A finished agent reads as what it did, which was the explicit ask.
    ///
    /// The narration it wrote getting there stays above the answer rather than
    /// being cleared by it: it reads as a small transcript, and a row that
    /// shrank from three lines to one the moment a turn ended would rearrange
    /// the sidebar under whoever was reading it.
    #[test]
    fn a_short_answer_keeps_the_narration_above_it() {
        let mut feed = Feed::default();
        feed.push("Reading the watcher.");
        feed.push("Running the tests.");
        feed.conclude("Both pass.");
        assert_eq!(
            feed.lines(),
            vec!["Reading the watcher.", "Running the tests.", "Both pass."]
        );
    }

    /// However much prose an agent writes, a `Feed` is three short strings.
    ///
    /// It runs per pane on a loop, and an agent can produce a great deal of
    /// text in one message; nothing here may grow with it. See `WINDOW`.
    #[test]
    fn a_very_long_message_does_not_grow_the_window() {
        let mut feed = Feed::default();
        feed.push(&"lorem ipsum dolor sit amet ".repeat(4_000));
        feed.conclude(&"consectetur adipiscing elit ".repeat(4_000));
        let lines = feed.lines();
        assert_eq!(lines.len(), CAPACITY);
        for line in &lines {
            assert!(line.chars().count() <= WIDTH, "a line wider than the row: {line}");
        }
    }

    #[test]
    fn a_step_that_fits_is_left_exactly_as_it_is() {
        let mut feed = Feed::default();
        feed.push("Written to haiku.txt.");
        assert_eq!(feed.lines(), vec!["Written to haiku.txt."]);
    }

    /// The complaint this whole stage exists for, pinned: no verb, ever.
    ///
    /// A transcript line is the agent's sentence and nothing else. `says` in
    /// front of it was a parser's vocabulary on a lock screen, and it is the
    /// half of the complaint that no amount of task folding would have fixed.
    #[test]
    fn a_transcript_line_carries_no_verb_at_all() {
        let mut feed = Feed::default();
        feed.push("Done. `fruit.txt` now contains banana.");
        assert_eq!(feed.lines(), vec!["Done. `fruit.txt` now contains banana."]);
        assert!(!feed.lines()[0].starts_with("says"), "the bridge is gone");
    }

    /// A planted credential in a TOOL ARGUMENT must not reach the action line.
    ///
    /// Stage 1 learned this on the blocked question; a tool argument is the
    /// same class of string and travels the same way — through the relay, onto
    /// a lock screen. The token is planted where a real one lives, in a `curl`
    /// command claude wrote down, and read back out through the real parser
    /// rather than passed by hand.
    ///
    /// It moved from the feed to `Phrase::action` with this stage, and the test
    /// moved with it: the string is the same string and it is bound for the
    /// same lock screen, so the guarantee had to follow it rather than stay
    /// behind guarding a path nothing takes.
    ///
    /// Both assertions are load-bearing. The `contains` check alone could pass
    /// on a line that merely truncated the token away, so the exact line is
    /// pinned too: with no redaction the same input yields
    /// `Running curl -H "Authorization: Bearer …`, which fails both.
    #[test]
    fn a_bearer_token_planted_in_a_tool_argument_does_not_reach_a_row() {
        let planted = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"curl -H \"Authorization: Bearer sk-live-9f8e7d6c\""}}]}}"#;
        let mut lines = Vec::new();
        for event in claude::parse_line(planted) {
            let TurnEvent::Did { verb, object } = event else { continue };
            assert!(object.contains("sk-live-9f8e7d6c"), "the fixture must really carry the secret");
            lines.push(Phrase::action(&verb, &object));
        }
        let line = lines[0].as_str();
        assert!(!line.contains("sk-"), "a live credential reached a row: {line}");
        assert_eq!(line, "Running curl -H \"Authorization: Bearer …");
    }

    /// The secret is stripped, not merely hidden by the truncation.
    #[test]
    fn a_secret_short_enough_to_fit_is_still_redacted() {
        assert_eq!(Phrase::action("bash", "TOKEN=abc123 ./run.sh").as_str(), "Running TOKEN=… ./run.sh");
    }

    /// An agent-authored phrase is credential-bearing text too.
    ///
    /// A task's `activeForm` and a subagent's description are written by the
    /// agent, in a string it chose, and they reach the same lock screen the
    /// feed does. There is one cleaning path and everything user-visible takes
    /// it — this is that claim, checked on the two types that are new.
    #[test]
    fn an_agent_authored_phrase_is_cleaned_exactly_as_a_step_is() {
        assert_eq!(Phrase::new("Deploying with TOKEN=sk-live-9f8e7d6c").as_str(), "Deploying with TOKEN=…");
        assert_eq!(Phrase::new("Auditing /Users/someone/Dev/secret/rules.rs").as_str(), "Auditing rules.rs");
    }

    /// An agent's prose is several lines; a row is one.
    #[test]
    fn a_step_spanning_lines_becomes_one_line() {
        let mut feed = Feed::default();
        feed.push("Done.\nThe file now\treads correctly.");
        assert_eq!(feed.lines(), vec!["Done. The file now reads correctly."]);
    }

    /// Claude's `step_object` yields nothing for a tool naming no file,
    /// command, pattern or description — the line is then the verb alone,
    /// with no stray space on either end.
    #[test]
    fn an_action_with_no_object_is_just_its_verb() {
        assert_eq!(Phrase::action("task", "").as_str(), "Delegating");
    }

    // ---------------------------------------------------------------------
    // A tool name, said in English
    // ---------------------------------------------------------------------

    /// The tools that actually turn up read as English, and the ones nobody
    /// predicted still read as something.
    ///
    /// The second half is the point. A translation table for every tool claude
    /// might ever ship is a table that is wrong the first time it ships one
    /// more, so the fallback is total: an unknown name comes back capitalized,
    /// with an MCP tool's separators as spaces, rather than falling off a match
    /// arm into a blank row.
    #[test]
    fn a_tool_name_is_said_in_english_and_an_unknown_one_still_reads() {
        for (verb, object, expected) in [
            ("write", "fruit.txt", "Writing fruit.txt"),
            ("bash", "cargo test", "Running cargo test"),
            ("grep", "TODO", "Searching TODO"),
            ("agent", "Audit the redaction rules", "Delegating Audit the redaction rules"),
            // The two names the complaint named, gone.
            ("taskupdate", "", "Planning"),
            ("says", "", "Says"),
            // Never seen before, and still legible.
            ("mcp__linear__create_issue", "", "Mcp linear create issue"),
            ("frobnicate", "widget.rs", "Frobnicate widget.rs"),
        ] {
            assert_eq!(Phrase::action(verb, object).as_str(), expected, "from {verb:?}");
        }
    }

    #[test]
    fn a_feed_nothing_has_happened_in_is_empty() {
        assert!(Feed::default().is_empty());
        assert!(Feed::default().lines().is_empty());
    }

    #[test]
    fn a_non_ascii_object_is_cut_by_characters_and_never_mid_character() {
        let line = Phrase::action("write", &"パ".repeat(100));
        assert_eq!(line.as_str().chars().count(), WIDTH);
        assert!(line.as_str().ends_with('…'));
    }

    /// The whole point, on real recorded lines: a cursor transcript, in the
    /// order it happened, as a person would read it — and with the tool call
    /// gone from it, because a transcript is what the agent SENT.
    #[test]
    fn a_real_transcript_is_what_the_agent_said_and_nothing_else() {
        let feed = feed_of(CURSOR_TURN, cursor::parse_line);
        assert_eq!(
            feed.lines(),
            vec!["Running that command now.", "Done. `fruit.txt` now contains `banana`."]
        );
        assert!(
            !feed.lines().iter().any(|line| line.contains("shell")),
            "the shell call is the signal line's business, not the transcript's: {:?}",
            feed.lines()
        );
    }

    /// Claude's fixture carries one tool call and one closing answer. Only the
    /// answer is transcript; the call becomes the action line beside it.
    ///
    /// Three lines of it, from the start, where the old feed showed the first
    /// forty characters and stopped — `Written to `haiku.txt`. **"esc to
    /// inter…`, which cuts off inside the word the sentence is about. Same
    /// answer, three times the sentence.
    #[test]
    fn a_real_claude_turn_says_what_it_concluded() {
        let feed = feed_of(CLAUDE_TURN, claude::parse_line);
        assert_eq!(
            feed.lines(),
            vec![
                "Written to `haiku.txt`. **\"esc to",
                "interrupt\"** is a hint line a terminal",
                "UI shows while busy; pressing Escape…",
            ]
        );
    }

    // ---------------------------------------------------------------------
    // Paths are narrowed here, for the same reason they are redacted here.
    // ---------------------------------------------------------------------

    /// The finding this narrowing exists for, read back through the real
    /// parser: an absolute path INSIDE a shell command, at a client scope that
    /// the wire's own rule says may never learn one.
    ///
    /// Both assertions matter. Without narrowing this line is
    /// `bash cd /Users/someone/Dev/acme-client-c…` — which contains the home
    /// directory, the user's name and the client's name, and fails the first
    /// assertion outright.
    #[test]
    fn an_absolute_path_inside_a_command_does_not_reach_a_row() {
        let planted = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"cd /Users/someone/Dev/acme-client-confidential && cargo test"}}]}}"#;
        let mut lines = Vec::new();
        for event in claude::parse_line(planted) {
            let TurnEvent::Did { verb, object } = event else { continue };
            assert!(object.contains("/Users/someone"), "the fixture must really carry the path");
            lines.push(Phrase::action(&verb, &object));
        }
        let line = lines[0].as_str();
        assert!(!line.contains("/Users/"), "a filesystem path reached a row: {line}");
        assert_eq!(line, "Running cd acme-client-confidential &&…");
    }

    /// A path is narrowed to the same thing claude's `file_path` already gets:
    /// the last segment. Privacy and legibility want the identical answer.
    #[test]
    fn a_path_becomes_its_last_segment() {
        for (raw, expected) in [
            ("/Users/someone/Dev/project/src/feed.rs", "feed.rs"),
            ("~/Dev/project/notes.md", "notes.md"),
            // A directory's trailing slash is not a segment of its own.
            ("ls /Users/someone/Dev/project/", "ls project"),
            // Quoting is the shell's, not the path's.
            ("cat \"/Users/someone/notes.txt\"", "cat notes.txt"),
            // A path with a SPACE in it arrives as two tokens, so the closing
            // quote survives on the second one. Known and left: the directory
            // layout is still gone, which is the part that had to go, and
            // reassembling shell quoting to save one character is a parser
            // this module has no business growing.
            ("cat \"/Users/someone/My Notes.txt\"", "cat My Notes.txt\""),
            // The flag is the flag; only its value is a path.
            ("cargo --manifest-path=/Users/someone/x/Cargo.toml build", "cargo --manifest-path=Cargo.toml build"),
        ] {
            let mut feed = Feed::default();
            feed.push(raw);
            assert_eq!(feed.lines()[0], expected, "from {raw:?}");
        }
    }

    /// The cost of a false positive is a line that stops making sense, so the
    /// shapes that merely resemble a path are pinned as untouched. A message
    /// with no path in it at all must come back byte for byte.
    #[test]
    fn ordinary_prose_flags_and_urls_are_left_exactly_as_they_are() {
        for raw in [
            "Done. Both tests pass.",
            "cargo test -p farcooler-core",
            "https://example.com/a/b/c",
            "Use the read/write lock.",
            // Relative paths say nothing about where this machine keeps things.
            "src/session_log/claude.rs",
            // One segment is not a layout, and `tmp` reads worse than `/tmp`.
            "ls /tmp",
            "cd ~/Dev",
        ] {
            let mut feed = Feed::default();
            feed.push(raw);
            assert_eq!(feed.lines()[0], raw, "a harmless step was narrowed");
        }
    }

    /// Narrowing runs AFTER redaction, so a line carrying both a path and a
    /// credential loses both rather than whichever the first pass reached.
    #[test]
    fn a_path_and_a_secret_in_one_command_both_go() {
        assert_eq!(
            Phrase::action("bash", "TOKEN=sk-live-9f8e7d /Users/someone/Dev/secret/deploy.sh").as_str(),
            "Running TOKEN=… deploy.sh"
        );
    }

    // ---------------------------------------------------------------------
    // The ladder: glyph, headline, line, rank.
    // ---------------------------------------------------------------------

    fn blocked_agent(name: &str, stuck_for: Duration, question: Option<&str>) -> Subject {
        Subject::Agent {
            name: name.to_string(),
            state: AgentState::Blocked,
            turn_elapsed: Some(stuck_for),
            state_age: stuck_for,
            question: question.map(str::to_string),
            signal: None,
        }
    }

    fn working_agent(name: &str, turn_elapsed: Duration, signal: Option<&str>) -> Subject {
        Subject::Agent {
            name: name.to_string(),
            state: AgentState::Working,
            turn_elapsed: Some(turn_elapsed),
            state_age: turn_elapsed,
            question: None,
            signal: signal.map(str::to_string),
        }
    }

    fn done_agent(name: &str, since: Duration, signal: Option<&str>) -> Subject {
        Subject::Agent {
            name: name.to_string(),
            state: AgentState::Done,
            turn_elapsed: None,
            state_age: since,
            question: None,
            signal: signal.map(str::to_string),
        }
    }

    fn failed_agent(name: &str, since: Duration, signal: Option<&str>) -> Subject {
        Subject::Agent {
            name: name.to_string(),
            state: AgentState::Failed,
            turn_elapsed: None,
            state_age: since,
            question: None,
            signal: signal.map(str::to_string),
        }
    }

    fn idle_agent(name: &str) -> Subject {
        Subject::Agent {
            name: name.to_string(),
            state: AgentState::Idle,
            turn_elapsed: None,
            state_age: Duration::from_secs(3600),
            question: None,
            signal: None,
        }
    }

    fn plan(done: u32, total: u32, active_form: Option<&str>) -> Plan {
        Plan { done, total, active_form: active_form.map(Phrase::new) }
    }

    fn running_command(command: &str, running_for: Duration) -> Subject {
        Subject::Command { command: command.to_string(), exit: None, state_age: running_for }
    }

    fn failed_command(command: &str, code: Option<i32>, signal: Option<i32>, since: Duration) -> Subject {
        Subject::Command { command: command.to_string(), exit: Some(Exit { code, signal }), state_age: since }
    }

    fn ok_command(command: &str, since: Duration) -> Subject {
        Subject::Command {
            command: command.to_string(),
            exit: Some(Exit { code: Some(0), signal: None }),
            state_age: since,
        }
    }

    /// The worked examples from the design doc's own ladder table, verified
    /// literally rather than trusted by inspection.
    #[test]
    fn the_designs_own_worked_examples_come_out_exactly_as_written() {
        let blocked = blocked_agent("codex", Duration::from_secs(90), None);
        assert_eq!(headline(&blocked), "codex needs you");

        let working = working_agent("claude", Duration::from_secs(240), None);
        assert_eq!(headline(&working), "claude 4m");

        let failed = failed_command("cargo test", Some(101), None, Duration::from_secs(120));
        assert_eq!(headline(&failed), "cargo test failed");
        assert_eq!(line(&failed), "cargo test · exit 101 · 2m");
    }

    // -- Each rung narrows the one below it --

    /// A blocked agent: `line` is the question itself, `headline` compresses
    /// it to "needs you", `glyph` compresses that further to one character.
    /// Each rung is derivable from the one below — the property the whole
    /// ladder exists to guarantee.
    #[test]
    fn each_rung_narrows_the_one_below_it_when_blocked() {
        let s = blocked_agent("codex", Duration::from_secs(90), Some("Run: cargo test?"));
        let l = line(&s);
        let h = headline(&s);
        let g = glyph(&s);
        assert_eq!(l, "Run: cargo test?", "line carries the question itself");
        assert!(l.ends_with('?'), "a blocked line is a question: {l:?}");
        assert!(h.ends_with("needs you"), "headline compresses that question, it does not drop it: {h:?}");
        assert_eq!(g, '?', "glyph compresses headline's verdict further, to one character");
    }

    #[test]
    fn each_rung_narrows_the_one_below_it_when_working() {
        let s = working_agent("claude", Duration::from_secs(240), Some("Writing haiku.txt"));
        let l = line(&s);
        let h = headline(&s);
        let g = glyph(&s);
        assert_eq!(l, "Writing haiku.txt", "line is the signal line");
        assert!(!h.contains("needs you") && !h.contains("done"), "headline must not claim blocked or done: {h:?}");
        assert!(h.contains("4m"), "headline elaborates line with how long, not what: {h:?}");
        assert_eq!(g, '●', "glyph agrees this is in-progress, not blocked or done");
    }

    #[test]
    fn each_rung_narrows_the_one_below_it_when_done() {
        let s = done_agent("claude", Duration::from_secs(30), Some("Writing haiku.txt"));
        let l = line(&s);
        let h = headline(&s);
        let g = glyph(&s);
        assert_eq!(l, "Writing haiku.txt", "a finished agent's line is the last thing it did");
        assert!(h.ends_with("done"), "headline: {h:?}");
        assert_eq!(g, '✓');
    }

    /// A turn that DIED must not read like one that succeeded.
    ///
    /// The finding this variant exists for: a cursor turn that ended on
    /// `status: "error"` folded to the same `✓ done` a clean turn does, so a
    /// fleet glanced at from a lock screen said every pane was fine. The
    /// failure mark is the one a failed command already uses, so a reader
    /// learns one vocabulary rather than two.
    #[test]
    fn each_rung_narrows_the_one_below_it_for_a_failed_turn() {
        let s = failed_agent("cursor", Duration::from_secs(30), Some("Running that command"));
        assert_eq!(line(&s), "Running that command", "the last thing it did before it died");
        assert_eq!(headline(&s), "cursor failed");
        assert_eq!(glyph(&s), '✗');

        // Nothing in the feed — the real fixture's failing turn wrote no step
        // at all — so the line falls back to the headline rather than to
        // silence, and still says the turn failed.
        let bare = failed_agent("cursor", Duration::from_secs(30), None);
        assert_eq!(line(&bare), "cursor failed");

        // And it is not simply "done" wearing a different word: the glyph a
        // clean turn gets is the one this must never be.
        assert_ne!(glyph(&s), glyph(&done_agent("cursor", Duration::from_secs(30), None)));
    }

    /// A failed turn is news of the same kind a failed command is, so the two
    /// sort together — and a permission prompt still beats both.
    #[test]
    fn a_failed_turn_ranks_with_done_and_below_blocked() {
        let age = Duration::from_secs(60);
        assert_eq!(rank(&failed_agent("cursor", age, None)), rank(&done_agent("claude", age, None)));
        assert!(rank(&blocked_agent("codex", age, Some("proceed?"))) < rank(&failed_agent("cursor", age, None)));
        assert!(rank(&failed_agent("cursor", age, None)) < rank(&working_agent("claude", age, None)));
    }

    #[test]
    fn each_rung_narrows_the_one_below_it_for_a_failed_command() {
        let s = failed_command("cargo test", Some(101), None, Duration::from_secs(132));
        let l = line(&s);
        let h = headline(&s);
        let g = glyph(&s);
        assert_eq!(l, "cargo test · exit 101 · 2m 12s");
        assert!(h.ends_with("failed"), "headline: {h:?}");
        assert_eq!(g, '✗');
    }

    #[test]
    fn each_rung_narrows_the_one_below_it_for_a_running_command() {
        let s = running_command("pnpm dev", Duration::from_secs(5));
        let l = line(&s);
        let h = headline(&s);
        let g = glyph(&s);
        assert_eq!(l, "pnpm dev · running · 5s");
        assert!(h.ends_with("running"), "headline: {h:?}");
        assert_eq!(g, '●');
    }

    #[test]
    fn each_rung_narrows_the_one_below_it_for_an_ok_command() {
        let s = ok_command("git status", Duration::from_secs(0));
        let l = line(&s);
        let h = headline(&s);
        let g = glyph(&s);
        assert_eq!(l, "git status · ok · 0s");
        assert!(h.ends_with("ok"), "headline: {h:?}");
        assert_eq!(g, '✓');
    }

    // -- Rungs stay within budget --

    #[test]
    fn headline_never_exceeds_its_width_even_for_a_long_name() {
        let s = blocked_agent(&"x".repeat(50), Duration::from_secs(1), None);
        assert!(headline(&s).chars().count() <= HEADLINE_WIDTH, "{}", headline(&s));
    }

    #[test]
    fn line_never_exceeds_its_width_even_for_a_long_question() {
        let s = blocked_agent("codex", Duration::from_secs(1), Some(&"is this really what you want to do? ".repeat(5)));
        assert!(line(&s).chars().count() <= WIDTH, "{}", line(&s));
    }

    // -- Ranking --

    #[test]
    fn blocked_outranks_done_outranks_working_outranks_idle() {
        let age = Duration::from_secs(60);
        let blocked = blocked_agent("codex", age, Some("proceed?"));
        let done = done_agent("codex", age, None);
        let working = working_agent("codex", age, None);
        let idle = idle_agent("codex");
        assert!(rank(&blocked) < rank(&done), "blocked must outrank done");
        assert!(rank(&done) < rank(&working), "done must outrank working");
        assert!(rank(&working) < rank(&idle), "working must outrank idle");
    }

    /// The property the ranking exists for: not just "blocked wins", but
    /// "the blocked agent that has cost the most time wins first".
    #[test]
    fn within_blocked_the_oldest_ranks_first() {
        let stuck_ten_minutes = blocked_agent("codex", Duration::from_secs(600), Some("proceed?"));
        let stuck_ten_seconds = blocked_agent("claude", Duration::from_secs(10), Some("proceed?"));
        assert!(
            rank(&stuck_ten_minutes) < rank(&stuck_ten_seconds),
            "the agent stuck longest must sort first, regardless of name"
        );
    }

    #[test]
    fn a_failed_command_fills_the_same_rungs_an_agent_does() {
        // A fleet is not only agents: a build that failed overnight is
        // exactly as worth seeing first as an agent waiting on a question is
        // worth seeing second, and it must fill every rung from facts a
        // non-agent pane actually has -- argv and an exit code, not a
        // session log it does not keep.
        let failed = failed_command("cargo build", Some(101), None, Duration::from_secs(60));
        let ok = ok_command("git status", Duration::from_secs(60));
        let running = running_command("pnpm dev", Duration::from_secs(60));
        assert!(rank(&failed) < rank(&running), "a failed command must outrank one still running");
        assert!(rank(&running) < rank(&ok), "a running command must outrank one that already finished clean");
        assert_eq!(glyph(&failed), '✗');
        assert_eq!(glyph(&ok), '✓');
        assert_eq!(glyph(&running), '●');
    }

    /// A failed command and a finished-unseen agent are the same kind of
    /// news -- "something happened while you were away" -- so at equal age
    /// they must sort together rather than one always beating the other.
    #[test]
    fn a_failed_command_and_a_finished_agent_share_a_tier() {
        let age = Duration::from_secs(60);
        let failed = failed_command("cargo test", Some(101), None, age);
        let done = done_agent("claude", age, None);
        assert_eq!(rank(&failed), rank(&done));
    }

    /// Printed literally so a reader can judge legibility directly rather
    /// than trust a passing assertion — see the task's "verify by running".
    #[test]
    fn print_all_four_rungs_for_representative_states() {
        let cases: Vec<(&str, Subject)> = vec![
            (
                "a blocked agent",
                blocked_agent("codex", Duration::from_secs(11 * 60), Some("Run: cargo test --release?")),
            ),
            (
                "a working agent on a task list",
                working_agent(
                    "overnight-fix",
                    Duration::from_secs(240),
                    signal(Some(&plan(3, 7, Some("Designing test matrix"))), 2, None).as_deref(),
                ),
            ),
            (
                "a working agent with no task list",
                working_agent("claude", Duration::from_secs(240), Some("Writing fruit.txt")),
            ),
            (
                "a finished agent",
                done_agent("cursor", Duration::from_secs(30), Some("Writing fruit.txt")),
            ),
            ("a failed non-agent command", failed_command("cargo test", Some(101), None, Duration::from_secs(132))),
        ];
        for (label, subject) in &cases {
            println!(
                "{label:32} glyph={:?}  headline={:<18?}  line={:?}  rank={}",
                glyph(subject),
                headline(subject),
                line(subject),
                rank(subject)
            );
        }
        // A regression here is a row that stopped being legible, not just a
        // string that changed -- so the literal text is pinned, not merely
        // printed.
        assert_eq!(headline(&cases[0].1), "codex needs you");
        assert_eq!(line(&cases[0].1), "Run: cargo test --release?");
        assert_eq!(headline(&cases[1].1), "overnight-fix 4m");
        assert_eq!(line(&cases[1].1), "3/7 · Designing test matrix · 2 agents");
        assert_eq!(headline(&cases[2].1), "claude 4m");
        assert_eq!(line(&cases[2].1), "Writing fruit.txt");
        assert_eq!(headline(&cases[3].1), "cursor done");
        assert_eq!(line(&cases[3].1), "Writing fruit.txt");
        assert_eq!(headline(&cases[4].1), "cargo test failed");
        assert_eq!(line(&cases[4].1), "cargo test · exit 101 · 2m 12s");
    }

    // ---------------------------------------------------------------------
    // The signal line: what the agent is, in one line, in priority order
    // ---------------------------------------------------------------------

    /// The layout the user chose, built from the parts a real log carries.
    #[test]
    fn the_signal_line_reads_as_the_layout_asked_for() {
        assert_eq!(
            signal(Some(&plan(3, 7, Some("Designing test matrix"))), 2, None).as_deref(),
            Some("3/7 · Designing test matrix · 2 agents")
        );
        // No subagents: the suffix is absent, not "0 agents".
        assert_eq!(
            signal(Some(&plan(3, 7, Some("Designing test matrix"))), 0, None).as_deref(),
            Some("3/7 · Designing test matrix")
        );
        // One reads as one. A row that says "1 agents" is a row nobody
        // proofread.
        assert_eq!(signal(Some(&plan(1, 2, None)), 1, None).as_deref(), Some("1/2 · 1 agent"));
    }

    /// A pane with no task list — codex, cursor, most claude sessions — falls
    /// to what it is doing, and that is not a gap being papered over.
    #[test]
    fn an_agent_with_no_task_list_reports_what_it_is_doing() {
        let action = Phrase::action("write", "fruit.txt");
        assert_eq!(signal(None, 0, Some(&action)).as_deref(), Some("Writing fruit.txt"));
        // An empty plan is not a plan. `0/0` would be a progress indicator
        // reporting that no progress is possible.
        assert_eq!(signal(Some(&plan(0, 0, None)), 0, Some(&action)).as_deref(), Some("Writing fruit.txt"));
        // Nothing at all yet: `None`, which `line` renders as the headline
        // rather than as a blank row.
        assert_eq!(signal(None, 0, None), None);
    }

    /// The same tool call, before and after the turn that made it ended.
    ///
    /// A finished agent keeps its last action on purpose — it is what a
    /// finished row is read for — but it used to keep it in the present
    /// tense, so a pane that stopped working half an hour ago went on
    /// claiming `Running cd …`. The fact is right; only the tense was wrong.
    #[test]
    fn a_finished_turns_action_reads_as_something_that_already_happened() {
        assert_eq!(Phrase::action("bash", "cd acme && cargo test").as_str(), "Running cd acme && cargo test");
        assert_eq!(Phrase::action_done("bash", "cd acme && cargo test").as_str(), "Ran cd acme && cargo test");

        assert_eq!(Phrase::action_done("write", "fruit.txt").as_str(), "Wrote fruit.txt");
        assert_eq!(Phrase::action_done("edit", "main.rs").as_str(), "Edited main.rs");
        assert_eq!(Phrase::action_done("grep", "needle").as_str(), "Searched needle");
        assert_eq!(Phrase::action_done("task", "the subagent").as_str(), "Delegated the subagent");

        // A tool nobody predicted has no verb to put in the past: the
        // fallback renders its NAME, and a name reads the same either way
        // rather than being mangled into a tense it never had.
        assert_eq!(
            Phrase::action_done("mcp__linear__create_issue", "FC-1").as_str(),
            Phrase::action("mcp__linear__create_issue", "FC-1").as_str()
        );
    }

    /// The join that can break, and the half that must not.
    ///
    /// The create-to-id join is by creation order, and the fold degrades to a
    /// count with no phrase rather than to a wrong phrase. So a plan with no
    /// `active_form` must still put its count on the row — losing the phrase
    /// costs context, and losing the count costs the whole feature.
    #[test]
    fn a_plan_whose_phrase_went_missing_still_reports_its_count() {
        assert_eq!(signal(Some(&plan(3, 7, None)), 0, None).as_deref(), Some("3/7"));
        assert_eq!(signal(Some(&plan(3, 7, Some("   "))), 2, None).as_deref(), Some("3/7 · 2 agents"));
    }

    /// The question outranks the count, and this is the priority stated as a
    /// test rather than as a comment.
    ///
    /// A blocked agent with a full task list in flight is exactly the case
    /// where the wrong choice is tempting: there is more to say than fits, and
    /// only one of those things needs a person.
    #[test]
    fn a_blocked_agent_shows_its_question_and_not_its_task_count() {
        let asking = Subject::Agent {
            name: "overnight-fix".to_string(),
            state: AgentState::Blocked,
            turn_elapsed: Some(Duration::from_secs(120)),
            state_age: Duration::from_secs(20),
            question: Some("Do you want to create fruit.txt?".to_string()),
            signal: signal(Some(&plan(3, 7, Some("Designing test matrix"))), 2, None),
        };
        assert_eq!(line(&asking), "Do you want to create fruit.txt?");
        assert_eq!(headline(&asking), "overnight-fix nee…");
        assert_eq!(glyph(&asking), '?');
    }

    /// A signal line is cut here like every other line: a task phrase an agent
    /// wrote at length must not push a row wider than a row.
    #[test]
    fn a_long_signal_line_is_cut_to_the_rows_width() {
        let long = signal(Some(&plan(3, 7, Some(&"Designing an extremely elaborate test matrix".repeat(3)))), 2, None)
            .expect("a plan produces a line");
        // At most, not exactly: `clipped` trims the space a cut landed after,
        // so a line cut mid-word and one cut after a word differ by that space.
        assert!(long.chars().count() <= WIDTH, "{long}");
        assert!(long.ends_with('…'), "{long}");
    }
}
