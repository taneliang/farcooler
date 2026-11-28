use super::{claude, codex, parse, STATE};

/// The end of a Codex builder's terminal bytes on Nov 27, 2026: its error state, then the limit it printed.
const CAPTURED: &[u8] = include_bytes!("workflow_runtime/fixtures/codex-usage-limit.bin");
/// That message as the pane showed it, wrapped where Codex wrapped it.
const SCREEN: &str = "  End of ticket brief.\n\n■ You’ve hit your usage limit. Visit https://chatgpt.com/settings/usage to purchase more credits or try again at Dec\n1st, 2026 1:23 AM.\n\n› Ask Codex to do anything";
const REASON: &str = "usage limit until Dec 1st, 2026 1:23 AM (Codex)";

#[test]
fn the_captured_codex_limit_says_until_when() {
    assert!(CAPTURED.windows(STATE.len()).any(|w| w == STATE), "the capture carries Codex's state reports");
    assert_eq!(codex(SCREEN, CAPTURED).as_deref(), Some(REASON));
}

#[test]
fn a_ticket_that_quotes_the_message_is_not_a_limit() {
    // This ticket's own brief: the words, but not on Codex's error line.
    let quoted = "Nov 27: Codex hit its usage limit ('You've hit your usage limit. Visit https://chatgpt.com/settings/usage\n\
        to purchase more credits or try again at Dec 1st, 2026 1:23 AM.'). Every builder sat idle.";
    assert_eq!(codex(quoted, b""), None);
    assert_eq!(codex("• Ran cat notes.txt\n  └ ■ You've hit your usage limit.", b""), None);
}

#[test]
fn codex_working_again_after_the_error_is_not_at_its_limit() {
    let mut raw = CAPTURED.to_vec();
    raw.extend_from_slice(b"\x1b]7501;state=working:app=codex\x1b\\");
    assert_eq!(codex(SCREEN, &raw), None);
    // A Codex that reports no state is read by its error line alone.
    assert_eq!(codex(SCREEN, b"plain output").as_deref(), Some(REASON));
}

#[test]
fn an_error_state_without_a_limit_message_is_not_called_a_limit() {
    assert_eq!(codex("■ stream disconnected before completion", CAPTURED), None);
}

#[test]
fn codex_rate_limits_and_relative_times_are_read() {
    assert_eq!(codex("■ You've hit your usage limit. Try again in 4 days 3 hours.", b"").as_deref(),
        Some("usage limit until 4 days 3 hours from now (Codex)"));
    assert_eq!(codex("■ exceeded retry limit, last status: 429 Too Many Requests", b"").as_deref(),
        Some("usage limit until an unknown time (Codex)"));
}

#[test]
fn claude_limit_messages_say_until_when() {
    // Both messages as Claude Code recorded them on this Mac.
    for (said, until) in [
        ("You've hit your session limit · resets 1pm (America/Los_Angeles)", "1pm (America/Los_Angeles)"),
        ("You've hit your weekly limit · resets Oct 6 at 5am (America/Los_Angeles)", "Oct 6 at 5am (America/Los_Angeles)"),
    ] {
        let payload = serde_json::json!({ "error": "rate_limit", "last_assistant_message": said });
        assert_eq!(claude(&payload), Some(format!("usage limit until {until} (Claude)")));
    }
    let bare = serde_json::json!({ "error": "rate_limit", "last_assistant_message": "API Error: Rate limited" });
    assert_eq!(claude(&bare).as_deref(), Some("usage limit until an unknown time (Claude)"));
    let auth = serde_json::json!({ "error": "authentication_failed", "last_assistant_message": "OAuth token revoked · Please run /login" });
    assert_eq!(claude(&auth), None);
}

#[test]
fn a_reason_reads_back_its_harness_and_time() {
    assert_eq!(parse(REASON), Some(("codex", "Dec 1st, 2026 1:23 AM")));
    assert_eq!(parse("usage limit until 1pm (Asia/Singapore) (Claude)"), Some(("claude", "1pm (Asia/Singapore)")));
    assert_eq!(parse("The build ended without changes."), None);
}
