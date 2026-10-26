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

/// A turn running beside background agents (claude 2.1.296, a main-thread
/// `sleep 60` begun while two agents ran; its registry `busy` for both). The
/// main loop is Working, so the daemon's background exemption from claude's
/// `busy` (`registry_turn::said_beside`) never applies to it, and a message
/// goes in only under the mid-turn fence. At 40 columns the mode line's
/// `esc to interrupt` is cut to `esc to interrupt…`, and the spinner row
/// above the box is what still says it.
#[test]
fn a_turn_running_beside_background_agents_is_working_to_its_box() {
    let r = Registry::built_in();
    for (name, screen) in [
        ("120 columns", include_str!("../captures/claude-2.1.296-working-beside-background-agents-120x40-e.txt")),
        ("40 columns", include_str!("../captures/claude-2.1.296-working-beside-background-agents-40x40-e.txt")),
    ] {
        assert!(r.background_running("claude", screen), "{name}: the agents are on it");
        assert_eq!(r.classify_main("claude", screen), Working, "{name}");
        assert_eq!(r.classify("claude", screen), Working, "{name}");
    }
}

/// claude 2.1.309 between turns with seven background shells and no agent
/// (`run_in_background` shells, against a stand-in API), at the owner's
/// phone width: empty, and with the owner's message pasted in as the daemon
/// pastes it (ov-795). The paste drops `· ↓ to manage` from the mode line and
/// keeps `· 7 shells`; there are no `◯` rows. Its registry said `shell`
/// throughout, so read as no background work, the box's read-back before
/// the Enter (`still_between`) refused it and left it as `paste_left`.
#[test]
fn background_shells_still_count_once_the_box_holds_a_message() {
    let r = Registry::built_in();
    let empty = include_str!("../captures/claude-2.1.309-shells-empty-59x41-e.txt");
    let pasted = include_str!("../captures/claude-2.1.309-shells-owner-59x41-e.txt");
    assert!(composer::printed(empty).contains("⏵⏵ auto mode on · 7 shells · ↓ to manage"));
    assert!(composer::printed(pasted).lines().any(|l| l.trim_end() == "  ⏵⏵ auto mode on · 7 shells"));
    assert!(!composer::printed(pasted).contains("◯"));
    for (name, screen) in [("empty", empty), ("pasted", pasted)] {
        assert!(r.background_running("claude", screen), "{name}");
        assert_eq!(r.classify_main("claude", screen), Idle, "{name}");
        assert_eq!(r.classify("claude", screen), Working, "{name}");
    }
    let owner = "Btw to make things go faster, if there are multiple trains waiting to land, if it’s safe you can merge \
                 them into the same train. Otherwise you’ll have a million trains queueing unnecessarily. Please update \
                 the manager skill also with such best practices";
    assert!(composer::drawn::Expected::default().then_paste(owner).shown_on("claude", pasted));
}

/// The count is read on the mode line only: the same words typed into the
/// box are a draft, not background work; a count cut short, or another
/// part of the line, is no count. The owner's own mode line (claude
/// 2.1.308, 59 columns) carries shells and a monitor in one part.
#[test]
fn only_the_mode_lines_own_count_is_background_work() {
    let r = Registry::built_in();
    let pasted = include_str!("../captures/claude-2.1.309-shells-owner-59x41-e.txt");
    let count = "\x1b[38;5;246m · \x1b[38;5;44m7 shells\x1b[39m";
    assert!(pasted.contains(count));
    let quiet = pasted.replace(count, "\x1b[39m");
    assert!(!r.background_running("claude", &quiet), "nothing running");
    let typed = quiet.replacen("❯\u{a0}Btw to make", "❯\u{a0}7 shells · 1 monitor", 1);
    let held = composer::read("claude", &typed);
    assert!(matches!(&held, Composer::Holds(h) if h.starts_with("7 shells · 1 monitor")), "{held:?}");
    assert!(!r.background_running("claude", &typed), "typed into the box");
    for (rest, counts) in [
        (" · 8 shells, 1 monitor · ← 3 agents · ↓…", true),
        (" · 1 shell", true),
        (" · 2 monitors", true),
        (" · 8 she…", false),
        (" · ← 3 agents", false),
        (" · shells", false),
        (" · 8 shells, 1 thing", false),
    ] {
        let line = pasted.replace(count, &format!("\x1b[38;5;246m{rest}\x1b[39m"));
        assert_eq!(r.background_running("claude", &line), counts, "{rest}");
    }
}
