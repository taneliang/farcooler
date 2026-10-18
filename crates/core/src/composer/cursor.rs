//! cursor-agent's input box (R-47, ov-455), so a message can be typed into a
//! cursor pane past the same gate as claude's and codex's.
//!
//! Read off cursor-agent 2026.08.11's own screens in `captures/`:
//!
//! - **The box** is one row, `  → ` and what's in it, at the foot of the
//!   transcript (`cursor-idle.txt`), with a blank row and then the model
//!   footer below it (`  Auto · 7.9%`, then the path). While a turn runs the
//!   row also carries `ctrl+c to stop` at its right (`cursor-working.txt`).
//! - **Empty**, the box shows a placeholder: `Add a follow-up` after a turn.
//!   Drawn dim, it reads as empty, as claude's and codex's do; read without
//!   escapes, the placeholders cursor is known to draw read as empty too,
//!   which is the one place this differs from the module's rule (a plain
//!   text placeholder reads as content). Typing exactly a placeholder's
//!   words is the cost.
//! - **A menu** uses the same arrow (`  → Run (once) (y)`, `cursor-blocked.txt`),
//!   with its other choices on the rows right below. So the box's row must
//!   have a blank row under it and the model footer after that; a menu's
//!   doesn't.
//!
//! - **A long draft wraps**, under its first row's text, four columns in, and
//!   is read whole. A menu's other choices sit four columns in too, so a
//!   screen that shows a menu (`MENUS`, what cursor's own dialogs say) has no
//!   box at all.
//!
//! Measured again on cursor-agent 2026.09.26 (120x40 panes, bracketed pastes
//! as `send-keys -l`; `captures/cursor-2026.09.26-*`, ov-632):
//!
//! - **A blank line in a paste** is a blank row inside the box, so the
//!   draft's rows are the arrow's and every row four columns in or blank
//!   until the footer, which only a blank row separates from the draft.
//!   Rows are joined with a space; whitespace is squeezed when comparing
//!   (`drawn::Expected`), because cursor wraps and rejoins a text as it likes.
//! - **A paste collapses** past 800 UTF-16 units to `[Pasted text #N +K
//!   lines]`, K the text's lines (`collapses`); an image's path alone is
//!   `[Image #N]`, as claude's, and text after it stays beside it. The
//!   threshold is not the 1,000 characters ov-627 reported: 801 is over it.
//! - **A text of hundreds of lines** scrolls its box to the last rows, which
//!   cursor draws with nothing to say so, and isn't read back.
//!
//! What isn't measured fails closed: a wrapped row that isn't four columns in
//! is no box, and nothing is sent.

use super::{Composer, text};

/// Longer than this, in UTF-16 units (line breaks count), a paste collapses
/// to `[Pasted text #N +K lines]`, K its lines, however few. Measured on
/// cursor-agent 2026.09.26: 800 `x` stay and 801 collapse, 400 emoji stay
/// and 401 collapse, `a`x399 + LF + `b`x400 stays and 400 + LF + 400
/// collapses. Twelve short lines stay as typed.
pub const COLLAPSES_PAST_UNITS: usize = 800;

/// Whether cursor collapses a paste of `text` into a placeholder.
pub fn collapses(text: &str) -> bool {
    text.encode_utf16().count() > COLLAPSES_PAST_UNITS
}

/// The placeholders cursor draws in an empty box.
const PLACEHOLDERS: [&str; 2] = ["Add a follow-up", "Plan, search, build anything"];

/// What cursor's dialogs say, read off `cursor-blocked.txt` and
/// `cursor-trust-gate.txt`: a screen with any of them near its arrow shows a
/// menu, not a box.
const MENUS: [&str; 4] = ["Run this command?", "Skip & tell the agent", "Trust this workspace", "Use arrow keys to navigate"];

/// How far above the arrow a dialog's question is read.
const MENU_ROWS: usize = 6;

/// The box on `lines`, a screen's rows with whether each cell is dim.
pub(super) fn read(lines: &[Vec<(char, bool)>]) -> Composer {
    let is_box = |row: &[(char, bool)]| text(row).starts_with("  → ") || text(row) == "  →";
    let Some(at) = lines.iter().rposition(|row| is_box(row)) else { return Composer::Unrecognized };
    // A dialog's question sits just above its arrow (`cursor-blocked.txt`);
    // one answered long ago can stay on the screen far above the box
    // (`cursor-idle.txt` still shows its trust gate), and doesn't count.
    let near = &lines[at.saturating_sub(MENU_ROWS)..];
    if near.iter().any(|r| MENUS.iter().any(|m| text(r).contains(m))) {
        return Composer::Unrecognized;
    }
    // The draft's rows under the first: wrapped ones four columns in, and
    // the blank rows a pasted blank line draws (`cursor-2026.09.26-paragraphs`).
    // The footer is the first row that is neither, and only a blank row can
    // stand between it and the draft; the draft ends at its last drawn row.
    let mut end = at;
    let mut next = at + 1;
    let footer = loop {
        let Some(row) = lines.get(next) else { break None };
        let t = text(row);
        if t.trim().is_empty() {
            next += 1;
        } else if t.starts_with("    ") {
            end = next;
            next += 1;
        } else {
            break Some(t);
        }
    };
    let footer_ok = footer.is_some_and(|t| t.starts_with("  ") && !t.starts_with("   ")) && next > end + 1;
    if !footer_ok {
        return Composer::Unrecognized;
    }
    let row = &lines[at][4.min(lines[at].len())..];
    // `ctrl+c to stop`, at the row's right while a turn runs: past a run of
    // three spaces, never the box's own.
    let shown = text(row);
    let hint = shown.find("   ").map_or(row.len(), |i| shown[..i].chars().count());
    let cells = &row[..hint.min(row.len())];
    let wrapped = lines[at + 1..=end].iter().map(|r| text(r).trim().to_string());
    let words = std::iter::once(text(cells).trim().to_string()).chain(wrapped).filter(|r| !r.is_empty()).collect::<Vec<_>>().join(" ");
    let words = words.trim().to_string();
    // Dim cells are a placeholder only when nothing was read below the arrow
    // and no collapsed paste is on the row: cursor draws `[Pasted text #N
    // +K lines]` dim, in cyan, and a draft can hold one beside more text or
    // begin with a blank line. Both are a person's content, never empty.
    let dim = end == at && cells.iter().all(|c| c.1 || c.0 == ' ') && !words.contains("[Pasted text #");
    if words.is_empty() || dim || PLACEHOLDERS.contains(&words.as_str()) {
        return Composer::Empty;
    }
    Composer::Holds(words)
}

#[cfg(test)]
mod tests {
    use super::super::{Composer, drawn::{self, Expected}, read};
    use super::collapses;

    fn capture(name: &str) -> String {
        std::fs::read_to_string(format!("{}/captures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    /// After a turn, and while one runs, the box shows its placeholder: empty.
    #[test]
    fn the_box_after_a_turn_and_during_one_is_empty() {
        assert_eq!(read("cursor", &capture("cursor-idle.txt")), Composer::Empty);
        assert_eq!(read("cursor", &capture("cursor-working.txt")), Composer::Empty);
    }

    /// A draft is held, a dim placeholder is not, and a hint at the row's
    /// right isn't part of what's typed.
    #[test]
    fn a_draft_is_held_and_a_dim_placeholder_is_empty() {
        let idle = capture("cursor-idle.txt");
        let draft = idle.replacen("→ Add a follow-up", "→ [from mac-ux] PR is up", 1);
        assert_eq!(read("cursor", &draft), Composer::Holds("[from mac-ux] PR is up".into()));
        let dim = idle.replacen("→ Add a follow-up", "→ \x1b[2mSomething new\x1b[0m", 1);
        assert_eq!(read("cursor", &dim), Composer::Empty);
        let working = capture("cursor-working.txt").replacen("→ Add a follow-up", "→ fix it", 1);
        assert_eq!(read("cursor", &working), Composer::Holds("fix it".into()));
    }

    /// A draft that wraps is read whole, its rows under full rows four
    /// columns in; a row under a short one is no box.
    #[test]
    fn a_wrapped_draft_is_read_whole() {
        let idle = capture("cursor-idle.txt");
        let width = idle.lines().map(|l| l.trim_end().chars().count()).max().unwrap();
        let first = format!("[from the orchestrator] {}", "x".repeat(width - 30));
        let wrapped = idle.replacen("  → Add a follow-up", &format!("  → {first}\n    then rebase"), 1);
        assert_eq!(read("cursor", &wrapped), Composer::Holds(format!("{first} then rebase")));
        let odd = idle.replacen("  → Add a follow-up", &format!("  → {first}\n  then rebase"), 1);
        assert_eq!(read("cursor", &odd), Composer::Unrecognized, "a row not four columns in");
    }

    /// A permission menu's arrow is no box, nor is the trust gate, nor a row
    /// with nothing like the footer below it.
    #[test]
    fn a_menu_is_never_the_box() {
        assert_eq!(read("cursor", &capture("cursor-blocked.txt")), Composer::Unrecognized);
        assert_eq!(read("cursor", &capture("cursor-trust-gate.txt")), Composer::Unrecognized);
        assert_eq!(read("cursor", "  → Add a follow-up\n"), Composer::Unrecognized, "no footer");
        let menu = " Run this command?\n  → Run (once) (y)\n    Skip & tell the agent what to do instead (esc or n)\n\n  Auto\n";
        assert_eq!(read("cursor", menu), Composer::Unrecognized);
    }

    /// Every state of the 2026.09.26 box, read off its own screens: the
    /// placeholder fresh, mid-turn and after a turn is empty.
    #[test]
    fn the_2026_09_26_placeholders_read_empty() {
        for name in ["fresh", "working", "after-turn"] {
            assert_eq!(read("cursor", &capture(&format!("cursor-2026.09.26-{name}-120x40-e.txt"))), Composer::Empty, "{name}");
        }
    }

    fn held(name: &str) -> Composer {
        read("cursor", &capture(&format!("cursor-2026.09.26-{name}-120x40-e.txt")))
    }

    /// What was typed is read back and compared whitespace squeezed, however
    /// cursor wrapped it or joined its rows.
    #[test]
    fn a_typed_message_is_read_back() {
        let said = "[from the orchestrator] Run the gates";
        assert_eq!(held("typed"), Composer::Holds(said.into()));
        assert!(Expected::typed_into("cursor", said).shown_by(&held("typed")));
        assert!(!Expected::typed_into("cursor", "[from the orchestrator] Run the gate").shown_by(&held("typed")));
        let words = "word ".repeat(130);
        assert!(Expected::typed_into("cursor", words.trim()).shown_by(&held("wrapped")), "{:?}", held("wrapped"));
        assert!(!Expected::typed_into("cursor", &"word ".repeat(129)).shown_by(&held("wrapped")));
    }

    /// A pasted blank line is a blank row inside the box, not the gap above
    /// the footer: the paragraphs are read whole, not refused as no box.
    #[test]
    fn a_text_with_blank_lines_is_read_whole() {
        let text = "First paragraph of the plan.\n\nSecond paragraph, after a blank line.\n\n\nThird, after two.";
        assert_eq!(
            held("paragraphs"),
            Composer::Holds("First paragraph of the plan. Second paragraph, after a blank line. Third, after two.".into())
        );
        assert!(Expected::typed_into("cursor", text).shown_by(&held("paragraphs")));
        let cut = text.replace("Third, after two.", "");
        assert!(!Expected::typed_into("cursor", &cut).shown_by(&held("paragraphs")), "the last paragraph is missing");
    }

    /// Past 800 units cursor draws `[Pasted text #N +K lines]` for a paste,
    /// K its lines; `Expected::plain` would never match it.
    #[test]
    fn a_long_paste_is_its_placeholder() {
        let text = "word ".repeat(250);
        assert_eq!(held("collapsed"), Composer::Holds("[Pasted text #24 +1 lines]".into()));
        assert!(Expected::typed_into("cursor", text.trim()).shown_by(&held("collapsed")));
        assert!(!Expected::plain(text.trim()).shown_by(&held("collapsed")), "claude's and codex's comparison");
        assert!(!Expected::typed_into("cursor", &format!("{}\n{}", text, text)).shown_by(&held("collapsed")), "+2, not +1");
        assert!(!Expected::typed_into("claude", text.trim()).shown_by(&held("collapsed")), "another agent's form");
        assert!(Expected::typed_into("cursor:auto", text.trim()).shown_by(&held("collapsed")), "a preset with a model");
    }

    /// The threshold, as measured: 800 UTF-16 units stay, 801 collapse, a
    /// line break counting as a unit; lines alone never collapse.
    #[test]
    fn a_paste_collapses_past_800_units() {
        assert!(!collapses(&"x".repeat(800)));
        assert!(collapses(&"x".repeat(801)));
        assert!(!collapses(&"😀".repeat(400)));
        assert!(collapses(&"😀".repeat(401)));
        assert!(!collapses(&format!("{}\n{}", "a".repeat(399), "b".repeat(400))));
        assert!(collapses(&format!("{}\n{}", "a".repeat(400), "b".repeat(400))));
        assert!(!collapses(&"a\n".repeat(12)));
        let two = format!("{}\n{}", "a".repeat(600), "b".repeat(600));
        assert!(Expected::typed_into("cursor", &two).shown_by(&Composer::Holds("[Pasted text #8 +2 lines]".into())));
    }

    /// An image's path pasted alone is `[Image #N]`, and the text after it
    /// stays beside it.
    #[test]
    fn an_image_and_the_text_after_it_are_read_back() {
        let both = held("image");
        assert_eq!(both, Composer::Holds("[Image #1]  what is this".into()));
        assert!(Expected::default().then_image().then_cursor_paste(" what is this").shown_by(&both));
        assert!(!Expected::default().then_cursor_paste(" what is this").shown_by(&both), "the image is missing");
        assert_eq!(drawn::images(&both), ["[Image #1]"]);
    }

    /// A person's draft that the dim-placeholder rule must not call empty:
    /// two collapsed pastes, a collapsed paste with a line typed under it,
    /// and a draft that begins with a blank line (review 1, B1). Each would
    /// let a message be pasted into the person's text.
    #[test]
    fn a_draft_with_dim_or_blank_first_cells_is_still_content() {
        let paste = |n: u32| format!("\x1b[2m\x1b[36m[Pasted text #{n} +1 lines]\x1b[0m");
        let screen = |rows: &str| format!("  Cursor Agent\n\n{rows}\n\n  Auto\n  /tmp/x\n");
        let two = screen(&format!("  → {} {}", paste(1), paste(2)));
        assert_eq!(read("cursor", &two), Composer::Holds("[Pasted text #1 +1 lines] [Pasted text #2 +1 lines]".into()));
        let under = screen(&format!("  → {}\n    and fix this", paste(1)));
        assert_eq!(read("cursor", &under), Composer::Holds("[Pasted text #1 +1 lines] and fix this".into()));
        let blank_first = screen("  →\n    second line");
        assert_eq!(read("cursor", &blank_first), Composer::Holds("second line".into()));
        let blank_two = screen("  →\n\n    third row");
        assert_eq!(read("cursor", &blank_two), Composer::Holds("third row".into()));
        // The one dim placeholder alone, and a plainly empty box, still are.
        assert_eq!(read("cursor", &screen("  → \x1b[2mAdd a follow-up\x1b[0m")), Composer::Empty);
        assert_eq!(read("cursor", &screen("  →")), Composer::Empty);
    }
}
