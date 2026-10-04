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
