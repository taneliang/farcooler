# User Review Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the seven fixes from the user review in
`docs/superpowers/specs/2026-08-06-user-review-fixes-design.md`.

**Architecture:** Capability first, then surfaces. `crates/vt` gains URL
detection and OSC 52 plumbing behind its existing C ABI; `crates/core` gains a
config key; `crates/daemon` gains two behavior changes that every client and the
CLI inherit. Only then do the three apps wire up gestures, clipboards and forms
against interfaces that already exist and are already tested.

**Tech Stack:** Rust 2024 (workspace, `alacritty_terminal` 0.26, `prost`,
`rusqlite`), Swift 6 / SwiftUI + AppKit (macOS), Swift 6 / SwiftUI + UIKit
(iOS), Kotlin / Compose (Android), protobuf over a hand-rolled framed transport.

## Global Constraints

- **US English throughout**, in code and in copy. Never "authorise", "colour",
  "centre".
- **Apple copy conventions:** title-case buttons, contractions allowed,
  "machine" not "host" in user-visible copy, never a raw Rust error in the UI.
- **Never run `cargo fmt`.** This tree is hand-formatted and CI skips
  `fmt --check` on purpose. Match surrounding style by hand.
- **A live Far Cooler app is running on this machine.** Never `pkill` by
  pattern; kill scratch daemons by PID only.
- `cargo` is at `~/.cargo/bin/cargo` — not on the default PATH in this shell.
- Rust verification is `cargo clippy --workspace --all-targets -- -D warnings`
  then `cargo test --workspace`. Clippy at `-D warnings` is a gate, not advice.
- **The C ABI header is hand-maintained.** `crates/vt/include/farcooler_vt.h`
  and `crates/vt/tests/header.rs` change in the same commit as `ffi.rs`, always.
- New protobuf fields take the next free tag and are never renumbered. Swift
  clients decode fields added after first release as **optional**, per the rule
  stated at `apps/macos/Sources/FarCooler/Model.swift:26`.
- Comments explain *why*, matching the density and voice of the file being
  edited. This codebase is unusually heavily commented; a bare change reads as
  unfinished.

---

## Task 1: URL detection in the VT core

**Files:**
- Create: `crates/vt/src/url.rs`
- Modify: `crates/vt/src/lib.rs` (add `pub mod url;`, expose
  `Terminal::term_mut` if needed for `RegexSearch`)

**Interfaces:**
- Consumes: `Terminal` and its private `term()` accessor from `lib.rs`.
- Produces: `url::UrlMatch { url: String, start_row: u16, start_column: u16,
  end_row: u16, end_column: u16 }` and
  `url::url_at(term: &Terminal, row: u16, column: u16) -> Option<UrlMatch>`.
  Task 2 wraps exactly this.

- [ ] **Step 1: Write the failing tests**

In `crates/vt/src/url.rs`, at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::Terminal;

    #[test]
    fn a_plain_url_is_found_from_any_cell_in_it() {
        let mut t = Terminal::new(60, 4);
        t.feed(b"see https://example.com/x for more");
        // "see " is 4 columns, so the URL spans columns 4..=26.
        for column in [4u16, 10, 26] {
            let found = url_at(&t, 0, column).expect("column {column} is inside the URL");
            assert_eq!(found.url, "https://example.com/x");
            assert_eq!((found.start_row, found.start_column), (0, 4));
        }
        assert!(url_at(&t, 0, 0).is_none(), "\"see\" is not a URL");
        assert!(url_at(&t, 0, 30).is_none(), "\"more\" is not a URL");
    }

    #[test]
    fn a_wrapped_url_is_found_whole_from_either_row() {
        // 20 columns, so this URL cannot fit on one row. Agent output wraps
        // long URLs constantly; a per-row scan would return half of one.
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
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `~/.cargo/bin/cargo test -p farcooler-vt url::`
Expected: FAIL — `url.rs` is not yet a module, or `url_at` is not defined.

- [ ] **Step 3: Write the implementation**

`crates/vt/src/url.rs`:

```rust
//! Finding the URL under a cell.
//!
//! Two sources, in this order: an OSC 8 hyperlink the program stated, then a
//! regex sweep over the grid. The order matters — the whole point of OSC 8 is
//! that the visible text and the target differ, so text inferred from a cell
//! the program already labelled would open the wrong thing.
//!
//! The regex path uses the EMULATOR's search rather than scanning a snapshot
//! row, and that is the load-bearing choice here: long URLs wrap across rows
//! constantly in agent output, and a per-row scan finds the first half of one
//! and calls it a URL. Only the grid knows which lines continue which.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::search::{Match, RegexSearch};

use crate::Terminal;

/// A URL on screen, and where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlMatch {
    pub url: String,
    /// Display coordinates — the space `grid::snapshot` reports in — so a
    /// renderer can underline the span without converting anything.
    pub start_row: u16,
    pub start_column: u16,
    pub end_row: u16,
    pub end_column: u16,
}

/// The schemes worth opening, stated rather than inherited.
///
/// Terminal output is not trusted input. An agent working overnight prints
/// whatever it read, and some of what it read came off the internet. A URL
/// opener that honours whatever the platform will dispatch is a local
/// application-launch primitive driven by text an attacker may have chosen, so
/// the set lives here, in the one place all three renderers read it from.
const SCHEMES: &[&str] = &[
    "http", "https", "mailto", "ftp", "ftps", "ssh", "git", "news", "file",
    "gemini", "gopher", "ipfs", "ipns", "magnet",
];

/// Alacritty's own hint pattern, which is well-tested against real output.
///
/// The trailing character class is what keeps a sentence's full stop, a
/// closing bracket or a shell quote out of the match.
fn pattern() -> String {
    let schemes = SCHEMES
        .iter()
        .map(|s| format!("{s}:"))
        .collect::<Vec<_>>()
        .join("|");
    format!("({schemes})[^\u{0}-\u{1F}\u{7F}-\u{9F}<>\"\\s{{-}}\\^⟨⟩`]+")
}

/// The URL covering a cell of the CURRENT VIEW, or `None`.
pub fn url_at(term: &Terminal, row: u16, column: u16) -> Option<UrlMatch> {
    let t = term.term();
    let grid = t.grid();
    if row as usize >= grid.screen_lines() || column as usize >= grid.columns() {
        return None;
    }
    // Indexing the grid ignores the scroll position — line 0 is the top of the
    // ACTIVE region and history is at negative lines — so the offset has to be
    // applied here, exactly as `grid::snapshot` does. Without it, a scrolled
    // back view finds whatever sits at the same position on the live screen.
    let offset = grid.display_offset() as i32;
    let point = Point::new(Line(row as i32 - offset), Column(column as usize));

    hyperlink_at(term, point, offset).or_else(|| regex_at(term, point, offset))
}

/// An OSC 8 hyperlink on the cell, expanded to every neighbouring cell that
/// carries the same one.
fn hyperlink_at(term: &Terminal, point: Point, offset: i32) -> Option<UrlMatch> {
    let t = term.term();
    let grid = t.grid();
    let link = grid[point.line][point.column].hyperlink()?;
    let columns = grid.columns();

    // Walk out along the line. A hyperlink spanning rows is walked by the same
    // loop, because `Point` comparison and the grid's own indexing carry the
    // line with them.
    let same = |p: Point| {
        grid[p.line][p.column]
            .hyperlink()
            .is_some_and(|h| h.uri() == link.uri())
    };

    let mut start = point;
    while let Some(previous) = step_left(start, columns) {
        if !same(previous) {
            break;
        }
        start = previous;
    }
    let mut end = point;
    while let Some(next) = step_right(end, columns, grid.screen_lines(), offset) {
        if !same(next) {
            break;
        }
        end = next;
    }

    Some(span(link.uri().to_string(), start, end, offset))
}

/// A regex match covering the cell.
fn regex_at(term: &Terminal, point: Point, offset: i32) -> Option<UrlMatch> {
    let t = term.term();
    // Built per call rather than cached on the handle. This runs on a hover or
    // a click, not per frame, and a lazily-compiled DFA held across feeds would
    // be state to invalidate for no measurable gain.
    let mut search = RegexSearch::new(&pattern()).ok()?;

    let grid = t.grid();
    let last_line = Line(grid.screen_lines() as i32 - 1 - offset);
    let first_line = Line(-(grid.history_size() as i32));

    // Search right from the start of the visible region and left from the end,
    // then keep the match that actually covers the point. `regex_search_left`
    // alone would find a match ENDING at or before the point, which for a
    // URL the pointer is in the middle of is the wrong half.
    let found: Match = t.regex_search_left(
        &mut search,
        point,
        Point::new(first_line, Column(0)),
    )?;
    let (start, end) = (*found.start(), *found.end());
    if start > point || end < point {
        // The nearest match is not the one under the pointer. Try rightward,
        // which covers a point sitting on the match's first cell.
        let forward = t.regex_search_right(
            &mut search,
            point,
            Point::new(last_line, Column(grid.columns() - 1)),
        )?;
        let (fs, fe) = (*forward.start(), *forward.end());
        if fs > point || fe < point {
            return None;
        }
        let text = t.bounds_to_string(fs, fe);
        return has_allowed_scheme(&text).then(|| span(text, fs, fe, offset));
    }

    let text = t.bounds_to_string(start, end);
    has_allowed_scheme(&text).then(|| span(text, start, end, offset))
}

/// The regex already restricts the scheme, but the check is repeated against
/// the produced TEXT rather than trusted from the pattern: the pattern is a
/// string built at runtime, and a future edit to it must not be able to widen
/// what gets opened without this failing.
fn has_allowed_scheme(text: &str) -> bool {
    SCHEMES.iter().any(|s| {
        text.len() > s.len() + 1
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

fn step_left(p: Point, columns: usize) -> Option<Point> {
    if p.column.0 > 0 {
        return Some(Point::new(p.line, Column(p.column.0 - 1)));
    }
    Some(Point::new(Line(p.line.0 - 1), Column(columns - 1)))
}

fn step_right(p: Point, columns: usize, lines: usize, offset: i32) -> Option<Point> {
    if p.column.0 + 1 < columns {
        return Some(Point::new(p.line, Column(p.column.0 + 1)));
    }
    let next = Line(p.line.0 + 1);
    // Never past the bottom of the visible region.
    if next.0 + offset >= lines as i32 {
        return None;
    }
    Some(Point::new(next, Column(0)))
}
```

Add to `crates/vt/src/lib.rs`, beside the other module declarations:

```rust
pub mod url;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `~/.cargo/bin/cargo test -p farcooler-vt url::`
Expected: PASS, 6 tests.

If `Terminal::term()` is `pub(crate)` and `url.rs` is inside the crate, it is
reachable — no visibility change needed. If `history_size()` is not on
`Dimensions`, use `grid.total_lines() - grid.screen_lines()`.

- [ ] **Step 5: Clippy, then commit**

```bash
~/.cargo/bin/cargo clippy -p farcooler-vt --all-targets -- -D warnings
git add crates/vt/src/url.rs crates/vt/src/lib.rs
git commit -m "feat(vt): find the URL under a cell"
```

---

## Task 2: The URL across the C ABI

**Files:**
- Modify: `crates/vt/src/ffi.rs`
- Modify: `crates/vt/include/farcooler_vt.h`

**Interfaces:**
- Consumes: `url::url_at` and `url::UrlMatch` from Task 1.
- Produces: `farcooler_vt_url_at(handle, row, column, span, out, capacity)
  -> usize` and `#[repr(C)] VtUrlSpan { start_row, start_column, end_row,
  end_column: u16 }`. Tasks 11, 13 and 15 call exactly this.

- [ ] **Step 1: Write the failing tests**

In `crates/vt/src/ffi.rs`'s `mod tests`:

```rust
fn url(h: *mut c_void, row: u16, column: u16) -> Option<(String, VtUrlSpan)> {
    let mut span = VtUrlSpan { start_row: 0, start_column: 0, end_row: 0, end_column: 0 };
    let needed =
        unsafe { farcooler_vt_url_at(h, row, column, &mut span, std::ptr::null_mut(), 0) };
    if needed == 0 {
        return None;
    }
    let mut buf = vec![0u8; needed];
    let n = unsafe {
        farcooler_vt_url_at(h, row, column, &mut span, buf.as_mut_ptr(), needed)
    };
    assert_eq!(n, needed);
    Some((String::from_utf8(buf).unwrap(), span))
}

#[test]
fn a_url_crosses_the_boundary_with_its_span() {
    let h = farcooler_vt_new(60, 4);
    feed(h, b"see https://example.com/x now");

    let (text, span) = url(h, 0, 10).expect("a URL is under this cell");
    assert_eq!(text, "https://example.com/x");
    assert_eq!((span.start_row, span.start_column), (0, 4));

    assert!(url(h, 0, 0).is_none(), "no URL under \"see\"");
    unsafe { farcooler_vt_free(h) };
}

#[test]
fn asking_for_a_url_with_a_short_buffer_writes_nothing() {
    // Same contract as encode_paste: report the size, write nothing. A
    // truncated URL is a different URL, and opening one would be worse than
    // opening none.
    let h = farcooler_vt_new(60, 4);
    feed(h, b"https://example.com/x");
    let mut span = VtUrlSpan { start_row: 0, start_column: 0, end_row: 0, end_column: 0 };
    let mut one = [0u8; 1];
    let needed = unsafe {
        farcooler_vt_url_at(h, 0, 2, &mut span, one.as_mut_ptr(), 1)
    };
    assert_eq!(needed, "https://example.com/x".len());
    assert_eq!(one[0], 0, "nothing written");
    unsafe { farcooler_vt_free(h) };
}

#[test]
fn the_url_span_layout_is_what_the_renderer_expects() {
    // Renderers read these fields by offset, same as VtCell.
    assert_eq!(std::mem::size_of::<VtUrlSpan>(), 8);
    assert_eq!(std::mem::align_of::<VtUrlSpan>(), 2);
}
```

And extend the existing `null_and_empty_arguments_are_survivable` test with:

```rust
        let mut span = VtUrlSpan { start_row: 0, start_column: 0, end_row: 0, end_column: 0 };
        assert_eq!(
            farcooler_vt_url_at(null, 0, 0, &mut span, std::ptr::null_mut(), 0),
            0
        );
```

- [ ] **Step 2: Run to verify failure**

Run: `~/.cargo/bin/cargo test -p farcooler-vt ffi::tests`
Expected: FAIL — `VtUrlSpan` and `farcooler_vt_url_at` undefined.

- [ ] **Step 3: Implement**

In `crates/vt/src/ffi.rs`, after `VtSnapshot`:

```rust
/// Where a URL sits on screen, in display coordinates.
///
/// A struct rather than four out-params because the renderers underline the
/// span, and four `*mut u16` at a call site is four chances to pass them in
/// the wrong order.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VtUrlSpan {
    pub start_row: u16,
    pub start_column: u16,
    pub end_row: u16,
    pub end_column: u16,
}
```

And the function, after `farcooler_vt_alt_screen`:

```rust
/// The URL under a cell, or 0 if there is none.
///
/// Returns the byte length the URL needs and writes nothing when that exceeds
/// `capacity` — the same contract `farcooler_vt_encode_paste` uses, and for a
/// sharper reason: a truncated URL is a DIFFERENT URL, and opening one would
/// be worse than opening nothing.
///
/// `span` is filled whenever a URL is found, including on the sizing call, so
/// a renderer can underline the match before it has anywhere to put the text.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_vt_url_at(
    handle: *mut c_void,
    row: u16,
    column: u16,
    span: *mut VtUrlSpan,
    out: *mut u8,
    capacity: usize,
) -> usize {
    let Some(h) = (unsafe { as_handle(handle) }) else { return 0 };
    let Some(found) = crate::url::url_at(&h.terminal, row, column) else { return 0 };

    if !span.is_null() {
        unsafe {
            *span = VtUrlSpan {
                start_row: found.start_row,
                start_column: found.start_column,
                end_row: found.end_row,
                end_column: found.end_column,
            };
        }
    }

    let bytes = found.url.as_bytes();
    if bytes.len() > capacity || out.is_null() {
        return bytes.len();
    }
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len()) };
    bytes.len()
}
```

In `crates/vt/include/farcooler_vt.h`, beside the other declarations:

```c
/// Where a URL sits on screen, in display coordinates.
typedef struct {
  uint16_t start_row;
  uint16_t start_column;
  uint16_t end_row;
  uint16_t end_column;
} FarCoolerVtUrlSpan;

/// The URL under a cell, or 0 if there is none. Returns the byte length
/// needed; writes nothing if that exceeds `capacity`. `span` is filled
/// whenever a URL is found, sizing calls included.
size_t farcooler_vt_url_at(void *handle, uint16_t row, uint16_t column,
                           FarCoolerVtUrlSpan *span, uint8_t *out,
                           size_t capacity);
```

- [ ] **Step 4: Run tests and the header check**

Run: `~/.cargo/bin/cargo test -p farcooler-vt`
Expected: PASS, including `tests/header.rs` — which enumerates
`farcooler_vt_*` declarations and would fail if the header missed the new one.

- [ ] **Step 5: Clippy, then commit**

```bash
~/.cargo/bin/cargo clippy -p farcooler-vt --all-targets -- -D warnings
git add crates/vt/src/ffi.rs crates/vt/include/farcooler_vt.h
git commit -m "feat(vt): expose the URL under a cell across the ABI"
```

---

## Task 3: OSC 52 through the core

**Files:**
- Modify: `crates/vt/src/lib.rs` (`Signals`, `Collector`)
- Modify: `crates/vt/src/ffi.rs` (`VtHandle`, `farcooler_vt_take_clipboard`)
- Modify: `crates/vt/include/farcooler_vt.h`

**Interfaces:**
- Produces: `Signals::clipboard: Option<String>` and
  `farcooler_vt_take_clipboard(handle, out, capacity) -> usize`.
  Tasks 10, 13 and 15 call exactly this.

- [ ] **Step 1: Write the failing tests**

In `crates/vt/src/lib.rs`'s `mod tests`:

```rust
/// Base64 of "hello", as a program would send it.
const HELLO_B64: &str = "aGVsbG8=";

#[test]
fn an_osc52_copy_reaches_the_signals_once() {
    let mut t = Terminal::new(40, 6);
    t.feed(format!("\x1b]52;c;{HELLO_B64}\x07").as_bytes());
    assert_eq!(t.take_signals().clipboard.as_deref(), Some("hello"));
    assert_eq!(t.take_signals().clipboard, None, "signals are drained");
}

#[test]
fn an_osc52_copy_terminated_by_st_works_too() {
    // BEL and ESC-backslash are both legal terminators and real programs use
    // both. tmux sends ST.
    let mut t = Terminal::new(40, 6);
    t.feed(format!("\x1b]52;c;{HELLO_B64}\x1b\\").as_bytes());
    assert_eq!(t.take_signals().clipboard.as_deref(), Some("hello"));
}

#[test]
fn a_program_asking_to_read_the_clipboard_gets_no_reply() {
    // The security property, so it is a test rather than a comment. This app
    // exists to run agents on machines nobody is sitting at; one of them being
    // able to read the watching Mac's clipboard is a data path in the wrong
    // direction over a link that carries terminal output.
    let mut t = Terminal::new(40, 6);
    t.feed(b"\x1b]52;c;?\x07");
    let s = t.take_signals();
    assert!(s.pty_writes.is_empty(), "nothing may go back to the program");
    assert_eq!(s.clipboard, None);
}

#[test]
fn a_selection_clipboard_write_is_ignored() {
    // X11's PRIMARY has no analogue on any of the three platforms, and
    // treating it as a copy would let a program overwrite the clipboard
    // through a channel nobody expects one on.
    let mut t = Terminal::new(40, 6);
    t.feed(format!("\x1b]52;p;{HELLO_B64}\x07").as_bytes());
    assert_eq!(t.take_signals().clipboard, None);
}
```

In `crates/vt/src/ffi.rs`'s `mod tests`:

```rust
#[test]
fn the_clipboard_crosses_the_boundary_and_drains_once() {
    let h = farcooler_vt_new(40, 6);
    feed(h, b"\x1b]52;c;aGVsbG8=\x07");

    let needed = unsafe {
        farcooler_vt_take_clipboard(h, std::ptr::null_mut(), 0)
    };
    assert_eq!(needed, 5);

    // A short buffer must not truncate: half a copied command is worse than
    // no copy at all.
    let mut one = [0u8; 1];
    assert_eq!(unsafe { farcooler_vt_take_clipboard(h, one.as_mut_ptr(), 1) }, 5);
    assert_eq!(one[0], 0);

    let mut buf = vec![0u8; needed];
    assert_eq!(
        unsafe { farcooler_vt_take_clipboard(h, buf.as_mut_ptr(), needed) },
        5
    );
    assert_eq!(&buf, b"hello");
    assert_eq!(
        unsafe { farcooler_vt_take_clipboard(h, buf.as_mut_ptr(), needed) },
        0,
        "taking drains"
    );
    unsafe { farcooler_vt_free(h) };
}
```

Extend `null_and_empty_arguments_are_survivable` with:

```rust
        assert_eq!(farcooler_vt_take_clipboard(null, std::ptr::null_mut(), 0), 0);
```

- [ ] **Step 2: Run to verify failure**

Run: `~/.cargo/bin/cargo test -p farcooler-vt`
Expected: FAIL — no `clipboard` field, no `farcooler_vt_take_clipboard`.

- [ ] **Step 3: Implement**

`crates/vt/src/lib.rs` — add to `Signals`:

```rust
    /// Text the program asked to put on the clipboard (OSC 52).
    ///
    /// The WRITE half only. `Config::osc52` defaults to `Osc52::OnlyCopy`, so
    /// a program asking to READ the clipboard is refused by the parser and
    /// never reaches here — see the test that asserts it. Copy is a program
    /// handing you something; paste is a program taking something.
    pub clipboard: Option<String>,
```

And in `Collector::send_event`, beside the `Title` arm:

```rust
            Event::ClipboardStore(ClipboardType::Clipboard, text) => {
                // Last writer wins within one drain. A program that copies
                // twice before the client looks meant the second one.
                s.clipboard = Some(text)
            }
```

with the import:

```rust
use alacritty_terminal::term::ClipboardType;
```

`ClipboardType::Selection` falls into the existing `_ => {}` arm, which is the
intended behavior — the test above pins it.

`crates/vt/src/ffi.rs` — add to `VtHandle`:

```rust
    /// Text the program asked to put on the clipboard, drained by the caller.
    pending_clipboard: Option<String>,
```

initialise it as `None` in `farcooler_vt_new`, and collect it in
`farcooler_vt_feed` beside the title:

```rust
    if let Some(text) = signals.clipboard {
        h.pending_clipboard = Some(text);
    }
```

Then, after `farcooler_vt_take_bell`:

```rust
/// Take text the program asked to put on the clipboard (OSC 52).
///
/// Returns the byte length it needs and drains only when it fits, so a short
/// buffer cannot truncate a copy — half a copied command is worse than no
/// copy. Length is bounded by the parser's own OSC limit; a second cap
/// invented here would be a number with nothing behind it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn farcooler_vt_take_clipboard(
    handle: *mut c_void,
    out: *mut u8,
    capacity: usize,
) -> usize {
    let Some(h) = (unsafe { as_handle(handle) }) else { return 0 };
    let Some(text) = h.pending_clipboard.as_ref() else { return 0 };
    let bytes = text.as_bytes();
    if bytes.len() > capacity || out.is_null() {
        return bytes.len();
    }
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len()) };
    let n = bytes.len();
    h.pending_clipboard = None;
    n
}
```

Header:

```c
/// Take text the program asked to put on the clipboard (OSC 52). Returns the
/// byte length needed; drains only when it fits. Reading the clipboard is not
/// supported, deliberately: a program must not be able to take what it was
/// not given.
size_t farcooler_vt_take_clipboard(void *handle, uint8_t *out, size_t capacity);
```

- [ ] **Step 4: Run tests**

Run: `~/.cargo/bin/cargo test -p farcooler-vt`
Expected: PASS.

- [ ] **Step 5: Clippy, then commit**

```bash
~/.cargo/bin/cargo clippy -p farcooler-vt --all-targets -- -D warnings
git add crates/vt/src/lib.rs crates/vt/src/ffi.rs crates/vt/include/farcooler_vt.h
git commit -m "feat(vt): a program can put text on the clipboard, and only that"
```

---

## Task 4: The branch prefix in the config file

**Files:**
- Modify: `crates/core/src/config.rs`

**Interfaces:**
- Produces: `config::branch_prefix_from(path: &Path) -> String` and
  `config::load_branch_prefix() -> String`, plus
  `config::DEFAULT_BRANCH_PREFIX: &str = "feat/"`. Task 5 calls
  `load_branch_prefix`.

- [ ] **Step 1: Write the failing tests**

In `crates/core/src/config.rs`'s `mod tests`:

```rust
#[test]
fn a_branch_prefix_is_read_from_its_own_table() {
    // A table, not a top-level key: TOML puts a bare scalar written below
    // `[themes.paper]` INSIDE that table, so `prefix = "elt/"` appended to an
    // existing file would silently become a theme's property and do nothing.
    let dir = scratch("branch-prefix");
    let path = dir.join("config.toml");
    std::fs::write(&path, "[branches]\nprefix = \"elt/\"\n").unwrap();
    assert_eq!(branch_prefix_from(&path), "elt/");
}

#[test]
fn no_config_file_yields_the_default_prefix() {
    assert_eq!(
        branch_prefix_from(std::path::Path::new("/nonexistent/config.toml")),
        DEFAULT_BRANCH_PREFIX
    );
}

#[test]
fn a_file_with_no_branches_table_yields_the_default() {
    let dir = scratch("branch-none");
    let path = dir.join("config.toml");
    std::fs::write(&path, "[adapters.codex]\nprogram = \"npx\"\n").unwrap();
    assert_eq!(branch_prefix_from(&path), DEFAULT_BRANCH_PREFIX);
}

#[test]
fn an_empty_prefix_opts_out_rather_than_falling_back() {
    // The distinction that makes this customizable at all: "" is a choice, and
    // treating it as "unset" would make opting out impossible.
    let dir = scratch("branch-empty");
    let path = dir.join("config.toml");
    std::fs::write(&path, "[branches]\nprefix = \"\"\n").unwrap();
    assert_eq!(branch_prefix_from(&path), "");
}

#[test]
fn a_prefix_is_trimmed_but_otherwise_taken_literally() {
    // `elt-` is as valid a convention as `elt/`, so no slash is added or
    // removed. Only surrounding whitespace goes, which is never intentional.
    let dir = scratch("branch-literal");
    let path = dir.join("config.toml");
    std::fs::write(&path, "[branches]\nprefix = \"  elt-  \"\n").unwrap();
    assert_eq!(branch_prefix_from(&path), "elt-");
}

#[test]
fn a_malformed_file_does_not_take_the_prefix_down_with_it() {
    // The rule adapters and themes already follow.
    let dir = scratch("branch-broken");
    let path = dir.join("config.toml");
    std::fs::write(&path, "[branches\nprefix = ").unwrap();
    assert_eq!(branch_prefix_from(&path), DEFAULT_BRANCH_PREFIX);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `~/.cargo/bin/cargo test -p farcooler-core config::`
Expected: FAIL — `branch_prefix_from` undefined.

- [ ] **Step 3: Implement**

Add to `ConfigFile`:

```rust
    #[serde(default)]
    branches: ConfigBranches,
```

and the table, beside `ConfigTheme`:

```rust
/// The `[branches]` table.
///
/// A table rather than a top-level key, and that is not a style choice: TOML
/// puts a bare top-level scalar written below `[themes.paper]` inside THAT
/// table, so a `prefix = "elt/"` appended to the end of an existing config
/// file would silently become a theme's property and do nothing at all. In a
/// file whose entire purpose is being hand-edited, that is a trap worth one
/// extra line to avoid.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ConfigBranches {
    /// Absent and empty are different answers. `None` means "say nothing, use
    /// the default"; `Some("")` means "no prefix", which is what makes opting
    /// out possible at all.
    #[serde(default)]
    pub prefix: Option<String>,
}

/// What a branch name gets in front of it when nothing says otherwise.
///
/// `feat/` because that is what `NewWorkspaceSheet` already suggested, so this
/// is the default users can already see rather than a new one. The two macOS
/// creation paths disagreed — the sheet hardcoded this and QuickCreate used
/// nothing — and there cannot be a customizable default until there is one
/// default.
pub const DEFAULT_BRANCH_PREFIX: &str = "feat/";
```

Then, beside `themes_from`:

```rust
/// The prefix this host puts in front of a derived branch name.
///
/// Read per call, like `themes_from` and for the same reason: a few hundred
/// bytes of TOML in exchange for edits taking effect without a daemon restart.
/// It matters more here than for themes, because the machine settings editor
/// writes this file — a value cached at startup would not reflect its own
/// writes.
pub fn branch_prefix_from(path: &Path) -> String {
    let Ok(text) = std::fs::read_to_string(path) else {
        return DEFAULT_BRANCH_PREFIX.to_string();
    };
    let parsed: ConfigFile = match toml::from_str(&text) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "ignoring a malformed config file");
            return DEFAULT_BRANCH_PREFIX.to_string();
        }
    };
    match parsed.branches.prefix {
        Some(p) => p.trim().to_string(),
        None => DEFAULT_BRANCH_PREFIX.to_string(),
    }
}

/// The host's branch prefix, found the same way the registry is.
pub fn load_branch_prefix() -> String {
    match config_path() {
        Some(path) => branch_prefix_from(&path),
        None => DEFAULT_BRANCH_PREFIX.to_string(),
    }
}
```

- [ ] **Step 4: Run tests**

Run: `~/.cargo/bin/cargo test -p farcooler-core config::`
Expected: PASS.

- [ ] **Step 5: Clippy, then commit**

```bash
~/.cargo/bin/cargo clippy -p farcooler-core --all-targets -- -D warnings
git add crates/core/src/config.rs
git commit -m "feat(core): a host can say what its branch names start with"
```

---

## Task 5: HostSettings on the wire

**Files:**
- Modify: `proto/farcooler.proto`
- Modify: `crates/daemon/src/wire.rs`
- Modify: `crates/cli/src/main.rs` (the `workspace list --json` envelope and
  `status --json`)

**Interfaces:**
- Consumes: `config::load_branch_prefix` from Task 4.
- Produces: `v1::HostSettings { branch_prefix: String }`, reachable as
  `Host.settings`, and a `"branch_prefix"` key in `workspace list --json`.
  Tasks 12, 14 and 16 read that key.

- [ ] **Step 1: Add the proto message**

In `proto/farcooler.proto`, before `message Host`:

```protobuf
// Settings this machine's config.toml decides, as opposed to facts about it.
//
// Its own message rather than scalars on `Host` so it can grow by a field: the
// machine settings editor adds to this, and a bare scalar would need a
// migration where a nested message needs a tag. Kept distinguishable from the
// live facts beside it (`self_health`, `live_terminal_count`) for the same
// reason — one is what the machine IS, the other is what someone chose.
message HostSettings {
  // Prepended to a branch name derived from a task description. Empty opts out
  // entirely; absent means the daemon's default, which is "feat/".
  //
  // Applied by the CLIENT, not here, because the composer shows you the branch
  // it is about to create — a prefix added daemon-side would make that preview
  // a lie. The daemon still validates the finished name.
  string branch_prefix = 1;
}
```

and inside `message Host`, after field 9:

```protobuf
  HostSettings settings = 10;
```

- [ ] **Step 2: Write the failing test**

In `crates/daemon/tests/rpc_over_socket.rs`, find the existing `host.get` test
(or add one beside the other request tests):

```rust
#[tokio::test]
async fn host_get_carries_this_machines_settings() {
    // The client applies the prefix, so it has to be told what it is. Riding
    // on `host.get` rather than a call of its own because every client already
    // makes this one.
    let fixture = Fixture::start().await;
    let host = fixture.host_get().await;
    let settings = host.settings.expect("settings are always sent");
    assert_eq!(settings.branch_prefix, farcooler_core::config::DEFAULT_BRANCH_PREFIX);
}
```

Use whatever helper the file already has for a `host.get` round trip; if there
is none, follow the pattern of the nearest existing test verbatim.

- [ ] **Step 3: Run to verify failure**

Run: `~/.cargo/bin/cargo test -p farcooler-daemon host_get_carries`
Expected: FAIL — no `settings` field.

- [ ] **Step 4: Fill it in `wire.rs`**

Find the function building `v1::Host` and add:

```rust
        // Read per call rather than cached on the service, so editing
        // config.toml — or the settings editor writing it — takes effect
        // without a daemon restart. Same reasoning as `theme.list`.
        settings: Some(farcooler_protocol::v1::HostSettings {
            branch_prefix: farcooler_core::config::load_branch_prefix(),
        }),
```

If `farcooler-core` is not already a dependency of the crate holding
`wire.rs`, it is — the daemon depends on it for `derive` and `validate`.

- [ ] **Step 5: Add it to the CLI's JSON**

In `crates/cli/src/main.rs`, in `WorkspaceCmd::List`'s JSON envelope beside
`live_panes`:

```rust
                        // The Mac app reads this on every refresh, which is
                        // why it rides the envelope it already parses rather
                        // than costing a second subprocess.
                        "branch_prefix": host_facts.settings
                            .as_ref()
                            .map(|s| s.branch_prefix.clone())
                            .unwrap_or_default(),
```

And in `status --json`, beside `"platform"`:

```rust
                "branchPrefix": host_facts.settings
                    .as_ref()
                    .map(|s| s.branch_prefix.clone())
                    .unwrap_or_default(),
```

- [ ] **Step 6: Run tests and commit**

```bash
~/.cargo/bin/cargo test --workspace
~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings
git add proto/farcooler.proto crates/daemon/src/wire.rs crates/cli/src/main.rs \
        crates/daemon/tests/rpc_over_socket.rs
git commit -m "feat: a machine tells its clients what branch names start with"
```

---

## Task 6: A new worktree comes with a terminal

**Files:**
- Modify: `proto/farcooler.proto` (rename `cli_preset` → `terminal_preset`)
- Modify: `crates/daemon/src/rpc.rs` (`workspace.create`)
- Modify: `crates/cli/src/main.rs` (`WorkspaceCmd::Create` flags, and the
  two `WorkspaceCreate` literals)
- Modify: `crates/client/src/session.rs` (`create_workspace` signature)
- Modify: `crates/daemon/tests/rpc_over_socket.rs`,
  `crates/daemon/tests/stdio_transport.rs` (field rename at 3 sites)

**Interfaces:**
- Produces: `Session::create_workspace(repository, task, branch, base,
  terminal_preset: &str)` — Tasks 14 and 16 call it with `"shell"`. CLI gains
  `--terminal <preset>` (default `shell`) and `--no-terminal`.

- [ ] **Step 1: Write the failing tests**

In `crates/daemon/tests/rpc_over_socket.rs`:

```rust
#[tokio::test]
async fn creating_a_workspace_with_a_preset_opens_a_terminal_in_it() {
    // The review: "When a new worktree is created, it should just open a
    // terminal as well." Done here rather than in each client, because it is a
    // product rule and a rule implemented three times is a rule three clients
    // can disagree about.
    let fixture = Fixture::start().await;
    let repo = fixture.register_repository().await;
    let ws = fixture.create_workspace(repo, "add auth", "feat/add-auth", "shell").await;
    assert_eq!(ws.terminals.len(), 1, "the worktree came with a terminal");
}

#[tokio::test]
async fn creating_a_workspace_with_no_preset_opens_nothing() {
    // Empty means none, so every existing caller keeps its behavior and the
    // CLI's `--no-terminal` has something to mean.
    let fixture = Fixture::start().await;
    let repo = fixture.register_repository().await;
    let ws = fixture.create_workspace(repo, "add auth", "feat/add-auth", "").await;
    assert!(ws.terminals.is_empty());
}
```

Adapt the fixture helpers to the file's existing shape — it already creates
workspaces, so this is a parameter added to a helper, not a new helper.

- [ ] **Step 2: Run to verify failure**

Run: `~/.cargo/bin/cargo test -p farcooler-daemon creating_a_workspace_with`
Expected: FAIL — the field is still `cli_preset` and nothing reads it.

- [ ] **Step 3: Rename the field**

In `proto/farcooler.proto`, in `message WorkspaceCreate`:

```protobuf
  // What to run in the terminal this workspace opens with, or empty for no
  // terminal at all.
  //
  // Tag 4, previously `cli_preset`, which nothing ever read — so the wire is
  // unchanged and the name now says what it does. Empty is the compatible
  // default: every caller that passed `String::new()` keeps its behavior.
  string terminal_preset = 4;
```

Then fix the six call sites the compiler will name:
`crates/cli/src/main.rs` ×2, `crates/client/src/session.rs` ×1,
`crates/daemon/tests/rpc_over_socket.rs` ×2,
`crates/daemon/tests/stdio_transport.rs` ×1.

- [ ] **Step 4: Honor it in the daemon**

In `crates/daemon/src/rpc.rs`, in `"workspace.create"`, after the workspace
exists and before `announce_fleet_changed`:

```rust
                // A worktree with nothing running in it is a directory. The
                // client asks for a shell; `startTask` asks for nothing,
                // because it creates its own agent terminal a moment later.
                //
                // A terminal that fails to start is logged, not fatal: the
                // worktree exists and is useful, and failing the call would
                // report an error for a workspace that WAS created — which
                // sends someone looking for a worktree that is already there.
                // The returned view shows no terminals, so the failure is
                // visible without being reported as the wrong failure.
                if !p.terminal_preset.trim().is_empty() {
                    if let Err(e) =
                        svc.create_terminal(ws.id, "", p.terminal_preset.trim()).await
                    {
                        tracing::warn!(
                            workspace = %ws.id, preset = %p.terminal_preset,
                            error = ?e,
                            "the worktree was created but its terminal was not"
                        );
                    }
                }
```

Check `create_terminal`'s title argument against
`crates/daemon/src/service.rs` — it runs `validate::display_name(title)`, so
if empty is refused, pass the preset as the title, matching what
`newMainTerminal` on macOS does (`title: ""` there goes through the CLI, which
may substitute). Verify by reading `validate::display_name` and use whichever
of `""` or the preset name it accepts.

- [ ] **Step 5: Add the CLI flags**

In `crates/cli/src/main.rs`, on `WorkspaceCmd::Create`:

```rust
        /// What to run in the terminal the new worktree opens with.
        #[arg(long, default_value = "shell")]
        terminal: String,
        /// Create the worktree with no terminal at all.
        ///
        /// For a caller that is about to create its own — the Mac app's task
        /// flow does exactly this, and would otherwise leave every task with
        /// an unused shell beside its agent.
        #[arg(long, conflicts_with = "terminal")]
        no_terminal: bool,
```

and pass `terminal_preset: if no_terminal { String::new() } else { terminal }`.

- [ ] **Step 6: Run tests and commit**

```bash
~/.cargo/bin/cargo test --workspace
~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings
git add proto/farcooler.proto crates/daemon/src/rpc.rs crates/cli/src/main.rs \
        crates/client/src/session.rs crates/daemon/tests/
git commit -m "feat: a new worktree opens with a terminal in it"
```

---

## Task 7: Removing a worktree closes its terminals

**Files:**
- Modify: `crates/daemon/src/service.rs` (`remove_worktree`)

**Interfaces:** No signature change. `remove_worktree` stops returning
`DomainError::RunningProcesses`.

- [ ] **Step 1: Write the failing tests**

In `crates/daemon/tests/rpc_over_socket.rs`:

```rust
#[tokio::test]
async fn removing_a_worktree_closes_the_terminals_in_it() {
    // The review: "Deleting a worktree should just automatically close
    // existing terminals." It used to refuse and tell the user to go and do by
    // hand the thing they had just asked for.
    let fixture = Fixture::start().await;
    let repo = fixture.register_repository().await;
    let ws = fixture.create_workspace(repo, "doomed", "feat/doomed", "shell").await;
    fixture.create_terminal(ws.id, "shell").await;

    fixture.remove_worktree(ws.id, "doomed").await.expect("removal closes them");
    assert!(fixture.workspaces().await.iter().all(|w| w.id != ws.id));
}

#[tokio::test]
async fn removing_a_worktree_still_refuses_the_main_checkout() {
    // The guard that must NOT be relaxed: the main checkout is the directory
    // the user works in.
    let fixture = Fixture::start().await;
    let repo = fixture.register_repository().await;
    let main = fixture
        .workspaces()
        .await
        .into_iter()
        .find(|w| w.is_main_checkout)
        .expect("the daemon adopts the main checkout");
    assert!(fixture.remove_worktree(main.id, &main.task_name).await.is_err());
}
```

- [ ] **Step 2: Run to verify failure**

Run: `~/.cargo/bin/cargo test -p farcooler-daemon removing_a_worktree`
Expected: FAIL on the first — `RunningProcesses`. The second should already
pass, and is here to prove the relaxation did not take it with it.

- [ ] **Step 3: Replace the refusal with the action**

In `crates/daemon/src/service.rs`, in `remove_worktree`, replace:

```rust
        let view = self.workspace_view(&ws).await?;
        if view
            .terminals
            .iter()
            .any(|t| matches!(t.state(), TerminalState::Running | TerminalState::Starting))
        {
            return Err(DomainError::RunningProcesses);
        }
```

with:

```rust
        // Closing what is running here is part of removing it, not a reason to
        // refuse. The user asked for the worktree gone; telling them to go and
        // stop four terminals by hand first is telling them to do the thing
        // they just asked for.
        //
        // The guard this replaces existed to keep a directory from being
        // deleted out from under a live process, and that property is kept —
        // by killing the process first, which is a different thing from
        // skipping the check. The unhealthy-inventory refusal above is what
        // makes this safe to do at all: without it, "nothing is running here"
        // is a lie precisely when tmux is unreachable.
        //
        // Two steps per terminal, rather than one, because that is the
        // sequence that already works: `stop_terminal` kills the pane and sets
        // intent Stopped, which is what makes the `remove_terminal` that
        // follows pass its own running check. `remove_root` deletes its
        // workspaces' terminals through the same pair, for the stated reason
        // that a hand-rolled deletion beside it would orphan the pane
        // `remain-on-exit` retains.
        for term in self.store.list_terminals_for_workspace(ws.id)? {
            let _ = self.stop_terminal(term.id).await;
            self.remove_terminal(term.id).await?;
        }
```

`stop_terminal`'s result is discarded because a terminal whose pane is already
gone has nothing to stop, and that is not a reason to keep the worktree.
`remove_terminal`'s is not: a record that will not delete would leave a
foreign key pointing at a workspace about to disappear.

- [ ] **Step 4: Run tests**

Run: `~/.cargo/bin/cargo test -p farcooler-daemon`
Expected: PASS. Watch for an existing test asserting the old refusal — if one
exists, it is now wrong and should be rewritten to assert the new behavior,
with a comment saying the rule changed and why.

- [ ] **Step 5: Clippy, then commit**

```bash
~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings
git add crates/daemon/src/service.rs crates/daemon/tests/
git commit -m "feat: removing a worktree closes the terminals in it"
```

---

## Task 8: macOS — focus moves on the click

**Files:**
- Modify: `apps/macos/Sources/FarCooler/DaemonClient.swift`

**Interfaces:**
- Produces: `DaemonClient.assumeFocus(_ terminal: String, in workspace: String)`
  and a `background:` parameter on `layout(_:_:_:)`.

- [ ] **Step 1: Add the optimistic flip**

In `DaemonClient.swift`, beside `activeGroup`:

```swift
    /// Mark a pane focused locally, before the daemon has been asked.
    ///
    /// Every daemon action here spawns a `farcooler` subprocess, which connects
    /// over a socket and runs `tmux select-pane`. The focus ring, the header
    /// tint and the keyboard claim are all driven by `PaneRect.focused` — a
    /// fact the DAEMON reports — so none of them moved until that whole round
    /// trip finished: locally a fork, an exec and a socket connect, and over
    /// ssh all of that plus the link. That delay is what the review noticed.
    ///
    /// So the answer is assumed and then confirmed. tmux remains the only
    /// authority: the reply replaces this wholesale a moment later, and a
    /// FAILED call re-reads rather than leaving the assumption standing — see
    /// `focusPane`.
    func assumeFocus(_ terminal: String, in workspace: String) {
        guard var groups = layouts[workspace],
            let index = groups.firstIndex(where: { $0.terminals.contains(terminal) })
        else { return }

        for g in groups.indices {
            // The pane's own group comes forward with it, which is what
            // `layout focus` does on the daemon side too.
            groups[g].active = g == index
            for p in groups[g].panes.indices {
                groups[g].panes[p].focused =
                    g == index && groups[g].panes[p].id == terminal
            }
        }
        layouts[workspace] = groups
    }
```

`PaneGroup` and `PaneRect` are `struct`s decoded from JSON with `var`
properties, so this mutates in place. If either field is `let`, change it to
`var` — they are already `var` per `Layout.swift`.

- [ ] **Step 2: Thread `background` through `layout`**

Change the signature and the one `run` call inside it:

```swift
    /// `background` skips the `busy` toggle, which is a `@Published` change
    /// that re-evaluates the whole view tree — terminal surface included — on
    /// every call. Only the focus paths pass true: a split or a preset change
    /// genuinely is the app doing something the user should see it doing, and
    /// `busy` is how it says so.
    @discardableResult
    func layout(
        _ workspace: Workspace, _ path: [String], _ rest: [String] = [],
        background: Bool = false
    ) async -> [PaneGroup] {
        let command = ["layout"] + path + [workspace.short] + rest
        guard let data = await run(command + ["--json"], background: background) else {
            return layouts[workspace.id] ?? []
        }
        ...
```

- [ ] **Step 3: Make the three focus paths optimistic**

```swift
    /// Focus a pane, which also brings its layout to the front.
    @discardableResult
    func focusPane(_ terminal: String, in workspace: Workspace) async -> [PaneGroup] {
        assumeFocus(terminal, in: workspace.id)
        let groups = await layout(workspace, ["focus"], [terminal], background: true)
        // The one new failure mode the optimism introduces, and the one thing
        // here that must not be left implicit: `layout` returns the LOCAL copy
        // when the command fails, which is now the copy carrying the
        // assumption — so a failed focus would leave the ring on a pane that
        // never got it. Re-read the truth instead.
        if groups.isEmpty || groups.first(where: { $0.focused == terminal }) == nil {
            await refreshLayout(workspace)
            return layouts[workspace.id] ?? []
        }
        return groups
    }

    @discardableResult
    func focusPane(step: String, in workspace: Workspace) async -> [PaneGroup] {
        // `--next`/`--prev` step through a pane order the app already holds,
        // so the target is knowable locally and the assumption is as safe as
        // it is for a named pane.
        if let group = activeGroup(workspace.id), !group.panes.isEmpty,
            let current = group.panes.firstIndex(where: \.focused)
        {
            let delta = step == "--prev" ? -1 : 1
            let next = (current + delta + group.panes.count) % group.panes.count
            assumeFocus(group.panes[next].id, in: workspace.id)
        }
        return await layout(workspace, ["focus"], [step], background: true)
    }

    @discardableResult
    func focusPane(number: Int, in workspace: Workspace) async -> [PaneGroup] {
        if let group = activeGroup(workspace.id), number >= 1, number <= group.panes.count {
            assumeFocus(group.panes[number - 1].id, in: workspace.id)
        }
        return await layout(workspace, ["focus"], ["--pane", "\(number)"], background: true)
    }
```

- [ ] **Step 4: Build and check by hand**

```bash
apps/macos/build-app.sh
```

Then, in the running app: split a pane, click between panes and confirm the
ring and header tint move on the click rather than after it. `⌃H`/`⌃L`,
`⌃B o` and `⌃B 1`/`⌃B 2` behave the same way.

- [ ] **Step 5: Commit**

```bash
git add apps/macos/Sources/FarCooler/DaemonClient.swift
git commit -m "fix(macos): focus moves when you click, not when tmux answers"
```

---

## Task 9: macOS — collapsing a repo in the sidebar

**Files:**
- Modify: `apps/macos/Sources/FarCooler/Preferences.swift`
- Modify: `apps/macos/Sources/FarCooler/SidebarViews.swift` (`ProjectHeader`)
- Modify: `apps/macos/Sources/FarCooler/ContentView.swift` (the sidebar body)

**Interfaces:**
- Produces: `Preferences.isProjectCollapsed(_ key: String) -> Bool` and
  `Preferences.toggleProject(_ key: String)`; `ProjectHeader` gains
  `isCollapsed: Bool` and `onToggleCollapse: (() -> Void)?`.

- [ ] **Step 1: Persist the set**

In `Preferences.swift`:

```swift
    /// Which projects are collapsed in the sidebar, by `groupKey`.
    ///
    /// Persisted, unlike `hiddenExpanded`'s `@State`, and deliberately:
    /// collapsing a repository is a statement about how you want the sidebar to
    /// look, and one that reset every launch would have to be re-made every
    /// launch.
    ///
    /// Newline-joined because `groupKey` already uses `\u{1}` as its own
    /// separator, and neither a machine name nor a project display name can
    /// contain a newline.
    @AppStorage("sidebar.collapsedProjects") private var collapsedProjects = ""

    func isProjectCollapsed(_ key: String) -> Bool {
        collapsedProjects.split(separator: "\n").contains(Substring(key))
    }

    func toggleProject(_ key: String) {
        var keys = collapsedProjects.split(separator: "\n").map(String.init)
        if let at = keys.firstIndex(of: key) {
            keys.remove(at: at)
        } else {
            keys.append(key)
        }
        collapsedProjects = keys.joined(separator: "\n")
        revision += 1
    }
```

`revision += 1` matches how `fontName` and `fontSize` already notify — check
the file and follow whichever mechanism those use.

- [ ] **Step 2: Add the chevron to the header**

In `ProjectHeader`, add the two properties and put the chevron in the gutter
the header already pads by:

```swift
    /// Whether this project's worktrees are hidden. `nil` for a silent host's
    /// placeholder, which has no worktrees to collapse.
    var isCollapsed: Bool = false
    var onToggleCollapse: (() -> Void)?
```

Replace `.padding(.leading, SidebarGrid.gutter)` on the `HStack` with a
leading chevron occupying that column:

```swift
            HStack(spacing: 6) {
                // The chevron goes in `SidebarGrid.gutter` — the column this
                // header already padded by and left empty — so collapsing
                // costs no horizontal space and lands in the same column every
                // other disclosure in the sidebar uses.
                if let onToggleCollapse {
                    Button(action: onToggleCollapse) {
                        Image(systemName: "chevron.right")
                            .font(.system(size: 9, weight: .semibold))
                            .rotationEffect(.degrees(isCollapsed ? 0 : 90))
                            .foregroundStyle(.tertiary)
                            .frame(width: SidebarGrid.gutter, alignment: .leading)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                } else {
                    Spacer().frame(width: SidebarGrid.gutter)
                }

                Text(name.uppercased())
                ...
```

and delete the trailing `.padding(.leading, SidebarGrid.gutter)`. The whole
row also toggles, since a section label is a big easy target:

```swift
        .contentShape(Rectangle())
        .onTapGesture { onToggleCollapse?() }
```

Place that *before* `.onHover`, and confirm the `+` and `…` buttons still take
their own clicks — `Button` inside a tapped container wins on macOS, but check
it in the app.

- [ ] **Step 3: Gate the rows in ContentView**

In the sidebar's `ForEach(groups)`, add the two arguments and wrap the rows:

```swift
                                isCollapsed: preferences.isProjectCollapsed(key),
                                onToggleCollapse: isSilentHost
                                    ? nil : { preferences.toggleProject(key) },
```

then:

```swift
                            if !preferences.isProjectCollapsed(key) {
                                ForEach(group.shown) { ws in
                                    WorkspaceSection( ... )
                                }
                                if !group.hidden.isEmpty {
                                    HiddenWorktrees( ... )
                                }
                            }
```

`ContentView` needs `@ObservedObject private var preferences = Preferences.shared`
if it does not already have one.

- [ ] **Step 4: Build and check by hand**

```bash
apps/macos/build-app.sh
```

Collapse a project; its worktrees and its Hidden section go, the count stays.
Relaunch: still collapsed. The `+` menu and the `…` menu still open.

- [ ] **Step 5: Commit**

```bash
git add apps/macos/Sources/FarCooler/Preferences.swift \
        apps/macos/Sources/FarCooler/SidebarViews.swift \
        apps/macos/Sources/FarCooler/ContentView.swift
git commit -m "feat(macos): a repo you are not working in can get out of the way"
```

---

## Task 10: macOS — OSC 52 onto the pasteboard

**Files:**
- Modify: `apps/macos/Sources/FarCooler/VTCore.swift`
- Modify: `apps/macos/Sources/FarCooler/TerminalRenderView.swift` (`tick`)

**Interfaces:**
- Consumes: `farcooler_vt_take_clipboard` from Task 3.
- Produces: `VTCore.takeClipboard() -> String?`.

- [ ] **Step 1: Add the accessor**

In `VTCore.swift`, following whatever `takePendingWrites` does:

```swift
    /// Text the program asked to put on the clipboard (OSC 52), or nil.
    ///
    /// Two calls: one to size, one to take. The core writes nothing when the
    /// buffer is short rather than truncating, because half a copied command
    /// is worse than no copy at all.
    func takeClipboard() -> String? {
        let needed = farcooler_vt_take_clipboard(handle, nil, 0)
        guard needed > 0 else { return nil }
        var buffer = [UInt8](repeating: 0, count: needed)
        let written = buffer.withUnsafeMutableBufferPointer {
            farcooler_vt_take_clipboard(handle, $0.baseAddress, needed)
        }
        guard written == needed else { return nil }
        return String(decoding: buffer, as: UTF8.self)
    }
```

Match the file's existing FFI idiom exactly — if `takePendingWrites` uses a
different pointer dance or a stored `handle` name, follow that.

- [ ] **Step 2: Drain it on the frame tick**

In `TerminalRenderView.tick()`, beside the bell:

```swift
        if core.takeBell() { NSSound.beep() }
        // OSC 52: the program handing you something. Drained on the tick with
        // the bell and the pty replies, because it arrives the same way they do
        // — as a side effect of feeding bytes.
        if let copied = core.takeClipboard() {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(copied, forType: .string)
        }
```

- [ ] **Step 3: Build and check by hand**

```bash
apps/macos/build-app.sh
```

In a terminal in the app:

```bash
printf '\033]52;c;%s\a' "$(printf 'from osc52' | base64)"
```

Then ⌘V somewhere. It should paste `from osc52`.

- [ ] **Step 4: Commit**

```bash
git add apps/macos/Sources/FarCooler/VTCore.swift \
        apps/macos/Sources/FarCooler/TerminalRenderView.swift
git commit -m "feat(macos): a program can put text on the clipboard"
```

---

## Task 11: macOS — ⌘-click a URL

**Files:**
- Modify: `apps/macos/Sources/FarCooler/VTCore.swift`
- Modify: `apps/macos/Sources/FarCooler/TerminalRenderView.swift`

**Interfaces:**
- Consumes: `farcooler_vt_url_at` and `VtUrlSpan` from Task 2.
- Produces: `VTCore.url(atRow:column:) -> (url: String, span: FarCoolerVtUrlSpan)?`.

- [ ] **Step 1: Add the accessor**

```swift
    /// The URL under a cell, with where it sits so it can be underlined.
    func url(atRow row: Int, column: Int) -> (url: String, span: FarCoolerVtUrlSpan)? {
        guard row >= 0, column >= 0 else { return nil }
        var span = FarCoolerVtUrlSpan(start_row: 0, start_column: 0, end_row: 0, end_column: 0)
        let needed = farcooler_vt_url_at(
            handle, UInt16(row), UInt16(column), &span, nil, 0)
        guard needed > 0 else { return nil }
        var buffer = [UInt8](repeating: 0, count: needed)
        let written = buffer.withUnsafeMutableBufferPointer {
            farcooler_vt_url_at(handle, UInt16(row), UInt16(column), &span, $0.baseAddress, needed)
        }
        guard written == needed else { return nil }
        return (String(decoding: buffer, as: UTF8.self), span)
    }
```

- [ ] **Step 2: Track the pointer and the modifier**

In `TerminalRenderView`, add state and a tracking area. The view has none
today, which is why it cannot currently know where the pointer is:

```swift
    // MARK: - Links
    //
    // ⌘-click, not shift-click as the review asked for. Shift is already the
    // selection override in `mouseDown` — the only way to copy text out of a
    // full-screen program that has grabbed the mouse — so overloading it would
    // make one gesture mean two things depending on what happened to be under
    // the pointer, and would cost the ability to start a selection on a line
    // containing a URL. In agent output that is most lines. ⌘-click is what
    // Terminal.app and iTerm2 use, so it is the gesture people already have.

    /// The link under the pointer while ⌘ is held, and where it sits.
    private var hoveredLink: (url: String, span: FarCoolerVtUrlSpan)?
    private var trackingArea: NSTrackingArea?
```

In `updateTrackingAreas`:

```swift
    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let existing = trackingArea { removeTrackingArea(existing) }
        let area = NSTrackingArea(
            rect: bounds,
            options: [.mouseMoved, .mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect],
            owner: self)
        addTrackingArea(area)
        trackingArea = area
    }
```

And the three handlers:

```swift
    override func mouseMoved(with event: NSEvent) {
        updateHoveredLink(for: event)
    }

    override func mouseExited(with event: NSEvent) {
        setHoveredLink(nil)
    }

    override func flagsChanged(with event: NSEvent) {
        // ⌘ pressed or released without the pointer moving still has to
        // underline or un-underline what is under it.
        super.flagsChanged(with: event)
        updateHoveredLink(for: event)
    }

    /// The link under the pointer, or nil when ⌘ is not held.
    private func updateHoveredLink(for event: NSEvent) {
        guard event.modifierFlags.contains(.command) else {
            setHoveredLink(nil)
            return
        }
        let point = cell(for: event)
        setHoveredLink(core.url(atRow: point.row, column: point.column))
    }

    private func setHoveredLink(_ link: (url: String, span: FarCoolerVtUrlSpan)?) {
        let changed = link?.url != hoveredLink?.url
        hoveredLink = link
        if link != nil {
            NSCursor.pointingHand.set()
        } else {
            NSCursor.iBeam.set()
        }
        if changed { needsDisplay = true }
    }
```

`cell(for:)` takes an `NSEvent` and already exists. `flagsChanged` delivers an
event whose `locationInWindow` is current, so this works for a bare modifier
press.

- [ ] **Step 3: Draw the underline**

In `draw(_:)`, inside the `withSnapshot` block, after `drawGlyphs`:

```swift
            drawLinkUnderline(snapshot, in: context)
```

and the method:

```swift
    /// Underline the ⌘-hovered link, on both rows when it wraps.
    private func drawLinkUnderline(_ snapshot: VTSnapshot, in context: CGContext) {
        guard let link = hoveredLink else { return }
        let first = max(0, Int(link.span.start_row))
        let last = min(snapshot.rows - 1, Int(link.span.end_row))
        guard first <= last else { return }

        context.setStrokeColor(Palette.cgColor(Palette.backgroundPacked).copy(alpha: 0) ?? .clear)
        for row in first...last {
            let from = row == first ? Int(link.span.start_column) : 0
            let to = row == last ? Int(link.span.end_column) : snapshot.columns - 1
            guard to >= from else { continue }
            let start = origin(row: row, column: from)
            let y = start.y + cellHeight - 1.5
            let cell = snapshot[row, min(from, snapshot.columns - 1)]
            context.setStrokeColor(Palette.cgColor(effectiveForeground(cell)))
            context.setLineWidth(1)
            context.move(to: CGPoint(x: start.x, y: y))
            context.addLine(to: CGPoint(x: origin(row: row, column: to + 1).x, y: y))
            context.strokePath()
        }
    }
```

Note this draws in the *unflipped* space `drawBackgrounds` uses, not the
text-space `drawGlyphs` flips into — so it goes outside that method, and `y`
is measured downward like `origin` reports. Delete the stray first
`setStrokeColor` line above the loop; it is dead.

- [ ] **Step 4: Open it on ⌘-click**

At the very top of `mouseDown`, before `claimKeyboard()`:

```swift
        // Before anything else, so a program tracking the mouse never sees the
        // click. Only these schemes, and only on an explicit modifier plus
        // click: terminal output is not trusted input — an agent prints
        // whatever it read — and the allowlist lives in the Rust core, where
        // all three renderers read it from.
        if event.modifierFlags.contains(.command) {
            let point = cell(for: event)
            if let link = core.url(atRow: point.row, column: point.column),
                let url = URL(string: link.url)
            {
                NSWorkspace.shared.open(url)
                return
            }
        }
```

- [ ] **Step 5: Build and check by hand**

```bash
apps/macos/build-app.sh
```

`echo "see https://example.com/x now"` in a pane. Hold ⌘: the URL underlines
and the cursor becomes a hand. ⌘-click opens it. Shift-drag still selects.
Resize the pane so the URL wraps and confirm both rows underline. Then
`printf 'x-evil://run\n'` and confirm ⌘-hover does nothing.

- [ ] **Step 6: Commit**

```bash
git add apps/macos/Sources/FarCooler/VTCore.swift \
        apps/macos/Sources/FarCooler/TerminalRenderView.swift
git commit -m "feat(macos): hold command to see the links, click to open one"
```

---

## Task 12: macOS — the prefix, the terminal, and the removal copy

**Files:**
- Modify: `apps/macos/Sources/FarCooler/Model.swift` (`Fleet.branchPrefix`)
- Modify: `apps/macos/Sources/FarCooler/DaemonClient.swift`
  (`createWorkspace`, `startTask`)
- Modify: `apps/macos/Sources/FarCooler/QuickCreate.swift` (`Branch.slug`)
- Modify: `apps/macos/Sources/FarCooler/Sheets.swift`
  (`suggestedBranch`, `RemoveWorkspaceSheet`)
- Modify: `apps/macos/Sources/FarCooler/ContentView.swift` (pass the prefix)

**Interfaces:**
- Consumes: `branch_prefix` in `workspace list --json` from Task 5;
  `--terminal`/`--no-terminal` from Task 6; the relaxed removal from Task 7.
- Produces: `Branch.slug(from:prefix:)`.

- [ ] **Step 1: Decode the prefix**

In `Model.swift`, in `Fleet`:

```swift
    /// What this machine says branch names start with.
    ///
    /// Optional, per the rule above: a client meeting an older daemon must not
    /// fail to decode the whole fleet over one absent key.
    var branchPrefix: String?
```

and in `CodingKeys`: `case branchPrefix = "branch_prefix"`. Update
`Fleet.empty` accordingly.

- [ ] **Step 2: Apply it in both creation paths**

In `QuickCreate.swift`, `Branch.slug` gains a prefix parameter:

```swift
    /// A git-safe slug, behind whatever this machine says branches start with.
    ///
    /// The prefix is applied HERE, client-side, and not by the daemon —
    /// because the composer below shows you the branch it is about to create,
    /// and a prefix added daemon-side would make that preview a lie. The
    /// daemon still validates the finished name, which is the check that
    /// actually protects git.
    static func slug(from text: String, prefix: String = "") -> String {
        ...
        let body = out.isEmpty ? "task" : out
        return prefix + body
    }
```

Keep the existing 48-character budget measured on the body, not on the
prefixed result — a long prefix must not eat the readable part.

`QuickCreate` gains `let branchPrefix: String` and uses
`Branch.slug(from: text, prefix: branchPrefix)` in its `branch` property, so
the preview under the composer shows the real thing.

In `Sheets.swift`, `NewWorkspaceSheet` gains `let branchPrefix: String` and
`suggestedBranch` stops hardcoding:

```swift
        return slug.isEmpty ? "" : branchPrefix + slug
```

with its `TextField` prompt following the same value.

Both call sites in `ContentView.swift` pass
`store.client(for:)?.fleet.branchPrefix ?? ""` — or for QuickCreate, the
prefix of the machine holding the selected project, since that is the machine
the branch will be created on.

- [ ] **Step 3: Ask for a terminal, or explicitly not**

In `DaemonClient.swift`:

```swift
    func createWorkspace(repo: String, task: String, branch: String, base: String) async -> String? {
        let failure = await runReportingError([
            "workspace", "create", repo, task, "--branch", branch, "--base", base,
            // A worktree with nothing running in it is a directory. `shell`
            // rather than an agent: this is the manual path, and `startTask` is
            // the one that starts an agent.
            "--terminal", "shell",
        ])
        await refresh()
        return failure
    }
```

and in `startTask`, the `workspace create` call gains `"--no-terminal"`, with:

```swift
        // No shell here: this creates its own agent terminal a moment below,
        // and a worktree opening with both would leave every task with an
        // unused shell beside its agent.
```

- [ ] **Step 4: Rewrite the removal sheet's copy**

In `Sheets.swift`, `RemoveWorkspaceSheet`: delete the `hasRunningTerminals`
branch that instructs the user, and make the neutral callout state what the
button does. Keep `hasRunningTerminals` as a *count* source:

```swift
                Callout(
                    icon: "info.circle.fill",
                    tone: .neutral,
                    text: runningCount == 0
                        ? "This deletes the working directory. The branch is kept, and nothing "
                            + "already committed or pushed is touched."
                        : "This closes \(runningCount) "
                            + (runningCount == 1 ? "terminal" : "terminals")
                            + " and deletes the working directory. The branch is kept, and "
                            + "nothing already committed or pushed is touched."
                )
```

Remove `.disabled(hasRunningTerminals)` from the confirm path and the
`TextField`. Rename the parameter `hasRunningTerminals: Bool` to
`runningCount: Int` and update `ContentView`'s call site to count rather than
test:

```swift
                runningCount: ws.terminals.filter {
                    let kind = StateKind.parse($0.state)
                    return kind == .running || kind == .starting
                }.count
```

- [ ] **Step 5: Build and check by hand**

```bash
apps/macos/build-app.sh
```

New Workspace: the branch field suggests `feat/<slug>`, and creating one lands
you in a shell. QuickCreate's preview shows `feat/<slug>`. Put
`[branches]\nprefix = "elt/"` in `~/.config/farcooler/config.toml`, relaunch,
and both show `elt/`. Remove a worktree with two terminals running: the sheet
says it will close two, and it does.

- [ ] **Step 6: Commit**

```bash
git add apps/macos/Sources/FarCooler/
git commit -m "feat(macos): one branch prefix, a terminal on creation, and removal that closes them"
```

---

## Task 13: iOS — OSC 52 and long-press a link

**Files:**
- Modify: `apps/ios/FarCooler/VTCore.swift`
- Modify: `apps/ios/FarCooler/TerminalView.swift`

**Interfaces:**
- Consumes: `farcooler_vt_take_clipboard`, `farcooler_vt_url_at` from Tasks 2–3.
- Produces: `VTCore.takeClipboard()`, `VTCore.url(atRow:column:)` — the same
  two accessors as macOS Tasks 10–11, so port them rather than reinventing.

- [ ] **Step 1: Rebuild the frameworks so the new symbols exist**

```bash
./scripts/build-ios-frameworks.sh
```

- [ ] **Step 2: Port the two accessors**

`apps/ios/FarCooler/VTCore.swift` gets `takeClipboard()` and
`url(atRow:column:)` verbatim from Task 10 Step 1 and Task 11 Step 1, adjusted
to this file's handle idiom.

- [ ] **Step 3: Drain the clipboard on the tick**

Wherever `TerminalView`'s render loop drains the bell and pending writes:

```swift
        if let copied = core.takeClipboard() {
            UIPasteboard.general.string = copied
        }
```

This is the only way to get text out of a terminal on this platform — there is
no selection here — so it matters more than it does on the Mac. Say so in a
comment.

- [ ] **Step 4: Add the long-press**

The terminal grid has a `UITapGestureRecognizer` for focus and a
`UIPanGestureRecognizer` for scrolling, and no long-press — so this adds one
rather than displacing anything:

```swift
        // Tap already means focus and cannot be made ambiguous, so the link
        // actions go on a gesture with nothing to lose. Android long-presses
        // to paste and keeps doing so; this platform has no terminal paste to
        // preserve, so a long-press away from a link does nothing.
        let hold = UILongPressGestureRecognizer(target: self, action: #selector(handleHold))
        addGestureRecognizer(hold)
```

```swift
    @objc private func handleHold(_ recognizer: UILongPressGestureRecognizer) {
        guard recognizer.state == .began else { return }
        let point = recognizer.location(in: self)
        guard let cell = self.cell(at: point),
            let link = core.url(atRow: cell.row, column: cell.column)
        else { return }
        onLink?(link.url)
    }
```

with `var onLink: ((String) -> Void)?` on the view, and the SwiftUI wrapper
presenting a confirmation dialog:

```swift
        .confirmationDialog(link ?? "", isPresented: linkPresented, titleVisibility: .visible) {
            Button("Open Link") { if let l = link, let u = URL(string: l) { openURL(u) } }
            Button("Copy Link") { UIPasteboard.general.string = link }
            Button("Cancel", role: .cancel) {}
        }
```

Convert a touch point to a cell with whatever the pan handler already uses; if
there is no such helper, add one beside it rather than duplicating the
arithmetic.

- [ ] **Step 5: Build**

```bash
apps/ios/generate-project.py
xcodebuild -project apps/ios/FarCooler.xcodeproj -scheme FarCooler \
  -destination 'generic/platform=iOS Simulator' -configuration Debug ARCHS=arm64 build
```

- [ ] **Step 6: Commit**

```bash
git add apps/ios/FarCooler/
git commit -m "feat(ios): a program can copy, and a long press opens a link"
```

---

## Task 14: iOS — the prefix and a terminal on creation

**Files:**
- Modify: `apps/ios/FarCooler/Model.swift` or `Store.swift` (wherever host
  facts land)
- Modify: `apps/ios/FarCooler/Connection.swift` (`createWorkspace`)
- Modify: `apps/ios/FarCooler/QuickTask.swift` (`slug`)
- Modify: `apps/ios/FarCooler/FleetView.swift` (`NewWorkspaceView`)

- [ ] **Step 1: Carry the prefix**

iOS talks to the daemon through `crates/client`, so the prefix arrives on
`Host.settings.branch_prefix` rather than through JSON. Find where `Connection`
stores host facts (`connection?.daemon` is read by `Settings.swift`) and store
`branchPrefix` beside it, defaulting to `"feat/"` when the field is absent so
an older daemon behaves like the new default rather than like no prefix.

- [ ] **Step 2: Apply it**

`QuickTask.slug` gains `prefix: String = ""` and returns `prefix + body`,
exactly as macOS's `Branch.slug` does in Task 12 Step 2 — same 48-character
budget on the body. Both call sites pass the connection's prefix.
`NewWorkspaceView`'s branch suggestion uses it too.

- [ ] **Step 3: Ask for a shell**

`Connection.createWorkspace` passes `terminalPreset: "shell"` through to
`Session::create_workspace`'s new parameter from Task 6. If iOS's quick-task
flow creates its own agent terminal afterwards, that path passes `""` for the
same reason macOS's `startTask` passes `--no-terminal`.

- [ ] **Step 4: Build and commit**

```bash
apps/ios/generate-project.py
xcodebuild -project apps/ios/FarCooler.xcodeproj -scheme FarCooler \
  -destination 'generic/platform=iOS Simulator' -configuration Debug ARCHS=arm64 build
git add apps/ios/FarCooler/
git commit -m "feat(ios): follow the machine's branch prefix, and open a terminal"
```

---

## Task 15: Android — OSC 52 and long-press a link

**Files:**
- Modify: `apps/android/app/src/main/java/com/farcooler/core/VtCore.kt`
- Modify: `apps/android/app/src/main/java/com/farcooler/core/Native.kt`
- Modify: `apps/android/app/src/main/java/com/farcooler/ui/TerminalCanvas.kt`
- Modify: `apps/android/app/src/main/java/com/farcooler/ui/TerminalScreen.kt`

- [ ] **Step 1: Rebuild the native libraries**

```bash
./scripts/build-android-libs.sh
```

- [ ] **Step 2: Declare and wrap the two new symbols**

`Native.kt` gains the two external declarations following its existing idiom;
`VtCore.kt` gains `takeClipboard(): String?` and
`urlAt(row: Int, column: Int): VtUrl?`, both with the size-then-take dance the
C contract requires.

- [ ] **Step 3: Drain the clipboard**

Where `TerminalScreen` drains the bell and pending writes:

```kotlin
        // OSC 52. Like iOS, this is the only way to get text out of a terminal
        // here — there is no selection — so it matters more than on the Mac.
        session.takeClipboard()?.let { scope.launch { clipboard.writeText("Far Cooler", it) } }
```

- [ ] **Step 4: Long-press over a link**

`TerminalCanvas`'s `detectTapGestures(onTap = …, onLongPress = …)` already
exists. `onLongPress` gains the offset it is handed, converted to a cell:

```kotlin
                detectTapGestures(
                    onTap = { onTap() },
                    onLongPress = { offset -> onLongPress(cellAt(offset)) })
```

and `TerminalScreen` decides between the two meanings:

```kotlin
                    // A long press over a link offers the link actions; a long
                    // press anywhere else keeps pasting, which is this
                    // platform's only way to get text INTO a terminal.
                    onLongPress = { cell ->
                        val link = session.urlAt(cell.row, cell.column)
                        if (link != null) {
                            linkSheet = link
                        } else {
                            scope.launch { clipboard.readText()?.let { session.paste(it) } }
                        }
                    },
```

with a `ModalBottomSheet` (or `AlertDialog`, matching whatever `Sheets.kt`
already uses) offering **Open Link** and **Copy Link**.

- [ ] **Step 5: Build and commit**

```bash
apps/android/gradlew -p apps/android assembleDebug
git add apps/android/
git commit -m "feat(android): a program can copy, and a long press opens a link"
```

---

## Task 16: Android — the prefix and a terminal on creation

**Files:**
- Modify: `apps/android/app/src/main/java/com/farcooler/net/Connection.kt`
- Modify: `apps/android/app/src/main/java/com/farcooler/model/QuickTask.kt`
- Modify: `apps/android/app/src/main/java/com/farcooler/ui/Sheets.kt`

- [ ] **Step 1: Carry, apply, and ask**

The same three changes as iOS Task 14: store `branchPrefix` from
`Host.settings` defaulting to `"feat/"`, give `TaskSlug.slug` a prefix
parameter applied to the body, and pass `"shell"` as the terminal preset from
the create sheet (`""` from any path that creates its own agent terminal).
`Sheets.kt:237` and `Sheets.kt:310` are the two `TaskSlug.slug` call sites.

- [ ] **Step 2: Build and commit**

```bash
apps/android/gradlew -p apps/android assembleDebug
git add apps/android/
git commit -m "feat(android): follow the machine's branch prefix, and open a terminal"
```

---

## Task 17: Documentation

**Files:**
- Modify: `README.md` (the config.toml section, if it documents keys)
- Create or modify: `docs/farcooler-design.md` (the OSC 52 and URL decisions)

- [ ] **Step 1: Document the config key**

Wherever `config.toml`'s tables are documented, add `[branches] prefix` with
its default and what empty means.

- [ ] **Step 2: Record the two security postures**

OSC 52's read half being off, and the URL scheme allowlist, both belong
somewhere a future reader will look before "improving" them. The spec has the
reasoning; the design doc should carry a sentence pointing at it.

- [ ] **Step 3: Full verification, then commit**

```bash
~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings
~/.cargo/bin/cargo test --workspace
apps/macos/build-app.sh
swift test --package-path apps/shared/AgentKit
git add README.md docs/
git commit -m "docs: record the review fixes and the two postures they set"
```

---

## Self-review

**Spec coverage.** Item 1 → Tasks 1, 2, 11, 13, 15. Item 2 → Tasks 3, 10, 13,
15. Item 3 → Task 9. Item 4 → Task 8. Item 5 → Tasks 6, 12, 14, 16. Item 6 →
Tasks 4, 5, 12, 14, 16. Item 7 → Tasks 7, 12. Every *Testing* bullet in the
spec appears in a task. The spec's *Not doing* list adds nothing to implement.

**Type consistency.** `url_at` → `farcooler_vt_url_at` → `VTCore.url(atRow:column:)`;
`Signals.clipboard` → `farcooler_vt_take_clipboard` → `VTCore.takeClipboard()`;
`branch_prefix_from`/`load_branch_prefix` → `HostSettings.branch_prefix` →
`Fleet.branchPrefix` → `Branch.slug(from:prefix:)`. `terminal_preset` is the
single spelling from proto through CLI. `VtUrlSpan` is the Rust name and
`FarCoolerVtUrlSpan` the C/Swift one, matching how `VtCell` already appears as
`FarCoolerVtCell` in Swift.

**Known imprecision, flagged rather than hidden.** Tasks 13–16 name files and
the shape of each change but do not paste final code for the iOS and Android
surfaces, because both depend on local idioms — how `VTCore.swift` holds its
handle, which sheet component `Sheets.kt` uses — that must be read before they
are written. Every one of those steps says what to follow. Tasks 1–12 are
paste-ready.
