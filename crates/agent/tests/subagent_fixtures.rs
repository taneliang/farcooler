//! Replay real captured frames from turns that dispatched subagents.
//!
//! Captured 2026-08-03 against `@agentclientprotocol/claude-agent-acp` — see
//! `docs/superpowers/specs/2026-08-03-acp-subagent-display-design.md` for how,
//! and `fixtures/capture_subagent.py` to take more. Live frames rather than
//! hand-written samples, so a pass here means the normalizer survived contact
//! with the wire rather than with its author's idea of the wire.

use farcooler_agent::acp::{normalize::update_to_events, wire};
use farcooler_agent::event::AgentEvent;

fn events_from(fixture: &str) -> Vec<AgentEvent> {
    let raw = std::fs::read_to_string(format!("tests/fixtures/{fixture}")).expect("fixture");
    let mut out = Vec::new();
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let rpc: wire::Rpc = serde_json::from_str(line).expect("frame parses");
        if let Some(n) = rpc.session_notification() {
            out.extend(update_to_events(&n.update));
        }
    }
    out
}

#[test]
fn no_word_of_a_subagents_is_attributed_to_the_agent_that_dispatched_it() {
    // The regression test for the bug this change exists for. Before it, this
    // capture rendered "I'll read the file." and "3 lines." as the top-level
    // agent speaking, and a reader concluded it had read the file itself.
    let events = events_from("subagent_with_cap.jsonl");
    let nested: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::Message { text, parent: Some(p), .. } => Some((text.as_str(), p.as_str())),
            _ => None,
        })
        .collect();
    assert!(
        nested.iter().any(|(t, _)| t.contains("I'll read the file")),
        "the subagent's own narration was not attributed to it: {events:#?}"
    );

    let dispatch = events
        .iter()
        .find_map(|e| match e {
            AgentEvent::ToolCall { id, subagent: true, .. } => Some(id.clone()),
            _ => None,
        })
        .expect("the capture dispatched a subagent");
    for (text, parent) in &nested {
        assert_eq!(parent, &dispatch, "{text:?} named a parent that is not the dispatch");
    }
}

#[test]
fn a_finished_dispatch_reports_what_the_subagent_cost() {
    let events = events_from("subagent_with_cap.jsonl");
    let summary = events
        .iter()
        .find_map(|e| match e {
            AgentEvent::ToolUpdate { subagent: Some(s), .. } => Some(s),
            _ => None,
        })
        .expect("a completed dispatch");
    assert_eq!(summary.agent_type, "general-purpose");
    assert!(summary.tokens > 0, "no token count: {summary:?}");
    assert!(summary.duration_ms > 0, "no duration: {summary:?}");
}

#[test]
fn a_subagents_tool_calls_nest_even_without_the_transcript_capability() {
    // Measured, and the reason the capability is not load-bearing: with
    // `subagent-transcript` off the adapter withholds the subagent's
    // NARRATION but still tags its tool calls. Turning the flag off would
    // therefore never have fixed the mis-nesting on its own.
    let events = events_from("subagent_without_cap.jsonl");
    assert!(
        events.iter().any(|e| matches!(e, AgentEvent::ToolCall { parent: Some(_), .. })),
        "no parented tool call in the without-capability capture: {events:#?}"
    );
    assert!(
        !events.iter().any(|e| matches!(e, AgentEvent::Message { parent: Some(_), .. })),
        "the without-capability capture should carry no subagent narration"
    );
}

#[test]
fn a_subagent_turn_produces_no_gaps() {
    // Nesting must not be bought with false "history missing" breaks. This is
    // also the property that made the original bug invisible — every frame
    // was a kind we modelled, so nothing complained — and it has to stay true
    // now for the right reason instead of by accident.
    for fixture in ["subagent_with_cap.jsonl", "subagent_without_cap.jsonl"] {
        let events = events_from(fixture);
        assert!(
            !events.iter().any(|e| matches!(e, AgentEvent::Gap { .. })),
            "{fixture} produced a gap"
        );
    }
}

#[test]
fn a_workflow_is_one_opaque_row_and_says_so() {
    // Pins the finding that scoped workflows out of this design: a Workflow
    // tool call is not a dispatch and emits nothing at all for the agents it
    // runs. If a future adapter starts tagging them, this test fails — which
    // is the signal that the workflow spec has become reachable.
    let events = events_from("workflow_dispatch.jsonl");
    assert!(
        !events.iter().any(|e| matches!(e, AgentEvent::ToolCall { subagent: true, .. })),
        "a workflow now marks itself as a subagent dispatch — revisit the spec"
    );
    assert!(
        !events.iter().any(|e| matches!(
            e,
            AgentEvent::Message { parent: Some(_), .. }
                | AgentEvent::ToolCall { parent: Some(_), .. }
        )),
        "a workflow now emits parented frames — revisit the spec"
    );
}

#[test]
fn the_json_the_apps_decode_is_the_json_this_emits() {
    // The cross-language seam, pinned. `AgentEvent` travels as `payload_json`
    // and both apps decode it by hand-written `CodingKeys`, so a field renamed
    // here is not a compile error anywhere — it is a field that silently stops
    // arriving, and a subagent that silently stops nesting.
    let events = events_from("subagent_with_cap.jsonl");

    let nested = events
        .iter()
        .find(|e| matches!(e, AgentEvent::Message { parent: Some(_), .. }))
        .expect("a subagent message");
    let json = serde_json::to_value(nested).expect("serializes");
    assert!(json["Message"]["parent"].is_string(), "got {json}");

    let dispatch = events
        .iter()
        .find(|e| matches!(e, AgentEvent::ToolCall { subagent: true, .. }))
        .expect("a dispatch");
    let json = serde_json::to_value(dispatch).expect("serializes");
    assert_eq!(json["ToolCall"]["subagent"], serde_json::json!(true), "got {json}");

    let finished = events
        .iter()
        .find(|e| matches!(e, AgentEvent::ToolUpdate { subagent: Some(_), .. }))
        .expect("a finished dispatch");
    let json = serde_json::to_value(finished).expect("serializes");
    let summary = &json["ToolUpdate"]["subagent"];
    // Exactly the keys `AgentKit.SubagentSummary.CodingKeys` names.
    for key in ["agent_type", "model", "tokens", "tool_uses", "duration_ms", "status"] {
        assert!(!summary[key].is_null(), "summary is missing {key}: {summary}");
    }
}
