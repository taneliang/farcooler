//! A subagent's own subagents, and lines read twice (ov-366, from ov-363's
//! reviews). Synthetic files in the shapes claude 2.1.290 writes (a nested
//! meta's `parentAgentId` and `spawnDepth`, an attachment-only notification,
//! uuid-less `queue-operation` lines); every word invented.

use std::path::Path;

use serde_json::{json, Value};

use super::fixtures::*;
use super::rows::*;
use super::SessionProjector;

const SESSION: &str = "c0ffee00-0000-4000-8000-0000000000aa";

fn jsonl(lines: &[Value]) -> String {
    lines.iter().map(|l| format!("{l}\n")).collect()
}

fn agent_call(uuid: &str, id: &str, description: &str, background: bool) -> Value {
    json!({"type":"assistant","uuid":uuid,"timestamp":"2026-10-06T10:00:01Z","message":{"content":[{"type":"tool_use","id":id,"name":"Agent","input":{"description":description,"subagent_type":"Explore","run_in_background":background}}]}})
}

fn launched(uuid: &str, id: &str, agent: &str) -> Value {
    json!({"type":"user","uuid":uuid,"timestamp":"2026-10-06T10:00:02Z","message":{"content":[{"type":"tool_result","tool_use_id":id,"content":"Async agent launched successfully."}]},"toolUseResult":{"isAsync":true,"status":"async_launched","agentId":agent}})
}

/// The nested agent's files sort BEFORE its parent's (`a1…` before `a2…`), so
/// one read meets its meta before the call it names.
fn write_nested(dir: &Path) -> std::path::PathBuf {
    let main = dir.join(format!("{SESSION}.jsonl"));
    std::fs::write(
        &main,
        jsonl(&[
            json!({"type":"user","promptId":"p1","promptSource":"typed","uuid":"u1","timestamp":"2026-10-06T10:00:00Z","message":{"content":"Research it."}}),
            agent_call("m1", "toolu_parent", "Survey the code", true),
            launched("m2", "toolu_parent", "a2parent"),
        ]),
    )
    .unwrap();
    let subs = dir.join(SESSION).join("subagents");
    std::fs::create_dir_all(&subs).unwrap();
    std::fs::write(
        subs.join("agent-a2parent.jsonl"),
        jsonl(&[
            json!({"type":"user","agentId":"a2parent","uuid":"s1","timestamp":"2026-10-06T10:00:01Z","message":{"content":"Survey the code"}}),
            // No launch result: the meta is the only join (a foreground
            // agent's result is written when it ends).
            agent_call("s2", "toolu_nested", "Read the parser", true),
        ]),
    )
    .unwrap();
    std::fs::write(
        subs.join("agent-a2parent.meta.json"),
        r#"{"agentType":"Explore","description":"Survey the code","toolUseId":"toolu_parent","spawnDepth":1,"requestShape":"background"}"#,
    )
    .unwrap();
    std::fs::write(
        subs.join("agent-a1nested.jsonl"),
        jsonl(&[
            json!({"type":"user","agentId":"a1nested","uuid":"n1","timestamp":"2026-10-06T10:00:03Z","message":{"content":"Read the parser"}}),
            json!({"type":"assistant","agentId":"a1nested","uuid":"n2","timestamp":"2026-10-06T10:00:04Z","message":{"content":[{"type":"tool_use","id":"toolu_read","name":"Read","input":{"file_path":"parser.rs"}}]}}),
        ]),
    )
    .unwrap();
    std::fs::write(
        subs.join("agent-a1nested.meta.json"),
        r#"{"agentType":"Explore","description":"Read the parser","toolUseId":"toolu_nested","parentAgentId":"a2parent","spawnDepth":2,"requestShape":"background"}"#,
    )
    .unwrap();
    main
}

#[test]
fn a_subagents_own_subagent_is_a_row_joined_in_one_read() {
    let scratch = Scratch::new("nested");
    let main = write_nested(scratch.path());
    let mut s = SessionProjector::open(main);
    s.poll();
    let p = s.projection();
    let nested = p.row("sub:toolu_nested").expect("the nested agent has a row");
    assert_eq!(nested.turn.as_deref(), Some("turn:p1"), "in the turn its parent belongs to");
    let nested = sub(p, "sub:toolu_nested");
    assert_eq!(nested.agent_id.as_deref(), Some("a1nested"));
    assert_eq!(nested.tool_count, 1, "its own transcript, joined: {nested:?}");
    assert_eq!(nested.current_action, "Read parser.rs");
    assert_eq!(nested.status, SubagentState::Running);
    assert_eq!(turn(p, "turn:p1").background_running, 2);

    // Ended the way 72% of real background agents are: only a queued
    // attachment in the parent's own file.
    let subs = scratch.path().join(SESSION).join("subagents");
    let notified = json!({"type":"attachment","agentId":"a2parent","uuid":"s4","timestamp":"2026-10-06T10:00:30Z","attachment":{"type":"queued_command","prompt":"<task-notification>\n<task-id>a1nested</task-id>\n<tool-use-id>toolu_nested</tool-use-id>\n<status>completed</status>\n<summary>Agent \"Read the parser\" finished</summary>\n</task-notification>","commandMode":"task-notification"}});
    let mut file = std::fs::OpenOptions::new().append(true).open(subs.join("agent-a2parent.jsonl")).unwrap();
    std::io::Write::write_all(&mut file, format!("{notified}\n").as_bytes()).unwrap();
    s.poll();
    let p = s.projection();
    assert_eq!(sub(p, "sub:toolu_nested").status, SubagentState::Completed);
    assert_eq!(turn(p, "turn:p1").background_running, 1, "the parent still runs");
}

fn queued(p: &super::Projection) -> usize {
    p.rows().iter().filter(|r| matches!(r.kind, RowKind::Queued(_))).count()
}

/// `/clear`, then `/resume` back: the first session's file is read again from
/// its start into the same projection. Its uuid-less enqueue lines added a
/// second Queued row each.
#[test]
fn a_resume_back_into_a_session_shown_adds_no_queued_rows() {
    let scratch = Scratch::new("resume-queue");
    let first = scratch.path().join("s-first.jsonl");
    let second = scratch.path().join("s-second.jsonl");
    std::fs::write(&first, QUEUE).unwrap();
    std::fs::write(&second, jsonl(&[json!({"type":"user","promptId":"q2","promptSource":"typed","uuid":"x1","message":{"content":"Elsewhere."}})])).unwrap();
    let mut s = SessionProjector::open(first.clone());
    s.poll();
    let before = queued(s.projection());
    assert!(before >= 2, "the fixture queues messages: {before}");
    let rows = s.projection().rows().len();
    s.rebind(second);
    s.poll();
    s.rebind(first.clone());
    s.poll();
    assert_eq!(queued(s.projection()), before);
    assert_eq!(s.projection().rows().len(), rows + 1, "only the other session's turn is new");

    // And a file rewritten in place with the same lines.
    std::fs::write(&first, "").unwrap();
    s.poll();
    std::fs::write(&first, QUEUE).unwrap();
    s.poll();
    assert_eq!(queued(s.projection()), before);
}

/// Twin lines within one read are still two (two dequeues in one
/// millisecond), and a twin appended later is a new one.
#[test]
fn twin_uuidless_lines_in_one_read_both_fold() {
    let scratch = Scratch::new("twins");
    let path = scratch.path().join("s-twins.jsonl");
    let enqueue = json!({"type":"queue-operation","operation":"enqueue","timestamp":"2026-10-06T12:00:05.000Z","content":"Again."});
    std::fs::write(&path, jsonl(&[enqueue.clone(), enqueue.clone()])).unwrap();
    let mut s = SessionProjector::open(path.clone());
    s.poll();
    assert_eq!(queued(s.projection()), 2);
    let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
    std::io::Write::write_all(&mut file, format!("{enqueue}\n").as_bytes()).unwrap();
    s.poll();
    assert_eq!(queued(s.projection()), 3);
}

/// A rebuild reads the main transcript before any subagent file, so a nested
/// agent notified only there (an enqueue in main, 15 of the corpus's 78) is
/// ended before its row exists. It must still end once the row is joined.
#[test]
fn a_nested_agent_notified_only_in_main_ends_on_a_rebuild() {
    let scratch = Scratch::new("nested-rebuild");
    let main = write_nested(scratch.path());
    let notified = json!({"type":"queue-operation","operation":"enqueue","timestamp":"2026-10-06T10:00:40.000Z","content":"<task-notification>\n<task-id>a1nested</task-id>\n<tool-use-id>toolu_nested</tool-use-id>\n<status>completed</status>\n<summary>Agent \"Read the parser\" finished</summary>\n</task-notification>"});
    let mut file = std::fs::OpenOptions::new().append(true).open(&main).unwrap();
    std::io::Write::write_all(&mut file, format!("{notified}\n").as_bytes()).unwrap();
    let mut s = SessionProjector::open(main);
    s.poll();
    let nested = sub(s.projection(), "sub:toolu_nested");
    assert_eq!(nested.status, SubagentState::Completed, "{nested:?}");
    assert_eq!(nested.ended_ms, Some(ms("10:00:40.000")), "at the notification's time");
}

/// A watch event that names only the subagents directory (a file created in
/// it) reads a subagent file that has just appeared.
#[test]
fn an_event_on_the_subagents_directory_reads_a_new_subagent() {
    let scratch = Scratch::new("new-subagent");
    let main = scratch.path().join(format!("{SESSION}.jsonl"));
    std::fs::write(
        &main,
        jsonl(&[
            json!({"type":"user","promptId":"p1","promptSource":"typed","uuid":"u1","timestamp":"2026-10-06T10:00:00Z","message":{"content":"Go."}}),
            agent_call("m1", "toolu_late", "Look around", true),
        ]),
    )
    .unwrap();
    let subs = scratch.path().join(SESSION).join("subagents");
    std::fs::create_dir_all(&subs).unwrap();
    let mut s = SessionProjector::open(main);
    s.poll();
    std::fs::write(subs.join("agent-alate.meta.json"), r#"{"agentType":"Explore","toolUseId":"toolu_late"}"#).unwrap();
    std::fs::write(
        subs.join("agent-alate.jsonl"),
        jsonl(&[json!({"type":"assistant","uuid":"n1","timestamp":"2026-10-06T10:00:04Z","message":{"content":[{"type":"tool_use","id":"toolu_r","name":"Read","input":{"file_path":"a.rs"}}]}})]),
    )
    .unwrap();
    s.poll_paths(std::slice::from_ref(&subs));
    let late = sub(s.projection(), "sub:toolu_late");
    assert_eq!(late.agent_id.as_deref(), Some("alate"), "{late:?}");
    assert_eq!(late.tool_count, 1);
}
