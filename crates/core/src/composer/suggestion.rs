//! The next prompt claude suggests, read off its empty box (ov-409).
//!
//! After a turn claude can draw a predicted next prompt, dim, in its empty
//! input box; Tab takes it into the draft. It is written nowhere else: the
//! transcript and the hooks never carry it (in the TUI it is held in memory;
//! only `--output-format stream-json` surfaces it), so the screen is the one
//! source, read with its escapes like the rest of this module.
//!
//! The same dim slot also shows hints that are not predictions, and these
//! are NOT offered:
//!
//! - `Try "how does <filepath> work?"`: a rotating example in a fresh
//!   session's box. It is a hint about what can be asked, carries
//!   placeholders such as `<filepath>`, and Tab does not take it in claude
//!   either (`claude-2.1.292-idle-placeholder-160x45-e.txt`).
//! - `Press up to edit queued messages` and its siblings, shown while a
//!   queue exists (`claude-2.1.290-working-queued-160x45-e.txt`).
//!
//! The caller offers a suggestion only while the agent rests: a hint shown
//! mid-turn is never a prediction.

use super::{Composer, cells, claude_box, content, text};

/// The longest suggestion kept, in characters. claude's are a sentence; a
/// dim paragraph is something else.
const MAX_CHARS: usize = 300;

/// The suggestion in `preset`'s empty box on `screen`, if there is one.
/// Only claude draws one. `None` for a box holding anything typed, for no
/// box, and for the hints above.
pub fn suggestion(preset: &str, screen: &str) -> Option<String> {
    if preset.split(':').next() != Some("claude") {
        return None;
    }
    let lines: Vec<_> = screen.lines().map(cells).collect();
    let rows = claude_box(&lines)?;
    if content(&rows) != Composer::Empty {
        return None;
    }
    let dim: String = rows.iter().map(|row| row.iter().filter(|(_, dim)| *dim).map(|(c, _)| *c).collect::<String>()).collect::<Vec<_>>().join(" ");
    let dim = dim.split_whitespace().collect::<Vec<_>>().join(" ");
    let blank = rows.iter().all(|row| text(row).trim().is_empty());
    (!blank && !is_hint(&dim) && dim.chars().count() <= MAX_CHARS).then_some(dim)
}

/// Whether `dim` is one of claude's hints rather than a prediction.
fn is_hint(dim: &str) -> bool {
    dim.is_empty() || dim.starts_with("Try \"") || dim.starts_with("Press ")
}

#[cfg(test)]
mod tests {
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
        assert_eq!(suggestion("claude", &capture("claude-2.1.292-working-160x45-e.txt")), None);
        assert_eq!(suggestion("claude", &capture("claude-2.1.292-after-turn-160x45-e.txt")), None);
        let queued = capture("claude-2.1.290-working-queued-160x45-e.txt");
        assert_eq!(super::super::read("claude", &queued), Composer::Empty);
        assert_eq!(suggestion("claude", &queued), None);
        // The words of a prediction in the very slot of the example: offered.
        let swapped = fresh.replace("Try \"how does <filepath> work?\"", "run the tests again");
        assert_eq!(suggestion("claude", &swapped).as_deref(), Some("run the tests again"));
    }

    #[test]
    fn only_claude_has_one_and_menus_have_none() {
        assert_eq!(suggestion("codex", &predicted()), None);
        assert_eq!(suggestion("claude", &capture("claude-blocked.txt")), None);
        assert_eq!(suggestion("claude", ""), None);
        let long = predicted().replace("wait for the background shell to finish", &"word ".repeat(80));
        assert_eq!(suggestion("claude", &long), None);
    }
}
