//! Finding the URL under a cell.
//!
//! Two sources, in this order: an OSC 8 hyperlink the program stated, then a
//! regex sweep over the grid. The order matters — the whole point of OSC 8 is
//! that the visible text and the target differ, so a URL inferred from text the
//! program had already labelled would open the wrong thing.
//!
//! The regex path sweeps the EMULATOR's grid rather than a snapshot row, and
//! that is the load-bearing choice here: long URLs wrap across rows constantly
//! in agent output, and a per-row scan finds the first half of one and calls it
//! a URL. Only the grid knows which lines continue which, and `RegexIter`
//! walks it the way Alacritty's own hints do.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Direction, Line, Point};
use alacritty_terminal::term::search::{RegexIter, RegexSearch};

use crate::Terminal;

/// A URL on screen, and where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlMatch {
    pub url: String,
    /// Display coordinates — the space `grid::snapshot` reports in — so a
    /// renderer can underline the span without converting anything. Clamped at
    /// the top, so a URL whose first row is scrolled off screen still reports a
    /// span the renderer can draw.
    pub start_row: u16,
    pub start_column: u16,
    pub end_row: u16,
    pub end_column: u16,
}

/// The schemes worth opening, stated rather than inherited.
///
/// Terminal output is not trusted input. An agent working overnight prints
/// whatever it read, and some of what it read came off the internet. A URL
/// opener that honors whatever the platform will dispatch is a local
/// application-launch primitive driven by text an attacker may have chosen, so
/// the set lives here — in the one place all three renderers read it from —
/// rather than three times over in three languages.
const SCHEMES: &[&str] = &[
    "http", "https", "mailto", "ftp", "ftps", "ssh", "git", "news", "file",
    "gemini", "gopher", "ipfs", "ipns", "magnet",
];

/// How far past the visible region to sweep, in lines.
///
/// A URL can begin on the last row above the view and continue onto the first
/// row of it, and a sweep bounded exactly by the view would find only the tail
/// and open a fragment. Two lines is enough for that overlap at any practical
/// width. Deliberately not the whole grid: this runs on every hover, and
/// scanning ten thousand lines of scrollback to decide a cursor shape would be
/// felt.
const OVERSCAN: i32 = 2;

/// The URL covering a cell of the CURRENT VIEW, or `None`.
pub fn url_at(term: &Terminal, row: u16, column: u16) -> Option<UrlMatch> {
    let t = term.term();
    let grid = t.grid();
    if row as usize >= grid.screen_lines() || column as usize >= grid.columns() {
        return None;
    }

    // Indexing the grid ignores the scroll position — line 0 is the top of the
    // ACTIVE region and history is at negative lines — so the offset is applied
    // here, exactly as `grid::snapshot` does. Without it, a view scrolled back
    // an hour would answer for whatever sits at the same position on the live
    // screen, and open it.
    let offset = grid.display_offset() as i32;
    let point = Point::new(Line(row as i32 - offset), Column(column as usize));

    hyperlink_at(term, point, offset).or_else(|| regex_at(term, point, offset))
}

/// The visible region, widened by `OVERSCAN` and clamped to what exists.
fn sweep_bounds(term: &Terminal, offset: i32) -> (Line, Line) {
    let grid = term.term().grid();
    let lines = grid.screen_lines() as i32;
    let oldest = -(grid.history_size() as i32);
    let top = (-offset - OVERSCAN).max(oldest);
    let bottom = (lines - 1 - offset + OVERSCAN).min(lines - 1);
    (Line(top), Line(bottom))
}

/// An OSC 8 hyperlink on the cell, expanded over every neighboring cell
/// carrying the same one.
fn hyperlink_at(term: &Terminal, point: Point, offset: i32) -> Option<UrlMatch> {
    let t = term.term();
    let grid = t.grid();
    let link = grid[point.line][point.column].hyperlink()?;
    let uri = link.uri().to_string();
    if !has_allowed_scheme(&uri) {
        return None;
    }

    let columns = grid.columns();
    let (top, bottom) = sweep_bounds(term, offset);
    let same = |p: Point| {
        grid[p.line][p.column].hyperlink().is_some_and(|h| h.uri() == uri)
    };

    // Walked cell by cell rather than by searching, because a hyperlink's
    // extent is not a pattern — it is wherever the program stopped setting it,
    // which can be one cell or a whole paragraph.
    let mut start = point;
    while let Some(previous) = step_left(start, columns, top) {
        if !same(previous) {
            break;
        }
        start = previous;
    }
    let mut end = point;
    while let Some(next) = step_right(end, columns, bottom) {
        if !same(next) {
            break;
        }
        end = next;
    }

    Some(span(uri, start, end, offset))
}

/// A regex match covering the cell.
fn regex_at(term: &Terminal, point: Point, offset: i32) -> Option<UrlMatch> {
    let t = term.term();
    // Built per call rather than cached on the handle. This runs on a hover or
    // a click, not per frame, and a lazily-compiled DFA held across feeds would
    // be state to invalidate for no measurable gain.
    let mut search = RegexSearch::new(&pattern()).ok()?;

    let columns = t.grid().columns();
    let (top, bottom) = sweep_bounds(term, offset);
    let start = Point::new(top, Column(0));
    let end = Point::new(bottom, Column(columns - 1));

    // Every match in the region, then the one that actually contains the
    // point. Searching left or right FROM the point instead would find the
    // nearest match rather than the containing one, and the nearest match to a
    // pointer sitting in the middle of a URL is that same URL only by luck.
    let found = RegexIter::new(start, end, Direction::Right, t, &mut search)
        .find(|m| m.contains(&point))?;

    // The pattern deliberately accepts a period, because a period is legal in a
    // path — which means a URL ending a sentence swallows the sentence's own
    // punctuation. Alacritty strips that in its binary rather than its library,
    // so the trim lives here.
    let raw = t.bounds_to_string(*found.start(), *found.end());
    let trimmed = trim_trailing(&raw);
    if trimmed.is_empty() || !has_allowed_scheme(trimmed) {
        return None;
    }

    // The span has to shrink with the text, or the underline would run past
    // the link and the click target would include a character that is not part
    // of it. Every character `trim_trailing` removes is single-width ASCII, so
    // one dropped character is one cell.
    let mut match_end = *found.end();
    for _ in 0..(raw.len() - trimmed.len()) {
        match_end = step_left(match_end, columns, top)?;
    }

    // The point has to still be inside what is left. Hovering the period at the
    // end of a sentence is hovering the sentence, not the link.
    if match_end < point {
        return None;
    }

    Some(span(trimmed.to_string(), *found.start(), match_end, offset))
}

/// Drop trailing characters a URL almost never really ends with.
///
/// Two rules, both of which exist because the surrounding prose is not part of
/// the link:
///
/// 1. Sentence punctuation goes. `https://example.com.` at the end of a
///    sentence is a link plus a period, and opening the period gets a 404.
/// 2. A closing bracket or quote goes unless the match contains its opener.
///    `(see https://example.com)` is a link inside parentheses; a Wikipedia
///    URL like `.../Foo_(bar)` is a link that genuinely ends in one, and
///    counting the pair is what tells them apart.
fn trim_trailing(text: &str) -> &str {
    let mut end = text.len();
    loop {
        let candidate = &text[..end];
        let Some(last) = candidate.chars().next_back() else { break };
        let drop = match last {
            '.' | ',' | ':' | ';' | '!' | '?' => true,
            ')' => count(candidate, '(') < count(candidate, ')'),
            ']' => count(candidate, '[') < count(candidate, ']'),
            '}' => count(candidate, '{') < count(candidate, '}'),
            '\'' => count(candidate, '\'') % 2 == 1,
            _ => false,
        };
        if !drop {
            break;
        }
        end -= last.len_utf8();
    }
    &text[..end]
}

fn count(text: &str, needle: char) -> usize {
    text.chars().filter(|c| *c == needle).count()
}

/// Alacritty's own hint pattern, which is well tested against real output.
///
/// The trailing character class is what keeps a sentence's period, a closing
/// bracket or a shell quote out of the match.
fn pattern() -> String {
    let schemes = SCHEMES.iter().map(|s| format!("{s}:")).collect::<Vec<_>>().join("|");
    format!("({schemes})[^\u{0}-\u{1F}\u{7F}-\u{9F}<>\"\\s{{-}}\\^⟨⟩`]+")
}

/// Whether text opens with a scheme this crate is willing to hand to a
/// platform opener.
///
/// Checked against the produced TEXT rather than trusted from the pattern.
/// The pattern is a string built at runtime, and a future edit widening it
/// must not be able to widen what gets opened without this refusing.
fn has_allowed_scheme(text: &str) -> bool {
    SCHEMES.iter().any(|s| {
        text.len() > s.len() + 1
            && text.is_char_boundary(s.len())
            && text[..s.len()].eq_ignore_ascii_case(s)
            && text.as_bytes()[s.len()] == b':'
    })
}

fn span(url: String, start: Point, end: Point, offset: i32) -> UrlMatch {
    UrlMatch {
        url,
        start_row: (start.line.0 + offset).max(0) as u16,
        start_column: start.column.0 as u16,
        end_row: (end.line.0 + offset).max(0) as u16,
        end_column: end.column.0 as u16,
    }
}

fn step_left(p: Point, columns: usize, top: Line) -> Option<Point> {
    if p.column.0 > 0 {
        return Some(Point::new(p.line, Column(p.column.0 - 1)));
    }
    let previous = Line(p.line.0 - 1);
    (previous >= top).then(|| Point::new(previous, Column(columns - 1)))
}

fn step_right(p: Point, columns: usize, bottom: Line) -> Option<Point> {
    if p.column.0 + 1 < columns {
        return Some(Point::new(p.line, Column(p.column.0 + 1)));
    }
    let next = Line(p.line.0 + 1);
    (next <= bottom).then(|| Point::new(next, Column(0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Terminal;

    #[test]
    fn a_plain_url_is_found_from_any_cell_in_it() {
        let mut t = Terminal::new(60, 4);
        t.feed(b"see https://example.com/x for more");
        // "see " is four columns, so the URL spans 4..=24.
        for column in [4u16, 10, 24] {
            let found = url_at(&t, 0, column).expect("this column is inside the URL");
            assert_eq!(found.url, "https://example.com/x");
            assert_eq!((found.start_row, found.start_column), (0, 4));
        }
        assert!(url_at(&t, 0, 0).is_none(), "\"see\" is not a URL");
        assert!(url_at(&t, 0, 30).is_none(), "\"more\" is not a URL");
    }

    #[test]
    fn a_wrapped_url_is_found_whole_from_either_row() {
        // Twenty columns, so this cannot fit on one row. Agent output wraps
        // long URLs constantly; a per-row scan would return half of one, and
        // half a URL is a different URL.
        let mut t = Terminal::new(20, 4);
        t.feed(b"https://example.com/a/very/long/path");
        let from_first = url_at(&t, 0, 3).expect("found from the first row");
        let from_second = url_at(&t, 1, 3).expect("found from the second row");
        assert_eq!(from_first.url, "https://example.com/a/very/long/path");
        assert_eq!(from_first.url, from_second.url);
        assert_eq!(from_first.start_row, 0);
        assert_eq!(from_first.end_row, 1);
    }

    #[test]
    fn an_osc8_hyperlink_beats_the_text_under_it() {
        // The whole point of OSC 8 is that the visible text and the target
        // differ, so inferring from the text would open the wrong thing.
        let mut t = Terminal::new(40, 4);
        t.feed(b"\x1b]8;;https://example.com/real\x1b\\click here\x1b]8;;\x1b\\");
        let found = url_at(&t, 0, 2).expect("the hyperlink covers this cell");
        assert_eq!(found.url, "https://example.com/real");
        assert_eq!((found.start_column, found.end_column), (0, 9));
    }

    #[test]
    fn a_scheme_outside_the_allowlist_is_not_returned() {
        // Terminal output is not trusted input: an agent prints whatever it
        // read. Opening arbitrary schemes would be a local app-launch
        // primitive driven by text an attacker may have chosen.
        let mut t = Terminal::new(60, 4);
        t.feed(b"x-evil-app://run?cmd=rm");
        assert!(url_at(&t, 0, 5).is_none());
    }

    #[test]
    fn an_osc8_hyperlink_to_a_refused_scheme_is_not_returned_either() {
        // The allowlist cannot be bypassed by the program STATING a target
        // rather than printing one. Both paths go through the same check.
        let mut t = Terminal::new(40, 4);
        t.feed(b"\x1b]8;;x-evil-app://run\x1b\\click here\x1b]8;;\x1b\\");
        assert!(url_at(&t, 0, 2).is_none());
    }

    #[test]
    fn a_url_is_found_where_the_pointer_is_when_scrolled_back() {
        // Row 0 of the VIEW, not row 0 of the live grid. Getting this wrong
        // opens whatever happens to sit at the same screen position.
        let mut t = Terminal::new(40, 3);
        t.feed(b"https://example.com/old\r\n");
        for _ in 0..10 {
            t.feed(b"filler\r\n");
        }
        assert!(url_at(&t, 0, 3).is_none(), "the live view shows filler");

        t.scroll(11);
        let found = url_at(&t, 0, 3).expect("scrolled back, the URL is on view row 0");
        assert_eq!(found.url, "https://example.com/old");
    }

    #[test]
    fn a_trailing_sentence_period_is_not_part_of_the_url() {
        let mut t = Terminal::new(60, 4);
        t.feed(b"go to https://example.com.");
        let found = url_at(&t, 0, 10).expect("found");
        assert_eq!(found.url, "https://example.com");
    }

    #[test]
    fn a_url_wrapped_in_parentheses_keeps_the_ones_it_owns() {
        // Both directions of the same rule. Prose parentheses are not part of
        // the link; a Wikipedia URL that genuinely ends in one is.
        let mut t = Terminal::new(70, 4);
        t.feed(b"(see https://example.com/x)");
        assert_eq!(url_at(&t, 0, 10).expect("found").url, "https://example.com/x");

        let mut t2 = Terminal::new(70, 4);
        t2.feed(b"https://en.wikipedia.org/wiki/Foo_(bar)");
        assert_eq!(
            url_at(&t2, 0, 10).expect("found").url,
            "https://en.wikipedia.org/wiki/Foo_(bar)"
        );
    }

    #[test]
    fn the_span_shrinks_with_the_trimmed_text() {
        // Or the underline would run past the link and the click target would
        // include a character that is not part of it.
        let mut t = Terminal::new(60, 4);
        t.feed(b"go to https://example.com.");
        let found = url_at(&t, 0, 10).expect("found");
        // "go to " is six columns; the URL is 19 characters, so 6..=24.
        assert_eq!((found.start_column, found.end_column), (6, 24));
        // And the period itself is not a link.
        assert!(url_at(&t, 0, 25).is_none(), "the sentence's period is not the link");
    }

    #[test]
    fn a_cell_outside_the_grid_is_not_a_crash() {
        // A renderer bug must not take the app down, and a pointer just past
        // the last column during a resize is not even a bug.
        let t = Terminal::new(20, 4);
        assert!(url_at(&t, 99, 0).is_none());
        assert!(url_at(&t, 0, 99).is_none());
    }
}
