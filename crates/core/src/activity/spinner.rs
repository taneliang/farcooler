//! The spinner row of a working agent (ov-394).

use super::{AgentRules, strip_ansi};

/// How many rows up from the box's top rule the spinner may be.
const SEARCH_ROWS: usize = 12;

/// Whether the spinner row (`AgentRules::spinner`) of a working agent is
/// drawn above the box: a glyph, a space, and a first word ending in `…`. A
/// done line (`✻ Worked for 3s`) has no ellipsis.
///
/// The box is the `❯` line with a rule of `─` above it. From that rule, walk
/// up past what claude draws under its spinner: one blank row, `⎿` rows (a
/// tip), and any indented row (a todo list, a tip or the spinner's own line
/// wrapped, a subagent's progress). The first row that isn't one of those is
/// the one that must be the spinner. A prose line in the transcript that
/// happens to start with `·` or `*` and a word ending in `…` (`* Building…`)
/// has the transcript's own rows (`⏺ …`, `✻ Worked for …`, at column 0)
/// between it and the box, or no box beneath it, and is not read as Working.
pub(super) fn spinning(rules: &AgentRules, screen: &str) -> bool {
    if rules.spinner.is_empty() {
        return false;
    }
    let plain = strip_ansi(screen);
    let mut lines: Vec<&str> = plain.lines().map(str::trim_end).collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    // The last prompt line: the box is drawn below everything.
    let Some(prompt) = lines.iter().rposition(|l| l.starts_with('❯')) else { return false };
    if prompt == 0 || !crate::composer::is_rule_text(lines[prompt - 1]) {
        return false;
    }
    let mut blank = false;
    for row in (0..prompt - 1).rev().take(SEARCH_ROWS) {
        let line = lines[row];
        if line.is_empty() {
            if std::mem::replace(&mut blank, true) {
                return false;
            }
            continue;
        }
        if line.starts_with(char::is_whitespace) || line.starts_with('⎿') {
            continue;
        }
        let mut words = line.split_whitespace();
        let glyph = words.next().unwrap_or_default();
        return rules.spinner.iter().any(|g| g == glyph)
            && words.next().is_some_and(|w| w.chars().count() > 1 && w.ends_with('…'));
    }
    false
}
