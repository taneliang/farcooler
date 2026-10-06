//! Hooks as provisional rows, and the transcript confirming them.

use serde_json::{json, Value};

use super::fixtures::*;
use super::rows::*;
use super::{HookEffect, Projection};

const RECORDED_HOOKS: &str = include_str!("fixtures/recorded-hooks.jsonl");

fn line(p: &mut Projection, json: Value) {
    p.fold_line(json.to_string().as_bytes());
}

fn prompt(id: &str, text: &str, ts: &str) -> Value {
    json!({"type":"user","promptId":id,"promptSource":"typed","timestamp":format!("2026-10-06T{ts}Z"),"message":{"role":"user","content":text}})
}

fn said(uuid: &str, text: &str, ts: &str, stop: &str) -> Value {
    json!({"type":"assistant","uuid":uuid,"timestamp":format!("2026-10-06T{ts}Z"),"message":{"content":[{"type":"text","text":text}],"stop_reason":stop}})
}

fn duration(ts: &str) -> Value {
    json!({"type":"system","subtype":"turn_duration","durationMs":1000,"timestamp":format!("2026-10-06T{ts}Z")})
}

#[test]
fn a_prompt_hook_is_a_provisional_turn_the_transcript_confirms_in_place() {
    let mut p = Projection::new();
    p.hook("UserPromptSubmit", &json!({"prompt_id":"p1","prompt":"Line one\nline two"}), ms("10:00:00.000"));
    let row = p.row("turn:p1").unwrap();
    assert!(row.provisional);
    assert_eq!(turn(&p, "turn:p1").prompt, "Line one\nline two", "the person's own words, at once");
    let ord = row.ord;
    line(&mut p, prompt("p1", "Line one\nline two", "10:00:00.200"));
    let row = p.row("turn:p1").unwrap();
    assert!(!row.provisional);
    assert_eq!(row.ord, ord, "the same row, firm");
    assert_eq!(turns(&p).len(), 1);
    assert_eq!(turn(&p, "turn:p1").started_ms, Some(ms("10:00:00.200")), "the transcript's time is the record");
}

#[test]
fn the_next_turns_hook_does_not_steal_the_tail_of_this_one() {
    let mut p = Projection::new();
    line(&mut p, prompt("p1", "first", "10:00:00.000"));
    p.hook("UserPromptSubmit", &json!({"prompt_id":"p2","prompt":"second"}), ms("10:00:05.000"));
    line(&mut p, said("a1", "First done.", "10:00:04.000", "end_turn"));
    line(&mut p, duration("10:00:04.010"));
    let first = turn(&p, "turn:p1");
    assert_eq!(first.outcome, Some(TurnOutcome::Finished));
    assert_eq!(first.duration_ms, Some(1000));
    assert_eq!(turn(&p, "turn:p2").outcome, None, "still open");
    assert_eq!(p.row("prose:a1:0").unwrap().turn.as_deref(), Some("turn:p1"));
}

#[test]
fn message_display_flushes_are_one_provisional_prose_the_transcript_confirms() {
    let mut p = Projection::new();
    line(&mut p, prompt("p1", "hi", "10:00:00.000"));
    let flush = |index: u64, delta: &str, last: bool| json!({"prompt_id":"p1","turn_id":"t1","message_id":"m1","index":index,"delta":delta,"final":last});
    p.hook("MessageDisplay", &flush(0, "Hello ", false), ms("10:00:01.000"));
    p.hook("MessageDisplay", &flush(0, "Hello ", false), ms("10:00:01.100"));
    p.hook("MessageDisplay", &flush(1, "world.", true), ms("10:00:02.000"));
    let rows = prose(&p);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].0.provisional);
    assert_eq!(rows[0].1.text, "Hello world.", "a repeated index is applied once");
    line(&mut p, said("a1", "Hello world.", "10:00:02.500", "end_turn"));
    let rows = prose(&p);
    assert_eq!(rows.len(), 1, "confirmed, not duplicated");
    assert!(!rows[0].0.provisional);
    assert_eq!(rows[0].0.id, "hprose:m1", "the id the client already has");
    assert!(rows[0].1.conclusion);
}

#[test]
fn a_flush_for_prose_the_transcript_already_wrote_adds_nothing() {
    let mut p = Projection::new();
    line(&mut p, prompt("p1", "hi", "10:00:00.000"));
    line(&mut p, said("a1", "Already written.", "10:00:01.000", "end_turn"));
    p.hook("MessageDisplay", &json!({"prompt_id":"p1","message_id":"m1","index":0,"delta":"Already written.","final":true}), ms("10:00:02.000"));
    assert_eq!(prose(&p).len(), 1);
}

#[test]
fn a_tool_hook_is_the_row_the_transcript_later_confirms_by_its_id() {
    let mut p = Projection::new();
    p.hook("UserPromptSubmit", &json!({"prompt_id":"p1","prompt":"go"}), ms("10:00:00.000"));
    p.hook("PreToolUse", &json!({"prompt_id":"p1","tool_use_id":"toolu_x","tool_name":"Bash","tool_input":{"command":"ls","description":"List files"}}), ms("10:00:01.000"));
    assert!(p.row("tool:toolu_x").unwrap().provisional);
    assert_eq!(tool(&p, "tool:toolu_x").summary, "List files");
    p.hook("PostToolUse", &json!({"prompt_id":"p1","tool_use_id":"toolu_x"}), ms("10:00:02.000"));
    assert_eq!(tool(&p, "tool:toolu_x").status, ToolStatus::Done);
    line(&mut p, prompt("p1", "go", "10:00:00.100"));
    line(&mut p, json!({"type":"assistant","uuid":"a1","timestamp":"2026-10-06T10:00:00.900Z","message":{"content":[{"type":"tool_use","id":"toolu_x","name":"Bash","input":{"command":"ls","description":"List files"}}]}}));
    let row = p.row("tool:toolu_x").unwrap();
    assert!(!row.provisional);
    assert_eq!(tool(&p, "tool:toolu_x").status, ToolStatus::Done, "the hook's result is kept until the transcript's");
    assert_eq!(p.rows().iter().filter(|r| matches!(r.kind, RowKind::Tool(_))).count(), 1);
}

#[test]
fn a_permission_is_held_until_its_tool_comes_back() {
    let mut p = fold(&EDITS.lines().take(4).collect::<Vec<_>>().join("\n"));
    let payload: Value = serde_json::from_str(include_str!("../../../../agent-hooks/tests/fixtures/claude-permission-request.json")).unwrap();
    p.hook("PermissionRequest", &payload, ms("11:00:04.500"));
    let ask = p.rows().iter().find_map(|r| match &r.kind { RowKind::Ask(a) if a.kind == AskKind::Permission => Some((r, a)), _ => None }).unwrap();
    assert!(ask.0.provisional);
    assert_eq!(ask.1.text, "Write /tmp/probe/probe-test.txt");
    assert!(!ask.1.answered);
    line(&mut p, json!({"type":"assistant","uuid":"aw","message":{"content":[{"type":"tool_use","id":"toolu_w","name":"Write","input":{"file_path":"/tmp/probe/probe-test.txt"}}]}}));
    line(&mut p, json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_w","content":"ok"}]}}));
    let answered = p.rows().iter().any(|r| matches!(&r.kind, RowKind::Ask(a) if a.kind == AskKind::Permission && a.answered));
    assert!(answered);
}

#[test]
fn stop_failure_marks_the_turn_failed_even_after_the_transcript_said_finished() {
    let mut p = fold(RECORDED);
    p.hook("StopFailure", &json!({"prompt_id":"f1b51c41-105e-4ffc-8b30-e7ca9913ad60","error":"authentication_failed","last_assistant_message":"OAuth token revoked · Please run /login"}), ms("16:23:18.628"));
    assert_eq!(
        turn(&p, "turn:f1b51c41-105e-4ffc-8b30-e7ca9913ad60").outcome,
        Some(TurnOutcome::Failed { detail: "OAuth token revoked · Please run /login".into() })
    );
}

/// The recorded session and its recorded hooks, interleaved by their own
/// clocks the way the daemon would receive them.
#[test]
fn the_recorded_hooks_and_transcript_together_leave_only_the_unwritten_prompts_provisional() {
    let mut events: Vec<(i64, usize, Result<Value, String>)> = Vec::new();
    let mut last = 0;
    for (n, l) in RECORDED.lines().enumerate() {
        let v: Value = serde_json::from_str(l).unwrap();
        let at = v["timestamp"].as_str().and_then(crate::session_log::claude::parse_iso8601_millis).unwrap_or(last);
        last = at;
        events.push((at, n, Err(l.to_string())));
    }
    for (n, l) in RECORDED_HOOKS.lines().enumerate() {
        let v: Value = serde_json::from_str(l).unwrap();
        let at = (v["t"].as_f64().unwrap() * 1000.0) as i64;
        events.push((at, 10_000 + n, Ok(v)));
    }
    events.sort_by_key(|(at, n, _)| (*at, *n));
    let mut p = Projection::for_session("7f8f9b88-9042-488a-9b94-27868139d02c");
    for (at, _, e) in &events {
        match e {
            Ok(hook) => {
                let effect = p.hook(hook["event"].as_str().unwrap(), &hook["p"], *at);
                assert_eq!(effect, HookEffect::None, "a resume of the same session is no rebind");
            }
            Err(l) => p.fold_line(l.as_bytes()),
        }
    }
    let provisional: Vec<&str> = p.rows().iter().filter(|r| r.provisional).map(|r| r.id.as_str()).collect();
    assert_eq!(provisional, ["turn:dfa0d3a2-e604-4324-95aa-6f2ef0a1d617", "turn:68fdc5fd-6930-4ce1-9e12-4870d13eb060"]);
    for id in &provisional {
        assert_eq!(turn(&p, id).outcome, Some(TurnOutcome::Unrecorded), "{id}");
    }
    assert_eq!(turns(&p).len(), 7, "five written, two only seen by the hook");
    let failed = turns(&p).iter().filter(|(_, t)| matches!(t.outcome, Some(TurnOutcome::Failed { .. }))).count();
    assert_eq!(failed, 5, "every written turn hit the revoked login");
    assert!(p.rows().iter().any(|r| matches!(&r.kind, RowKind::Notice(n) if n.kind == NoticeKind::Resumed)));
}

#[test]
fn session_start_for_a_new_session_rebinds_and_one_for_this_session_does_not() {
    let mut p = Projection::for_session("old");
    assert_eq!(p.hook("SessionStart", &json!({"session_id":"old","source":"startup"}), 1), HookEffect::None);
    assert_eq!(p.hook("SessionStart", &json!({"session_id":"old","source":"compact"}), 2), HookEffect::None);
    let effect = p.hook("SessionStart", &json!({"session_id":"new","source":"clear","transcript_path":"/x/new.jsonl"}), 3);
    assert_eq!(
        effect,
        HookEffect::Rebind { session_id: "new".into(), transcript_path: Some("/x/new.jsonl".into()), source: "clear".into() }
    );
    let kinds: Vec<NoticeKind> = p.rows().iter().filter_map(|r| match &r.kind { RowKind::Notice(n) => Some(n.kind), _ => None }).collect();
    assert_eq!(kinds, [NoticeKind::Compacted, NoticeKind::Cleared]);
    assert_eq!(p.hook("SessionStart", &json!({"session_id":"new","source":"resume"}), 4), HookEffect::None);
}

#[test]
fn a_compaction_hook_and_its_boundary_record_are_one_notice() {
    let mut p = fold(&COMPACT.lines().take(3).collect::<Vec<_>>().join("\n"));
    p.hook("SessionStart", &json!({"source":"compact"}), ms("12:04:59.000"));
    for l in COMPACT.lines().skip(3) {
        p.fold_line(l.as_bytes());
    }
    let compacted: Vec<&Row> = p.rows().iter().filter(|r| matches!(&r.kind, RowKind::Notice(n) if n.kind == NoticeKind::Compacted)).collect();
    assert_eq!(compacted.len(), 1);
    assert!(!compacted[0].provisional);
}

#[test]
fn a_subagent_stop_before_its_join_ends_it_when_the_join_arrives() {
    let mut p = Projection::new();
    p.hook("SubagentStop", &json!({"agent_id":"abg1"}), ms("10:00:30.000"));
    for l in BACKGROUND.lines().take(7) {
        p.fold_line(l.as_bytes());
    }
    assert_eq!(sub(&p, "sub:toolu_bg").status, SubagentState::Completed);
}

#[test]
fn hooks_this_build_does_not_read_change_nothing() {
    let mut p = fold(EDITS);
    let before = p.rows().to_vec();
    p.hook("Notification", &json!({"message":"Claude is waiting for your input"}), 1);
    p.hook("SessionEnd", &json!({"reason":"other"}), 2);
    p.hook("SomethingNew", &json!({}), 3);
    assert_eq!(p.rows(), &before[..]);
}
