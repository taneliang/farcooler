//! `test/fixtures/agent-rows.json` is what this boundary writes (ov-371):
//! a page holding every row kind the projector folds, and a follow with an
//! insert, an update and a removal, each row serialized by the projector's
//! own types and wrapped by `rows_args`. The Mac's and the phone's decoder
//! (`AgentRowWire.swift`) read the same file, so a field renamed in Rust
//! fails here first, with the JSON to regenerate the fixture from.

use farcooler_core::session_log::projector::*;
use farcooler_protocol::v1::{AgentRow as WireRow, AgentRowChange, AgentRowChangeKind, AgentRowChanges, AgentRowPage};
use serde_json::{Value, json};

const FIXTURE: &str = include_str!("../../../../test/fixtures/agent-rows.json");

fn row(ord: u64, rev: u64, id: &str, turn: Option<&str>, kind: RowKind) -> Row {
    Row { ord, rev, id: id.into(), turn: turn.map(str::to_string), provisional: false, retracted: false, born: 0, kind }
}

fn wire(row: &Row) -> WireRow {
    WireRow { id: row.id.clone(), ord: row.ord, rev: row.rev, row_json: serde_json::to_string(row).expect("a row serializes") }
}

/// One of each kind, and every enum's tagged and untagged spellings.
fn rows() -> Vec<Row> {
    let t = Some("turn:p1");
    vec![
        row(0, 9, "turn:p1", None, RowKind::Turn(Turn {
            prompt: "Fix the build\nand the tests".into(),
            origin: TurnOrigin::Typed,
            started_ms: Some(1_000),
            ended_ms: Some(61_000),
            duration_ms: Some(60_000),
            outcome: Some(TurnOutcome::Failed { detail: "API error".into() }),
            background_running: 1,
            activity: Some(Activity::Busy),
            suggestion: Some("run the tests again".into()),
            images: vec![PromptImage { mime: "image/png".into() }],
            source: None,
            tokens: 52_100,
        })),
        row(1, 2, "prose:1", t, RowKind::Prose(Prose { text: "Looking at **main.rs**.".into(), conclusion: false, at_ms: Some(2_000) })),
        row(2, 3, "think:1", t, RowKind::Thinking(Thinking { started_ms: Some(2_500), ended_ms: Some(4_500) })),
        row(3, 4, "tool:toolu_1", t, RowKind::Tool(Tool {
            name: "Edit".into(),
            summary: "src/main.rs".into(),
            status: ToolStatus::Done,
            started_ms: Some(5_000),
            ended_ms: Some(5_400),
            diff: vec![Hunk { old_start: 3, old_lines: 1, new_start: 3, new_lines: 1, lines: vec!["-a".into(), "+b".into()] }],
            file_path: Some("/w/src/main.rs".into()),
            input: None,
            result: None,
        })),
        row(4, 8, "sub:toolu_2", t, RowKind::Subagent(Subagent {
            tool_use_id: "toolu_2".into(),
            agent_id: Some("a1".into()),
            agent_type: "Explore".into(),
            description: "Find the callers".into(),
            background: true,
            status: SubagentState::Killed,
            started_ms: Some(6_000),
            ended_ms: None,
            tool_count: 7,
            current_action: "Grep fn main".into(),
            last_ms: Some(9_000),
            tokens: 87_200,
        })),
        // A question the runner's hook holds (ov-370): its id and its
        // options are what a view answers it with.
        row(5, 5, "ask:1", t, RowKind::Ask(Ask {
            kind: AskKind::Question,
            text: "Which color?".into(),
            tool: Some("AskUserQuestion".into()),
            asked_ms: Some(7_000),
            answered_ms: None,
            answered: false,
            held: Some("hook-ask-1".into()),
            questions: vec![AskQuestion {
                question: "Which color?".into(),
                header: "Color".into(),
                options: vec![
                    AskOption { label: "Red".into(), description: "Warm".into() },
                    AskOption { label: "Blue".into(), description: "Calm".into() },
                ],
                multi_select: false,
            }],
            plan: None,
            answered_by: None,
        })),
        row(6, 6, "queued:1", t, RowKind::Queued(Queued { text: "and then the docs".into(), state: QueuedState::Waiting, at_ms: Some(8_000) })),
        row(7, 7, "notice:1", t, RowKind::Notice(Notice { kind: NoticeKind::Compacted, text: "Context compacted".into(), at_ms: None })),
        row(8, 9, "handoff:1", t, RowKind::Handoff(Handoff { reason: "A panel is open".into(), at_ms: Some(9_500) })),
        row(9, 9, "gap:1", t, RowKind::Gap(Gap { reason: GapReason::Unknown("x-new".into()), count: 2 })),
    ]
}

fn written() -> Value {
    let rows = rows();
    let page = AgentRowPage { terminal_id: Default::default(), epoch: 4, rev: 9, rows: rows.iter().map(wire).collect(), more_before: true };
    let mut running = rows[3].clone();
    running.rev = 11;
    if let RowKind::Tool(tool) = &mut running.kind {
        tool.status = ToolStatus::Running;
        tool.ended_ms = None;
    }
    let fresh = row(10, 10, "prose:2", Some("turn:p1"), RowKind::Prose(Prose { text: "Done.".into(), conclusion: true, at_ms: Some(10_000) }));
    let change = |kind: AgentRowChangeKind, row: Option<&Row>, id: &str, rev: u64| AgentRowChange {
        kind: kind as i32,
        id: id.into(),
        rev,
        row: row.map(wire),
    };
    let follow = AgentRowChanges {
        terminal_id: Default::default(),
        epoch: 4,
        rev: 12,
        changes: vec![
            change(AgentRowChangeKind::Insert, Some(&fresh), "prose:2", 10),
            change(AgentRowChangeKind::Update, Some(&running), "tool:toolu_1", 11),
            change(AgentRowChangeKind::Remove, None, "queued:1", 12),
        ],
        reset: false,
    };
    json!({ "page": super::rows_args::page_of(&page), "follow": super::rows_args::changes_of(&follow) })
}

#[test]
fn the_shared_fixture_is_what_the_rows_boundary_writes() {
    let fixture: Value = serde_json::from_str(FIXTURE).expect("the fixture is JSON");
    assert_eq!(
        fixture,
        written(),
        "test/fixtures/agent-rows.json is not what rows_args writes; regenerate it:\n{}",
        serde_json::to_string_pretty(&written()).unwrap_or_default()
    );
}

/// Every kind is in the page, so a decoder test reading it can't pass by
/// reading nothing.
#[test]
fn the_fixture_holds_every_row_kind() {
    let page = &written()["page"]["rows"];
    let kinds: Vec<String> = page
        .as_array()
        .expect("rows")
        .iter()
        .filter_map(|row| row["kind"].as_object().and_then(|k| k.keys().next().cloned()))
        .collect();
    assert_eq!(kinds, ["Turn", "Prose", "Thinking", "Tool", "Subagent", "Ask", "Queued", "Notice", "Handoff", "Gap"]);
}

const HINT_FIXTURE: &str = include_str!("../../../../test/fixtures/agent-rows-hint.json");

/// A fresh session's page: no turn, only claude's `Try` example on its
/// `Hint` row (ov-409), as the boundary writes it. Swift and Kotlin decode
/// the same file.
fn hint_written() -> Value {
    let hint = row(0, 3, "hint:composer", None, RowKind::Hint(Hint { text: "Try \"how does <filepath> work?\"".into() }));
    let page = AgentRowPage { terminal_id: Default::default(), epoch: 7, rev: 3, rows: vec![wire(&hint)], more_before: false };
    super::rows_args::page_of(&page)
}

#[test]
fn the_hint_fixture_is_what_the_rows_boundary_writes() {
    let fixture: Value = serde_json::from_str(HINT_FIXTURE).expect("the fixture is JSON");
    assert_eq!(
        fixture,
        hint_written(),
        "test/fixtures/agent-rows-hint.json is not what rows_args writes; regenerate it:\n{}",
        serde_json::to_string_pretty(&hint_written()).unwrap_or_default()
    );
}

const POLISH_FIXTURE: &str = include_str!("../../../../test/fixtures/agent-rows-polish.json");

/// What ov-452 added, as the boundary writes it: a scheduled task's turn, a
/// tool row's input and result, and a task list's checklist row. Swift
/// decodes the same file.
fn polish_written() -> Value {
    let t = Some("turn:p2");
    let rows = [
        row(0, 4, "turn:p2", None, RowKind::Turn(Turn {
            prompt: "Check-in for the project.\nRe-evaluate the plan.".into(),
            origin: TurnOrigin::Scheduled,
            started_ms: Some(1_000),
            ended_ms: None,
            duration_ms: None,
            outcome: None,
            background_running: 0,
            activity: None,
            suggestion: None,
            images: Vec::new(),
            source: None,
            tokens: 0,
        })),
        row(1, 2, "tool:toolu_c", t, RowKind::Tool(Tool {
            name: "CronCreate".into(),
            summary: String::new(),
            status: ToolStatus::Done,
            started_ms: Some(2_000),
            ended_ms: Some(2_100),
            diff: Vec::new(),
            file_path: None,
            input: Some("cron: 4 15 3 10 *\nrecurring: false".into()),
            result: Some("Scheduled f56f5668".into()),
        })),
        row(2, 4, "tasks:turn:p2", t, RowKind::Tasks(Tasks {
            items: vec![
                TaskItem { subject: "Read the code".into(), status: TaskStatus::Completed },
                TaskItem { subject: "Fix it".into(), status: TaskStatus::InProgress },
                TaskItem { subject: "Ship it".into(), status: TaskStatus::Pending },
            ],
        })),
    ];
    let page = AgentRowPage { terminal_id: Default::default(), epoch: 8, rev: 4, rows: rows.iter().map(wire).collect(), more_before: false };
    super::rows_args::page_of(&page)
}

#[test]
fn the_polish_fixture_is_what_the_rows_boundary_writes() {
    let written = polish_written();
    let fixture: Value = serde_json::from_str(POLISH_FIXTURE).unwrap_or(Value::Null);
    assert_eq!(
        fixture,
        written,
        "test/fixtures/agent-rows-polish.json is not what rows_args writes; regenerate it:\n{}",
        serde_json::to_string_pretty(&written).unwrap_or_default()
    );
}
