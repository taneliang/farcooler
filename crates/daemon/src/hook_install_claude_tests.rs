//! claude's hook settings, as ov-364 left them.

use std::path::Path;

use super::*;

/// ov-364's four reach every claude pane Far Cooler launches, none waiting.
#[test]
fn claude_gets_the_subagent_and_failure_hooks() {
    let settings = claude_settings(Path::new("/tmp/h.sock"));
    for event in ["SubagentStart", "SubagentStop", "Notification", "StopFailure"] {
        let command = settings["hooks"][event][0]["hooks"][0]["command"].as_str().unwrap_or_default();
        assert!(command.contains(&format!("--event {event} ")) && !command.contains("--gating"), "{event}");
    }
}

/// claude waits on no call's end, and on everything else: the fence, the
/// gate, and every boundary a late arrival could misapply.
#[test]
fn only_a_calls_end_goes_unwaited() {
    let settings = claude_settings(Path::new("/tmp/h.sock"));
    let hooks = settings["hooks"].as_object().expect("hooks");
    assert!(hooks.len() >= 12, "{hooks:?}");
    for (event, entries) in hooks {
        let unwaited = entries[0]["hooks"][0]["async"] == serde_json::json!(true);
        let expected = matches!(event.as_str(), "PostToolUse" | "PostToolUseFailure");
        assert_eq!(unwaited, expected, "{event}");
    }
}
