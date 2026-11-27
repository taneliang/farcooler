//! The words a row's lines are cut to (moved from `fold.rs`).

use super::fold::SUMMARY_CHARS;
use super::record::Input;

/// Collapses whitespace, and cuts to `max` characters.
pub(super) fn squeeze(text: &str, max: usize) -> String {
    let mut out = String::with_capacity(text.len().min(max.saturating_mul(4)));
    for (n, word) in text.split_whitespace().enumerate() {
        if n > 0 {
            out.push(' ');
        }
        out.push_str(word);
        if out.len() > max.saturating_mul(4) {
            break;
        }
    }
    match out.char_indices().nth(max) {
        Some((cut, _)) => {
            out.truncate(cut);
            out.push('…');
            out
        }
        None => out,
    }
}

/// Cuts to `max` characters, keeping line breaks.
pub(super) fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

/// The one input field a person reads first, in the spike's order.
pub(super) fn summarize(input: &Input<'_>) -> String {
    let first = [
        &input.description,
        &input.command,
        &input.file_path,
        &input.pattern,
        &input.path,
        &input.subject,
        &input.url,
        &input.query,
    ]
    .into_iter()
    .find_map(|field| field.get());
    first.map(|s| squeeze(s, SUMMARY_CHARS)).unwrap_or_default()
}
