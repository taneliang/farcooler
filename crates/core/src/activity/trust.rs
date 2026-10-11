//! Claude Code's folder-trust prompt (ov-493): the first screen claude draws
//! in a folder it hasn't been told to trust, and everything waits behind it.
//!
//! Far Cooler makes a lane's worktree under its own runtime directory, which
//! no person ever opened claude in, so a lane's agent can open on this prompt
//! and sit there. `classify` already reads it as Blocked (`claude-trust-gate
//! .txt` holds the real screen); this names it, so the runner can offer the
//! owner a one-tap answer and say why a lane hasn't started. It's a security
//! prompt: Far Cooler never answers it on its own.
//!
//! Read off the footer, as every other state is, so a transcript quoting the
//! words can't raise it. The real screen, from claude 2.1.233:
//!
//! ```text
//!  Claude Code'll be able to read, edit, and execute files here.
//!
//!  Security guide
//!
//!  ❯ 1. Yes, I trust this folder
//!    2. No, exit
//!
//!  Enter to confirm · Esc to cancel
//! ```
//!
//! Earlier claudes asked "Do you trust the files in this folder?" over
//! `1. Yes, proceed`; every wording seen ends its options with `No, exit`, so
//! that and a trust phrase together are what's matched.

use super::{QUESTION_LINES, footer_text};

/// What a Needs You row and a lane notice call the prompt.
pub const QUESTION: &str = "Trust this folder?";

/// The phrases, any one of which names the prompt beside `EXIT`.
const TRUST: [&str; 2] = ["trust this folder", "trust the files in this folder"];

/// Option 2 of every wording of the prompt.
const EXIT: &str = "2. No, exit";

/// Option 1 under claude's cursor: what one Enter accepts.
const YES_SELECTED: &str = "❯ 1. Yes";

/// Whether `screen`'s footer is claude's folder-trust prompt.
pub fn shown(screen: &str) -> bool {
    let footer = footer_text(screen, QUESTION_LINES);
    footer.contains(EXIT) && TRUST.iter().any(|t| footer.contains(t))
}

/// Whether the prompt is up with its cursor on Yes, so that one Enter
/// trusts the folder and nothing else.
pub fn yes_selected(screen: &str) -> bool {
    shown(screen) && footer_text(screen, QUESTION_LINES).contains(YES_SELECTED)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::Registry;
    use farcooler_protocol::v1::AgentActivity;

    const GATE: &str = include_str!("../../captures/claude-trust-gate.txt");

    /// The real screen is the prompt, with Yes under the cursor, and asks
    /// the one question a row can show.
    #[test]
    fn the_real_trust_gate_is_named() {
        assert!(shown(GATE));
        assert!(yes_selected(GATE));
        let r = Registry::built_in();
        assert_eq!(r.classify("claude", GATE), AgentActivity::Blocked);
        assert_eq!(r.blocked_question("claude", GATE).as_deref(), Some(QUESTION));
    }

    /// The cursor moved to No: still the prompt, but Enter would exit, so
    /// it isn't offered as a yes.
    #[test]
    fn a_cursor_on_no_is_not_a_yes() {
        let moved = GATE.replace(" ❯ 1. Yes, I trust this folder", "   1. Yes, I trust this folder").replace("   2. No, exit", " ❯ 2. No, exit");
        assert!(shown(&moved));
        assert!(!yes_selected(&moved));
    }

    /// Earlier claude's wording is the same prompt.
    #[test]
    fn the_older_wording_is_the_same_prompt() {
        let older = " Do you trust the files in this folder?\n\n /tmp/x\n\n ❯ 1. Yes, proceed\n   2. No, exit\n\n Enter to confirm · Esc to exit\n";
        assert!(yes_selected(older));
    }

    /// Every other real screen isn't it, including a permission prompt,
    /// which also puts a cursor on `1. Yes`, and a transcript quoting the
    /// prompt above an idle box.
    #[test]
    fn no_other_screen_is_the_trust_prompt() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/captures");
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if !name.ends_with(".txt") || name == "claude-trust-gate.txt" {
                continue;
            }
            let screen = std::fs::read_to_string(&path).unwrap();
            assert!(!shown(&screen), "{name} read as claude's trust prompt");
        }
        let idle = include_str!("../../captures/claude-idle-fresh.txt");
        let quoted = format!("⏺ It asked: 1. Yes, I trust this folder / 2. No, exit\n{}", "\n".repeat(20)) + idle;
        assert!(!shown(&quoted));
    }
}
