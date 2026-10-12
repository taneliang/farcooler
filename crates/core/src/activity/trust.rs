//! Claude Code's folder-trust prompt (ov-493): the first screen claude draws
//! in a folder it hasn't been told to trust, and everything waits behind it.
//!
//! Far Cooler makes a lane's worktree under its own runtime directory, which
//! no person opened claude in, so a lane's agent can open on this prompt and
//! sit there. `classify` already reads it as Blocked; this names it, so the
//! runner can offer the owner a one-tap answer and say why a lane hasn't
//! started. It's a security prompt: Far Cooler never answers it on its own,
//! and a Trust Folder tap presses one Enter, so this must never mistake
//! another dialog for it (ov-493 review, B1).
//!
//! **Matched by its whole shape, from the bottom up**, never by words
//! anywhere on the screen. A permission dialog also puts `❯ 1. Yes` under
//! its cursor, and its command or diff can quote any words at all, so:
//!
//! 1. the last line is the prompt's own footer, `Enter to confirm · Esc to …`
//!    (a permission dialog's is `Esc to cancel · Tab to amend`);
//! 2. the two lines above it, adjacent, are exactly its two options:
//!    `1. Yes, …` naming trust (or `1. Yes, proceed`), then `2. No, exit`,
//!    with no third option anywhere;
//! 3. above them, its title: `Accessing workspace:` or `Do you trust …?`,
//!    followed by the folder's path, which a row shows;
//! 4. nothing of a permission dialog anywhere in the window: `Do you want`,
//!    `Tab to amend`, a tool's heading.
//!
//! The real screen, `captures/claude-trust-gate.txt`, is claude 2.1.233's
//! (2026-08-16); the claude on this runner is now 2.1.296, which hasn't been
//! captured (no real claude runs in this lane). Small wording changes are
//! tolerated in the slots above (the footer's tail, option 1's words, either
//! title); the shape is not loosened. Earlier claudes asked "Do you trust the
//! files in this folder?" over `1. Yes, proceed`, inside a box.

use super::footer_lines;

/// What a Needs You row and a lane notice call the prompt.
pub const QUESTION: &str = "Trust this folder?";

/// How far up the title may be: the prompt's explanation wraps in a narrow
/// pane, and its path can be long.
const WINDOW: usize = 30;

/// Anything of a permission dialog. Its presence anywhere in the window
/// rules the prompt out.
const PERMISSION: [&str; 5] = ["Do you want", "Tab to amend", "Bash command", "Edit file", "Write file"];

/// The prompt as read off one screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    /// The cursor is on option 1, so one Enter trusts the folder.
    pub yes_selected: bool,
    /// The folder it asks about, as the screen prints it.
    pub folder: Option<String>,
}

/// One line of the window, with the box an older claude drew around it and
/// the cursor taken off: (text, under the cursor).
fn bare(line: &str) -> (String, bool) {
    let t = line.trim().trim_matches(|c| matches!(c, '│' | '╭' | '╮' | '╰' | '╯' | '─')).trim();
    match t.strip_prefix('❯') {
        Some(rest) => (rest.trim().to_string(), true),
        None => (t.to_string(), false),
    }
}

/// Option 1, tolerating its words: a short `1. Yes, …` that grants trust.
fn is_yes(text: &str) -> bool {
    text.len() <= 40
        && text.starts_with("1. Yes")
        && (text.to_lowercase().contains("trust") || text == "1. Yes, proceed")
}

/// Claude's folder-trust prompt, if `screen` is it. See this module's docs.
pub fn read(screen: &str) -> Option<Prompt> {
    let lines: Vec<(String, bool)> = footer_lines(screen, WINDOW).iter().map(|l| bare(l)).collect();
    if lines.iter().any(|(t, _)| PERMISSION.iter().any(|p| t.contains(p))) {
        return None;
    }
    let shown: Vec<usize> = (0..lines.len()).filter(|&i| !lines[i].0.is_empty()).collect();
    let [.., yes, no, footer] = shown.as_slice() else { return None };
    if !lines[*footer].0.starts_with("Enter to confirm · Esc to") {
        return None;
    }
    if *no != *yes + 1 || lines[*no].0 != "2. No, exit" || !is_yes(&lines[*yes].0) {
        return None;
    }
    if lines.iter().any(|(t, _)| t.starts_with("3.")) {
        return None;
    }
    let title = (0..*yes).rev().find(|&i| {
        let t = &lines[i].0;
        t == "Accessing workspace:" || (t.starts_with("Do you trust") && t.ends_with('?'))
    })?;
    // The path: the lines after the title up to a blank one, rejoined where
    // the pane wrapped it.
    let folder: String = lines[title + 1..*yes]
        .iter()
        .skip_while(|(t, _)| t.is_empty())
        .take_while(|(t, _)| !t.is_empty())
        .map(|(t, _)| t.as_str())
        .collect();
    let folder = (folder.starts_with('/') || folder.starts_with('~')).then_some(folder);
    Some(Prompt { yes_selected: lines[*yes].1 && !lines[*no].1, folder })
}

/// Whether `screen` is claude's folder-trust prompt.
pub fn shown(screen: &str) -> bool {
    read(screen).is_some()
}

/// Whether the prompt is up with its cursor on Yes, so that one Enter
/// trusts the folder and nothing else.
pub fn yes_selected(screen: &str) -> bool {
    read(screen).is_some_and(|p| p.yes_selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::Registry;
    use farcooler_protocol::v1::AgentActivity;

    const GATE: &str = include_str!("../../captures/claude-trust-gate.txt");

    /// The real screen is the prompt, with Yes under the cursor, the folder
    /// it asks about, and the one question a row can show.
    #[test]
    fn the_real_trust_gate_is_named() {
        let read = read(GATE).expect("the trust prompt");
        assert!(read.yes_selected);
        assert_eq!(
            read.folder.as_deref(),
            Some("/private/tmp/claude-502/-Users-e-liang-Dev-overnight/9dd767ba-acbf-4124-87b3-c7d8662f60e7/scratchpad/probe")
        );
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

    /// Earlier claude's wording, in its box, is the same prompt; and so is a
    /// small change of words in the slots that may move.
    #[test]
    fn small_changes_of_wording_are_the_same_prompt() {
        let older = "╭──────────────╮\n│ Do you trust the files in this folder? │\n│                │\n│ /tmp/x         │\n│                │\n│ ❯ 1. Yes, proceed │\n│   2. No, exit  │\n╰──────────────╯\n   Enter to confirm · Esc to exit\n";
        assert_eq!(read(older), Some(Prompt { yes_selected: true, folder: Some("/tmp/x".into()) }));
        let reworded = GATE.replace("1. Yes, I trust this folder", "1. Yes, trust this folder").replace("Esc to cancel", "Esc to exit");
        assert!(yes_selected(&reworded));
    }

    /// A permission dialog whose command, or whose diff, quotes the prompt's
    /// every line, kept within the window, is never the prompt: its own
    /// footer and options are what's at the bottom.
    #[test]
    fn a_permission_dialog_quoting_the_prompt_is_not_it() {
        let quoting = [
            " Bash command",
            "",
            "   grep -rn \"Accessing workspace:\\|1. Yes, I trust this folder\\|2. No, exit\\|Enter to confirm · Esc to cancel\" crates/",
            "   Search for the trust prompt",
            "",
            " Do you want to proceed?",
            " ❯ 1. Yes",
            "   2. Yes, and don't ask again for grep commands in /repo",
            "   3. No, and tell Claude what to do differently (esc)",
            "",
            " Esc to cancel · Tab to amend",
        ]
        .join("\n");
        assert!(!shown(&quoting), "a command quoting it");
        let diff = [
            " Edit file",
            " crates/core/captures/claude-trust-gate.txt",
            " Accessing workspace:",
            "",
            " /repo",
            "",
            " ❯ 1. Yes, I trust this folder",
            "   2. No, exit",
            "",
            " Enter to confirm · Esc to cancel",
            " Do you want to make this edit to claude-trust-gate.txt?",
            " ❯ 1. Yes",
            "   2. No, exit",
            "",
            " Enter to confirm · Esc to cancel",
        ]
        .join("\n");
        assert!(!shown(&diff), "a diff of it, ending in its own footer even");
        let r = Registry::built_in();
        assert_ne!(r.blocked_question("claude", &quoting).as_deref(), Some(QUESTION));
    }

    /// Every other real screen isn't it, and neither is a transcript quoting
    /// the prompt right above an idle box.
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
        let idle = include_str!("../../captures/claude-idle-fresh.txt").trim_end();
        let quoted = format!("⏺ It said:\n Accessing workspace:\n\n /x\n\n ❯ 1. Yes, I trust this folder\n   2. No, exit\n\n Enter to confirm · Esc to cancel\n{idle}\n");
        assert!(!shown(&quoted));
    }
}
