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
//!   (`codex-idle-after-turn.txt`). A paste's blank lines are blank rows in
//!   the box, read up to a footer that can only be the model's
//!   (`codex-0.153.4-blank-lines-160x45-e.txt`, ov-416).
//! - cursor: a row `  → ` with a blank row and the model footer below it
//!   (`cursor-idle.txt`); see `cursor` for its placeholders (R-47).
//!
//! Placeholder text is drawn DIM (SGR 2) and doesn't count as content: a box
//! showing only a dim suggestion is empty. The cursor sits on a placeholder's
//! first character, drawn in reverse video (SGR 7) and not dim, so a
//! reverse-video character followed directly by dim text is the
//! placeholder's too: claude 2.1.290's `Press up to edit queued messages`
//! (`claude-2.1.290-working-queued-160x45-e.txt`). That needs the screen WITH
//! its escapes (`capture-pane -e`). Read without them, a placeholder is
//! indistinguishable from typing, so it counts as content, which fails closed.
//! The plain-text captures here are read that way: codex's `› Explain this
//! codebase` and claude's suggested prompt both read as `Holds`.

pub mod codex;
mod cursor;
pub mod draft;
pub mod drawn;
mod suggestion;
#[cfg(test)]
mod titled_rule_tests;

pub use suggestion::{hint, suggestion};

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
/// `codex`, `cursor`, or any with `:<model>`). Any other agent is
/// `Unrecognized`.
pub fn read(preset: &str, screen: &str) -> Composer {
    let lines: Vec<Vec<(char, bool)>> = screen.lines().map(cells).collect();
    match preset.split(':').next().unwrap_or_default() {
        "claude" => claude(&lines),
        "codex" => codex(&lines),
        "cursor" => cursor::read(&lines),
        _ => Composer::Unrecognized,
    }
}

/// Whether `held` is `sent` as a box shows it: the same characters, with any
/// whitespace a box wraps or reflows ignored.
pub fn holds_exactly(held: &Composer, sent: &str) -> bool {
    let squeeze = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    matches!(held, Composer::Holds(text) if squeeze(text) == squeeze(sent))
}

/// What `screen` prints, its escape sequences dropped, a line to a line.
pub fn printed(screen: &str) -> String {
    screen.lines().map(|line| text(&cells(line))).collect::<Vec<_>>().join("\n")
}

/// A line's printed characters, each with whether it was drawn dim. Escape
/// sequences are dropped; SGR 2 turns dim on, 22 and a reset turn it off.
/// The cursor on a dim placeholder's first character counts as dim (see
/// this module's docs): SGR 7 turns reverse video on, 27 and a reset off.
fn cells(line: &str) -> Vec<(char, bool)> {
    styled(line).into_iter().map(|(c, dim, _)| (c, dim)).collect()
}

/// Whether foreground color `256` (an xterm palette index) is a grey: the
/// bright black (8) and the grey ramp (232 to 255) read as one, as claude's
/// own footers draw (`38;5;244`, `38;5;246`).
fn grey_index(n: &str) -> bool {
    n.parse::<u8>().is_ok_and(|n| n == 8 || n >= 232)
}

/// Whether true color `r;g;b` is a mid grey: equal channels, within a few
/// steps, neither near black nor near white.
fn grey_rgb(r: &str, g: &str, b: &str) -> bool {
    let (Ok(r), Ok(g), Ok(b)) = (r.parse::<i32>(), g.parse::<i32>(), b.parse::<i32>()) else { return false };
    (r - g).abs() <= 8 && (g - b).abs() <= 8 && (r - b).abs() <= 8 && (60..=200).contains(&g)
}

/// `cells`, each also with whether it was drawn in a grey foreground
/// (SGR 90, or a 256 or true color grey). A placeholder a program colors
/// rather than dims is a grey one; only `composer::suggestion` asks, since a
/// grey draft would otherwise read as no draft at all.
fn styled(line: &str) -> Vec<(char, bool, bool)> {
    attributed(line).into_iter().map(|(c, dim, grey, _)| (c, dim, grey)).collect()
}

/// `cells`, each with whether it was drawn in reverse video too: claude
/// draws its own cursor as one reverse-video cell (`draft`).
fn cells_with_reverse(line: &str) -> Vec<(char, bool, bool)> {
    attributed(line).into_iter().map(|(c, dim, _, reverse)| (c, dim, reverse)).collect()
}

/// A line's printed characters, each with whether it was drawn dim, grey
/// and in reverse video: `styled` and `cells_with_reverse` both read it.
fn attributed(line: &str) -> Vec<(char, bool, bool, bool)> {
    let mut out: Vec<(char, bool, bool)> = Vec::new();
    let mut reversed = Vec::new();
    let (mut dim, mut reverse, mut grey) = (false, false, false);
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            if !c.is_control() {
                out.push((c, dim, grey));
                reversed.push(reverse);
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
                (dim, reverse, grey) = (false, false, false);
            }
            while let Some(f) = fields.next() {
                match f {
                    "" | "0" => (dim, reverse, grey) = (false, false, false),
                    "90" => grey = true,
                    other if matches!(other.parse::<u8>(), Ok(30..=37 | 39 | 91..=97)) => grey = false,
                    "2" => dim = true,
                    "22" => dim = false,
                    "7" => reverse = true,
                    "27" => reverse = false,
                    // Colors carry their own operands, which are not
                    // attributes: `38;5;2` is a color, not dim.
                    "38" | "48" | "58" => {
                        let foreground = f == "38";
                        match fields.next() {
                            Some("5") => {
                                let n = fields.next().unwrap_or_default();
                                if foreground {
                                    grey = grey_index(n);
                                }
                            }
                            Some("2") => {
                                let (r, g, b) = (
                                    fields.next().unwrap_or_default(),
                                    fields.next().unwrap_or_default(),
                                    fields.next().unwrap_or_default(),
                                );
                                if foreground {
                                    grey = grey_rgb(r, g, b);
                                }
                            }
                            _ => {
                                if foreground {
                                    grey = false;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    for i in 0..out.len().saturating_sub(1) {
        let (next, next_dim, next_grey) = out[i + 1];
        if reversed[i] && !next.is_whitespace() {
            if next_dim {
                out[i].1 = true;
            }
            if next_grey {
                out[i].2 = true;
            }
        }
    }
    out.into_iter().zip(reversed).map(|((c, dim, grey), rev)| (c, dim, grey, rev)).collect()
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
    is_rule_text(&text(row))
}

/// Whether a printed row is claude's rule above or below its box: a run of
/// `─`, which claude may break once for the session's title,
/// `──── User test issues (2) ─` (`claude-orchestrator-titled-rule-103x65-e.txt`,
/// ov-430). The title is set off by a space on each side, and the rule
/// goes on after it by at least one `─`.
pub(crate) fn is_rule_text(row: &str) -> bool {
    let t = row.trim_end();
    let lead = t.chars().take_while(|&c| c == '─').count();
    if lead < 10 {
        return false;
    }
    let rest = &t[t.char_indices().nth(lead).map_or(t.len(), |(i, _)| i)..];
    if rest.is_empty() {
        return true;
    }
    let tail = rest.trim_end_matches('─');
    rest.starts_with(' ') && tail.len() < rest.len() && tail.ends_with(' ') && !tail.trim().is_empty()
}

fn indented(row: &[(char, bool)]) -> bool {
    row.len() >= 2 && row[0].0 == ' ' && row[1].0 == ' '
}

fn claude(lines: &[Vec<(char, bool)>]) -> Composer {
    claude_box(lines).map_or(Composer::Unrecognized, |rows| content(&rows))
}

/// Where claude's box sits in `lines`: the prompt line and the row of the
/// rule that closes it.
fn claude_box_at(lines: &[Vec<(char, bool)>]) -> Option<(usize, usize)> {
    // The last prompt line on the screen: the box is drawn below everything.
    let start = lines.iter().rposition(|row| {
        row.first().map(|c| c.0) == Some('❯') && matches!(row.get(1).map(|c| c.0), None | Some(' ' | '\u{a0}'))
    })?;
    if start == 0 || !is_rule(&lines[start - 1]) {
        return None;
    }
    let mut end = start + 1;
    while end < lines.len() && !is_rule(&lines[end]) {
        if !indented(&lines[end]) && !text(&lines[end]).trim().is_empty() {
            return None;
        }
        end += 1;
    }
    if end == lines.len() {
        return None;
    }
    let below: String = lines[end + 1..].iter().map(|r| text(r)).collect::<Vec<_>>().join(" ");
    if below.contains("NORMAL") {
        return None;
    }
    Some((start, end))
}

/// claude's input box on a screen, as its rows past the `❯ ` marker; `None`
/// where it is no box this module recognizes.
fn claude_box(lines: &[Vec<(char, bool)>]) -> Option<Vec<&[(char, bool)]>> {
    let (start, end) = claude_box_at(lines)?;
    let first = &lines[start][2.min(lines[start].len())..];
    Some(std::iter::once(first).chain(lines[start + 1..end].iter().map(|r| &r[2.min(r.len())..])).collect())
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
    // A blank line, then the model footer, `<model> · <path>`; or, while a
    // turn runs with something in the box, the hint that Tab queues it
    // (`codex-0.153.4-working-paste-160x45-e.txt`).
    let blank = |row: usize| lines.get(row).is_some_and(|r| text(r).trim().is_empty());
    let footer = lines.get(end + 1).is_some_and(|r| {
        let t = text(r);
        t.starts_with("  ") && (t.contains(" · ") || t.trim_start().starts_with("tab to queue message"))
    });
    if !blank(end) || !footer {
        // Blank rows inside the box: a paste's empty lines
        // (`codex-0.153.4-blank-lines-160x45-e.txt`). Read only up to a
        // footer that can be nothing else, the screen's last row with
        // anything on it, so a picker's rows and hint below a box
        // (`codex-0.153.4-mention-no-matches-160x45-e.txt`) never pass for
        // the box's own.
        let Some(last) = lines.iter().rposition(|r| !text(r).trim().is_empty()) else { return Composer::Unrecognized };
        let inside = |r: &Vec<(char, bool)>| indented(r) || text(r).trim().is_empty();
        if last <= end + 1 || !blank(last - 1) || !is_model_footer(&lines[last]) || !lines[end..last].iter().all(inside) {
            return Composer::Unrecognized;
        }
        end = last - 1;
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

/// codex's model footer and nothing else: `  <model> <effort> · <path>`, the
/// path codex's working directory (`/…` or `~/…`), after it perhaps `Vim:
/// Normal`; or `tab to queue message` while a turn runs. A picker's hint,
/// `enter insert · esc close · ←/→ switch search modes`, has the dot but
/// no path after it.
fn is_model_footer(row: &[(char, bool)]) -> bool {
    let t = text(row);
    if !t.starts_with("  ") || t.starts_with("   ") {
        return false;
    }
    let t = t.trim();
    t.starts_with("tab to queue message") || t.rsplit_once(" · ").is_some_and(|(_, path)| path.starts_with(['/', '~']))
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_protocol::v1::AgentActivity;

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
            ("cursor", "cursor-blocked.txt"),
            ("cursor", "cursor-trust-gate.txt"),
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

    /// A message pasted into a working agent's box, from real screens
    /// (claude 2.1.290, codex 0.153.4, with escapes): the box is read while the
    /// turn runs, codex's footer then being the hint that Tab queues it.
    #[test]
    fn a_working_agents_box_holds_what_was_pasted() {
        let classify = |preset: &str, screen: &str| crate::activity::Registry::built_in().classify(preset, screen);
        // claude drops `esc to interrupt` from its footer while its box holds
        // something, so this one classifies by its box alone.
        let claude = capture("claude-2.1.290-working-paste-160x45-e.txt");
        assert_eq!(read("claude", &claude), Composer::Holds("queued note: reply PINEAPPLE".into()));
        let codex = capture("codex-0.153.4-working-paste-160x45-e.txt");
        assert_eq!(classify("codex", &codex), AgentActivity::Working);
        assert_eq!(read("codex", &codex), Composer::Holds("queued note: reply PINEAPPLE".into()));
        assert_eq!(read("codex", &codex.replace("tab to queue message", "tab to see more")), Composer::Unrecognized);
    }

    /// Once claude has queued a message its box shows a dim hint with the
    /// cursor, in reverse video, on the hint's first letter: empty. A
    /// character someone typed under the cursor is still theirs.
    #[test]
    fn the_cursor_on_a_dim_placeholder_is_the_placeholder() {
        let queued = capture("claude-2.1.290-working-queued-160x45-e.txt");
        assert_eq!(crate::activity::Registry::built_in().classify("claude", &queued), AgentActivity::Working);
        assert_eq!(read("claude", &queued), Composer::Empty);
        let rule = "─".repeat(20);
        let boxed = |line: &str| format!("{rule}\n{line}\n{rule}\n  ? for shortcuts\n");
        assert_eq!(read("claude", &boxed("❯\u{a0}\x1b[7mP\x1b[0;2mress up\x1b[0m")), Composer::Empty);
        assert_eq!(read("claude", &boxed("❯\u{a0}\x1b[7mP\x1b[0mress up")), Composer::Holds("Press up".into()));
        assert_eq!(read("claude", &boxed("❯\u{a0}fix\x1b[7m \x1b[0m")), Composer::Holds("fix".into()));
        assert_eq!(read("claude", &boxed("❯\u{a0}\x1b[7mx\x1b[27m\x1b[2m \x1b[0m")), Composer::Holds("x".into()));
    }

    /// claude 2.1.292's real screens, with escapes: the dim `Try "…"`
    /// suggestion in a fresh box, the box mid-turn, and the box after a
    /// turn are all empty, whatever the footer says about the permission
    /// mode or agents. The `auto mode` and `←3 agents` footers are put in by
    /// hand, from the owner's screen: the sandbox has neither.
    #[test]
    fn claude_2_1_292_suggestion_and_footers_read_empty() {
        let classify = |screen: &str| crate::activity::Registry::built_in().classify("claude", screen);
        let fresh = capture("claude-2.1.292-idle-placeholder-160x45-e.txt");
        let working = capture("claude-2.1.292-working-160x45-e.txt");
        let after = capture("claude-2.1.292-after-turn-160x45-e.txt");
        assert!(printed(&fresh).contains("❯\u{a0}Try \"how does <filepath> work?\""));
        assert_eq!(classify(&fresh), AgentActivity::Idle);
        assert_eq!(classify(&working), AgentActivity::Working);
        assert_eq!(classify(&after), AgentActivity::Idle);
        let auto = "⏵⏵ auto mode on (shift+tab to cycle) · ←3 agents";
        for (name, screen, footer) in [
            ("fresh", &fresh, "⏸ manual mode on · ? for shortcuts · ← for agents"),
            ("working", &working, "⏵⏵ accept edits on\x1b[38;5;246m (shift+tab to cycle) · esc to interrupt · ← for agents"),
            ("after", &after, "⏵⏵ accept edits on\x1b[38;5;246m (shift+tab to cycle) · ← for agents"),
        ] {
            assert!(screen.contains(footer), "{name}: the footer as captured");
            assert_eq!(read("claude", screen), Composer::Empty, "{name}");
            assert_eq!(read("claude", &screen.replace(footer, auto)), Composer::Empty, "{name}, auto mode");
        }
        // The suggestion is empty for its dim alone: drawn plain, it's a draft.
        let plain = fresh.replace("\x1b[2mTry", "Try");
        assert_eq!(read("claude", &plain), Composer::Holds("Try \"how does <filepath> work?\"".into()));
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
