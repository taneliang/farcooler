//! The text a person left in claude's box, read off its screen with its line
//! breaks, for Bring Here (ov-369, R-28): moved into a native composer, so a
//! send there has one draft and not two.
//!
//! `composer::read` squeezes a box's whitespace, which is enough to say
//! whether it's empty; this puts the line breaks back. claude 2.1.292's
//! rules, measured in a sandbox (`captures/claude-2.1.292-draft-*`):
//! - its box's text is the pane's width less 4 columns wide, the `❯ ` or the
//!   two-column indent before it and two columns after;
//! - it wraps at a space and drops that space; a word longer than a row
//!   starts where it is and is split at the edge;
//! - a typed line break starts a row of its own, and a blank line is an empty
//!   row; a line's own leading spaces are kept, after the indent;
//! - its cursor is a reverse-video cell, after the last character when it's
//!   at the end;
//! - a tall draft is windowed: at most `max_rows` rows show, `❯` on the
//!   first of them whichever line that is, and nothing says rows are hidden.
//!
//! So at each row boundary: a next row that's empty or starts with a space,
//! or whose first word would have fit on this one, follows a typed line
//! break; otherwise the row wrapped, at a space, or inside a word longer
//! than a row. Widths are counted in characters, so a row of wide characters
//! may read a line break as a space, or the other way: the words are kept.

use super::{cells_with_reverse, claude_box_at};

/// What claude's box holds, for Bring Here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Draft {
    /// The box is there and nothing has been typed in it.
    Empty,
    /// The box holds this.
    Holds(Held),
    /// No box this module knows is on the screen.
    Unrecognized,
}

/// A draft as the box shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    /// Its text: line breaks as `\n`, wrapped rows joined, trailing
    /// whitespace dropped.
    pub text: String,
    /// How many rows the box draws.
    pub rows: usize,
    /// claude's cursor is right after the last character.
    pub cursor_at_end: bool,
    /// It shows a paste claude collapsed or an image, `[Pasted text #N …]`
    /// or `[Image #N]`, whose content the screen doesn't hold.
    pub placeholder: bool,
}

/// The most rows claude 2.1.292 draws its box in, in a pane `rows` tall:
/// half of it, less 5 (measured at 20, 24, 30, 40, 45, 50, 60 and 80 rows).
/// A box this tall may be hiding rows.
pub fn max_rows(rows: u32) -> usize {
    (rows as usize / 2).saturating_sub(5)
}

/// Whether `text` could be a box showing `whole` with some of its end
/// deleted: `whole` starts with it, whitespace ignored.
pub fn is_prefix_of(text: &str, whole: &str) -> bool {
    squeezed(whole).starts_with(&squeezed(text))
}

fn squeezed(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// claude's box on `screen` (captured with its escapes, `-e`), in a pane
/// `columns` wide.
pub fn claude(screen: &str, columns: u32) -> Draft {
    let full: Vec<Vec<(char, bool, bool)>> = screen.lines().map(cells_with_reverse).collect();
    let plain: Vec<Vec<(char, bool)>> = full.iter().map(|row| row.iter().map(|&(c, dim, _)| (c, dim)).collect()).collect();
    let Some((start, end)) = claude_box_at(&plain) else { return Draft::Unrecognized };
    let width = (columns as usize).saturating_sub(4).max(1);
    // Each row's typed characters after its two columns of prefix, and
    // where claude's cursor is, if it's drawn there.
    let mut rows: Vec<String> = Vec::new();
    let mut cursor: Option<(usize, usize)> = None;
    for (n, row) in full[start..end].iter().enumerate() {
        let cells = &row[2.min(row.len())..];
        let mut typed = String::new();
        for &(c, dim, reverse) in cells {
            if reverse && cursor.is_none() {
                cursor = Some((n, typed.chars().count()));
            }
            if !dim {
                typed.push(c);
            }
        }
        rows.push(typed.trim_end().to_string());
    }
    let last = rows.len() - 1;
    let shown = rows.len();
    if rows.iter().all(|r| r.trim().is_empty()) {
        return Draft::Empty;
    }
    let mut text = rows[0].clone();
    for pair in rows.windows(2) {
        text.push_str(joint(&pair[0], &pair[1], width));
        text.push_str(&pair[1]);
    }
    let text = text.trim_end().to_string();
    // At the end: on the last row with something on it, at or after its
    // last character; or alone on an empty row below it, which claude draws
    // when the last row is full.
    let content_last = rows.iter().rposition(|r| !r.trim().is_empty()).unwrap_or(0);
    let cursor_at_end = match cursor {
        Some((row, at)) if row == content_last => at >= rows[row].chars().count(),
        Some((row, 0)) => row > content_last && row == last,
        _ => false,
    };
    let placeholder = text.contains("[Pasted text #") || text.contains("[Image #");
    Draft::Holds(Held { text, rows: shown, cursor_at_end, placeholder })
}

/// What goes between rows `above` and `below` of a box `width` wide: a typed
/// line break, the space claude wrapped at, or nothing inside a split word.
fn joint(above: &str, below: &str, width: usize) -> &'static str {
    if above.is_empty() || below.is_empty() || below.starts_with(char::is_whitespace) {
        return "\n";
    }
    let len = |s: &str| s.chars().count();
    let first = below.split(char::is_whitespace).next().unwrap_or_default();
    if len(above) + 1 + len(first) <= width {
        return "\n";
    }
    let tail = above.rsplit(char::is_whitespace).next().unwrap_or_default();
    // Only a word longer than a row is split: one that fit would have
    // wrapped whole.
    if len(tail) + len(first) > width { "" } else { " " }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture(name: &str) -> String {
        std::fs::read_to_string(format!("{}/captures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    fn held(name: &str) -> Held {
        match claude(&capture(name), 100) {
            Draft::Holds(held) => held,
            other => panic!("{name}: {other:?}"),
        }
    }

    #[test]
    fn a_draft_keeps_its_line_breaks_and_joins_what_claude_wrapped() {
        let held = held("claude-2.1.292-draft-multiline-100x40-e.txt");
        let url = format!("https://example.com/{}", "a".repeat(110));
        assert_eq!(
            held.text,
            format!(
                "Fix the login bug first.\n\nThen check that the session cookie survives a restart, and that the redirect \
                 after sign-in goes back to the page the person was on rather than the home page.\n  indented: {url} end"
            )
        );
        assert_eq!(held.rows, 6);
        assert!(held.cursor_at_end);
        assert!(!held.placeholder);
    }

    #[test]
    fn one_line_and_a_draft_mid_turn() {
        let one = held("claude-2.1.292-draft-one-line-100x40-e.txt");
        assert_eq!((one.text.as_str(), one.rows, one.cursor_at_end), ("just one line", 1, true));
        let working = held("claude-2.1.292-draft-working-100x40-e.txt");
        assert_eq!(working.text, "while you work: also look at the logout path\nand the tests");
        assert!(working.cursor_at_end);
        let after = held("claude-2.1.292-draft-working-after-ctrl-u-100x40-e.txt");
        assert_eq!(after.text, "while you work: also look at the logout path");
        assert!(is_prefix_of(&after.text, &working.text));
        assert!(!is_prefix_of(&working.text, &after.text));
    }

    #[test]
    fn a_moved_cursor_a_paste_and_a_tall_draft_say_so() {
        let moved = held("claude-2.1.292-draft-cursor-moved-100x40-e.txt");
        assert!(!moved.cursor_at_end);
        assert_eq!(moved.text, held("claude-2.1.292-draft-multiline-100x40-e.txt").text);
        let pasted = held("claude-2.1.292-draft-pasted-100x40-e.txt");
        assert!(pasted.placeholder);
        assert_eq!(pasted.text, "[Pasted text #1 +3 lines] and more");
        // Twenty lines in a 40-row pane: 15 show, rows 6 to 20.
        let tall = held("claude-2.1.292-draft-tall-100x40-e.txt");
        assert_eq!(tall.rows, 15);
        assert_eq!(max_rows(40), 15);
        assert!(tall.text.starts_with("row 6\nrow 7\n"));
    }

    #[test]
    fn an_empty_box_and_no_box() {
        assert_eq!(claude(&capture("claude-2.1.292-idle-placeholder-160x45-e.txt"), 160), Draft::Empty);
        assert_eq!(claude(&capture("claude-blocked.txt"), 100), Draft::Unrecognized);
    }

    #[test]
    fn the_joints_measured() {
        // Fits after the row: typed.
        assert_eq!(joint("short", "next", 96), "\n");
        // Doesn't fit: wrapped at a space.
        assert_eq!(joint("w ".repeat(47).trim_end(), "next", 96), " ");
        // A word longer than a row, split at the edge.
        assert_eq!(joint(&format!("x {}", "a".repeat(94)), &"a".repeat(30), 96), "");
        // A line's own indent, and blank lines.
        assert_eq!(joint(&"w".repeat(96), "  indented", 96), "\n");
        assert_eq!(joint("", "x", 96), "\n");
        assert_eq!(joint("x", "", 96), "\n");
    }

    #[test]
    fn the_box_as_read_holds_the_same_characters() {
        let screen = capture("claude-2.1.292-draft-multiline-100x40-e.txt");
        let Draft::Holds(held) = claude(&screen, 100) else { panic!() };
        assert!(super::super::holds_exactly(&super::super::read("claude", &screen), &held.text));
    }
}
