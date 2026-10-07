//! The spinner row of a working agent (ov-394).

use super::{AgentRules, footer_lines};

/// Whether the row drawn just above the box is a spinner row
/// (`AgentRules::spinner`): a glyph, a space, and a first word ending in `…`.
/// A done line (`✻ Worked for 3s`) has no ellipsis.
///
/// Only that row counts: the box is the `❯` line with a rule of `─` above it,
/// and the spinner is the row above that rule, with one blank row allowed
/// between (claude draws it so), and the `⎿` tip rows it draws under it. A
/// prose line in the transcript that happens to start with `·` or `*` and a
/// word ending in `…` (`* Building…`) sits further up, or has no box beneath
/// it, and is not read as Working (ov-394).
pub(super) fn spinning(rules: &AgentRules, screen: &str) -> bool {
    if rules.spinner.is_empty() {
        return false;
    }
    let lines = footer_lines(screen, usize::MAX);
    // The last prompt line: the box is drawn below everything.
    let Some(prompt) = lines.iter().rposition(|l| l.starts_with('❯')) else { return false };
    let is_rule = |l: &String| l.chars().count() >= 10 && l.chars().all(|c| c == '─');
    if prompt == 0 || !is_rule(&lines[prompt - 1]) {
        return false;
    }
    // Up from the rule, past one blank row and any `⎿` rows (the tip claude
    // draws under its spinner: `claude-2.1.290-working-long-paste`).
    let mut row = prompt - 1;
    let mut blank = false;
    while let Some(above) = row.checked_sub(1) {
        row = above;
        let line = &lines[row];
        if line.starts_with('⎿') {
            continue;
        }
        if line.is_empty() {
            if std::mem::replace(&mut blank, true) {
                return false;
            }
            continue;
        }
        let mut words = line.split(' ');
        let glyph = words.next().unwrap_or_default();
        return rules.spinner.iter().any(|g| g == glyph)
            && words.next().is_some_and(|w| w.chars().count() > 1 && w.ends_with('…'));
    }
    false
}
