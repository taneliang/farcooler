//! The agent tray (ov-453): what each running agent has used, and a
//! subagent's own transcript read as a conversation of its own, from the
//! real fixtures in `crates/core/fixtures/session-logs/`.

use super::fixtures::*;
use super::rows::*;
use super::{SessionProjector, subagent_transcript};

const LAUNCHES: &str = include_str!("../../../fixtures/session-logs/claude-subagents.jsonl");
const SIDECHAIN: &str = include_str!("../../../fixtures/session-logs/claude-subagent-transcript.jsonl");
const COMPLETE: &str = include_str!("../../../fixtures/session-logs/claude-complete-turn.jsonl");

/// The background agent `claude-subagents.jsonl` launches, with
/// `claude-subagent-transcript.jsonl` as its own file.
const AGENT: &str = "a000000000000000b";
const SESSION: &str = "c0ffee00-0000-4000-8000-000000000453";

fn laid_out(scratch: &Scratch) -> std::path::PathBuf {
    let main = scratch.path().join(format!("{SESSION}.jsonl"));
    std::fs::write(&main, LAUNCHES).unwrap();
    let subs = scratch.path().join(SESSION).join("subagents");
    std::fs::create_dir_all(&subs).unwrap();
    std::fs::write(subs.join(format!("agent-{AGENT}.jsonl")), SIDECHAIN).unwrap();
    main
}

/// The newest call's context and answer together (31,613 in the fixture's
/// last record), as claude's panel counts it, not a sum over the whole run.
#[test]
fn a_running_agent_shows_the_tokens_its_newest_call_used() {
    let scratch = Scratch::new("tray-tokens");
    let mut s = SessionProjector::open(laid_out(&scratch));
    s.poll();
    let row = sub(s.projection(), "sub:toolu_015abdB6hcuQYTnrL45JDDYm");
    assert_eq!(row.agent_id.as_deref(), Some(AGENT));
    assert_eq!(row.status, SubagentState::Running, "launched in the background, not ended");
    assert_eq!(row.tokens, 31_613, "the last assistant record's usage, whole");
    assert_eq!(row.tool_count, 4);
    let json = serde_json::to_value(s.projection().row("sub:toolu_015abdB6hcuQYTnrL45JDDYm").unwrap()).unwrap();
    assert_eq!(json["kind"]["Subagent"]["tokens"], 31_613, "and on the wire");
}

#[test]
fn a_turn_shows_the_tokens_of_mains_newest_call() {
    let p = fold(COMPLETE);
    let (_, turn) = turns(&p)[0];
    assert_eq!(turn.tokens, 34_466);
}

/// Opened on its own, a subagent's file is a conversation in the view's
/// style: the task it was given as the prompt, then its calls and its words.
#[test]
fn a_subagents_transcript_reads_as_a_conversation_of_its_own() {
    let scratch = Scratch::new("tray-drill");
    let main = laid_out(&scratch);
    let path = subagent_transcript(&main, AGENT).expect("a plain id");
    assert_eq!(path, scratch.path().join(SESSION).join("subagents").join(format!("agent-{AGENT}.jsonl")));
    let mut s = SessionProjector::open(path);
    s.poll();
    let p = s.projection();
    let kinds: Vec<&str> = p
        .rows()
        .iter()
        .map(|r| match &r.kind {
            RowKind::Turn(_) => "turn",
            RowKind::Tool(_) => "tool",
            RowKind::Thinking(_) => "thinking",
            RowKind::Prose(_) => "prose",
            _ => "other",
        })
        .collect();
    let (_, turn) = turns(p)[0];
    assert_eq!(turn.prompt, "[cut: the task the subagent was given]");
    assert_eq!(kinds.iter().filter(|k| **k == "tool").count(), 4, "{kinds:?}");
    assert_eq!(kinds.last(), Some(&"prose"), "its last words close it: {kinds:?}");
    assert_eq!(turn.tokens, 31_613);
}

#[test]
fn a_subagent_id_never_reaches_outside_its_folder() {
    let main = std::path::Path::new("/p/s.jsonl");
    assert_eq!(subagent_transcript(main, "ab_1-c"), Some("/p/s/subagents/agent-ab_1-c.jsonl".into()));
    for bad in ["", "../x", "a/b", "a.b", "a b", "..", &"a".repeat(129)] {
        assert_eq!(subagent_transcript(main, bad), None, "{bad:?}");
    }
    let rollout = std::path::Path::new("/c/rollout-2026-10-07T12-22-14-01a117d0-fd5c-7fe3-8f0f-5f2afdf7906d.jsonl");
    assert_eq!(subagent_transcript(rollout, "a1"), None, "codex follows no subagent files");
}
