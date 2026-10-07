//! Codex's rollout and hooks into rows (ov-378), against real captures: a
//! codex-cli 0.153.4 TUI in a sandbox, driven through a stand-in API
//! (`crates/core/fixtures/session-logs/README.md`, "codex-tui-0.153.4").

use serde_json::Value;

use super::files::{is_codex_rollout, SessionProjector};
use super::fixtures::Scratch;
use super::fold::Projection;
use super::rows::*;
use super::HookEffect;
use crate::session_log::claude::parse_iso8601_millis;

const DIR: &str = "codex-tui-0.153.4";
const MAIN_NAME: &str = "rollout-2026-10-07T12-22-14-01a117d0-fd5c-7fe3-8f0f-5f2afdf7906d.jsonl";
const MAIN: &str = include_str!("../../../fixtures/session-logs/codex-tui-0.153.4/rollout-2026-10-07T12-22-14-01a117d0-fd5c-7fe3-8f0f-5f2afdf7906d.jsonl");
const NEW: &str = include_str!("../../../fixtures/session-logs/codex-tui-0.153.4/rollout-2026-10-07T12-28-45-01a117d6-f550-7013-a939-b02d182c0321.jsonl");
const HOOKS_TWO_FAIL_NEW: &str = include_str!("../../../fixtures/session-logs/codex-tui-0.153.4/hooks-two-fail-new.jsonl");
const HOOKS_APPROVE_INTERRUPT: &str = include_str!("../../../fixtures/session-logs/codex-tui-0.153.4/hooks-approve-interrupt.jsonl");
const OLD_SHAPE: &str = include_str!("../../../fixtures/session-logs/codex-complete-turn.jsonl");
const ITEM_SHAPE: &str = include_str!("../../../fixtures/session-logs/codex-item-completed-turn.jsonl");

fn fold(text: &str) -> Projection {
    let mut p = Projection::new();
    for line in text.lines() {
        p.fold_codex_line(line.as_bytes());
    }
    p
}

/// Each row as one line a person could check against the session.
fn shown(p: &Projection) -> Vec<String> {
    p.rows()
        .iter()
        .filter(|r| !r.retracted)
        .map(|r| match &r.kind {
            RowKind::Turn(t) => format!("turn {:?} {:?}", t.prompt, t.outcome),
            RowKind::Prose(x) => format!("  prose {:?}{}", x.text, if x.conclusion { " (conclusion)" } else { "" }),
            RowKind::Tool(t) => format!("  tool {} {:?} {:?}", t.name, t.summary, t.status),
            RowKind::Queued(q) => format!("  steer {:?} {:?}", q.text, q.state),
            RowKind::Ask(a) => format!("  ask {:?} {:?} answered={}", a.kind, a.text, a.answered),
            other => format!("  {other:?}"),
        })
        .collect()
}

fn newest_activity(p: &Projection) -> Option<Activity> {
    p.rows().iter().rev().find_map(|r| match &r.kind {
        RowKind::Turn(t) => Some(t.activity),
        _ => None,
    })?
}

#[test]
fn a_real_session_folds_into_its_turns() {
    let p = fold(MAIN);
    let want = [
        r#"turn "hello there" Some(Finished)"#,
        r#"  prose "done: hello there" (conclusion)"#,
        r#"turn "SLOW 4" Some(Finished)"#,
        r#"  prose "Slept, and here I am." (conclusion)"#,
        // Approved.
        r#"turn "RUNX touch made.txt" Some(Finished)"#,
        r#"  tool Bash "touch made.txt" Done"#,
        r#"  prose "done: RUNX touch made.txt" (conclusion)"#,
        // Denied with Esc: codex aborts the turn.
        r#"turn "RUNX touch second.txt" Some(Interrupted)"#,
        r#"  tool Bash "touch second.txt" Failed"#,
        r#"turn "ASK please" Some(Finished)"#,
        r#"  ask Question "Which one?" answered=true"#,
        r#"  prose "done: ASK please" (conclusion)"#,
        // Enter while busy steers the running turn.
        r#"turn "SLOW 4" Some(Finished)"#,
        r#"  prose "Slept, and here I am." (conclusion)"#,
        r#"  steer "later msg" Sent"#,
        r#"  prose "done: later msg" (conclusion)"#,
        r#"turn "!sleep 2" Some(Finished)"#,
        r#"  tool Bash "sleep 2" Done"#,
        // Tab queues: a turn of its own.
        r#"turn "SLOW 4" Some(Finished)"#,
        r#"  prose "Slept, and here I am." (conclusion)"#,
        r#"turn "queued one" Some(Finished)"#,
        r#"  prose "done: queued one" (conclusion)"#,
        r#"turn "TWO" Some(Finished)"#,
        r#"  prose "Looking around first.""#,
        r#"  tool Bash "echo two" Done"#,
        r#"  prose "done: TWO" (conclusion)"#,
        r#"turn "RUN ls nonexistent" Some(Finished)"#,
        r#"  tool Bash "ls nonexistent" Failed"#,
        r#"  prose "done: RUN ls nonexistent" (conclusion)"#,
    ];
    assert_eq!(shown(&p), want);
    assert_eq!(p.stats().gaps, 0, "every record of 0.153.4 is known");
    assert!(p.rows().iter().all(|r| !r.provisional), "a rollout alone is the record");
    // codex's own durations, not the span between two records.
    let durations: Vec<Option<i64>> = p.rows().iter().filter_map(|r| match &r.kind {
        RowKind::Turn(t) => Some(t.duration_ms),
        _ => None,
    }).collect();
    assert_eq!(&durations[..3], &[Some(375), Some(4106), Some(12262)]);
}

/// The activity a view shows is busy exactly while the rollout's last turn
/// boundary is a `task_started`, at every line of the session.
#[test]
fn activity_follows_every_turn_boundary() {
    let mut p = Projection::new();
    let mut open = None;
    let mut busy_lines = 0;
    for (n, line) in MAIN.lines().enumerate() {
        p.fold_codex_line(line.as_bytes());
        let record: Value = serde_json::from_str(line).unwrap();
        match record.pointer("/payload/type").and_then(Value::as_str) {
            Some("task_started") if record["type"] == "event_msg" => open = Some(true),
            Some("task_complete" | "turn_aborted") => open = Some(false),
            _ => {}
        }
        let want = open.map(|o| if o { Activity::Busy } else { Activity::Idle });
        busy_lines += usize::from(want == Some(Activity::Busy));
        assert_eq!(newest_activity(&p), want, "line {}", n + 1);
    }
    assert!(busy_lines > 50, "the check saw turns run: {busy_lines}");
}

#[test]
fn a_rollout_read_twice_adds_nothing() {
    let mut p = fold(MAIN);
    let (rows, rev) = (p.rows().len(), p.revision());
    for line in MAIN.lines() {
        p.fold_codex_line(line.as_bytes());
    }
    assert_eq!((p.rows().len(), p.revision()), (rows, rev));
}

/// A rollout line and a hook, ordered as they reached the daemon: by the
/// rollout's own timestamp and the hook logger's clock.
fn merged<'a>(rollout: &'a str, hooks: &'a str) -> Vec<(i64, Result<&'a str, Value>)> {
    let mut all: Vec<(i64, Result<&str, Value>)> = Vec::new();
    for line in rollout.lines() {
        let record: Value = serde_json::from_str(line).unwrap();
        let at = record["timestamp"].as_str().and_then(parse_iso8601_millis).unwrap();
        all.push((at, Ok(line)));
    }
    for line in hooks.lines() {
        let hook: Value = serde_json::from_str(line).unwrap();
        all.push((hook["at_ms"].as_i64().unwrap(), Err(hook)));
    }
    all.sort_by_key(|(at, _)| *at);
    all
}

/// Hooks put rows up first under the rollout's own ids, and the rollout
/// confirms each in place: one turn per prompt, one row per call, and the
/// held permission tied to its call.
#[test]
fn hooks_go_first_and_the_rollout_confirms_them() {
    let mut p = Projection::for_session("01a117d6-f550-7013-a939-b02d182c0321");
    let mut seen = Vec::new();
    for (at, event) in merged(NEW, HOOKS_APPROVE_INTERRUPT) {
        match event {
            Ok(line) => p.fold_codex_line(line.as_bytes()),
            Err(hook) => {
                let name = hook["argv"].as_str().unwrap();
                assert_eq!(p.codex_hook(name, &hook["payload"], at), HookEffect::None);
                seen.push((name.to_string(), newest_activity(&p)));
            }
        }
    }
    let activity_after = |event: &str| seen.iter().find(|(e, _)| e == event).and_then(|(_, a)| *a);
    assert_eq!(activity_after("UserPromptSubmit"), Some(Activity::Busy));
    assert_eq!(activity_after("PermissionRequest"), Some(Activity::Waiting), "a held approval is waiting");
    assert_eq!(activity_after("PostToolUse"), Some(Activity::Busy), "answered, the turn runs on");
    assert_eq!(activity_after("Interrupt"), Some(Activity::Idle));
    assert_eq!(
        shown(&p),
        [
            r#"turn "hello again" Some(Finished)"#,
            r#"  prose "done: hello again" (conclusion)"#,
            r#"turn "RUNX touch third.txt" Some(Finished)"#,
            r#"  tool Bash "touch third.txt" Done"#,
            r#"  ask Permission "Bash touch third.txt" answered=true"#,
            r#"  prose "done: RUNX touch third.txt" (conclusion)"#,
            r#"turn "SLOW 3" Some(Interrupted)"#,
        ]
    );
    assert!(p.rows().iter().all(|r| !r.provisional), "{:#?}", p.rows().iter().filter(|r| r.provisional).collect::<Vec<_>>());
}

/// Before the rollout has a word of it, a prompt's hook is a turn already,
/// under the id the rollout will use.
#[test]
fn a_prompt_hook_alone_is_a_running_turn() {
    let mut p = Projection::new();
    let hook: Value = serde_json::from_str(HOOKS_APPROVE_INTERRUPT.lines().next().unwrap()).unwrap();
    p.codex_hook("UserPromptSubmit", &hook["payload"], 1);
    let turn = &p.rows()[0];
    let id = format!("turn:{}", hook["payload"]["turn_id"].as_str().unwrap());
    assert_eq!((turn.id.as_str(), turn.provisional), (id.as_str(), true));
    assert!(matches!(&turn.kind, RowKind::Turn(t) if t.prompt == "RUNX touch third.txt" && t.origin == TurnOrigin::Typed));
    assert_eq!(newest_activity(&p), Some(Activity::Busy));
}

/// `/new` starts a thread with a rollout of its own; its `SessionStart`
/// moves the projection there.
#[test]
fn a_new_thread_s_session_start_rebinds() {
    let mut p = Projection::for_session("01a117d0-fd5c-7fe3-8f0f-5f2afdf7906d");
    let start = HOOKS_TWO_FAIL_NEW.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()).find(|h| h["argv"] == "SessionStart").unwrap();
    match p.codex_hook("SessionStart", &start["payload"], 1) {
        HookEffect::Rebind { session_id, transcript_path, .. } => {
            assert_eq!(session_id, "01a117d6-f550-7013-a939-b02d182c0321");
            assert!(transcript_path.is_some_and(|t| t.ends_with("rollout-2026-10-07T12-28-45-01a117d6-f550-7013-a939-b02d182c0321.jsonl")));
        }
        other => panic!("{other:?}"),
    }
}

/// codex before 0.147: the prompt in `event_msg/user_message`, the reply in
/// both `agent_message` and the history, shown once.
#[test]
fn the_older_shape_folds_too() {
    let p = fold(OLD_SHAPE);
    assert_eq!(shown(&p), [r#"turn "say hi" Some(Finished)"#, r#"  prose "Hi." (conclusion)"#]);
}

/// codex 0.147's items, cut down to them alone: a file change is an edit of
/// its file, a command a `Bash` call, two replies under one scrubbed id two
/// rows.
#[test]
fn the_item_shape_folds_too() {
    let p = fold(ITEM_SHAPE);
    let rows = shown(&p);
    assert!(rows[0].starts_with(r#"turn "Create a file called fruit.txt"#), "{rows:#?}");
    assert!(rows.iter().any(|r| r == r#"  tool Edit "/Users/example/project/fruit.txt" Done"#), "{rows:#?}");
    assert!(rows.iter().any(|r| r.starts_with(r#"  tool Bash "rg --fixed-strings"#)), "{rows:#?}");
    assert!(rows.iter().any(|r| r == r#"  prose "Created `fruit.txt` containing `banana`." (conclusion)"#), "{rows:#?}");
    assert!(rows.iter().any(|r| r.starts_with("  prose \"I\u{2019}ll create")), "{rows:#?}");
    assert!(p.rows().iter().any(|r| matches!(r.kind, RowKind::Thinking(_))));
}

/// A line codex hasn't finished writing is a gap, not a row.
#[test]
fn an_unreadable_line_is_a_gap() {
    let mut p = Projection::new();
    p.fold_codex_line(br#"{"timestamp":"2026-10-07T19:22:30.251Z","type":"event_msg","payload":{"type":"task_sta"#);
    p.fold_codex_line(br#"{"timestamp":"2026-10-07T19:22:30.251Z","type":"something_new","payload":{}}"#);
    assert_eq!(p.stats().gaps, 2);
}

/// A session projector picks the fold by the file's name, and names its
/// session by the thread id the name ends in.
#[test]
fn a_projector_on_a_rollout_reads_it_as_codex() {
    let dir = Scratch::new("codex-rollout");
    let path = dir.path().join(DIR).join(MAIN_NAME);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, MAIN).unwrap();
    assert!(is_codex_rollout(&path));
    assert!(!is_codex_rollout(&dir.path().join("01a117d0-fd5c-7fe3-8f0f-5f2afdf7906d.jsonl")));
    let mut session = SessionProjector::open(path);
    assert!(session.is_codex());
    session.poll();
    assert_eq!(shown(session.projection()), shown(&fold(MAIN)));
    // A hook naming another thread is that thread's.
    let start = serde_json::json!({ "session_id": "01a117d0-fd5c-7fe3-8f0f-5f2afdf7906d", "source": "startup" });
    assert_eq!(session.hook("SessionStart", &start, 1), HookEffect::None, "this thread: no move");
}

/// Compaction and a failed request, in the shapes the corpus holds
/// (`context_compacted` from codex 0.14x, `error` as codex names it).
#[test]
fn compaction_and_a_failed_request_are_notices() {
    let p = fold(concat!(
        r#"{"timestamp":"2026-08-24T07:36:50.000Z","type":"event_msg","payload":{"type":"task_started","turn_id":"t1"}}"#, "\n",
        r#"{"timestamp":"2026-08-24T07:36:57.517Z","type":"compacted","payload":{"message":"","replacement_history":[]}}"#, "\n",
        r#"{"timestamp":"2026-08-24T07:36:57.518Z","type":"event_msg","payload":{"type":"context_compacted"}}"#, "\n",
        r#"{"timestamp":"2026-08-24T07:36:58.000Z","type":"event_msg","payload":{"type":"error","message":"stream disconnected before completion"}}"#, "\n",
    ));
    let notices: Vec<(NoticeKind, &str)> = p.rows().iter().filter_map(|r| match &r.kind {
        RowKind::Notice(n) => Some((n.kind, n.text.as_str())),
        _ => None,
    }).collect();
    assert_eq!(notices, [(NoticeKind::Compacted, "Context compacted"), (NoticeKind::ApiError, "stream disconnected before completion")]);
}

/// What a real codex rollout costs to rebuild, read from wherever
/// `FARCOOLER_CODEX_ROLLOUT` names (never copied here: it's the owner's).
/// `cargo test --release -p farcooler-core --lib codex_rebuild -- --ignored --nocapture`
#[test]
#[ignore]
fn codex_rebuild_cost() {
    let Some(path) = std::env::var_os("FARCOOLER_CODEX_ROLLOUT") else { return };
    let path = std::path::PathBuf::from(path);
    let started = std::time::Instant::now();
    let scanned = crate::session_log::codex_turn::last_turn(&path);
    let scan_took = started.elapsed();
    let started = std::time::Instant::now();
    let mut session = SessionProjector::open(path);
    let lines = session.poll();
    let p = session.projection();
    let gaps: Vec<String> = p.rows().iter().filter_map(|r| match &r.kind {
        RowKind::Gap(g) => Some(format!("{:?}x{}", g.reason, g.count)),
        _ => None,
    }).collect();
    println!("{lines} lines, {} rows, {} gaps {gaps:?}, {:?}", p.rows().len(), p.stats().gaps, started.elapsed());
    // The scan and the fold agree on whether the last turn is open.
    let open = p.rows().iter().rev().find_map(|r| match &r.kind {
        RowKind::Turn(t) => Some(t.outcome.is_none()),
        _ => None,
    });
    println!("scan {scanned:?} in {scan_took:?}, fold open {open:?}");
}
