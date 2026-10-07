//! The spinner rule reads only the row above the box (ov-394): a prose line
//! that looks like a spinner, higher up the transcript, is not Working.

use super::*;
use farcooler_protocol::v1::AgentActivity::{Idle, Working};

const RULE: &str = "────────────────────────────────────────────────────────────";

/// A claude screen: `above` the box, then the box, then a footer that says
/// nothing of a turn, so only the spinner rule can read Working.
fn screen(above: &[&str]) -> String {
    format!("{}\n{RULE}\n❯ \n{RULE}\n  ? for shortcuts\n", above.join("\n"))
}

fn classify(screen: &str) -> AgentActivity {
    Registry::built_in().classify("claude", screen)
}

#[test]
fn a_spinner_row_above_the_box_reads_working() {
    assert_eq!(classify(&screen(&["⏺ ok", "", "✻ Zigzagging…", ""])), Working);
    assert_eq!(classify(&screen(&["⏺ ok", "* Building…"])), Working, "directly above the rule");
    // The tip claude draws under its spinner, as in `working-long-paste`.
    assert_eq!(classify(&screen(&["✳ Moonwalking…", "  ⎿  Tip: Use ctrl+v to paste images"])), Working);
}

/// The bug: prose starting `* ` or `· ` with a word ending in `…`, further up
/// the transcript, read as a spinner.
#[test]
fn prose_that_looks_like_a_spinner_is_not_working() {
    for prose in ["* Building…", "· Thinking…", "✻ Cooking…"] {
        let shown = screen(&[prose, "", "⏺ ok that's the plan", "", "✻ Worked for 6s", ""]);
        assert_eq!(classify(&shown), Idle, "{prose}");
    }
    assert_eq!(classify(&screen(&["* Building…", "", "", ""])), Idle, "two blank rows above the rule");
    assert_eq!(classify(&screen(&["* Building…", "⏺ ok"])), Idle, "a transcript row between");
}

/// No box, no spinner: a dialog or a bare transcript has nothing the rule
/// anchors to.
#[test]
fn a_spinner_looking_row_with_no_box_below_is_not_working() {
    assert_eq!(classify("⏺ ok\n✻ Zigzagging…\n  ? for shortcuts\n"), Idle);
    assert_eq!(classify(&format!("✻ Zigzagging…\n{RULE}\n  ? for shortcuts\n")), Idle, "a rule but no prompt");
}

/// What claude draws under its spinner besides a tip, in the shapes the
/// review of ov-394 named. Built by hand from those shapes, not captured: no
/// capture of a todo list under a spinner exists yet, so a real one should
/// replace these when one is taken.
#[test]
fn rows_claude_indents_under_its_spinner_are_skipped() {
    let todo = ["✶ Zigzagging… (12s · ↑ 1.2k tokens)", "  ⎿  ☐ First task", "     ☐ Second task", "     ☒ Third task", ""];
    assert_eq!(classify(&screen(&todo)), Working, "a todo list");
    let tip = ["✻ Zigzagging…", "  ⎿  Tip: Use /memory to view and manage Claude memory and the", "     notes it keeps for you", ""];
    assert_eq!(classify(&screen(&tip)), Working, "a tip that wraps");
    let wrapped = ["✻ Zigzagging… (123s · ↑ 12.3k tokens · esc", "  to interrupt)", ""];
    assert_eq!(classify(&screen(&wrapped)), Working, "a spinner line that wraps");
    let subagent = ["✢ Delegating…", "  ⎿  Agent \"Count the lines\"", "     ⏺ Read(src/main.rs)", "     ⏺ Bash(wc -l src/*.rs)", ""];
    assert_eq!(classify(&screen(&subagent)), Working, "a subagent's progress");
    // Indented rows are not a way past a transcript's own rows.
    let behind = ["* Building…", "⏺ ok", "  ⎿  done", ""];
    assert_eq!(classify(&screen(&behind)), Idle);
}
