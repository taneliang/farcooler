//! Turns Claude Code writes for itself, read for what they are (ov-474).
//!
//! Claude delivers three things as a user turn that no person typed: another
//! session's message (a subagent's final report arrives as one, wrapped in
//! `<agent-message from="…">` with a safety preamble and its lines indented),
//! a background task's `<task-notification>`, and `<system-reminder>` blocks.
//! Shown as written they are raw XML in a person's bubble, so the projector
//! names the kind and cleans the words, and each view gives it its own row.
//!
//! `System` reminders are folded into one muted, expandable note rather than
//! hidden. The turn they open is real (the agent woke and answered), so it
//! keeps a row for its outcome and run time, and the person can read what
//! the harness told the agent. A reminder with words around it is not a
//! harness turn: those words are the prompt, and the blocks are cut from it.

use std::borrow::Cow;

use serde::Serialize;

use super::fold::{clip, squeeze, tag, PROMPT_CHARS};

/// What a harness turn is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum HarnessKind {
    /// A subagent's final report (`[Subagent hand-back]`). The view names the
    /// subagent from `agent_id`, and links to its transcript.
    Report,
    /// Another session's message that is not a hand-back.
    Message,
    /// A background command or agent finished: one muted status line.
    Status,
    /// `<system-reminder>`s, folded into one muted, expandable row.
    Note,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Harness {
    pub kind: HarnessKind,
    /// A `Status`' one line. Empty for the rest, which the view titles.
    pub title: String,
    /// A `Report`'s or `Message`'s words with the harness's framing cut and
    /// its indent removed; a `Note`'s reminders. Empty for a `Status`.
    pub body: String,
    /// The sending agent's id, for a `Report` or a `Message`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

/// A turn's words as a view shows them, and what kind of turn it is.
pub(super) struct Reading<'t> {
    pub prompt: Cow<'t, str>,
    pub harness: Option<Harness>,
}

const PEER_PREFIX: &str = "Another Claude session sent a message:";
const HAND_BACK: &str = "[Subagent hand-back]";
const REPORT_FOLLOWS: &str = "The report follows:";
const REMINDER: (&str, &str) = ("<system-reminder>", "</system-reminder>");

pub(super) fn read(text: &str) -> Reading<'_> {
    let trimmed = text.trim_start();
    let peer = trimmed.strip_prefix(PEER_PREFIX).map_or(trimmed, str::trim_start);
    let harness = if peer.starts_with("<agent-message") {
        message(peer)
    } else if trimmed.starts_with("<task-notification>") {
        Some(status(trimmed))
    } else if trimmed.starts_with(REMINDER.0) {
        return reminders(text);
    } else {
        None
    };
    match harness {
        Some(h) => {
            let prompt = if h.title.is_empty() { h.body.clone() } else { h.title.clone() };
            Reading { prompt: Cow::Owned(prompt), harness: Some(h) }
        }
        None => Reading { prompt: Cow::Borrowed(text), harness: None },
    }
}

fn message(text: &str) -> Option<Harness> {
    let open_end = text.find('>')?;
    let open = &text[..open_end];
    let agent_id = open
        .split_once("from=\"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(id, _)| id.trim().to_string())
        .filter(|id| !id.is_empty());
    let inside = &text[open_end + 1..];
    // The closing tag is at column zero; a report's own lines are indented, so
    // a forged one inside it is not found.
    let body = inside.find("\n</agent-message>").map_or(inside, |at| &inside[..at]);
    let body = body.trim_start_matches('\n');
    let report = body.trim_start().starts_with(HAND_BACK);
    let words = if report {
        let after = body.find(REPORT_FOLLOWS).map_or_else(|| body.split_once('\n').map_or("", |(_, rest)| rest), |at| &body[at + REPORT_FOLLOWS.len()..]);
        unindent(after)
    } else {
        body.trim().to_string()
    };
    let kind = if report { HarnessKind::Report } else { HarnessKind::Message };
    Some(Harness { kind, title: String::new(), body: clip(&words, PROMPT_CHARS), agent_id })
}

/// The harness indents every line of a report by two spaces; a blank line is
/// two spaces and nothing else.
fn unindent(text: &str) -> String {
    let lines: Vec<&str> = text.trim_matches('\n').lines().map(|l| l.strip_prefix("  ").unwrap_or(l)).collect();
    lines.join("\n")
}

fn status(text: &str) -> Harness {
    let line = match tag(text, "summary") {
        Some(summary) => status_line(summary.trim()),
        None => "A background task finished".to_string(),
    };
    Harness { kind: HarnessKind::Status, title: line, body: String::new(), agent_id: None }
}

/// The summary claude wrote, as one line in a fixed shape. Its three known
/// forms (`Agent "x" finished`, `Agent "x" failed: why`, `Background command
/// "x" completed (exit code 0)`) are reworded; any other is kept as written,
/// less its straight quotes.
fn status_line(summary: &str) -> String {
    if let Some(rest) = summary.strip_prefix("Agent \"") {
        if let Some((name, why)) = rest.split_once("\" failed: ") {
            return format!("Agent failed: {} ({})", squeeze(name, 80), squeeze(why, 80));
        }
        if let Some(name) = rest.strip_suffix("\" finished") {
            return format!("Agent finished: {}", squeeze(name, 80));
        }
    }
    if let Some(rest) = summary.strip_prefix("Background command \"") {
        let rest = format!("\"{rest}");
        if let Some((cmd, tail)) = rest.rsplit_once("\" ") {
            let cmd = squeeze(cmd.trim_start_matches('"'), 80);
            if let Some(code) = tail.strip_prefix("completed (exit code ").and_then(|c| c.strip_suffix(')')) {
                return format!("Background command finished: {cmd} (exit {code})");
            }
            if let Some(code) = tail.strip_prefix("failed with exit code ") {
                return format!("Background command failed: {cmd} (exit {code})");
            }
            if let Some(why) = tail.strip_prefix("was stopped") {
                let why = why.trim_start_matches(" because ").trim_start_matches(" after ").trim();
                let why = if why.is_empty() { String::new() } else { format!(" ({})", squeeze(why, 80)) };
                return format!("Background command stopped: {cmd}{why}");
            }
        }
    }
    squeeze(&summary.replace('"', ""), 160)
}

/// A user turn that is `<system-reminder>` blocks and nothing a person wrote:
/// one note. With words beside the blocks, those words are the prompt.
fn reminders(text: &str) -> Reading<'_> {
    let (mut kept, mut notes, mut rest) = (String::new(), Vec::new(), text);
    while let Some(from) = rest.find(REMINDER.0) {
        kept.push_str(&rest[..from]);
        let inner = &rest[from + REMINDER.0.len()..];
        let Some(len) = inner.find(REMINDER.1) else {
            notes.push(inner.trim().to_string());
            rest = "";
            break;
        };
        notes.push(inner[..len].trim().to_string());
        rest = &inner[len + REMINDER.1.len()..];
    }
    kept.push_str(rest);
    if !kept.trim().is_empty() {
        return Reading { prompt: Cow::Owned(kept.trim().to_string()), harness: None };
    }
    let body = clip(&notes.join("\n\n"), PROMPT_CHARS);
    let harness = Harness { kind: HarnessKind::Note, title: String::new(), body: body.clone(), agent_id: None };
    Reading { prompt: Cow::Owned(body), harness: Some(harness) }
}
