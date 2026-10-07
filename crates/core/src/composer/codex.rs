//! What codex's box does with a paste, so a message composed into it can be
//! read back, and refused before anything is typed where the box would take
//! the Enter as something other than a send (ov-416).
//!
//! Measured on codex-cli 0.153.4 (a sandbox `HOME` and `CODEX_HOME`, a
//! stand-in API, a 160x45 and an 80x24 pane), with bracketed pastes:
//!
//! - **A paste collapses** to `[Pasted Content N chars]` past 1,000
//!   characters (Unicode scalar values: 1,000 `é` or `😀` stay, 1,001
//!   collapse; 600 `👍🏽` are 1,200), N being how many. Line breaks don't
//!   collapse it: forty short lines stay as typed. A CR LF counts as one.
//!   The rollout records the whole text.
//! - **An image's path** pasted alone becomes `[Image #N]` (PNG, JPEG, GIF
//!   and WebP; a path with a space in single quotes); two paths in one paste,
//!   or a path with other text, stay text. N counts from 1 in each message.
//!   codex puts a space after the placeholder.
//! - **The box scrolls** when the text is taller than the pane leaves it, its
//!   marker on the first row still shown (`codex-0.153.4-tall-paste-80x24-e.txt`):
//!   it shows `rows - 4` rows, so a taller text can't be read back.
//! - **A picker opens** while the word at the cursor, the text's last, starts
//!   `@` (files) or `$` followed by a name (skills), even with no match; Enter
//!   then inserts its pick instead of sending. `me@example.com` and `$5`
//!   open none. A `/` at the start opens the command popup.
//! - **Enter** after a bracketed paste sends, even in the same write. Typed
//!   characters followed by Enter in the same write are a paste burst, and
//!   that Enter is a line break; 5 ms apart it sends. Vim mode (`/vim`)
//!   changes neither: a paste in Normal mode goes in, and Enter sends.
//! - **A trailing backslash** is sent as typed (claude's would be a line
//!   break).
//!
//! Captures: `codex-0.153.4-*-e.txt` in `captures/`.

/// Longer than this, in characters, a paste collapses.
pub const COLLAPSES_PAST_CHARS: usize = 1_000;

/// The rows of a pane that codex's box can't have: the blank row and the
/// footer below it, and two above.
const ROWS_NOT_THE_BOX: u32 = 4;

/// Whether codex collapses a paste of `text` into a placeholder.
pub fn collapses(text: &str) -> bool {
    text.chars().count() > COLLAPSES_PAST_CHARS
}

/// Whether a box holding `text` opens one of codex's pickers, which takes
/// the Enter: its last word starts `@`, or `$` and a letter. Read wider than
/// measured (a `$` before a letter even where no skill matched), never
/// narrower.
pub fn opens_picker(text: &str) -> bool {
    let Some(last) = text.split_whitespace().last() else { return false };
    let mut chars = last.chars();
    match chars.next() {
        Some('@') => true,
        Some('$') => chars.next().is_some_and(|c| !c.is_ascii_digit()),
        _ => false,
    }
}

/// Whether codex's box shows `text` whole, after `images` placeholders, in a
/// pane `columns` wide and `rows` tall, so it can be read back. Erring
/// toward no: the lines are wrapped a word at a time, a column narrower than
/// codex's, every character past ASCII two columns wide, with a row to
/// spare.
pub fn fits(text: &str, images: usize, columns: u32, rows: u32) -> bool {
    let shown = rows.saturating_sub(ROWS_NOT_THE_BOX + 1) as usize;
    if collapses(text) {
        return shown >= 1;
    }
    let width = (columns.saturating_sub(3) as usize).max(1);
    let placeholders = "[Image #10] ".repeat(images);
    let needed: usize = text
        .split('\n')
        .enumerate()
        .map(|(n, line)| if n == 0 { wrapped_rows(&format!("{placeholders}{line}"), width) } else { wrapped_rows(line, width) })
        .sum();
    needed <= shown
}

/// The rows `line` takes wrapped a word at a time into `width` columns, a
/// word wider than a row broken across rows.
fn wrapped_rows(line: &str, width: usize) -> usize {
    let cells = |w: &str| w.chars().map(|c| if c.is_ascii() { 1 } else { 2 }).sum::<usize>();
    let (mut rows, mut used) = (1, 0);
    for word in line.split(' ') {
        let w = cells(word);
        let gap = usize::from(used > 0);
        if used + gap + w <= width {
            used += gap + w;
            continue;
        }
        if used > 0 {
            rows += 1;
        }
        rows += w.saturating_sub(1) / width;
        used = w - (w.saturating_sub(1) / width) * width;
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::super::drawn::Expected;
    use super::super::{Composer, read};
    use super::*;

    fn capture(name: &str) -> String {
        std::fs::read_to_string(format!("{}/captures/codex-0.153.4-{name}-e.txt", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    /// The threshold, as measured: past 1,000 characters, whatever their width.
    #[test]
    fn a_paste_collapses_past_1000_characters() {
        assert!(!collapses(&"x".repeat(1000)));
        assert!(collapses(&"x".repeat(1001)));
        assert!(!collapses(&"😀".repeat(1000)));
        assert!(collapses(&"👍🏽".repeat(600)));
        assert!(!collapses(&"line\n".repeat(150)), "line breaks don't collapse it");
    }

    /// Real screens read back against what codex draws: lines as pasted,
    /// blank ones inside the box too; a long paste and an image as their
    /// placeholders, the count exact.
    #[test]
    fn a_box_is_read_back_against_what_codex_draws() {
        assert_eq!(read("codex", &capture("idle-160x45")), Composer::Empty);
        let three = read("codex", &capture("three-lines-160x45"));
        assert!(Expected::default().then_codex_paste("three\nshort\nlines").shown_by(&three), "{three:?}");
        assert!(!Expected::default().then_codex_paste("three\nshort\nline").shown_by(&three));
        let blanks = read("codex", &capture("blank-lines-160x45"));
        assert!(Expected::default().then_codex_paste("first\n\nthird\n\n\nsixth").shown_by(&blanks), "{blanks:?}");
        let long = read("codex", &capture("long-paste-160x45"));
        assert!(Expected::default().then_codex_paste(&"x".repeat(1001)).shown_by(&long), "{long:?}");
        assert!(!Expected::default().then_codex_paste(&"x".repeat(1002)).shown_by(&long), "the count is exact");
        assert!(!Expected::default().then_paste(&"x".repeat(1001)).shown_by(&long), "claude's form");
        let both = read("codex", &capture("image-and-long-paste-160x45"));
        let text = format!(" {}", "y".repeat(1200));
        assert!(Expected::default().then_image().then_codex_paste(&text).shown_by(&both), "{both:?}");
        assert!(!Expected::default().then_codex_paste(&text).shown_by(&both), "the image is missing");
        assert_eq!(super::super::drawn::images(&both), ["[Image #1]"]);
    }

    /// A picker or the command popup below the box is never read as the box
    /// holding what was pasted, and a scrolled box is never read whole.
    #[test]
    fn a_picker_a_popup_or_a_scrolled_box_never_reads_as_the_paste() {
        for (name, pasted) in [
            ("slash-init-160x45", "/init"),
            ("mention-popup-160x45", "look at @READ"),
            ("mention-no-matches-160x45", "try @zzzq"),
            ("skill-no-matches-160x45", "try $zzzq"),
        ] {
            let held = read("codex", &capture(name));
            assert_eq!(held, Composer::Unrecognized, "{name}");
            assert!(!Expected::default().then_codex_paste(pasted).shown_by(&held), "{name}");
        }
        let rows: Vec<String> = (1..=30).map(|n| format!("row {n}")).collect();
        let tall = read("codex", &capture("tall-paste-80x24"));
        assert!(!Expected::default().then_codex_paste(&rows.join("\n")).shown_by(&tall), "{tall:?}");
        assert!(!fits(&rows.join("\n"), 0, 80, 24), "refused before it's typed");
    }

    /// A text whose last word would open a picker; words that open none.
    #[test]
    fn a_last_word_that_opens_a_picker() {
        for text in ["look at @READ", "try $zzzq", "x\n@README.md", "@", "end @"] {
            assert!(opens_picker(text), "{text}");
        }
        for text in ["mail me@example.com", "cost is $5", "@README.md and more", "$skill then text", "plain", ""] {
            assert!(!opens_picker(text), "{text}");
        }
    }

    /// What fits: the twenty-row box of a 24-row pane, every line counted
    /// at least once, wide characters twice; a collapsed paste is one row.
    #[test]
    fn what_fits_in_the_box() {
        let lines = |n: usize| (1..=n).map(|i| format!("row {i}")).collect::<Vec<_>>().join("\n");
        assert!(fits(&lines(19), 0, 80, 24));
        assert!(!fits(&lines(20), 0, 80, 24));
        assert!(fits(&lines(40), 0, 160, 45));
        assert!(!fits(&lines(41), 0, 160, 45));
        assert!(fits(&"x".repeat(1001), 3, 20, 10), "collapsed");
        assert!(!fits(&"界".repeat(800), 0, 80, 24), "wide characters, uncollapsed");
        assert!(fits(&"x".repeat(1000), 0, 80, 24), "thirteen rows");
        assert!(!fits(&"x".repeat(1000), 0, 40, 24), "twenty-seven rows");
        assert!(!fits(&lines(18), 3, 20, 24), "the placeholders wrap the first line");
        assert!(fits(&lines(18), 1, 80, 24));
    }
}
