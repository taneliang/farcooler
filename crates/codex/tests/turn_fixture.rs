//! A real codex turn, normalized.
//!
//! `fixtures/turn_basic.jsonl` is every frame `codex app-server` 0.146.0 sent
//! during one turn, captured by `fixtures/capture_turn.py` rather than written
//! by hand. The same rule the `identity` table lives by: read off a running
//! instance, never guessed.

use farcooler_agent_core::event::{AgentEvent, EndReason, Role};
use farcooler_codex::normalize::{Origin, frame_to_events};

/// Every frame in the fixture, as the events a transcript would show.
fn events_from_fixture(origin: Origin) -> Vec<AgentEvent> {
    let raw = include_str!("fixtures/turn_basic.jsonl");
    let mut events = Vec::new();
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let value: serde_json::Value = serde_json::from_str(line).expect("fixture line is JSON");
        // Responses to our own requests carry no method and are not transcript
        // material; the notifications are what a session renders.
        let Some(method) = value.get("method").and_then(|m| m.as_str()) else { continue };
        let params = value.get("params").cloned().unwrap_or(serde_json::Value::Null);
        events.extend(frame_to_events(method, &params, origin));
    }
    events
}

/// Just the user's own words, in order.
fn prompts(events: &[AgentEvent]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::Message { role: Role::User, text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_live_turn_leaves_the_prompt_to_the_client_that_already_showed_it() {
    // The doubling that showed up on screen: the client appends what you typed
    // the moment you send it, so taking codex's echo renders it twice.
    assert!(prompts(&events_from_fixture(Origin::Live)).is_empty());
}

#[test]
fn a_replayed_turn_carries_the_prompt_because_nothing_else_would() {
    // The other half. A resumed conversation has no local echo behind it, so
    // dropping these unconditionally would restore an agent talking to itself.
    assert_eq!(
        prompts(&events_from_fixture(Origin::Replay)),
        ["Reply with exactly: hi"],
        "the prompt, exactly once"
    );
}

#[test]
fn a_real_turn_produces_the_conversation_and_nothing_else() {
    let events = events_from_fixture(Origin::Live);

    let agent: String = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::Message { role: Role::Agent, text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(agent, "hi", "the deltas reassemble into the answer, not twice over");

    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEvent::TurnEnded { reason: EndReason::EndTurn }
        )),
        "the turn has to end or the pane says Working forever"
    );
}

#[test]
fn a_real_turn_contains_no_gaps() {
    // The one that catches an unmapped frame the moment codex adds one. A Gap
    // here is not a crash — it is a visible break in the transcript — so this
    // failing means a user would have seen "history missing" for a frame that
    // was simply new.
    let gaps: Vec<_> = events_from_fixture(Origin::Live)
        .into_iter()
        .filter(|e| matches!(e, AgentEvent::Gap { .. }))
        .collect();
    assert!(gaps.is_empty(), "unmapped frames in a real turn: {gaps:?}");
}

/// A real `thread/read` result, captured by `fixtures/capture_turn.py`'s
/// sibling probe rather than written by hand.
#[test]
fn a_resumed_thread_restores_both_sides_of_the_conversation() {
    // History does NOT arrive as notifications — `thread/resume` streams only
    // bookkeeping and its `initialTurnsPage` comes back null, so a pane that
    // waited for item/* frames showed an empty conversation. It has to be
    // asked for with thread/read.
    let raw = include_str!("fixtures/thread_read.json");
    let result: serde_json::Value = serde_json::from_str(raw).expect("fixture is JSON");
    let events = farcooler_codex::normalize::history_to_events(&result);

    assert_eq!(prompts(&events), ["Say: apple"], "the prompt is restored");

    let agent: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::Message { role: Role::Agent, text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    // In replay there are no deltas, so the finished item is the ONLY place
    // the agent's words exist. Skipping it the way the live path does would
    // restore a conversation where only the user ever spoke.
    assert_eq!(agent, ["apple"], "and so is the answer");

    assert!(
        !events.iter().any(|e| matches!(e, AgentEvent::Gap { .. })),
        "restoring history must not draw a break: {events:?}"
    );
}
