//! The word under a cell.
//!
//! `url_at` answers "is this a link the core knows?". Some links are not the
//! core's to know: a task key like `ov-190` is a link only when the app has
//! read that board, so the app decides. What the app needs from the grid is the
//! context its rule reads: the whole whitespace-delimited word, so the
//! characters either side of the key and the `://` check across the word see
//! what the reader saw. A word runs between ASCII whitespace, as the rule's
//! fixture reads it. Soft wraps are followed, because agent output wraps
//! constantly and a per-row word is a fragment.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags;

use crate::Terminal;

/// How far the walk goes either way, in cells. It runs on every hover, so it
/// is bounded; no real word is longer.
const WALK_LIMIT: usize = 1024;

/// A word on screen and where a cell sits in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordAt {
    pub text: String,
    /// The UTF-16 offset of the asked cell's character in `text`, the unit
    /// both Swift and Kotlin index strings by.
    pub offset: u32,
}

fn is_spacer(flags: Flags) -> bool {
    flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
}

/// The word covering a cell of the CURRENT VIEW, or `None` for a blank cell or
/// one outside the grid.
pub fn word_at(term: &Terminal, row: u16, column: u16) -> Option<WordAt> {
    let t = term.term();
    let grid = t.grid();
    let columns = grid.columns();
    if row as usize >= grid.screen_lines() || column as usize >= columns {
        return None;
    }
    // The scroll position applies exactly as in `url_at`: the grid indexes the
    // live screen, and a scrolled-back view must answer for what it shows.
    let offset = grid.display_offset() as i32;
    let top = Line(-(grid.history_size() as i32));
    let bottom = Line(grid.screen_lines() as i32 - 1);
    let mut here = Point::new(Line(row as i32 - offset), Column(column as usize));

    // A click on the right half of a wide character lands on its spacer.
    if is_spacer(grid[here].flags) && here.column.0 > 0 {
        here.column = Column(here.column.0 - 1);
    }
    let blank = |p: Point| {
        let cell = &grid[p];
        !is_spacer(cell.flags) && (cell.c == '\0' || cell.c.is_ascii_whitespace())
    };
    if blank(here) {
        return None;
    }

    // Left: the cells before, nearest first.
    let mut before: Vec<Point> = Vec::new();
    let mut at = here;
    while before.len() < WALK_LIMIT {
        let previous = if at.column.0 > 0 {
            Point::new(at.line, Column(at.column.0 - 1))
        } else {
            let line = Line(at.line.0 - 1);
            if line < top || !grid[Point::new(line, Column(columns - 1))].flags.contains(Flags::WRAPLINE)
            {
                break;
            }
            Point::new(line, Column(columns - 1))
        };
        if is_spacer(grid[previous].flags) {
            // Holds no character of its own; step over it.
            at = previous;
            continue;
        }
        if blank(previous) {
            break;
        }
        before.push(previous);
        at = previous;
    }

    let mut text = String::new();
    for p in before.iter().rev() {
        push_cell(&mut text, &grid[*p]);
    }
    let offset = text.encode_utf16().count() as u32;
    push_cell(&mut text, &grid[here]);

    // Right: the cells after.
    let mut at = here;
    let mut walked = 0;
    while walked < WALK_LIMIT {
        walked += 1;
        let next = if at.column.0 + 1 < columns {
            Point::new(at.line, Column(at.column.0 + 1))
        } else {
            if !grid[at].flags.contains(Flags::WRAPLINE) || at.line + 1 > bottom {
                break;
            }
            Point::new(Line(at.line.0 + 1), Column(0))
        };
        if is_spacer(grid[next].flags) {
            at = next;
            continue;
        }
        if blank(next) {
            break;
        }
        push_cell(&mut text, &grid[next]);
        at = next;
    }

    Some(WordAt { text, offset })
}

fn push_cell(text: &mut String, cell: &alacritty_terminal::term::cell::Cell) {
    text.push(cell.c);
    if let Some(extra) = cell.zerowidth() {
        text.extend(extra.iter());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(t: &Terminal, row: u16, column: u16) -> Option<(String, u32)> {
        word_at(t, row, column).map(|w| (w.text, w.offset))
    }

    #[test]
    fn a_word_with_punctuation_is_whole_from_any_of_its_cells() {
        let mut t = Terminal::new(40, 3);
        t.feed(b"see (ov-190). ok");
        // "see " is four cells; "(ov-190)." is 9, so columns 4..=12.
        for column in 4..=12u16 {
            assert_eq!(
                word(&t, 0, column),
                Some(("(ov-190).".to_string(), u32::from(column) - 4)),
                "column {column}"
            );
        }
        assert_eq!(word(&t, 0, 3), None, "the space is no word");
        assert_eq!(word(&t, 0, 30), None, "an empty cell is no word");
    }

    #[test]
    fn a_word_soft_wrapped_over_two_rows_is_found_whole_from_either() {
        let mut t = Terminal::new(10, 3);
        // Eight cells of lead-in, then the key crosses the edge.
        t.feed(b"abcdefgh ov-190 z");
        // Row 0 is "abcdefgh o", row 1 is "v-190 z".
        assert_eq!(word(&t, 0, 9), Some(("ov-190".to_string(), 0)));
        assert_eq!(word(&t, 1, 4), Some(("ov-190".to_string(), 5)));
    }

    #[test]
    fn a_hard_newline_ends_the_word() {
        let mut t = Terminal::new(10, 3);
        t.feed(b"abcdefghij\r\nov-190");
        assert_eq!(word(&t, 1, 0), Some(("ov-190".to_string(), 0)));
        assert_eq!(word(&t, 0, 9), Some(("abcdefghij".to_string(), 9)));
    }

    #[test]
    fn a_scrolled_back_view_answers_for_the_view() {
        let mut t = Terminal::new(40, 3);
        t.feed(b"old ov-1\r\n");
        for _ in 0..10 {
            t.feed(b"filler\r\n");
        }
        assert_eq!(word(&t, 0, 7), None, "the live view shows filler, which ends at column 5");
        t.scroll(11);
        assert_eq!(word(&t, 0, 5), Some(("ov-1".to_string(), 1)));
        assert_eq!(word(&t, 0, 7), Some(("ov-1".to_string(), 3)));
    }

    #[test]
    fn a_wide_character_before_the_key_counts_two_utf16_units() {
        let mut t = Terminal::new(40, 3);
        // The emoji is two UTF-16 units and two cells.
        t.feed("\u{1F600}ov-190".as_bytes());
        assert_eq!(word(&t, 0, 2), Some(("\u{1F600}ov-190".to_string(), 2)));
        // The spacer half of the emoji answers as the emoji.
        assert_eq!(word(&t, 0, 1), Some(("\u{1F600}ov-190".to_string(), 0)));
    }

    #[test]
    fn a_cell_outside_the_grid_is_none() {
        let t = Terminal::new(20, 4);
        assert_eq!(word(&t, 99, 0), None);
        assert_eq!(word(&t, 0, 99), None);
    }
}
