//! claude 2.1.301's box scrolled, read back by its tail (ov-618), from real
//! screens: a scratch claude, its pastes bracketed and sent as the daemon
//! sends them. See `drawn`'s docs.

use super::drawn::{Expected, tallest};
use super::{Composer, claude_rows, read};

fn capture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/captures/claude-2.1.301-{name}-e.txt", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

/// The text the captures were pasted from: numbered words, `n` characters
/// of them, the last a `Z`.
fn words(n: usize) -> String {
    let mut s = String::new();
    let mut i = 0;
    while s.len() < n {
        s.push_str(&format!("w{i} the quick brown fox jumps over a lazy dog and then rests "));
        i += 1;
    }
    s.truncate(n);
    format!("{}Z", s.trim_end())
}

/// `n` numbered lines, each padded to 34 characters.
fn lines(n: usize) -> String {
    (1..=n).map(|i| format!("{:x<34}", format!("line {i:03} of {n:03} "))).collect::<Vec<_>>().join("\n")
}

fn pasted(text: &str) -> Expected {
    Expected::default().then_paste(text)
}

/// The pane heights measured, and the rows the box grew to before it
/// scrolled.
#[test]
fn the_box_grows_to_half_the_pane_less_five() {
    for (pane, rows) in [(16, 3), (20, 5), (24, 7), (30, 10), (35, 12), (40, 15), (45, 17), (50, 20)] {
        assert_eq!(tallest(pane), rows, "{pane} rows");
    }
    assert_eq!(tallest(8), 0);
}

/// Each scrolled shape: one long line, two and three lines too short to
/// collapse, double-width text, and mid-turn. `shown_by` alone, as before,
/// refuses each.
#[test]
fn a_scrolled_box_is_read_back_by_its_tail() {
    let three = vec![words(250); 3].join("\n");
    let two = format!("{}\n{}", words(390), words(390));
    let cjk = "日本語".repeat(120);
    for (name, text, pane) in [
        ("scrolled-700-40x24", words(699), 24),
        ("scrolled-700-80x24", words(699), 24),
        ("scrolled-working-700-40x24", words(699), 24),
        ("scrolled-two-lines-40x45", two, 45),
        ("scrolled-three-lines-40x45", three, 45),
        ("scrolled-cjk-40x45", cjk, 45),
    ] {
        let screen = capture(name);
        assert_eq!(claude_rows(&screen), Some(tallest(pane)), "{name}: at its tallest");
        assert!(!pasted(&text).shown_by(&read("claude", &screen)), "{name}: its head is out of sight");
        assert!(pasted(&text).shown_on("claude", &screen), "{name}");
        assert!(pasted(&text).shown_on("claude:haiku", &screen), "{name}, a preset with its model");
    }
}

/// A box that shows the whole paste, or its placeholder, reads as before.
#[test]
fn a_whole_or_collapsed_box_reads_as_before() {
    let whole = capture("whole-500-40x45");
    assert_eq!(claude_rows(&whole), Some(15));
    assert!(pasted(&words(500)).shown_on("claude", &whole));
    assert!(pasted(&lines(200)).shown_on("claude", &capture("collapsed-200-lines-40x45")));
    assert!(!pasted(&lines(199)).shown_on("claude", &capture("collapsed-200-lines-40x45")), "+198, not +199");
    assert!(pasted(&words(20000)).shown_on("claude", &capture("collapsed-20000-40x45")));
}

/// What else a box holds is still refused: text typed after the paste, a
/// draft before a placeholder, a tail one character off, a box short of its
/// tallest, any agent but claude, and a box that's no box.
#[test]
fn a_box_holding_more_or_other_text_is_refused() {
    let text = words(699);
    let typed = capture("scrolled-700-then-typed-40x24");
    assert!(read("claude", &typed) != Composer::Empty);
    assert!(!pasted(&text).shown_on("claude", &typed), "typed after the paste");
    let draft = capture("draft-then-ten-lines-40x24");
    assert!(!pasted(&lines(10)).shown_on("claude", &draft), "a draft before the placeholder");
    let screen = capture("scrolled-700-40x24");
    let off = format!("{}Y", &text[..text.len() - 1]);
    assert!(!pasted(&off).shown_on("claude", &screen), "the last character differs");
    let lost = text.replacen("w11", "w12", 1);
    assert!(!pasted(&lost).shown_on("claude", &screen), "a word in sight differs");
    assert!(!pasted(&text).shown_on("claude", &format!("{screen}\n\n")), "in a pane of 26 rows, short of its tallest, 8");
    let rows: Vec<&str> = screen.lines().collect();
    assert!(!pasted(&text).shown_on("claude", &rows[2..].join("\n")), "in a pane of 22 rows, past its tallest, 6");
    assert!(!pasted(&text).shown_on("codex", &screen));
    assert!(!pasted(&text).shown_on("claude", &screen.replacen("❯", ">", 1)), "no box");
}

/// The tail is read piece by piece: the top may cut text or fall between
/// pieces, never cut a placeholder, and nothing may be left above.
#[test]
fn the_tail_is_read_piece_by_piece() {
    let rule = "─".repeat(38);
    // A box of seven rows, at its tallest in a 24-row pane: fourteen blank
    // rows, the rules, the box and a footer.
    let boxed = |rows: [&str; 7]| {
        let mut screen = format!("{}{rule}\n❯\u{a0}{}\n", "\n".repeat(14), rows[0]);
        for row in &rows[1..] {
            screen.push_str(&format!("  {row}\n"));
        }
        screen.push_str(&format!("{rule}\n  ? for shortcuts\n"));
        screen
    };
    let body = ["b2", "b3", "b4", "b5", "b6", "b7"];
    let tail = boxed(["b1", body[0], body[1], body[2], body[3], body[4], body[5]]);
    let text = "a0 b1 b2 b3 b4 b5 b6 b7";
    assert!(Expected::default().then_image().then_paste(text).shown_on("claude", &tail), "the cut in the text");
    let text_only = boxed(["a0", "b1", body[1], body[2], body[3], body[4], body[5]]);
    assert!(Expected::default().then_image().then_paste(" a0 b1 b3 b4 b5 b6 b7").shown_on("claude", &text_only));
    let image = boxed(["[Image #4] a0", "b1", body[1], body[2], body[3], body[4], body[5]]);
    let both = Expected::default().then_image().then_image().then_paste(" a0 b1 b3 b4 b5 b6 b7");
    assert!(both.shown_on("claude", &image), "the top between the images");
    let cut = boxed(["#4] a0", "b1", body[1], body[2], body[3], body[4], body[5]]);
    assert!(!both.shown_on("claude", &cut), "the top through a placeholder");
    assert!(!Expected::default().then_paste(" a0 b1 b3 b4 b5 b6 b7").shown_on("claude", &image), "an image left above");
}

/// What the screen can't show: a draft, or an earlier paste, scrolled
/// wholly out of sight above the text. The box reads as holding the paste
/// alone; what vouches for the rest is the daemon's proof the box was empty
/// before the first paste (`compose::box_still_empty`).
#[test]
fn what_scrolled_out_of_sight_is_not_read() {
    let text = words(699);
    for name in ["scrolled-draft-then-700-40x24", "scrolled-ten-lines-then-700-40x24"] {
        assert!(pasted(&text).shown_on("claude", &capture(name)), "{name}");
    }
}
