//! A claude whose turn has ended while background agents run (ov-468):
//! Working to the fleet, Idle to its box (`activity/background.rs`).

use super::*;
use crate::composer::{self, Composer};
use farcooler_protocol::v1::AgentActivity::{Idle, Working};

/// claude 2.1.296, its turn over (`⏺ Dispatched.`, then `✻ Waiting for 2
/// background agents to finish`) and two background agents still running,
/// taken off a sandboxed claude against a stand-in API. Its registry said
/// `busy` throughout; a message typed and entered here ran at once as a
/// turn of its own, a `user` record and no `enqueue`.
const WAITING: &str = include_str!("../captures/claude-2.1.296-background-agents-main-idle-120x40-e.txt");

/// The same pane with `hello mid wait` typed in its box: the mode line drops
/// its tray hint, and the agent rows are what still say it.
const WAITING_DRAFT: &str = include_str!("../captures/claude-2.1.296-background-agents-draft-120x40-e.txt");

#[test]
fn a_claude_waiting_on_background_agents_is_working_to_the_fleet_and_idle_to_its_box() {
    let r = Registry::built_in();
    for (name, screen) in [("waiting", WAITING), ("waiting with a draft", WAITING_DRAFT)] {
        assert_eq!(r.classify("claude", screen), Working, "{name}: the fleet still says it");
        assert!(r.background_running("claude", screen), "{name}");
        assert_eq!(r.classify_main("claude", screen), Idle, "{name}: its box takes a message");
    }
    assert_eq!(composer::read("claude", WAITING), Composer::Empty);
    assert_eq!(composer::read("claude", WAITING_DRAFT), Composer::Holds("hello mid wait".into()));
}

/// The older captures of the same state, two of them a background shell or
/// one narrow agent row: each main loop between turns.
#[test]
fn every_background_capture_is_between_turns() {
    let r = Registry::built_in();
    for (name, screen) in [
        ("agents", include_str!("../captures/claude-background-agents-main-idle.txt")),
        ("shell", include_str!("../captures/claude-background-shell-main-idle-80col.txt")),
        ("40 columns", include_str!("../captures/claude-background-agent-main-idle-40col.txt")),
    ] {
        assert!(r.background_running("claude", screen), "{name}");
        assert_eq!(r.classify_main("claude", screen), Idle, "{name}");
    }
}

/// A turn running is a turn, background agents or not; nothing running is
/// no background work.
#[test]
fn a_running_turn_or_an_idle_one_is_read_as_before() {
    let r = Registry::built_in();
    let working = include_str!("../captures/claude-2.1.292-working-160x45-e.txt");
    assert_eq!(r.classify_main("claude", working), Working);
    assert!(!r.background_running("claude", working));
    let idle = include_str!("../captures/claude-idle-nothing-running.txt");
    assert_eq!(r.classify_main("claude", idle), Idle);
    assert!(!r.background_running("claude", idle));
    assert!(!r.background_running("codex", include_str!("../captures/codex-working.txt")));
}
