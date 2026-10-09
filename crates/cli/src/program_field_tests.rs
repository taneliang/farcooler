//! `program`, what a terminal was launched as, in both projections (ov-218).

use crate::{terminal_event_json, worktree_list_terminal_json};

/// A claude that renamed itself to its session's title still says it was
/// launched as `claude`, in the list and in events alike: the Mac's pane
/// header names the program by this, and `preset` by then is the title.
#[test]
fn a_renamed_program_is_still_named_by_what_was_launched() {
    let t = farcooler_protocol::v1::Terminal {
        command_preset: "claude".to_string(),
        current_command: "Fix the login bug".to_string(),
        ..Default::default()
    };
    for projected in [worktree_list_terminal_json(&t), terminal_event_json(&t)] {
        assert_eq!(projected["program"], "claude");
        assert_eq!(projected["preset"], "Fix the login bug");
    }
}

/// A claude typed into a shell, which named its session: launched as
/// `shell`, labeled by its title, and running claude, in the list and in
/// events alike. The Mac offers the conversation view by `runningAgent`
/// (ov-443); without it, this pane was never offered one.
#[test]
fn the_agent_running_in_a_shell_is_named() {
    let t = farcooler_protocol::v1::Terminal {
        command_preset: "shell".to_string(),
        current_command: "Fix the login bug".to_string(),
        running_agent: Some("claude".to_string()),
        ..Default::default()
    };
    for projected in [worktree_list_terminal_json(&t), terminal_event_json(&t)] {
        assert_eq!(projected["runningAgent"], "claude");
        assert_eq!(projected["program"], "shell");
    }
    let none = farcooler_protocol::v1::Terminal::default();
    for projected in [worktree_list_terminal_json(&none), terminal_event_json(&none)] {
        assert!(projected["runningAgent"].is_null(), "absent when nothing runs");
    }
}
