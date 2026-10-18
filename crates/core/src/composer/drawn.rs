//! What claude draws in its box for what was pasted into it, so a paste can
//! be read back when the box doesn't show the text as typed (ov-367).
//!
//! Measured on claude 2.1.290 (haiku, a stand-in API, a 160- and an 80-column
//! pane), with bracketed pastes:
//!
//! - **A paste collapses** to `[Pasted text #N]` when it's longer than 800
//!   UTF-16 units (800 `x` or `é` stay, 801 collapse; 400 emoji stay, 401
//!   collapse), or `[Pasted text #N +K lines]` when it has three or more
//!   line breaks, K being how many (four lines show `+3 lines`). Both rules
//!   hold whatever the pane's width; a long paste with one break shows
//!   `+1 lines`. A CR LF is one break, and so is a CR alone. Three lines
//!   stay as typed. (tmux's `paste-buffer` turns each LF into a CR, so a CR
//!   LF it pastes is two: the daemon sends the bytes as they are.)
//! - **An image's path** pasted on its own, or several separated by spaces,
//!   becomes `[Image #N]` each; a path with other text stays text.
//! - **N** counts both kinds from the session's start, so it can't be known
//!   before the paste: it's read back.
//! - **The hook and the transcript** carry a collapsed paste's whole text, and
//!   an image as its `[Image #N]`: `UserPromptSubmit`'s `prompt` and the
//!   `enqueue` record alike.
//! - **A `/` at the start** opens claude's command popup above the box, its
//!   highlighted row drawn `  ❯ /name` and the rest `    /name`. The
//!   highlight is the best match, which may be another command by an alias:
//!   `/cost` highlights `/usage (cost)`, and Enter runs `/usage`. A space
//!   closes it. Captures: `claude-2.1.290-slash-*-e.txt`.
//!
//! - **A tall box scrolls** (claude 2.1.301, ov-618). The box grows to at
//!   most `tallest` rows, half the pane's height less five, whatever its
//!   width: 3 rows in a 16-row pane, 7 in 24, 17 in 45, 20 in 50, idle or
//!   mid-turn alike. A paste that doesn't collapse but wraps to more rows
//!   shows only its last ones, the `❯` on the first of them mid-text, with
//!   nothing drawn to say so: 700 characters at 80x24, 300 at 40x24. Read
//!   back by its tail (`Expected::shown_on`). Captures:
//!   `claude-2.1.301-scrolled-*-e.txt`.
//!
//! Captures: `claude-2.1.290-image-and-long-paste-160x45-e.txt`,
//! `claude-2.1.290-three-lines-160x45-e.txt`,
//! `claude-2.1.290-working-long-paste-160x45-e.txt`.

use super::{Composer, cells, text};

/// Longer than this, in UTF-16 units, a paste collapses.
pub const COLLAPSES_PAST_UNITS: usize = 800;
/// With this many line breaks or more, a paste collapses.
pub const COLLAPSES_AT_BREAKS: usize = 3;
/// The fewest rows a scrolled box is read back by its tail from: fewer show
/// too little of the text to stand for it.
pub const TAIL_FROM_ROWS: usize = 3;

/// The most rows claude 2.1.301 lets its box grow to in a pane `pane_rows`
/// high, before it scrolls (measured from 16 to 50 rows; this module's
/// docs): the rule Bring Here measured on 2.1.292 (`draft::max_rows`).
pub fn tallest(pane_rows: u32) -> usize {
    super::draft::max_rows(pane_rows)
}

/// One piece of what the box should show.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    /// Shown as is, whitespace aside.
    Text(String),
    /// A collapsed paste with this many line breaks.
    Pasted { breaks: usize },
    /// An image's placeholder, any number.
    Image,
    /// codex's collapsed paste, `[Pasted Content N chars]` (`codex`).
    Content { chars: usize },
}

/// What a box should show after a series of pastes: their drawn forms in
/// order. Whitespace is ignored, as the box wraps and reflows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Expected {
    pieces: Vec<Piece>,
}

impl Expected {
    /// `text` shown exactly: what `holds_exactly` checks.
    pub fn plain(text: &str) -> Expected {
        Expected { pieces: vec![Piece::Text(squeeze(text))] }
    }

    /// Then `pasted`, as claude draws a paste of it: as is, or collapsed.
    pub fn then_paste(mut self, pasted: &str) -> Expected {
        let breaks = pasted.matches('\n').count();
        let piece = if collapses(pasted) { Piece::Pasted { breaks } } else { Piece::Text(squeeze(pasted)) };
        self.pieces.push(piece);
        self
    }

    /// Then `pasted`, as codex 0.153.4 draws a paste of it: as is, or past
    /// `codex::COLLAPSES_PAST_CHARS` as `[Pasted Content N chars]`.
    pub fn then_codex_paste(mut self, pasted: &str) -> Expected {
        let chars = pasted.chars().count();
        let piece = if super::codex::collapses(pasted) { Piece::Content { chars } } else { Piece::Text(squeeze(pasted)) };
        self.pieces.push(piece);
        self
    }

    /// Then one image's placeholder.
    pub fn then_image(mut self) -> Expected {
        self.pieces.push(Piece::Image);
        self
    }

    /// Whether `held` shows exactly this.
    pub fn shown_by(&self, held: &Composer) -> bool {
        let Composer::Holds(held) = held else { return false };
        let mut rest = squeeze(held);
        for piece in &self.pieces {
            let after = match piece {
                Piece::Text(want) => rest.strip_prefix(want.as_str()).map(str::to_string),
                Piece::Pasted { breaks } => placeholder(&rest, "[Pastedtext#").and_then(|after| match breaks {
                    0 => after.strip_prefix(']').map(str::to_string),
                    k => after.strip_prefix(&format!("+{k}lines]")).map(str::to_string),
                }),
                Piece::Image => placeholder(&rest, "[Image#").and_then(|after| after.strip_prefix(']').map(str::to_string)),
                Piece::Content { chars } => {
                    rest.strip_prefix(&format!("[PastedContent{chars}chars]")).map(str::to_string)
                }
            };
            match after {
                Some(after) => rest = after,
                None => return false,
            }
        }
        rest.is_empty()
    }

    /// Whether `screen` shows this in `preset`'s box: as `shown_by` does,
    /// or, for claude, scrolled (ov-618). A scrolled box is exactly
    /// `tallest` rows for the pane's height, at least `TAIL_FROM_ROWS`, and
    /// shows this one's end and nothing else (`tail_shown_by`). The height
    /// is the capture's own, a row a line (`capture-pane -p` prints every
    /// row), so a resize can't set the two apart.
    ///
    /// What scrolled out of sight above can't be read, as a collapsed
    /// paste's text never is. Every caller reads its box empty right before
    /// its first paste (`box_still_empty`) and refuses once anyone typed
    /// through Far Cooler since (`last_input`); a draft typed straight into
    /// the terminal between that capture and the paste is the race left.
    /// A compose also checks after its Enter that claude's record of the
    /// prompt is the text sent (`took`), and a send mid-turn that the queue
    /// holds it (`queued`); a tell or an answer between turns has no such
    /// check, so for them those two guards are all.
    pub fn shown_on(&self, preset: &str, screen: &str) -> bool {
        let held = super::read(preset, screen);
        if self.shown_by(&held) {
            return true;
        }
        let Composer::Holds(held) = &held else { return false };
        let pane_rows = u32::try_from(screen.lines().count()).unwrap_or(u32::MAX);
        preset.split(':').next() == Some("claude")
            && super::claude_rows(screen).is_some_and(|rows| rows >= TAIL_FROM_ROWS && rows == tallest(pane_rows))
            && self.tail_shown_by(held)
    }

    /// Whether `held` is this one's end: read from the last piece back,
    /// the box's top cutting a text piece or falling between two pieces,
    /// every piece below the cut whole, and nothing in the box besides.
    /// A cut through a placeholder isn't read.
    fn tail_shown_by(&self, held: &str) -> bool {
        let mut rest = squeeze(held);
        for piece in self.pieces.iter().rev() {
            if rest.is_empty() {
                // The top falls between pieces: each above was read back
                // after its own paste.
                return true;
            }
            let before = match piece {
                Piece::Text(want) => match rest.strip_suffix(want.as_str()) {
                    Some(before) => Some(before.to_string()),
                    None => return want.ends_with(rest.as_str()),
                },
                Piece::Pasted { breaks } => {
                    let end = if *breaks == 0 { "]".to_string() } else { format!("+{breaks}lines]") };
                    placeholder_before(&rest, "[Pastedtext#", &end)
                }
                Piece::Image => placeholder_before(&rest, "[Image#", "]"),
                Piece::Content { chars } => rest.strip_suffix(&format!("[PastedContent{chars}chars]")).map(str::to_string),
            };
            match before {
                Some(before) => rest = before,
                None => return false,
            }
        }
        rest.is_empty()
    }
}

/// Whether claude collapses a paste of `text` into a placeholder.
pub fn collapses(text: &str) -> bool {
    text.matches('\n').count() >= COLLAPSES_AT_BREAKS || text.encode_utf16().count() > COLLAPSES_PAST_UNITS
}

/// `rest` past `head` and the digits after it, or `None`.
fn placeholder(rest: &str, head: &str) -> Option<String> {
    let after = rest.strip_prefix(head)?;
    let digits = after.chars().take_while(char::is_ascii_digit).count();
    (digits > 0).then(|| after[digits..].to_string())
}

/// `rest` short of a placeholder `head`, digits, `end` at its end, or `None`.
fn placeholder_before(rest: &str, head: &str, end: &str) -> Option<String> {
    let numbered = rest.strip_suffix(end)?;
    let digits = numbered.chars().rev().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    numbered[..numbered.len() - digits].strip_suffix(head).map(str::to_string)
}

/// The `[Image #N]` placeholders a box holds, in order, as drawn.
pub fn images(held: &Composer) -> Vec<String> {
    let Composer::Holds(held) = held else { return Vec::new() };
    let mut found = Vec::new();
    let mut rest = held.as_str();
    while let Some(at) = rest.find("[Image #") {
        let after = &rest[at + "[Image #".len()..];
        let digits = after.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && after[digits..].starts_with(']') {
            found.push(format!("[Image #{}]", &after[..digits]));
        }
        rest = after;
    }
    found
}

/// The command claude's popup highlights above its box, `/name` with any
/// alias dropped (`/usage (cost)` is `/usage`), or `None` when no popup is
/// up. Only the rows directly above the box's top rule are read, up to a
/// blank one.
pub fn highlighted_command(screen: &str) -> Option<String> {
    let lines: Vec<String> = screen.lines().map(|l| text(&cells(l))).collect();
    let start = lines.iter().rposition(|l| l.starts_with('❯') && matches!(l.chars().nth(1), None | Some(' ' | '\u{a0}')))?;
    let rule = start.checked_sub(1)?;
    let mut highlighted = None;
    for line in lines[..rule].iter().rev() {
        if line.trim().is_empty() {
            break;
        }
        if let Some(row) = line.strip_prefix("  ❯ ").filter(|r| r.starts_with('/')) {
            if highlighted.is_some() {
                return None;
            }
            highlighted = row.split_whitespace().next().map(str::to_string);
        }
    }
    highlighted
}

fn squeeze(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

#[cfg(test)]
mod tests {
    use super::super::read;
    use super::*;

    fn capture(name: &str) -> String {
        std::fs::read_to_string(format!("{}/captures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    /// The thresholds, as measured: 800 UTF-16 units, three line breaks.
    #[test]
    fn a_paste_collapses_past_800_units_or_at_three_breaks() {
        assert!(!collapses(&"x".repeat(800)));
        assert!(collapses(&"x".repeat(801)));
        assert!(!collapses(&"é".repeat(800)));
        assert!(!collapses(&"😀".repeat(400)));
        assert!(collapses(&"😀".repeat(401)));
        assert!(!collapses("a\nb\nc"));
        assert!(collapses("a\nb\nc\nd"));
        assert!(collapses(&format!("{}\n{}", "y".repeat(500), "y".repeat(400))));
    }

    /// Real screens: a short multi-line paste is shown as typed; an image and
    /// a five-line paste are their placeholders, whatever their numbers.
    #[test]
    fn a_box_is_read_back_against_what_claude_draws() {
        let three = read("claude", &capture("claude-2.1.290-three-lines-160x45-e.txt"));
        assert!(Expected::default().then_paste("first line\nsecond line\nthird line").shown_by(&three));
        assert!(!Expected::default().then_paste("first line\nsecond line\nthird lines").shown_by(&three));
        let steps = "step 1\nstep 2\nstep 3\nstep 4\nstep 5";
        let both = read("claude", &capture("claude-2.1.290-image-and-long-paste-160x45-e.txt"));
        assert!(Expected::default().then_image().then_paste(steps).shown_by(&both), "{both:?}");
        assert_eq!(images(&both), ["[Image #50]"]);
        assert!(!Expected::default().then_paste(steps).shown_by(&both), "the image is missing");
        assert!(!Expected::default().then_image().then_paste("step 1\nstep 2\nstep 3\nstep 4").shown_by(&both), "+3, not +4");
        assert!(!Expected::default().then_image().then_image().then_paste(steps).shown_by(&both));
        let working = read("claude", &capture("claude-2.1.290-working-long-paste-160x45-e.txt"));
        assert!(Expected::default().then_paste("later 1\nlater 2\nlater 3\nlater 4\nlater 5").shown_by(&working));
        assert!(Expected::plain("fix it").shown_by(&Composer::Holds("fix  it".into())));
        assert!(!Expected::plain("").shown_by(&Composer::Empty));
    }

    /// A long paste with no break has no `+K lines`; text typed around a
    /// placeholder is read in order.
    #[test]
    fn a_placeholder_is_read_in_its_place() {
        let long = "x".repeat(900);
        let held = |s: &str| Composer::Holds(s.into());
        assert!(Expected::default().then_paste(&long).shown_by(&held("[Pasted text #7]")));
        assert!(!Expected::default().then_paste(&long).shown_by(&held("[Pasted text #7 +1 lines]")));
        assert!(!Expected::default().then_paste(&long).shown_by(&held("[Pasted text #]")));
        let args = Expected::plain("/init").then_paste(" a\nb\nc\nd");
        assert!(args.shown_by(&held("/init [Pasted text #3 +3 lines]")));
        assert!(!args.shown_by(&held("/init [Pasted text #3 +3 lines] more")));
        assert_eq!(images(&held("[Image #3] [Image #4]look [Image #x]")), ["[Image #3]", "[Image #4]"]);
    }

    /// The popup's highlight is the command it names, alias dropped, and only
    /// when the popup is up.
    #[test]
    fn the_popups_highlight_is_read() {
        assert_eq!(highlighted_command(&capture("claude-2.1.290-slash-exact-160x45-e.txt")).as_deref(), Some("/init"));
        assert_eq!(highlighted_command(&capture("claude-2.1.290-slash-alias-160x45-e.txt")).as_deref(), Some("/usage"));
        assert_eq!(highlighted_command(&capture("claude-2.1.290-three-lines-160x45-e.txt")), None);
        assert_eq!(highlighted_command(&capture("claude-idle-fresh.txt")), None);
        // The popup is no dialog: the screen still reads idle, its box holding
        // what was pasted.
        let classify = |screen: &str| crate::activity::Registry::built_in().classify("claude", screen);
        for name in ["slash-exact", "slash-alias", "three-lines", "image-and-long-paste"] {
            let screen = capture(&format!("claude-2.1.290-{name}-160x45-e.txt"));
            assert_eq!(classify(&screen), farcooler_protocol::v1::AgentActivity::Idle, "{name}");
        }
        assert_eq!(read("claude", &capture("claude-2.1.290-slash-alias-160x45-e.txt")), Composer::Holds("/cost".into()));
    }

    /// After a long paste was queued, claude 2.1.290 keeps `paste again to
    /// expand` where `esc to interrupt` was. It still works, and says so with
    /// its spinner row (`✻ Working…`) and the queued rows' `ctrl+x ctrl+s to
    /// send now`: either alone reads Working (ov-392). A turn's done line
    /// (`✻ Worked for 3s`) has no ellipsis and doesn't.
    #[test]
    fn a_queued_long_paste_still_reads_working() {
        use farcooler_protocol::v1::AgentActivity::{Idle, Working};
        let classify = |screen: &str| crate::activity::Registry::built_in().classify("claude", screen);
        let screen = capture("claude-2.1.290-working-queued-long-paste-120x45-e.txt");
        assert!(crate::composer::printed(&screen).contains("paste again to expand"));
        assert_eq!(read("claude", &screen), Composer::Empty);
        assert_eq!(classify(&screen), Working);
        let no_spinner = screen.replace("Working… ", "");
        assert!(!crate::composer::printed(&no_spinner).contains("Working"));
        assert_eq!(classify(&no_spinner), Working, "the queued rows alone");
        let no_queue = screen.replace("ctrl+x ctrl+s to send now", "");
        assert_eq!(classify(&no_queue), Working, "the spinner alone");
        let done = no_queue.replace("Working… ", "Worked for 3s · done");
        assert_eq!(classify(&done), Idle, "a done line");
    }
}
