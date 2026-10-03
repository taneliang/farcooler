//! What an agent's input box holds, read off its screen.
//!
//! For one caller that has to be sure: the daemon typing an answered decision
//! into an agent (`watch::answer_wake`). It types only into a box this module
//! POSITIVELY recognizes, and only when that box is empty. Anything else (a
//! menu, a picker, a permission prompt, a screen drawn in a way not seen
//! before) is `Unrecognized`, and the daemon doesn't type.
//!
//! So this is the opposite of `activity::classify`, whose fallback is Idle. A
//! wrong answer here fails closed: the answer waits, and in the end is noted
//! as not delivered.
//!
//! The shapes, from `captures/`:
//!
//! - claude: a line starting `❯` plus a space or NBSP at column 0, with a rule
//!   of `─` directly above it and another directly below the box. Lines
//!   between them are the box's continuation, indented two columns.
//!   (`claude-idle-fresh.txt`, `claude-working.txt`)
//! - codex: a line starting `› ` at column 0, then continuation lines indented
//!   two columns, a blank line, and the model footer (`<model> · <path>`).
//!   (`codex-idle-after-turn.txt`)
//!
//! Placeholder text is drawn DIM (SGR 2) and doesn't count as content: a box
//! showing only a dim suggestion is empty. That needs the screen WITH its
//! escapes (`capture-pane -e`). Read without them, a placeholder is
//! indistinguishable from typing, so it counts as content, which fails closed.
//! The plain-text captures here are read that way: codex's `› Explain this
//! codebase` and claude's suggested prompt both read as `Holds`.

/// What an input box holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Composer {
    /// The box is there and nothing has been typed in it.
    Empty,
    /// The box is there and holds this, its lines joined with single spaces.
    Holds(String),
    /// No box this module knows is on the screen.
    Unrecognized,
}

/// The input box on `screen`, for the agent `preset` names (`claude`,
/// `codex`, or either with `:<model>`). Any other agent is `Unrecognized`.
pub fn read(preset: &str, screen: &str) -> Composer {
    let lines: Vec<Vec<(char, bool)>> = screen.lines().map(cells).collect();
    match preset.split(':').next().unwrap_or_default() {
        "claude" => claude(&lines),
        "codex" => codex(&lines),
        _ => Composer::Unrecognized,
    }
}

/// Whether `held` is `sent` as a box shows it: the same characters, with any
/// whitespace a box wraps or reflows ignored.
pub fn holds_exactly(held: &Composer, sent: &str) -> bool {
    let squeeze = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    matches!(held, Composer::Holds(text) if squeeze(text) == squeeze(sent))
}

/// A line's printed characters, each with whether it was drawn dim. Escape
/// sequences are dropped; SGR 2 turns dim on, 22 and a reset turn it off.
fn cells(line: &str) -> Vec<(char, bool)> {
    let mut out = Vec::new();
    let mut dim = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            if !c.is_control() {
                out.push((c, dim));
            }
            continue;
        }
        if chars.peek() != Some(&'[') {
            // Not a CSI: drop the escape and the one character after it.
            chars.next();
            continue;
        }
        chars.next();
        let mut params = String::new();
        let mut last = None;
        for p in chars.by_ref() {
            if ('\x40'..='\x7e').contains(&p) {
                last = Some(p);
                break;
            }
            params.push(p);
        }
        if last == Some('m') {
            let mut fields = params.split(';').peekable();
            if params.is_empty() {
                dim = false;
            }
            while let Some(f) = fields.next() {
                match f {
                    "" | "0" => dim = false,
                    "2" => dim = true,
                    "22" => dim = false,
                    // Colors carry their own operands, which are not
                    // attributes: `38;5;2` is a color, not dim.
                    "38" | "48" | "58" => match fields.next() {
                        Some("5") => {
                            fields.next();
                        }
                        Some("2") => {
                            fields.next();
                            fields.next();
                            fields.next();
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
    }
    out
}

fn text(cells: &[(char, bool)]) -> String {
    cells.iter().map(|(c, _)| *c).collect()
}

/// What a box's lines hold: the non-dim characters, whitespace squeezed.
fn content(rows: &[&[(char, bool)]]) -> Composer {
    let typed: Vec<String> = rows
        .iter()
        .map(|row| row.iter().filter(|(_, dim)| !dim).map(|(c, _)| *c).collect::<String>())
        .collect();
    let joined = typed.join(" ").split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.is_empty() { Composer::Empty } else { Composer::Holds(joined) }
}

fn is_rule(row: &[(char, bool)]) -> bool {
    let t = text(row);
    let t = t.trim_end();
    t.chars().count() >= 10 && t.chars().all(|c| c == '─')
}

fn indented(row: &[(char, bool)]) -> bool {
    row.len() >= 2 && row[0].0 == ' ' && row[1].0 == ' '
}

fn claude(lines: &[Vec<(char, bool)>]) -> Composer {
    // The last prompt line on the screen: the box is drawn below everything.
    let Some(start) = lines.iter().rposition(|row| {
        row.first().map(|c| c.0) == Some('❯') && matches!(row.get(1).map(|c| c.0), None | Some(' ' | '\u{a0}'))
    }) else {
        return Composer::Unrecognized;
    };
    if start == 0 || !is_rule(&lines[start - 1]) {
        return Composer::Unrecognized;
    }
    let mut end = start + 1;
    while end < lines.len() && !is_rule(&lines[end]) {
        if !indented(&lines[end]) && !text(&lines[end]).trim().is_empty() {
            return Composer::Unrecognized;
        }
        end += 1;
    }
    if end == lines.len() {
        return Composer::Unrecognized;
    }
    // Vim mode's NORMAL: keys there are commands, not text.
    let below: String = lines[end + 1..].iter().map(|r| text(r)).collect::<Vec<_>>().join(" ");
    if below.contains("NORMAL") {
        return Composer::Unrecognized;
    }
    let first = &lines[start][2.min(lines[start].len())..];
    let rows: Vec<&[(char, bool)]> =
        std::iter::once(first).chain(lines[start + 1..end].iter().map(|r| &r[2.min(r.len())..])).collect();
    content(&rows)
}

fn codex(lines: &[Vec<(char, bool)>]) -> Composer {
    let Some(start) = lines.iter().rposition(|row| {
        row.first().map(|c| c.0) == Some('›') && matches!(row.get(1).map(|c| c.0), None | Some(' '))
    }) else {
        return Composer::Unrecognized;
    };
    let mut end = start + 1;
    while end < lines.len() && indented(&lines[end]) && !text(&lines[end]).trim().is_empty() {
        end += 1;
    }
    // A blank line, then the model footer, `<model> · <path>`.
    let blank = lines.get(end).is_some_and(|r| text(r).trim().is_empty());
    let footer = lines.get(end + 1).is_some_and(|r| {
        let t = text(r);
        t.starts_with("  ") && t.contains(" · ")
    });
    if !blank || !footer {
        return Composer::Unrecognized;
    }
    let first = &lines[start][2.min(lines[start].len())..];
    // A numbered choice under the marker is a menu, never a box.
    let head = text(first);
    let head = head.trim_start();
    if head.split_once('.').is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())) {
        return Composer::Unrecognized;
    }
    let rows: Vec<&[(char, bool)]> =
        std::iter::once(first).chain(lines[start + 1..end].iter().map(|r| &r[2.min(r.len())..])).collect();
    content(&rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture(name: &str) -> String {
        std::fs::read_to_string(format!("{}/captures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    /// claude's empty box, idle and mid-turn alike: this module reads the box,
    /// and whether the agent is busy is the classifier's question.
    #[test]
    fn claudes_empty_box_is_empty() {
        assert_eq!(read("claude", &capture("claude-idle-fresh.txt")), Composer::Empty);
        assert_eq!(read("claude:opus", &capture("claude-working.txt")), Composer::Empty);
    }

    /// A suggestion read without its escapes can't be told from typing, so it
    /// is content: the reader fails closed.
    #[test]
    fn a_suggestion_without_its_dim_is_content() {
        let held = read("claude", &capture("claude-idle-nothing-running.txt"));
        assert_eq!(held, Composer::Holds("wait for the background shell to finish".into()));
        assert_eq!(read("codex", &capture("codex-idle-after-turn.txt")), Composer::Holds("Explain this codebase".into()));
    }

    /// The same placeholders drawn dim, as the agents draw them, are empty.
    #[test]
    fn a_dim_placeholder_is_empty() {
        let claude = capture("claude-idle-nothing-running.txt")
            .replace("❯\u{a0}wait for the background shell to finish", "❯\u{a0}\x1b[2mwait for the background shell to finish\x1b[0m");
        assert_eq!(read("claude", &claude), Composer::Empty);
        let codex = capture("codex-idle-after-turn.txt").replace("› Explain this codebase", "› \x1b[2mExplain this codebase\x1b[22m");
        assert_eq!(read("codex", &codex), Composer::Empty);
        // A color is not dim, whatever its operands.
        let colored = capture("codex-idle-after-turn.txt").replace("› Explain", "› \x1b[38;5;2mExplain");
        assert_eq!(read("codex", &colored), Composer::Holds("Explain this codebase".into()));
    }

    /// Every menu and prompt in the corpus is no box at all.
    #[test]
    fn a_menu_or_a_prompt_is_no_box() {
        for (preset, name) in [
            ("claude", "claude-blocked.txt"),
            ("claude", "claude-asking.txt"),
            ("claude", "claude-trust-gate.txt"),
            ("claude", "claude-permission-hook-waiting.txt"),
            ("codex", "codex-blocked.txt"),
            ("codex", "codex-trust-gate.txt"),
            ("cursor", "cursor-idle.txt"),
            ("claude", "codex-idle-after-turn.txt"),
            ("codex", "claude-idle-fresh.txt"),
        ] {
            assert_eq!(read(preset, &capture(name)), Composer::Unrecognized, "{preset} on {name}");
        }
    }

    /// A box with a draft holds it, across wrapped lines; vim's NORMAL mode
    /// is no box to type into.
    #[test]
    fn a_draft_is_held_and_normal_mode_is_refused() {
        let idle = capture("claude-idle-fresh.txt");
        let draft = idle.replacen("❯\u{a0}\n", "❯\u{a0}fix the\n  flaky test\n", 1);
        assert_eq!(read("claude", &draft), Composer::Holds("fix the flaky test".into()));
        assert!(holds_exactly(&read("claude", &draft), "fix the flaky test"));
        assert!(holds_exactly(&read("claude", &draft), "fix theflaky test"), "wrapping may eat a space");
        assert!(!holds_exactly(&read("claude", &draft), "fix the flaky tests"));
        assert!(!holds_exactly(&Composer::Empty, ""));
        let normal = idle.replace("manual mode on", "-- NORMAL -- manual mode on");
        assert_eq!(read("claude", &normal), Composer::Unrecognized);
    }

    /// codex's box is the marker line, a blank line and the model footer;
    /// without the footer, or with a numbered choice under the marker, it's
    /// no box.
    #[test]
    fn codexs_box_needs_its_footer_and_is_never_a_choice() {
        let box_ = "• Done.\n\n› \x1b[2mExplain this codebase\x1b[0m\n\n  gpt-5 high · ~/src\n";
        assert_eq!(read("codex", box_), Composer::Empty);
        assert_eq!(read("codex", "• Done.\n\n› \x1b[2mExplain\x1b[0m\n"), Composer::Unrecognized, "no footer");
        assert_eq!(read("codex", "\n› 1. Yes, proceed\n\n  gpt-5 high · ~/src\n"), Composer::Unrecognized, "a choice");
    }
}
