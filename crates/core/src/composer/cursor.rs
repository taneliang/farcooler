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
//! What isn't measured fails closed: a draft that wraps onto a second row is
//! no box this reads, so a message too long for one row is never sent.

use super::{Composer, text};

/// The placeholders cursor draws in an empty box.
const PLACEHOLDERS: [&str; 2] = ["Add a follow-up", "Plan, search, build anything"];

/// The box on `lines`, a screen's rows with whether each cell is dim.
pub(super) fn read(lines: &[Vec<(char, bool)>]) -> Composer {
    let is_box = |row: &[(char, bool)]| text(row).starts_with("  → ") || text(row) == "  →";
    let Some(at) = lines.iter().rposition(|row| is_box(row)) else { return Composer::Unrecognized };
    let blank = |row: usize| lines.get(row).is_some_and(|r| text(r).trim().is_empty());
    let footer = lines.iter().skip(at + 1).find(|r| !text(r).trim().is_empty());
    let footer_ok = footer.is_some_and(|r| {
        let t = text(r);
        t.starts_with("  ") && !t.starts_with("   ")
    });
    if !blank(at + 1) || !footer_ok {
        return Composer::Unrecognized;
    }
    let row = &lines[at][4.min(lines[at].len())..];
    // `ctrl+c to stop`, at the row's right while a turn runs: past a run of
    // three spaces, never the box's own.
    let shown = text(row);
    let end = shown.find("   ").map_or(row.len(), |i| shown[..i].chars().count());
    let cells = &row[..end.min(row.len())];
    let words = text(cells).trim().to_string();
    if words.is_empty() || cells.iter().all(|c| c.1 || c.0 == ' ') || PLACEHOLDERS.contains(&words.as_str()) {
        return Composer::Empty;
    }
    Composer::Holds(words)
}

#[cfg(test)]
mod tests {
    use super::super::{Composer, read};

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

    /// A permission menu's arrow is no box, nor is the trust gate, nor a row
    /// with nothing like the footer below it.
    #[test]
    fn a_menu_is_never_the_box() {
        assert_eq!(read("cursor", &capture("cursor-blocked.txt")), Composer::Unrecognized);
        assert_eq!(read("cursor", &capture("cursor-trust-gate.txt")), Composer::Unrecognized);
        assert_eq!(read("cursor", "  → Add a follow-up\n"), Composer::Unrecognized, "no footer");
        assert_eq!(read("cursor", "  → Run (once) (y)\n    Skip (n)\n\n  Auto\n"), Composer::Unrecognized);
    }
}
