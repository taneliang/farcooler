//! The conversation view reads text first (ov-452): a scheduled task says
//! it is one, a message is shown once, a tool row opens to its input and
//! result, and a task list is a checklist.

use super::detail::{DETAIL_CHARS, DETAIL_LINES};
use super::fixtures::*;
use super::rows::*;
use super::Projection;

const TASK_LIST: &str = include_str!("../../../fixtures/session-logs/claude-task-list.jsonl");

fn shown(p: &Projection) -> Vec<&Row> {
    p.rows().iter().filter(|r| !r.retracted).collect()
}

fn queued(p: &Projection) -> Vec<(&str, QueuedState)> {
    shown(p).into_iter().filter_map(|r| match &r.kind { RowKind::Queued(q) => Some((q.text.as_str(), q.state)), _ => None }).collect()
}

/// A scheduled task firing as claude 2.1.285 writes it (shapes from the
/// owner's transcripts, words invented): the enqueue, the dequeue, the
/// `scheduled_task_fire` record, then the prompt, `isMeta` with `promptSource:
/// system`, `turnOrigin: scheduled` and the task's id.
const SCHEDULED: &str = r#"{"type":"user","promptId":"p1","promptSource":"typed","turnOrigin":"human","uuid":"u1","timestamp":"2026-10-06T09:00:00.000Z","message":{"content":"Set up the check-in."}}
{"type":"system","subtype":"turn_duration","durationMs":1000,"uuid":"u2","timestamp":"2026-10-06T09:00:01.000Z"}
{"type":"queue-operation","operation":"enqueue","timestamp":"2026-10-06T10:00:00.002Z","content":"Check-in for the project. Re-evaluate the plan.\nThen run the loop."}
{"type":"queue-operation","operation":"dequeue","timestamp":"2026-10-06T10:00:00.050Z"}
{"type":"system","subtype":"scheduled_task_fire","content":"Running scheduled task (Oct 6 10:00am)","isMeta":false,"uuid":"u3","timestamp":"2026-10-06T10:00:00.000Z","taskId":"f56f5668","cron":"0 10 6 10 *","prompt":"Check-in for the project."}
{"type":"user","promptId":"p2","isMeta":true,"promptSource":"system","scheduledTaskId":"f56f5668","scheduledFireId":"u3","turnOrigin":"scheduled","uuid":"u4","timestamp":"2026-10-06T10:00:00.060Z","message":{"content":"Check-in for the project. Re-evaluate the plan.\nThen run the loop."}}
"#;

#[test]
fn a_scheduled_task_is_a_turn_that_says_so_and_its_queue_row_is_taken_back() {
    let p = fold(SCHEDULED);
    let t = turn(&p, "turn:p2");
    assert_eq!(t.origin, TurnOrigin::Scheduled, "turnOrigin and scheduledTaskId say it was a scheduled task");
    assert_eq!(t.prompt, "Check-in for the project. Re-evaluate the plan.\nThen run the loop.");
    assert!(queued(&p).is_empty(), "the turn shows the prompt; a queue row beside it said it twice: {:?}", queued(&p));
    assert_eq!(p.stats().gaps, 0, "scheduled_task_fire is read silently");

    // Without either field it is what it was: a system prompt.
    let older = SCHEDULED.replace(r#""scheduledTaskId":"f56f5668","#, "").replace(r#""turnOrigin":"scheduled","#, "");
    assert_eq!(turn(&fold(&older), "turn:p2").origin, TurnOrigin::System);
}

#[test]
fn a_message_sent_as_its_own_turn_shows_once_and_one_taken_mid_turn_stays() {
    let p = fold(QUEUE);
    assert_eq!(
        queued(&p),
        [("Also update the docs.", QueuedState::Sent), ("One more idea.", QueuedState::Withdrawn)],
        "the absorbed message has only its row; the dequeued one is its turn"
    );
    assert_eq!(turn(&p, "turn:p2").origin, TurnOrigin::Queued);
    assert_eq!(turn(&p, "turn:p2").prompt, "Then run the tests.");
    let retracted = p.rows().iter().filter(|r| r.retracted && matches!(r.kind, RowKind::Queued(_))).count();
    assert_eq!(retracted, 1, "kept in place, so ord stays its index");
}

#[test]
fn a_tool_row_carries_its_input_and_its_result() {
    let lines = [
        r#"{"type":"user","promptId":"p1","promptSource":"typed","uuid":"u1","message":{"content":"Arm it."}}"#,
        r#"{"type":"assistant","uuid":"a1","message":{"content":[{"type":"tool_use","id":"t1","name":"CronCreate","input":{"cron":"4 15 3 10 *","recurring":false,"prompt":"Restart chain.\nRe-arm it."}},{"type":"tool_use","id":"t2","name":"Bash","input":{"command":"date","description":"Get current time"}},{"type":"tool_use","id":"t3","name":"Read","input":{"file_path":"/w/a.rs"}}]}}"#,
        r#"{"type":"user","uuid":"u2","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"Scheduled f56f5668 (4 15 3 10 *)"},{"type":"tool_result","tool_use_id":"t2","content":[{"type":"text","text":"Sat Oct  3 15:04:02 PDT 2026"},{"type":"image","source":{"type":"base64","data":"AAAA"}}]}]}}"#,
    ];
    let mut text = lines.join("\n");
    let long: String = (0..200).map(|n| format!("{n}: let x = \\\"y\\\";\\n")).collect();
    text.push_str(&format!("\n{{\"type\":\"user\",\"uuid\":\"u3\",\"message\":{{\"content\":[{{\"type\":\"tool_result\",\"tool_use_id\":\"t3\",\"content\":\"{long}\"}}]}}}}\n"));
    let p = fold(&text);

    let cron = tool(&p, "tool:t1");
    assert_eq!(cron.summary, "", "CronCreate has no summary field, which is why it needs to open");
    assert_eq!(cron.input.as_deref(), Some("cron: 4 15 3 10 *\nrecurring: false\nprompt:\nRestart chain.\nRe-arm it."));
    assert_eq!(cron.result.as_deref(), Some("Scheduled f56f5668 (4 15 3 10 *)"));

    let bash = tool(&p, "tool:t2");
    assert_eq!(bash.input.as_deref(), Some("command: date\ndescription: Get current time"));
    assert_eq!(bash.result.as_deref(), Some("Sat Oct  3 15:04:02 PDT 2026"), "the text part, not the image");

    let read = tool(&p, "tool:t3").result.as_deref().unwrap();
    assert_eq!(read.lines().count(), DETAIL_LINES, "a file dump is cut");
    assert!(read.chars().count() <= DETAIL_CHARS + 1);
    assert!(read.starts_with("0: let x = \"y\";") && read.ends_with('…'), "{read}");
}

#[test]
fn a_hooks_tool_row_takes_the_transcripts_input_and_result_when_confirmed() {
    let mut p = fold(r#"{"type":"user","promptId":"p1","promptSource":"typed","uuid":"u1","message":{"content":"Go."}}"#);
    let hook = serde_json::json!({"prompt_id":"p1","tool_use_id":"t1","tool_name":"Bash","tool_input":{"command":"ls","description":"List"}});
    p.hook("PreToolUse", &hook, 1);
    assert_eq!(tool(&p, "tool:t1").input.as_deref(), Some("command: ls\ndescription: List"), "the hook's input, ahead of the record");
    p.fold_line(br#"{"type":"assistant","uuid":"a1","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls","description":"List"}}]}}"#);
    p.fold_line(br#"{"type":"user","uuid":"u2","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"a.rs\nb.rs"}]}}"#);
    let t = tool(&p, "tool:t1");
    assert_eq!((t.status, t.result.as_deref()), (ToolStatus::Done, Some("a.rs\nb.rs")));
}

fn tasks(p: &Projection) -> Vec<(&str, Vec<(&str, TaskStatus)>)> {
    shown(p)
        .into_iter()
        .filter_map(|r| match &r.kind {
            RowKind::Tasks(t) => Some((r.id.as_str(), t.items.iter().map(|i| (i.subject.as_str(), i.status)).collect())),
            _ => None,
        })
        .collect()
}

#[test]
fn the_task_tools_build_one_checklist_row_and_no_tool_rows() {
    let p = fold(TASK_LIST);
    let lists = tasks(&p);
    assert_eq!(lists.len(), 1, "a turn's changes are one row: {lists:?}");
    assert_eq!(
        lists[0].1,
        [
            ("Solve: Design a test matrix for 3 boolean parameters", TaskStatus::InProgress),
            ("Solve: Identify edge cases for string parsing with escapes", TaskStatus::Pending),
        ],
        "made on each create's result, moved by the update that came back; the one with no result yet changed nothing"
    );
    let names: Vec<&str> = shown(&p).iter().filter_map(|r| match &r.kind { RowKind::Tool(t) => Some(t.name.as_str()), _ => None }).collect();
    assert_eq!(names, ["TaskList"], "the writes are the checklist; a read stays a tool row");
}

#[test]
fn a_todo_write_replaces_the_list_and_a_new_turn_gets_its_own_row() {
    let todo = |id: &str, a: &str, b: &str| {
        format!(
            r#"{{"type":"assistant","uuid":"a{id}","message":{{"content":[{{"type":"tool_use","id":"{id}","name":"TodoWrite","input":{{"todos":[{{"content":"Read the code","status":"{a}","activeForm":"Reading"}},{{"content":"Fix it","status":"{b}","activeForm":"Fixing"}}]}}}}]}}}}"#
        )
    };
    let lines = [
        r#"{"type":"user","promptId":"p1","promptSource":"typed","uuid":"u1","message":{"content":"Fix it."}}"#.to_string(),
        todo("t1", "in_progress", "pending"),
        todo("t2", "completed", "in_progress"),
        r#"{"type":"user","promptId":"p2","promptSource":"typed","uuid":"u2","message":{"content":"And finish."}}"#.to_string(),
        todo("t3", "completed", "completed"),
    ];
    let mut p = fold(&lines.join("\n"));
    let lists = tasks(&p);
    assert_eq!(
        lists,
        [
            ("tasks:turn:p1", vec![("Read the code", TaskStatus::Completed), ("Fix it", TaskStatus::InProgress)]),
            ("tasks:turn:p2", vec![("Read the code", TaskStatus::Completed), ("Fix it", TaskStatus::Completed)]),
        ]
    );
    // A hook's announcement of a list write puts up no row of its own.
    let before = p.rows().len();
    let hook = serde_json::json!({"prompt_id":"p2","tool_use_id":"t4","tool_name":"TodoWrite","tool_input":{"todos":[]}});
    p.hook("PreToolUse", &hook, 1);
    p.hook("PostToolUse", &hook, 2);
    assert_eq!(p.rows().len(), before);
}
