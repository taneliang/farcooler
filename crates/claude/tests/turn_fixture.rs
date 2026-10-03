//! A real Claude turn, normalized, with no network.
//!
//! `fixtures/turn_basic.jsonl` is every frame `claude` 2.1.226 sent during one
//! turn, captured by `fixtures/capture_turn.py` rather than written by hand.
//! The capture uses the same flags `handshake::launch_args` sends, so it
//! includes the `stream_event` deltas `--include-partial-messages` buys — and
//! therefore the doubling those deltas invite, since the CLI sends the finished
//! `assistant` message on top of them.

use farcooler_agent_core::event::{AgentEvent, EndReason, FailureKind, Role};
use farcooler_claude::normalize::Live;

/// The capture, through ONE `Live` in order — the way a pane sees it.
///
/// Stateful on purpose: whether a finished `assistant` block is drawn depends
/// on whether the deltas for that message were actually seen, so replaying the
/// frames through independent normalizers would not be the same test.
fn events() -> Vec<AgentEvent> {
    let mut live = Live::default();
    include_str!("fixtures/turn_basic.jsonl")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<serde_json::Value>(l).expect("fixture line is JSON"))
        .flat_map(|v| live.frame_to_events(&v))
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
    // "hi", not "hihi". The CLI sent this answer TWICE in the capture — once
    // as `text_delta` "h" and "i", then again as a finished `assistant` block
    // holding "hi" — and reading both is how every reply would render a second
    // time underneath itself.
    assert_eq!(spoken, "hi");

    // And piece by piece rather than in one go, which is the whole point of
    // asking for partial messages: a pane that gets one event at the end sits
    // on Working for the length of the answer.
    let pieces = all
        .iter()
        .filter(|e| matches!(e, AgentEvent::Message { role: Role::Agent, .. }))
        .count();
    assert_eq!(pieces, 2, "the answer streamed as its two deltas: {all:?}");

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

/// A real on-disk transcript, restored.
///
/// `fixtures/transcript.jsonl` is the file Claude Code itself wrote for a
/// session that said "Remember the word: pineapple. Just say ok." and got back
/// "ok". Copied out of ~/.claude/projects rather than composed.
#[test]
fn a_saved_transcript_restores_both_sides_of_the_conversation() {
    // `--resume` replays nothing — it attaches the session and sends four hook
    // frames and a control response. So a pane that waited for a replay showed
    // an empty chat, and the transcript on disk is the only source there is.
    let raw = include_str!("fixtures/transcript.jsonl");
    let events = farcooler_claude::normalize::history_to_events(raw);

    let prompts: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::Message { role: Role::User, text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(prompts.len(), 1, "the prompt is restored exactly once: {events:?}");
    assert!(prompts[0].contains("pineapple"), "{prompts:?}");

    let answers: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::Message { role: Role::Agent, text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(answers, ["ok"], "and so is the answer");

    // The file interleaves record types the wire never sends —
    // queue-operation, attachment. Left unlisted they became a Gap apiece,
    // which is a scissors icon between every restored turn.
    let gaps: Vec<_> = events.iter().filter(|e| matches!(e, AgentEvent::Gap { .. })).collect();
    assert!(gaps.is_empty(), "restoring history must not report loss: {gaps:?}");
}

/// A turn refused with a 401, through one `Live`.
///
/// `fixtures/synthetic_turn_401.jsonl` is SYNTHETIC: written by hand in the
/// shape claude 2.x sends for a refused key — an `assistant` frame with
/// `"model":"<synthetic>"` and an `error` word, then a `result` with
/// `"subtype":"success","is_error":true` and `api_error_status`. Capturing a
/// real one means running the real CLI with a bad key, which lanes may not.
#[test]
fn a_refused_key_ends_the_turn_failed_and_puts_no_words_in_the_agents_mouth() {
    let mut live = Live::default();
    let all: Vec<AgentEvent> = include_str!("fixtures/synthetic_turn_401.jsonl")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<serde_json::Value>(l).expect("fixture line is JSON"))
        .flat_map(|v| live.frame_to_events(&v))
        .collect();

    // Read only `stop_reason`, this turn ended as `EndTurn`: a pane whose key
    // had been revoked reported every turn as a quiet success.
    let ended: Vec<_> = all
        .iter()
        .filter_map(|e| match e {
            AgentEvent::TurnEnded { reason } => Some(reason),
            _ => None,
        })
        .collect();
    let [EndReason::Failed { kind, detail }] = ended.as_slice() else {
        panic!("one turn, ended as failed: {all:?}");
    };
    assert_eq!(*kind, FailureKind::Auth);
    assert!(detail.contains("401"), "the CLI's own sentence is kept for logs: {detail}");

    // The CLI's error sentence is not the agent speaking.
    assert!(
        !all.iter().any(|e| matches!(e, AgentEvent::Message { role: Role::Agent, .. })),
        "the API error was drawn as the agent's answer: {all:?}"
    );
}
