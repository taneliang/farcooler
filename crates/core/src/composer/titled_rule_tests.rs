//! The orchestrator's screen (ov-430): claude draws the session's title into
//! the box's top rule, an update notice above it, and `⧉ 2` in its footer.
//! Read as no box at all, the runner refused to wake it: "its screen isn't
//! one Far Cooler recognizes".

use super::*;
use crate::activity::Registry;
use farcooler_protocol::v1::AgentActivity;

const CAPTURE: &str = "claude-orchestrator-titled-rule-103x65-e.txt";

fn capture() -> String {
    std::fs::read_to_string(format!("{}/captures/{CAPTURE}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

/// The same screen between turns: no spinner, no `esc to interrupt`.
fn idle() -> String {
    capture()
        .lines()
        .filter(|l| !l.contains("Coalescing") && !l.contains("Tip: Use /btw"))
        .map(|l| l.replace(" · esc to interrupt", "").replace(" · \u{1b}[38;5;246mesc to interrupt", ""))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_titled_rule_is_a_rule() {
    for rule in [
        "────────────────────────────────────────────────────────── User test issues and onboarding flow (2) ─",
        "──────────── Fix the title ─────────",
        "──────────────────────────",
    ] {
        assert!(is_rule_text(rule), "{rule}");
    }
    for not in [
        "─── short ───",
        "────────────── no closing run",
        "────────────── a title ─ then more words",
        "──────────────────────────x",
        "──────────────  ─",
        "a ────────────── b ─",
    ] {
        assert!(!is_rule_text(not), "{not}");
    }
}

#[test]
fn the_orchestrators_box_reads_empty() {
    assert_eq!(read("claude", &capture()), Composer::Empty);
    assert_eq!(read("claude", &idle()), Composer::Empty);
    assert_eq!(draft::claude(&capture(), 103), draft::Draft::Empty);
    assert_eq!(draft::claude(&idle(), 103), draft::Draft::Empty);
}

#[test]
fn a_draft_in_a_titled_box_is_read() {
    let screen = capture().replace("\u{276f}\u{a0}\u{1b}[39m", "\u{276f}\u{a0}\u{1b}[39mhalf a thought");
    assert_eq!(read("claude", &screen), Composer::Holds("half a thought".into()));
}

#[test]
fn the_orchestrators_screen_classifies_working_then_idle() {
    let registry = Registry::built_in();
    assert_eq!(registry.classify("claude", &capture()), AgentActivity::Working);
    assert_eq!(registry.classify("claude", &idle()), AgentActivity::Idle);
}

/// A spinner above a titled rule is found without the footer saying so.
#[test]
fn the_spinner_above_a_titled_rule_counts() {
    let screen = capture().replace(" · esc to interrupt", "").replace(" · \u{1b}[38;5;246mesc to interrupt", "");
    assert!(!screen.contains("esc to interrupt"));
    assert_eq!(Registry::built_in().classify("claude", &screen), AgentActivity::Working);
}
