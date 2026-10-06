//! A `SubagentStart` folded out of order, and a subagent's own stop
//! (ov-364 review 1, findings 1 and 3, fixed in ov-366).

use serde_json::json;

use super::fixtures::*;
use super::rows::*;
use super::{Projection, SessionProjector, SubagentMeta};

fn launch(p: &mut Projection, id: &str) {
    p.hook("PreToolUse", &json!({"prompt_id":"p1","tool_use_id":id,"tool_name":"Agent","tool_input":{"subagent_type":"Explore","description":"Look around","run_in_background":true}}), ms("10:00:01.000"));
}

fn start(p: &mut Projection, agent: &str) {
    p.hook("SubagentStart", &json!({"prompt_id":"p1","agent_id":agent,"agent_type":"Explore"}), ms("10:00:01.100"));
}

fn meta(tool: &str) -> SubagentMeta {
    SubagentMeta { tool_use_id: Some(tool.into()), agent_type: Some("Explore".into()), ..SubagentMeta::default() }
}

/// SubagentStart(b) folded before PreToolUse(B): b is tied to A, the only
/// untied row then, and a to B. The meta files name the rows, and win.
fn swapped() -> Projection {
    let mut p = Projection::new();
    p.hook("UserPromptSubmit", &json!({"prompt_id":"p1","prompt":"go"}), ms("10:00:00.000"));
    launch(&mut p, "toolu_a");
    start(&mut p, "b1");
    launch(&mut p, "toolu_b");
    start(&mut p, "a1");
    assert_eq!(sub(&p, "sub:toolu_a").agent_id.as_deref(), Some("b1"), "the race, as the review found it");
    assert_eq!(sub(&p, "sub:toolu_b").agent_id.as_deref(), Some("a1"));
    p
}

#[test]
fn a_hook_tie_out_of_order_is_undone_by_the_meta_files() {
    let mut p = swapped();
    // b's transcript, while it sits on A's row.
    p.fold_subagent_line("b1", None, json!({"type":"assistant","uuid":"b-1","timestamp":"2026-10-06T10:00:02Z","message":{"content":[{"type":"tool_use","id":"toolu_g","name":"Grep","input":{"pattern":"fence"}}]}}).to_string().as_bytes());
    assert!(!p.is_joined("b1"), "a tie by type alone is not final");
    p.join_by_meta("b1", &meta("toolu_b"));
    p.join_by_meta("a1", &meta("toolu_a"));
    let (a, b) = (sub(&p, "sub:toolu_a"), sub(&p, "sub:toolu_b"));
    assert_eq!((a.agent_id.as_deref(), b.agent_id.as_deref()), (Some("a1"), Some("b1")));
    assert_eq!((b.tool_count, b.current_action.as_str()), (1, "Grep fence"), "b's work goes with it");
    assert_eq!((a.tool_count, a.current_action.as_str()), (0, ""), "and leaves A");
    assert!(p.is_joined("a1") && p.is_joined("b1"));
}

#[test]
fn a_launch_result_naming_the_row_also_undoes_a_hook_tie() {
    let mut p = swapped();
    p.fold_line(json!({"type":"user","uuid":"r-b","timestamp":"2026-10-06T10:00:02Z","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_b","content":"launched"}]},"toolUseResult":{"isAsync":true,"status":"async_launched","agentId":"b1"}}).to_string().as_bytes());
    assert_eq!(sub(&p, "sub:toolu_b").agent_id.as_deref(), Some("b1"));
    assert_eq!(sub(&p, "sub:toolu_a").agent_id, None, "a1 left B; its meta places it");
    p.join_by_meta("a1", &meta("toolu_a"));
    assert_eq!(sub(&p, "sub:toolu_a").agent_id.as_deref(), Some("a1"));
}

/// The same race on the daemon's path: hooks first, then a read of the
/// files, whose meta join must not skip an agent the hook tied.
#[test]
fn a_read_of_the_meta_files_corrects_a_hook_tie() {
    let scratch = Scratch::new("hook-tie");
    let session = "c0ffee00-0000-4000-8000-0000000000bb";
    let main = scratch.path().join(format!("{session}.jsonl"));
    std::fs::write(&main, "").unwrap();
    let subs = scratch.path().join(session).join("subagents");
    std::fs::create_dir_all(&subs).unwrap();
    for (agent, tool) in [("a1", "toolu_a"), ("b1", "toolu_b")] {
        std::fs::write(subs.join(format!("agent-{agent}.meta.json")), format!(r#"{{"agentType":"Explore","toolUseId":"{tool}"}}"#)).unwrap();
        std::fs::write(subs.join(format!("agent-{agent}.jsonl")), "").unwrap();
    }
    let mut s = SessionProjector::open(main);
    *s.projection_mut() = swapped();
    s.poll();
    let p = s.projection();
    assert_eq!(sub(p, "sub:toolu_a").agent_id.as_deref(), Some("a1"));
    assert_eq!(sub(p, "sub:toolu_b").agent_id.as_deref(), Some("b1"));
}

/// A Stop or StopFailure that carries `agent_id` is that subagent's end: its
/// row ends, and the main turn goes on.
#[test]
fn a_subagents_own_stop_ends_only_its_row() {
    let mut p = Projection::new();
    p.hook("UserPromptSubmit", &json!({"prompt_id":"p1","prompt":"go"}), ms("10:00:00.000"));
    launch(&mut p, "toolu_a");
    start(&mut p, "a1");
    launch(&mut p, "toolu_b");
    p.hook("SubagentStart", &json!({"prompt_id":"p1","agent_id":"b1","agent_type":"Explore"}), ms("10:00:01.200"));
    p.hook("Stop", &json!({"prompt_id":"p1","agent_id":"a1"}), ms("10:00:05.000"));
    p.hook("StopFailure", &json!({"prompt_id":"p1","agent_id":"b1","error":"overloaded"}), ms("10:00:06.000"));
    assert_eq!(sub(&p, "sub:toolu_a").status, SubagentState::Completed);
    assert_eq!(sub(&p, "sub:toolu_b").status, SubagentState::Failed);
    assert_eq!(turn(&p, "turn:p1").outcome, None, "the main turn is still open");
    p.hook("Stop", &json!({"prompt_id":"p1"}), ms("10:00:07.000"));
    assert_eq!(turn(&p, "turn:p1").outcome, Some(TurnOutcome::Finished));
}
