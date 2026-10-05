//! The section of a pull request's description that Far Cooler writes
//! (ov-314, ov-305 section 4.3): rendered from a card, and merged into a
//! description without touching a word a reviewer wrote.
//!
//! The pure half of `plan lane pr-body`, a child of `plan_pr_body.rs`: no I/O,
//! so a golden file can pin every line of it.
//!
//! ```text
//! <!-- farcooler:begin -->
//! ## What and why          the card's intent, and its theme
//! ## Acceptance            the card's lines, ticked as they are
//! ## Agent review          the latest finding's headline, and what was not checked
//! ## Decided for you      rulings on the lane's cards, each reversible by a reply
//! ## Captures              links the card's notes carry
//! ## Cost                 only when the workspace opted in
//! <!-- farcooler:end -->
//! ```
//!
//! **Three sections are always there** (what and why, acceptance, agent
//! review) so a reviewer finds the same shape every time. The other three
//! appear only when they have something to say: an empty "Decided for you"
//! is the absence of a call to push back on, and a heading with nothing under
//! it is noise.
//!
//! **Where the words come from.** The card's `intent` and `acceptance`, as
//! they stand. The agent review is the card's `finding` notes, which the
//! orchestrator writes as a bold headline then detail: the latest one's
//! headline is the summary, the count is the rounds, and any line that says
//! `Not checked:` is carried over. A note a later note superseded is history
//! and is left out, as it is everywhere on the board. Captures are links:
//! markdown images and bare image or video URLs in the card's notes, never an
//! upload (GitHub has no API for attachments; the design keeps them on an
//! orphan branch and links them by URL).
//!
//! **Text from the card is made safe for a public page.** Agents write it, a
//! pull request publishes it: a marker string in it would break the next
//! merge, an `@name` would ping a person, and `Closes #12` would close an issue
//! when the pull request merges. `neutralize` puts a zero-width joiner
//! (U+200D, invisible) inside each: after the `@`, after the closing keyword,
//! and inside `<!--`. Backticks would show.
//!
//! **Merging.** Everything outside the markers is the reviewers', byte for
//! byte, line endings included. With both markers the span between them is
//! replaced; with neither the section is appended; with anything else
//! (a lone marker, a second pair) it refuses; a marker inside a fenced code
//! block is a quotation, not a marker, and a description whose only markers
//! are quotations is refused too, since appending would put the new section
//! inside the unclosed block and grow it on every run, because guessing which text is
//! Far Cooler's is how a reviewer's words get overwritten.

use farcooler_core::usage_words::{NOT_REPORTED, tokens};
use farcooler_protocol::v1::{self as pb};

pub const BEGIN: &str = "<!-- farcooler:begin -->";
pub const END: &str = "<!-- farcooler:end -->";

/// Everything the section is drawn from.
pub struct Input<'a> {
    pub detail: &'a pb::TaskDetail,
    pub lane: &'a pb::Lane,
    /// The theme the card is in, by name.
    pub theme: Option<&'a str>,
    /// Rulings on any of the lane's cards, in the order the board lists them.
    pub rulings: Vec<&'a pb::BoardRuling>,
    /// Whether the workspace opted in to a cost line (`pr_cost_line`).
    pub cost_line: bool,
}

const ZWJ: char = '\u{200d}';

/// The words GitHub reads as closing an issue when a pull request merges.
const CLOSING: [&str; 9] = ["close", "closes", "closed", "fix", "fixes", "fixed", "resolve", "resolves", "resolved"];

/// Whether what follows a closing keyword names an issue: `#12`, a
/// `owner/repo#12`, or an issue URL.
fn names_an_issue(rest: &str) -> bool {
    let rest = rest.strip_prefix(':').unwrap_or(rest).trim_start();
    let token = rest.split_whitespace().next().unwrap_or("");
    // A reference in backticks is code, and closes nothing.
    if token.starts_with('`') {
        return false;
    }
    token.starts_with("http") || token.split_once('#').is_some_and(|(_, n)| n.starts_with(|c: char| c.is_ascii_digit()))
}

/// `text` with nothing in it that GitHub would act on: see the module's note.
///
/// Left alone: anything inside a backtick code span on its line (a code span
/// pings nobody, and `@MainActor` should copy cleanly) and any `http(s)://`
/// token (a link with an `@` in its path must stay a working link). A fence
/// opener at the start of a line is broken, so card text can't open a fenced
/// block that hides the markers after it.
pub fn neutralize(text: &str) -> String {
    let text = text.replace("<!--", &format!("<!{ZWJ}--"));
    text.split('\n').map(neutralize_line).collect::<Vec<_>>().join("\n")
}

fn neutralize_line(line: &str) -> String {
    let indent = line.len() - line.trim_start().len();
    let (lead, body) = line.split_at(indent);
    let opens_a_fence = body.starts_with("```") || body.starts_with("~~~");
    let mut out = String::from(lead);
    let chars: Vec<char> = body.chars().collect();
    let mut i = 0;
    if opens_a_fence {
        out.push(chars[0]);
        out.push(ZWJ);
        i = 1;
    }
    while i < chars.len() {
        let c = chars[i];
        let before_is_word = i > 0 && (chars[i - 1].is_alphanumeric() || matches!(chars[i - 1], '_' | '-' | '/'));
        if c == '`'
            && let Some(close) = (i + 1..chars.len()).find(|&j| chars[j] == '`')
        {
            out.extend(&chars[i..=close]);
            i = close + 1;
            continue;
        }
        if !before_is_word && (starts_with_at(&chars, i, "http://") || starts_with_at(&chars, i, "https://")) {
            let end = (i..chars.len()).find(|&j| chars[j].is_whitespace()).unwrap_or(chars.len());
            out.extend(&chars[i..end]);
            i = end;
            continue;
        }
        if c == '@' && !before_is_word && chars.get(i + 1).is_some_and(|n| n.is_alphanumeric()) {
            out.push('@');
            out.push(ZWJ);
            i += 1;
            continue;
        }
        if c.is_alphabetic() && !before_is_word {
            let end = (i..chars.len()).find(|&j| !chars[j].is_alphabetic()).unwrap_or(chars.len());
            let word: String = chars[i..end].iter().collect();
            out.push_str(&word);
            let rest: String = chars[end..].iter().collect();
            if CLOSING.contains(&word.to_lowercase().as_str()) && rest.starts_with([':', ' ', '\t']) && names_an_issue(&rest) {
                out.push(ZWJ);
            }
            i = end;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn starts_with_at(chars: &[char], at: usize, prefix: &str) -> bool {
    prefix.chars().enumerate().all(|(k, p)| chars.get(at + k) == Some(&p))
}

/// The section, markers included, with no trailing newline.
pub fn render(input: &Input) -> String {
    let task = input.detail.task.clone().unwrap_or_default();
    let notes = current_notes(&input.detail.notes);
    let mut out = vec![BEGIN.to_string(), "## What and why".into()];

    let intent = task.intent.trim();
    let intent = neutralize(if intent.is_empty() { task.title.trim() } else { intent });
    let intent = intent.as_str();
    let theme = input.theme.map(neutralize);
    let theme = theme.as_deref();
    match (theme, intent.contains('\n')) {
        (Some(theme), false) => out.push(format!("{intent} \u{b7} Theme: {theme}")),
        (Some(theme), true) => {
            out.push(intent.to_string());
            out.push(format!("Theme: {theme}"));
        }
        (None, _) => out.push(intent.to_string()),
    }

    out.push(String::new());
    out.push("## Acceptance".into());
    if task.acceptance.is_empty() {
        out.push("The card has no acceptance lines yet.".into());
    }
    for line in &task.acceptance {
        out.push(format!("- [{}] {}", if line.met { "x" } else { " " }, neutralize(line.text.trim())));
    }

    out.push(String::new());
    out.push("## Agent review".into());
    out.extend(review_lines(&notes));

    let called: Vec<String> = input.rulings.iter().filter_map(|r| ruling_line(r)).collect();
    if !called.is_empty() {
        out.push(String::new());
        out.push("## Decided for you".into());
        out.extend(called);
    }

    let links = captures(&notes);
    if !links.is_empty() {
        out.push(String::new());
        out.push("## Captures".into());
        out.extend(links.iter().map(|(name, url)| format!("- [{}]({url})", neutralize(name))));
    }

    if input.cost_line {
        out.push(String::new());
        out.push("## Cost".into());
        out.push(cost_line(input.lane));
    }
    out.push(END.to_string());
    out.join("\n")
}

/// The notes still standing: those no later note names in `supersedes`.
fn current_notes(notes: &[pb::TaskNote]) -> Vec<&pb::TaskNote> {
    notes.iter().filter(|n| !notes.iter().any(|m| m.supersedes.as_deref() == Some(n.id.as_ref()))).collect()
}

/// A line of a note's markdown as plain words: no bullet, no heading mark,
/// no bold.
fn plain(line: &str) -> String {
    let line = line.trim().trim_start_matches(['-', '*', '#']).trim();
    line.replace("**", "").replace("__", "").trim().to_string()
}

/// A sentence cut at `max` characters, with an ellipsis when it was cut.
fn short(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{}\u{2026}", cut.trim_end())
}

/// What a finding says when it names something the review didn't look at.
const LABEL: &str = "not checked:";

fn review_lines(notes: &[&pb::TaskNote]) -> Vec<String> {
    let findings: Vec<&&pb::TaskNote> =
        notes.iter().filter(|n| n.kind == pb::TaskNoteKind::Finding as i32).collect();
    let Some(latest) = findings.last() else {
        return vec!["No agent review yet.".into()];
    };
    let headline = latest.body.lines().map(plain).find(|l| !l.is_empty()).unwrap_or_default();
    let rounds = findings.len();
    let mut lines = vec![format!("{} ({rounds} round{})", neutralize(&short(&headline, 300)), if rounds == 1 { "" } else { "s" })];
    let mut unchecked: Vec<String> = Vec::new();
    for note in &findings {
        for line in note.body.lines().map(plain) {
            if let Some(head) = line.get(..LABEL.len())
                && head.eq_ignore_ascii_case(LABEL)
            {
                let said = line[LABEL.len()..].trim().to_string();
                if !said.is_empty() && !unchecked.contains(&said) {
                    unchecked.push(said);
                }
            }
        }
    }
    lines.extend(unchecked.into_iter().map(|u| format!("Not checked: {}", neutralize(&u))));
    lines
}

/// One ruling as a bullet, or `None` for one that was reversed: a reversed
/// call is undone, and a reviewer has nothing to push back on.
fn ruling_line(r: &pb::BoardRuling) -> Option<String> {
    let trim = |s: &str| s.trim().trim_end_matches('.').to_string();
    let head = format!("- R-{} {}", r.number, neutralize(&trim(&r.decision)));
    match pb::BoardRulingState::try_from(r.state) {
        Ok(pb::BoardRulingState::Reversed) => None,
        Ok(pb::BoardRulingState::Confirmed) => Some(format!("{head}. The owner confirmed it.")),
        _ => Some(format!("{head} (reversible: {}). Reply \"reverse R-{}\".", neutralize(&trim(&r.reversal)), r.number)),
    }
}

/// Image and video links in the notes: markdown images, then bare URLs,
/// each once, in the order they were written.
fn captures(notes: &[&pb::TaskNote]) -> Vec<(String, String)> {
    const ENDINGS: [&str; 7] = [".png", ".jpg", ".jpeg", ".gif", ".webp", ".mp4", ".mov"];
    let is_capture = |url: &str| {
        let path = url.split(['?', '#']).next().unwrap_or("").to_lowercase();
        (url.starts_with("https://") || url.starts_with("http://")) && ENDINGS.iter().any(|e| path.ends_with(e))
    };
    let name_of = |url: &str| url.split(['?', '#']).next().unwrap_or(url).rsplit('/').next().unwrap_or(url).to_string();
    let mut found: Vec<(String, String)> = Vec::new();
    let mut add = |name: String, url: String| {
        if !found.iter().any(|(_, u)| *u == url) {
            found.push((name, url));
        }
    };
    for note in notes {
        let body = note.body.as_str();
        // `![alt](url)`
        let mut rest = body;
        while let Some(at) = rest.find("![") {
            let after = &rest[at + 2..];
            let Some(close) = after.find("](") else { break };
            let Some(end) = after[close + 2..].find(')') else { break };
            let (alt, url) = (&after[..close], &after[close + 2..close + 2 + end]);
            if is_capture(url) {
                let name = if alt.trim().is_empty() { name_of(url) } else { alt.trim().to_string() };
                add(name, url.to_string());
            }
            rest = &after[close + 2 + end..];
        }
        // A bare URL, not already inside a markdown link or image.
        for word in body.split_whitespace() {
            let word = word.trim_matches(|c: char| matches!(c, '(' | ')' | '<' | '>' | ',' | '.' | ';' | '"'));
            if word.starts_with("http") && !word.contains("](") && is_capture(word) {
                add(name_of(word), word.to_string());
            }
        }
    }
    found
}

/// "About 120K tokens, Sonnet build + Opus review": what the lane spent, and
/// by whom. A lane's spend is the lane's, and a card on a lane of several
/// gets an even share, which the line says.
fn cost_line(lane: &pb::Lane) -> String {
    let spend = lane.spend.unwrap_or_default();
    let total = spend.input_tokens + spend.output_tokens + spend.cache_read_tokens + spend.cache_write_tokens;
    let mut distinct: Vec<&[u8]> = Vec::new();
    for c in &lane.cards {
        if !distinct.contains(&c.task_id.as_ref()) {
            distinct.push(c.task_id.as_ref());
        }
    }
    let cards = distinct.len().max(1) as u64;
    let mut line = match (total, cards) {
        (0, _) => NOT_REPORTED.to_string(),
        (t, 1) => format!("{} tokens", tokens(t)),
        (t, n) => format!("About {} tokens, the lane's spend split evenly across {n} cards", tokens(t / n)),
    };
    let mut who: Vec<String> = Vec::new();
    for agent in &lane.agents {
        let role = match pb::LaneAgentRole::try_from(agent.role) {
            Ok(pb::LaneAgentRole::Build) => "build",
            Ok(pb::LaneAgentRole::Review) => "review",
            Ok(pb::LaneAgentRole::Fix) => "fix",
            _ => continue,
        };
        let model = if agent.model.is_empty() { agent.harness.as_str() } else { agent.model.as_str() };
        let mut chars = model.chars();
        let named = chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default();
        let entry = neutralize(&format!("{named} {role}"));
        if !named.is_empty() && !who.contains(&entry) {
            who.push(entry);
        }
    }
    if total > 0 && !who.is_empty() {
        line.push_str(&format!(", {}", who.join(" + ")));
    }
    line
}

/// Why a description can't be merged into.
#[derive(Debug, PartialEq, Eq)]
pub struct Damaged(pub String);

/// Where `marker` begins in `text`, outside fenced code blocks.
fn marker_positions(text: &str, marker: &str) -> Vec<usize> {
    let mut found = Vec::new();
    let mut fenced = false;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            fenced = !fenced;
        } else if !fenced {
            found.extend(line.match_indices(marker).map(|(at, _)| offset + at));
        }
        offset += line.len();
    }
    found
}

/// `existing` with `section` in place of the one it holds, or on the end when
/// it holds none. Nothing outside the markers changes by a byte.
pub fn merge(existing: &str, section: &str) -> Result<String, Damaged> {
    let begins = marker_positions(existing, BEGIN);
    let ends = marker_positions(existing, END);
    match (begins.as_slice(), ends.as_slice()) {
        ([], []) if existing.contains(BEGIN) || existing.contains(END) => Err(Damaged(
            "The only Far Cooler markers in this description are inside a code block, or after one that never closes, \
             so Far Cooler can't tell where its section is. Close the code block, or remove the markers, and run this again."
                .to_string(),
        )),
        ([], []) => {
            if existing.trim().is_empty() {
                return Ok(section.to_string());
            }
            let gap = if existing.ends_with("\n\n") { "" } else if existing.ends_with('\n') { "\n" } else { "\n\n" };
            Ok(format!("{existing}{gap}{section}"))
        }
        ([from], [to]) if from < to => Ok(format!("{}{section}{}", &existing[..*from], &existing[to + END.len()..])),
        _ => Err(damaged(begins.len(), ends.len())),
    }
}

fn damaged(begins: usize, ends: usize) -> Damaged {
    Damaged(format!(
        "This description has {begins} begin and {ends} end markers, so Far Cooler can't tell which text is its own. \
         Make it one of each, in order, or remove them, and run this again."
    ))
}
