//! Replay real captured ACP frames. No live agent, no credentials in CI.
//!
//! The fixtures are real inbound frames from a live adapter session (see
//! `docs/superpowers/specs/2026-08-01-gate1-acp-findings.md`), not hand-written
//! samples, so a passing suite here means the normalizer survives contact with
//! the actual wire format, not just the shapes its author guessed at.

use farcooler_acp::{normalize::update_to_events, wire};
use farcooler_agent_core::event::{AgentEvent, AgentGapReason, Role};

const FIXTURES: [&str; 2] = ["session_basic.jsonl", "session_permission.jsonl"];

fn events_from(fixture: &str) -> Vec<AgentEvent> {
    let raw = std::fs::read_to_string(format!("tests/fixtures/{fixture}")).expect("fixture");
    let mut out = Vec::new();
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let rpc: wire::Rpc = serde_json::from_str(line).expect("frame parses");
        // Frames that are not a `session/update` notification — RPC results
        // like `initialize`'s or `session/prompt`'s, and adapter-initiated
        // requests like `fs/read_text_file` or `session/request_permission`
        // — are not this normalizer's job and are skipped here exactly as a
        // real driver would skip them (they are handled elsewhere in the
        // pipeline, not lost).
        if let Some(n) = rpc.session_notification() {
            out.extend(update_to_events(&n.update));
        }
    }
    out
}

#[test]
fn a_captured_session_yields_agent_messages() {
    let events = events_from("session_basic.jsonl");
    assert!(
        events.iter().any(|e| matches!(e, AgentEvent::Message { role: Role::Agent, .. })),
        "no agent message in a real captured turn: {events:#?}"
    );
}

#[test]
fn a_captured_session_yields_tool_calls() {
    let events = events_from("session_basic.jsonl");
    assert!(
        events.iter().any(|e| matches!(e, AgentEvent::ToolCall { .. })),
        "no tool call in a turn that edited a file: {events:#?}"
    );
}

#[test]
fn an_update_nobody_wrote_a_rule_for_becomes_a_gap_not_a_silence() {
    // The honesty rule, at the one place it is cheapest to break: an unknown
    // variant must be visible, or a future adapter change silently shortens
    // every transcript.
    let json = r#"{"sessionUpdate":"something_from_a_future_version"}"#;
    let update: wire::SessionUpdate = serde_json::from_str(json).expect("parses as unknown");
    let events = update_to_events(&update);
    assert_eq!(events, vec![AgentEvent::Gap { reason: AgentGapReason::Unparsed }]);
}

#[test]
fn no_frame_in_a_real_fixture_produces_a_spurious_gap() {
    // The regression guard for `available_commands_update`: both fixtures
    // contain a real one (60+ commands, tens of KB), and before that variant
    // existed it fell into `Unknown` and became a `Gap` on every single
    // turn. The two tests above only prove SOME events came out right; this
    // is the one that would have caught the false "history missing" break
    // that a live user would actually have seen every turn.
    for fixture in FIXTURES {
        let events = events_from(fixture);
        let gaps: Vec<&AgentEvent> =
            events.iter().filter(|e| matches!(e, AgentEvent::Gap { .. })).collect();
        assert!(gaps.is_empty(), "{fixture} produced a spurious Gap: {gaps:#?}");
    }
}

#[test]
fn a_real_capture_yields_the_slash_commands_a_picker_needs() {
    // The menu is the only source the composer's `/` picker has. Parsing the
    // update but dropping its contents would leave the picker permanently
    // empty — a failure that looks like "this agent has no commands" rather
    // than like a bug.
    let commands: Vec<String> = events_from("session_basic.jsonl")
        .into_iter()
        .filter_map(|e| match e {
            AgentEvent::CommandsAvailable { commands } => Some(commands),
            _ => None,
        })
        .flatten()
        .map(|c| c.name)
        .collect();

    assert!(!commands.is_empty(), "a real capture carried no commands");
    // Named rather than merely counted: a list of empty strings would satisfy
    // a length check and still render a menu of blank rows.
    assert!(
        commands.iter().all(|c| !c.trim().is_empty()),
        "every command needs a name to show: {commands:?}"
    );
}
