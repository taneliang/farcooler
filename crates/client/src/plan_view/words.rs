//! Every word the plan's sections and ticket cards say (ov-1016), in one place.
//!
//! Each fixed phrase is written once, in sentence case, and `Style` title-cases
//! it for the Apple apps and the web, which show a state the way the board's
//! statuses read ("In Review"), while Android keeps Material's sentence case
//! ("In review"). Only the fixed phrase is cased: a key, a login, a count or a
//! commit put beside it is left as it is, so "Waiting on alice" never becomes
//! "Waiting on Alice".

use serde::{Deserialize, Serialize};

/// Which casing a platform shows a label in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    /// Title Case, as the Mac and iPhone say a state.
    #[default]
    Apple,
    /// Sentence case, as Material says one.
    Material,
}

/// Words a title leaves lowercase unless they start or end it.
const SMALL: &[&str] = &["a", "an", "and", "as", "at", "but", "by", "for", "in", "nor", "of", "on", "or", "the", "to", "with"];

impl Style {
    /// A fixed phrase, written in sentence case, in this style's case.
    pub fn label(self, sentence: &str) -> String {
        match self {
            Style::Material => capitalize(sentence),
            Style::Apple => title_case(sentence),
        }
    }

    /// A label that comes already in Title Case (the workflow engine's), in this
    /// style's case.
    pub fn from_title(self, title: &str) -> String {
        match self {
            Style::Material => sentence_case(title),
            Style::Apple => title.to_string(),
        }
    }
}

/// The first letter in capitals.
pub fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// "Waiting for the next landing" as "Waiting for the Next Landing": every word
/// capitalized but the small ones inside the phrase. A word already holding a
/// capital ("CI", "PR") is kept as written.
pub fn title_case(sentence: &str) -> String {
    let words: Vec<&str> = sentence.split(' ').collect();
    let last = words.len().saturating_sub(1);
    words
        .iter()
        .enumerate()
        .map(|(i, word)| {
            let bare = word.trim_end_matches([',', '.', ':']);
            if i != 0 && i != last && SMALL.contains(&bare) {
                return word.to_string();
            }
            capitalize(word)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// "Awaiting Approval" as "Awaiting approval": every word after the first that
/// is a capital followed by lowercase letters is lowered; an acronym ("CI") and
/// anything else is kept.
pub fn sentence_case(title: &str) -> String {
    title
        .split(' ')
        .enumerate()
        .map(|(i, word)| {
            let mut chars = word.chars();
            let first = chars.next();
            let rest: String = chars.collect();
            let capitalized = first.is_some_and(char::is_uppercase)
                && !rest.is_empty()
                && rest.chars().all(|c| !c.is_alphabetic() || c.is_lowercase());
            if i > 0 && capitalized { word.to_lowercase() } else { word.to_string() }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// "1st", "2nd", "3rd", "4th", … "11th", "12th", "13th", "21st".
pub fn ordinal(n: u64) -> String {
    if (11..=13).contains(&(n % 100)) {
        return format!("{n}th");
    }
    match n % 10 {
        1 => format!("{n}st"),
        2 => format!("{n}nd"),
        3 => format!("{n}rd"),
        _ => format!("{n}th"),
    }
}

/// How long, coarsely: "45 min", "2 h", "3 d".
pub fn span(ms: i64) -> String {
    let minutes = ms.max(0) / 60_000;
    match minutes {
        0..60 => format!("{minutes} min"),
        60..1440 => format!("{} h", minutes / 60),
        _ => format!("{} d", minutes / 1440),
    }
}

/// How long ago, coarsely: "just now", "5 min ago", "3 h ago", "2 d ago".
pub fn ago(at: i64, now: i64) -> String {
    let minutes = (now - at).max(0) / 60_000;
    match minutes {
        0 => "just now".into(),
        1..60 => format!("{minutes} min ago"),
        60..1440 => format!("{} h ago", minutes / 60),
        _ => format!("{} d ago", minutes / 1440),
    }
}

/// A model's display name. Exact matches only; an unknown id stays literal.
/// Presentation only, never a launch or pricing key.
pub fn model(raw: &str) -> &str {
    match raw {
        "gpt-6.1-sol" => "GPT-6.1 Sol",
        "gpt-6.1-terra" => "GPT-6.1 Terra",
        "gpt-6-luna" => "GPT-6 Luna",
        "gemini-2.5-pro" => "Gemini 2.5 Pro",
        "claude-opus-5-5" => "Claude Opus 5.5",
        "claude-opus-5" => "Claude Opus 5",
        "claude-sonnet-5" => "Claude Sonnet 5",
        _ => raw,
    }
}

/// The words with the recorded model id added, for assistive technology, only
/// when the display name changed it.
pub fn accessible_model(words: &str, raw: &str) -> String {
    if raw.is_empty() || model(raw) == raw { words.to_string() } else { format!("{words}, Model ID: {raw}") }
}

/// "6.1 million", "340 thousand", "812": a count a screen reader says whole.
pub fn spoken_tokens(n: u64) -> String {
    let short = farcooler_core::usage_words::tokens(n);
    for (suffix, word) in [("K", " thousand"), ("M", " million"), ("B", " billion")] {
        if let Some(stem) = short.strip_suffix(suffix) {
            return format!("{stem}{word}");
        }
    }
    short
}

pub const DOT: &str = " \u{00B7} ";

/// The phrases screens outside the plan say that the plan also says: the shell's
/// close question, the Agents card's empty state, a card's answer row (ov-1041).
/// Asked for with `{"schema":1,"words":{"style":"apple"}}` through the same
/// bridge as the view, so no platform has a second copy.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Words {
    /// The Agents card's title when nothing is running: "No Agents Running".
    pub agents_none: String,
    /// The pipeline stage's title, including when its list has emptied.
    pub needs_you: String,
    /// The label of a card's answer row: "Waiting on You".
    pub waiting_on_you: String,
    /// What a blocked agent is doing, in the middle of a sentence: "waiting on you".
    pub blocked_doing: String,
    /// Why a handle can't be reused yet (ov-1002), in the one sentence every platform shows.
    pub handle_waited_on: String,
}

impl Words {
    pub fn new(style: Style) -> Self {
        Self {
            agents_none: style.label("no agents running"),
            needs_you: super::column::PipeStage::Stuck.title().into(),
            waiting_on_you: style.label("waiting on you"),
            blocked_doing: "waiting on you".to_string(),
            handle_waited_on: "A workflow or a run still names someone who had that handle. Choose another handle, or ask the Owner to cancel those runs and publish a workflow that doesn\u{2019}t name them.".to_string(),
        }
    }
}
