//! What the runner holds for a pane reaches a projector opened after it
//! (ov-830 review, B1 and S1): a projector opens only while somebody views
//! its pane, and a rebuild from the transcript alone knows neither.

use farcooler_core::session_log::projector::held::HELD_ASK;
use farcooler_core::session_log::projector::{QueuedState, RowKind};
use serde_json::json;
use uuid::Uuid;

use crate::session_projectors::SessionProjectors;

fn transcript(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let path = dir.path().join("s.jsonl");
    std::fs::write(&path, "").unwrap();
    path
}

/// claude's own words: a prompt and the `AskUserQuestion` it answered
/// with, not yet answered (`held-question.jsonl`, recorded, ov-370).
const ASKED: &str = include_str!("../../core/src/session_log/projector/fixtures/held-question.jsonl");
const HOOKS: &str = include_str!("../../core/src/session_log/projector/fixtures/held-question-hooks.jsonl");

/// The transcript as it stood while the question waited: the prompt, then
/// the question (the recording has them the other way round, written a few
/// ms apart; the answer and the reply after them are left out).
fn asking(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let path = dir.path().join("asked.jsonl");
    let lines: Vec<&str> = ASKED.lines().collect();
    std::fs::write(&path, format!("{}\n{}\n", lines[1], lines[0])).unwrap();
    path
}

/// The question's `PermissionRequest`, as the runner folds it: its hold's id
/// added, and naming `transcript` as its own.
fn question(id: &str, transcript: &std::path::Path) -> serde_json::Value {
    let line = HOOKS.lines().find(|l| l.contains("\"PermissionRequest\"")).expect("the request");
    let mut payload = serde_json::from_str::<serde_json::Value>(line).unwrap()["payload"].clone();
    payload["transcript_path"] = json!(transcript);
    payload[HELD_ASK] = json!(id);
    payload
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

fn not_sent(projectors: &SessionProjectors, terminal: Uuid) -> usize {
    projectors.page(terminal, None, 1000).unwrap().into_iter().filter(|r| matches!(r.kind, RowKind::Notice(_))).count()
}

fn held_ask(projectors: &SessionProjectors, terminal: Uuid) -> Option<String> {
    let rows = projectors.page(terminal, None, 1000)?;
    rows.into_iter().find_map(|r| match r.kind {
        RowKind::Ask(a) => a.held,
        _ => None,
    })
}

/// A question claude asked while nobody viewed the pane, answered from its
/// row once a view opens. Goes red with `replay` gone from `finish`.
#[test]
fn an_ask_held_while_nobody_views_shows_answerable_when_the_view_opens() {
    let dir = tempfile::tempdir().unwrap();
    let path = asking(&dir);
    let projectors = SessionProjectors::default();
    let terminal = Uuid::now_v7();
    projectors.hook(terminal, "PermissionRequest", &question("hook-ask-1", &path));
    assert!(!projectors.is_open(terminal), "nobody views it");
    projectors.open(terminal, path.clone());
    assert_eq!(held_ask(&projectors, terminal).as_deref(), Some("hook-ask-1"), "the view's row can't answer it");

    // Through the drop and back: still held, still answerable.
    projectors.forget(terminal);
    projectors.open(terminal, path);
    assert_eq!(held_ask(&projectors, terminal).as_deref(), Some("hook-ask-1"));
}

/// An ask that ended while nobody viewed is not replayed, however it ended;
/// and a request that reaches the projectors after its quick answer isn't
/// kept.
#[test]
fn an_ask_that_ended_is_not_replayed() {
    let dir = tempfile::tempdir().unwrap();
    let path = asking(&dir);
    let projectors = SessionProjectors::default();
    let ended = Uuid::now_v7();
    projectors.hook(ended, "PermissionRequest", &question("hook-ask-2", &path));
    projectors.ask_gone(ended, "hook-ask-2");
    projectors.open(ended, path.clone());
    assert_eq!(held_ask(&projectors, ended), None, "an ended hold came back");

    let late = Uuid::now_v7();
    projectors.ask_gone(late, "hook-ask-3");
    projectors.hook(late, "PermissionRequest", &question("hook-ask-3", &path));
    projectors.open(late, path);
    assert_eq!(held_ask(&projectors, late), None, "a request folded after its answer was kept");
}

/// A message the runner holds is Queued in a projector opened after the
/// drop, and one given up on while nobody viewed says why. Goes red with
/// the journal gone (`runner_held` a no-op without a projector, as before).
#[test]
fn a_queued_row_survives_the_drop_and_a_give_up_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let projectors = SessionProjectors::default();
    let terminal = Uuid::now_v7();
    projectors.open(terminal, transcript(&dir));
    projectors.runner_held(terminal, "m1", "Run the tests", now() - 2_000);
    projectors.forget(terminal);
    projectors.open(terminal, transcript(&dir));
    let queued = |p: &SessionProjectors| {
        p.page(terminal, None, 1000).unwrap().into_iter().find_map(|r| match r.kind {
            RowKind::Queued(q) if r.id == "held:m1" && !r.retracted => Some(q.state),
            _ => None,
        })
    };
    assert_eq!(queued(&projectors), Some(QueuedState::Waiting), "the Queued row went with the projector");

    // Given up on while nobody views.
    projectors.forget(terminal);
    projectors.runner_dropped(terminal, "m1", "Claude didn't take “Run the tests”.", now() - 1_000);
    projectors.open(terminal, transcript(&dir));
    assert_eq!(queued(&projectors), Some(QueuedState::Withdrawn));
    let notices = projectors.page(terminal, None, 1000).unwrap().into_iter().filter(|r| matches!(r.kind, RowKind::Notice(_))).count();
    assert_eq!(notices, 1, "the reason it wasn't sent is said once");

    // Typed: claude's own record shows it from then on.
    let told = Uuid::now_v7();
    projectors.runner_held(told, "m2", "Ship it", 3_000);
    projectors.runner_told(told, "m2");
    projectors.open(told, transcript(&dir));
    assert!(projectors.page(told, None, 1000).unwrap().iter().all(|r| r.id != "held:m2"));
}


/// A give-up is said in the turn it happened in, not under a later one
/// (ov-830 re-review, fix 1): the pane's next prompt lets it go, and so does
/// age, for a pane with no hooks to say its prompts. Goes red with given-up
/// messages kept until the pane is gone.
#[test]
fn a_give_up_isnt_said_again_after_a_later_turn() {
    let dir = tempfile::tempdir().unwrap();
    let projectors = SessionProjectors::default();
    let terminal = Uuid::now_v7();
    projectors.runner_held(terminal, "m1", "Run the tests", now() - 2_000);
    projectors.runner_dropped(terminal, "m1", "Claude didn't take “Run the tests”.", now() - 1_000);
    // The person moves on: a later turn.
    projectors.hook(terminal, "UserPromptSubmit", &json!({ "prompt": "Now the docs", "prompt_id": "p2" }));
    let later = dir.path().join("later.jsonl");
    std::fs::write(&later, "{\"type\":\"user\",\"promptId\":\"p2\",\"promptSource\":\"typed\",\"uuid\":\"u2\",\"message\":{\"content\":\"Now the docs\"}}\n").unwrap();
    projectors.open(terminal, later.clone());
    assert_eq!(not_sent(&projectors, terminal), 0, "\"not sent\" under a later turn");

    // No hooks: an hour on, it's gone too.
    let old = Uuid::now_v7();
    projectors.runner_dropped(old, "m2", "Claude didn't take “Ship it”.", now() - 60 * 60 * 1000);
    projectors.open(old, later);
    assert_eq!(not_sent(&projectors, old), 0, "an hour-old give-up came back");
}

/// An ask answered through `HookAsks` is not replayed into a view opened
/// while the answer waits on the hook's ack, before the row's own note
/// (ov-830 re-review, M3). Goes red with `ask_gone` taken out of
/// `HookAsks::settle_by`.
#[tokio::test]
async fn an_ask_answered_through_the_runner_isnt_replayed() {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    let dir = tempfile::tempdir().unwrap();
    let path = asking(&dir);
    let projectors = crate::session_projectors::global();
    let asks = Arc::new(crate::hook_asks::HookAsks::new(Arc::new(Mutex::new(None))));
    let pane = Uuid::now_v7();
    let request = question("pending", &path);
    let shape = crate::hook_asks::AskShape::of(Some("AskUserQuestion"), &request["tool_input"]);
    let (id, rx) = asks.hold_shaped(pane, Some("AskUserQuestion"), shape, farcooler_agent_hooks::wire::LONGEST_HOLD);
    projectors.hook(pane, "PermissionRequest", &question(&id, &path));

    // The hook takes the answer and holds its ack until the view has looked.
    let (looked, release) = tokio::sync::oneshot::channel::<()>();
    let (took, taken) = tokio::sync::oneshot::channel::<()>();
    let hook = tokio::spawn(async move {
        let settled = rx.await.expect("an ending");
        let _ = took.send(());
        let _ = release.await;
        let _ = settled.ack.expect("a device's answer waits on its ack").send(());
    });
    let answers = HashMap::from([("Which color should the button be?".to_string(), "Blue".to_string())]);
    let answer = {
        let (asks, id) = (asks.clone(), id.clone());
        tokio::spawn(async move { asks.answer_with(pane, &id, "allow", &answers, "Mac").await })
    };
    taken.await.expect("the hook took it");
    projectors.open(pane, path);
    assert_eq!(held_ask(projectors, pane), None, "an answered ask came back held");
    let _ = looked.send(());
    hook.await.unwrap();
    assert_eq!(answer.await.unwrap(), Ok(()));
    projectors.forget_terminal(pane);
}
