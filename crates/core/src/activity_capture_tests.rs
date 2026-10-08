//! What every capture in `captures/` classifies as (ov-394), by the command
//! its agent runs. A table, so a change to a rule that moves a real screen
//! fails by name. Taken from origin/main's rules before the spinner rule was
//! anchored, and unchanged by it.

use super::*;
use farcooler_protocol::v1::AgentActivity::{Blocked, Idle, Working};

#[test]
fn every_capture_classifies_as_it_was_read() {
    let table: &[(&str, &str, AgentActivity)] = &[
        ("claude-2.1.290-image-and-long-paste-160x45-e.txt", "claude", Idle),
        ("claude-2.1.290-slash-alias-160x45-e.txt", "claude", Idle),
        ("claude-2.1.290-slash-exact-160x45-e.txt", "claude", Idle),
        ("claude-2.1.290-three-lines-160x45-e.txt", "claude", Idle),
        ("claude-2.1.290-working-long-paste-160x45-e.txt", "claude", Working),
        ("claude-2.1.290-working-paste-160x45-e.txt", "claude", Working),
        ("claude-2.1.290-working-queued-160x45-e.txt", "claude", Working),
        ("claude-2.1.290-working-queued-long-paste-120x45-e.txt", "claude", Working),
        ("claude-2.1.292-after-turn-160x45-e.txt", "claude", Idle),
        ("claude-2.1.292-draft-cursor-moved-100x40-e.txt", "claude", Idle),
        ("claude-2.1.292-draft-multiline-100x40-e.txt", "claude", Idle),
        ("claude-2.1.292-draft-one-line-100x40-e.txt", "claude", Idle),
        ("claude-2.1.292-draft-pasted-100x40-e.txt", "claude", Idle),
        ("claude-2.1.292-draft-tall-100x40-e.txt", "claude", Idle),
        ("claude-2.1.292-draft-working-100x40-e.txt", "claude", Working),
        ("claude-2.1.292-draft-working-after-ctrl-u-100x40-e.txt", "claude", Working),
        ("claude-2.1.292-idle-placeholder-160x45-e.txt", "claude", Idle),
        ("claude-2.1.292-working-160x45-e.txt", "claude", Working),
        ("claude-after-a-keyboard-yes.txt", "claude", Working),
        ("claude-asking.txt", "claude", Blocked),
        ("claude-background-agent-main-idle-40col.txt", "claude", Working),
        ("claude-background-agents-main-idle.txt", "claude", Working),
        ("claude-background-shell-main-idle-80col.txt", "claude", Working),
        ("claude-blocked.txt", "claude", Blocked),
        ("claude-idle-after-background-agents.txt", "claude", Idle),
        ("claude-idle-fresh.txt", "claude", Idle),
        ("claude-idle-nothing-running.txt", "claude", Idle),
        ("claude-idle-transcript-says-esc-to-interrupt.txt", "claude", Idle),
        ("claude-permission-hook-waiting.txt", "claude", Blocked),
        ("claude-trust-gate.txt", "claude", Blocked),
        ("claude-working.txt", "claude", Working),
        ("codex-0.153.4-blank-lines-160x45-e.txt", "codex", Idle),
        ("codex-0.153.4-idle-160x45-e.txt", "codex", Idle),
        ("codex-0.153.4-image-and-long-paste-160x45-e.txt", "codex", Idle),
        ("codex-0.153.4-long-paste-160x45-e.txt", "codex", Idle),
        ("codex-0.153.4-mention-no-matches-160x45-e.txt", "codex", Idle),
        ("codex-0.153.4-mention-popup-160x45-e.txt", "codex", Idle),
        ("codex-0.153.4-skill-no-matches-160x45-e.txt", "codex", Idle),
        ("codex-0.153.4-slash-init-160x45-e.txt", "codex", Idle),
        ("codex-0.153.4-tall-paste-80x24-e.txt", "codex", Idle),
        ("codex-0.153.4-three-lines-160x45-e.txt", "codex", Idle),
        ("codex-0.153.4-working-paste-160x45-e.txt", "codex", Working),
        ("codex-blocked.txt", "codex", Blocked),
        ("codex-idle-after-turn.txt", "codex", Idle),
        ("codex-trust-gate.txt", "codex", Blocked),
        ("codex-working.txt", "codex", Working),
        ("cursor-blocked.txt", "cursor-agent", Blocked),
        ("cursor-idle.txt", "cursor-agent", Idle),
        ("cursor-trust-gate.txt", "cursor-agent", Blocked),
        ("cursor-working.txt", "cursor-agent", Working),
    ];
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/captures");
    let on_disk = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".txt"))
        .collect::<std::collections::BTreeSet<_>>();
    let listed = table.iter().map(|(n, ..)| n.to_string()).collect::<std::collections::BTreeSet<_>>();
    assert_eq!(on_disk, listed, "a capture is missing from the table, or the table names one that's gone");
    let registry = Registry::built_in();
    for (name, command, want) in table {
        let screen = std::fs::read_to_string(format!("{dir}/{name}")).unwrap();
        assert_eq!(registry.classify(command, &screen), *want, "{name}");
    }
}
