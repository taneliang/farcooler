//! What claude's empty box says, dim or grey, read off its screen (ov-409).
//!
//! After a turn claude can draw a predicted next prompt in its empty input
//! box; Tab takes it into the draft. It is written nowhere else: the
//! transcript and the hooks never carry it (in the TUI it is held in memory;
//! only `--output-format stream-json` surfaces it), so the screen is the one
//! source, read with its escapes like the rest of this module. It is drawn
//! dim (SGR 2) like claude's other placeholders; a grey foreground (SGR 90,
//! a 256 or true color grey) counts too, so a prediction drawn in color is
//! not missed.
//!
//! The same slot shows three kinds of text, told apart by their words:
//!
//! - a prediction (`suggestion`): anything else. The composer offers it, and
//!   Tab or a tap takes it into the draft.
//! - the generic example (`hint`), `Try "how does <filepath> work?"`, in a
//!   fresh session's box (`claude-2.1.292-idle-placeholder-160x45-e.txt`).
//!   The composer shows it as its placeholder, as a hint: claude does not
//!   take it on Tab either, and it has placeholders such as `<filepath>`.
//! - a queue hint, `Press up to edit queued messages` and its siblings
//!   (`claude-2.1.290-working-queued-160x45-e.txt`): shown by neither.
//!
//! The caller reads these only while the agent rests: a dim line shown
//! mid-turn is never a prediction.

use super::{cells, claude_box_at, styled};

/// The longest text kept, in characters. claude's are a sentence; a dim
/// paragraph is something else.
const MAX_CHARS: usize = 300;

/// The prediction in `preset`'s empty box on `screen`, if there is one. Only
/// claude draws one. `None` for a box holding anything typed, for no box,
/// and for the example and the queue hints.
pub fn suggestion(preset: &str, screen: &str) -> Option<String> {
    placeholder(preset, screen).filter(|words| !is_example(words) && !is_queue_hint(words))
}

/// The generic `Try "…"` example in claude's empty box, if that is what it
/// shows.
pub fn hint(preset: &str, screen: &str) -> Option<String> {
    placeholder(preset, screen).filter(|words| is_example(words))
}

/// The dim or grey words of claude's box, when nothing else is in it.
fn placeholder(preset: &str, screen: &str) -> Option<String> {
    if preset.split(':').next() != Some("claude") {
        return None;
    }
    let plain: Vec<_> = screen.lines().map(cells).collect();
    let lines: Vec<_> = screen.lines().map(styled).collect();
    let (start, end) = claude_box_at(&plain)?;
    let mut words = String::new();
    for row in &lines[start..end] {
        for &(c, dim, grey) in row.iter().skip(2) {
            if c.is_whitespace() {
                words.push(' ');
            } else if dim || grey {
                words.push(c);
            } else {
                // Typed: a draft, not a placeholder.
                return None;
            }
        }
        words.push(' ');
    }
    let words = words.split_whitespace().collect::<Vec<_>>().join(" ");
    (!words.is_empty() && words.chars().count() <= MAX_CHARS).then_some(words)
}

fn is_example(words: &str) -> bool {
    words.starts_with("Try \"")
}

fn is_queue_hint(words: &str) -> bool {
    words.starts_with("Press ")
}

#[cfg(test)]
mod tests {
    use super::super::Composer;
    use super::*;
    use farcooler_protocol::v1::AgentActivity;

    fn capture(name: &str) -> String {
        std::fs::read_to_string(format!("{}/captures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    /// The one real prediction in the corpus, `claude-idle-nothing-running`,
    /// drawn dim as claude draws its placeholders (that capture predates
    /// `-e`; every other dim in this module is a real escape).
    fn predicted() -> String {
        capture("claude-idle-nothing-running.txt")
            .replace("❯\u{a0}wait for the background shell to finish", "❯\u{a0}\x1b[2mwait for the background shell to finish\x1b[0m")
    }

    #[test]
    fn a_dim_prediction_in_an_empty_box_is_offered() {
        let screen = predicted();
        assert_eq!(crate::activity::Registry::built_in().classify("claude", &screen), AgentActivity::Idle);
        assert_eq!(suggestion("claude", &screen).as_deref(), Some("wait for the background shell to finish"));
        assert_eq!(suggestion("claude:opus", &screen).as_deref(), Some("wait for the background shell to finish"));
        // The cursor on its first letter, in reverse video: still the prediction.
        let cursor = screen.replace("\x1b[2mwait", "\x1b[7mw\x1b[0;2mait");
        assert_eq!(suggestion("claude", &cursor).as_deref(), Some("wait for the background shell to finish"));
        // Wrapped onto the box's second row.
        let wrapped = screen.replace("shell to finish\x1b[0m", "shell to\x1b[0m\n  \x1b[2mfinish\x1b[0m");
        assert_eq!(suggestion("claude", &wrapped).as_deref(), Some("wait for the background shell to finish"));
    }

    /// Drawn plain it cannot be told from typing, so it is not offered
    /// (and `composer::read` calls it a draft).
    #[test]
    fn what_is_not_dim_is_not_a_suggestion() {
        let plain = capture("claude-idle-nothing-running.txt");
        assert_eq!(suggestion("claude", &plain), None);
        let typed = predicted().replace("\x1b[2mwait for the background shell to finish\x1b[0m", "fix the bug");
        assert_eq!(suggestion("claude", &typed), None);
        // A draft with a dim tail is a draft.
        let mixed = predicted().replace("\x1b[2mwait", "fix \x1b[2mwait");
        assert_eq!(suggestion("claude", &mixed), None);
    }

    /// claude 2.1.292's real screens: the generic example, the box mid-turn
    /// and after a turn offer nothing. The `Try` example is dim in an empty
    /// box exactly as a prediction is, so only its words tell them apart.
    #[test]
    fn the_generic_example_and_the_hints_are_not_predictions() {
        let fresh = capture("claude-2.1.292-idle-placeholder-160x45-e.txt");
        assert!(super::super::printed(&fresh).contains("❯\u{a0}Try \"how does <filepath> work?\""));
        assert_eq!(super::super::read("claude", &fresh), Composer::Empty);
        assert_eq!(suggestion("claude", &fresh), None);
        // The example is the box's hint, and only the example is.
        assert_eq!(hint("claude", &fresh).as_deref(), Some("Try \"how does <filepath> work?\""));
        assert_eq!(hint("claude:opus", &fresh).as_deref(), Some("Try \"how does <filepath> work?\""));
        assert_eq!(hint("codex", &fresh), None);
        assert_eq!(suggestion("claude", &capture("claude-2.1.292-working-160x45-e.txt")), None);
        assert_eq!(hint("claude", &capture("claude-2.1.292-working-160x45-e.txt")), None);
        assert_eq!(hint("claude", &capture("claude-2.1.292-after-turn-160x45-e.txt")), None);
        assert_eq!(suggestion("claude", &capture("claude-2.1.292-after-turn-160x45-e.txt")), None);
        let queued = capture("claude-2.1.290-working-queued-160x45-e.txt");
        assert_eq!(super::super::read("claude", &queued), Composer::Empty);
        assert_eq!(suggestion("claude", &queued), None);
        assert_eq!(hint("claude", &queued), None, "a queue hint is shown by neither");
        // The words of a prediction in the very slot of the example: offered.
        let swapped = fresh.replace("Try \"how does <filepath> work?\"", "run the tests again");
        assert_eq!(suggestion("claude", &swapped).as_deref(), Some("run the tests again"));
        assert_eq!(hint("claude", &swapped), None, "a prediction is not the example");
    }

    #[test]
    fn only_claude_has_one_and_menus_have_none() {
        assert_eq!(suggestion("codex", &predicted()), None);
        assert_eq!(suggestion("claude", &capture("claude-blocked.txt")), None);
        assert_eq!(suggestion("claude", ""), None);
        let long = predicted().replace("wait for the background shell to finish", &"word ".repeat(80));
        assert_eq!(suggestion("claude", &long), None);
    }

    /// A prediction drawn in a grey foreground rather than dim: SGR 90, a 256
    /// grey and a true color grey are each offered; a color, or the default
    /// foreground a person's typing gets, is not.
    #[test]
    fn a_grey_prediction_is_offered_like_a_dim_one() {
        let words = "wait for the background shell to finish";
        let plain = capture("claude-idle-nothing-running.txt");
        let drawn = |sgr: &str| plain.replace(&format!("❯\u{a0}{words}"), &format!("❯\u{a0}\x1b[{sgr}m{words}\x1b[0m"));
        for sgr in ["90", "38;5;246", "38;5;244", "38;5;8", "38;2;128;128;128"] {
            assert_eq!(suggestion("claude", &drawn(sgr)).as_deref(), Some(words), "SGR {sgr}");
        }
        // Not greys: a color, white, and black-ish ones are a person's or claude's own text.
        for sgr in ["32", "91", "38;5;2", "38;5;196", "38;2;200;40;40", "38;2;255;255;255", "38;2;20;20;20", "37"] {
            assert_eq!(suggestion("claude", &drawn(sgr)), None, "SGR {sgr}");
        }
        // A reset ends the grey: what follows in the default color is typed.
        let mixed = plain.replace(&format!("❯\u{a0}{words}"), "❯\u{a0}\x1b[90mwait\x1b[0m for");
        assert_eq!(suggestion("claude", &mixed), None);
        // And the Try example, drawn grey, is still the hint.
        let fresh = capture("claude-2.1.292-idle-placeholder-160x45-e.txt").replace("\x1b[2mTry", "\x1b[90mTry");
        assert_eq!(hint("claude", &fresh).as_deref(), Some("Try \"how does <filepath> work?\""));
        assert_eq!(suggestion("claude", &fresh), None);
        // The cursor on the first letter, in reverse video, over grey words.
        let cursor = drawn("90").replace("\x1b[90mwait", "\x1b[7mw\x1b[0;90mait");
        assert_eq!(suggestion("claude", &cursor).as_deref(), Some(words));
    }
}
