//! A real Claude turn, normalized, with no network.
//!
//! `fixtures/turn_basic.jsonl` is every frame `claude` 2.1.226 sent during one
//! turn, captured by `fixtures/capture_turn.py` rather than written by hand.

use farcooler_agent_core::event::{AgentEvent, EndReason, Role};
use farcooler_claude::normalize::frame_to_events;

fn events() -> Vec<AgentEvent> {
    include_str!("fixtures/turn_basic.jsonl")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<serde_json::Value>(l).expect("fixture line is JSON"))
        .flat_map(|v| frame_to_events(&v))
        .collect()
}

#[test]
fn a_real_turn_produces_the_answer_and_ends() {
    let all = events();

    let spoken: String = all
        .iter()
        .filter_map(|e| match e {
            AgentEvent::Message { role: Role::Agent, text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(spoken, "hi");

    assert!(
        all.iter().any(|e| matches!(e, AgentEvent::TurnEnded { reason: EndReason::EndTurn })),
        "the turn has to end or a pane says Working forever"
    );
}

#[test]
fn a_real_turn_does_not_echo_the_prompt_back() {
    // The doubling that shipped on codex. Here the client's own copy is the
    // only one, because a `user` frame's text blocks are deliberately ignored.
    assert!(
        !events().iter().any(|e| matches!(e, AgentEvent::Message { role: Role::User, .. })),
        "the client already drew what you typed"
    );
}

#[test]
fn a_real_turn_contains_no_gaps() {
    // Catches an unmapped frame the moment the CLI adds one. A Gap is a
    // visible break in the transcript, so this failing means a user would have
    // seen "history missing" for a frame that was simply new.
    let gaps: Vec<_> = events().into_iter().filter(|e| matches!(e, AgentEvent::Gap { .. })).collect();
    assert!(gaps.is_empty(), "unmapped frames in a real turn: {gaps:?}");
}
