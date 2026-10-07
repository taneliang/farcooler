//! A question's and a plan's `PermissionRequest`, over the real socket
//! (ov-370). Each is held, offered to no surface that answers with Allow and
//! Deny, and answered only in its own terms: a question with its answers, a
//! plan with Approve or Keep Planning. The first answer wins; a second, and
//! one after the hold ended, find nothing held.

use std::collections::HashMap;

use farcooler_daemon::hook_asks::AnswerRefused;

use super::*;

/// A wait for something that will happen, cut short when it does: long, for
/// a loaded CI runner.
const PATIENCE: Duration = Duration::from_secs(30);

fn a_dialog_request(tool: &str, input: serde_json::Value) -> HookLine {
    let mut line = a_permission_request(Agent::Claude, "sess-1");
    line.payload["tool_name"] = serde_json::json!(tool);
    line.payload["tool_input"] = input;
    line
}

fn question_input() -> serde_json::Value {
    serde_json::json!({ "questions": [{
        "question": "Which color should the button be?",
        "header": "Color",
        "options": [{ "label": "Red", "description": "Warm and loud" }, { "label": "Blue", "description": "Calm and quiet" }],
        "multiSelect": false,
    }] })
}

fn plan_input() -> serde_json::Value {
    serde_json::json!({ "plan": "# Plan\n\n1. Make the button blue.", "planFilePath": "/tmp/plans/p.md" })
}

/// A claude pane whose hook is holding `line`, and its held id.
struct Dialog {
    ingress: HookIngress,
    terminal: Uuid,
    seen: Seen,
    asking: Asking,
    id: String,
    _dir: tempfile::TempDir,
}

async fn a_held_dialog(worktree: &str, line: &HookLine, hold: Option<Duration>) -> Dialog {
    let dir = tempfile::tempdir().unwrap();
    let (store, terminal) = store_with_terminal(worktree, "claude", Some("sess-1"));
    let mut ingress = ingress_claiming(store, &[terminal]);
    if let Some(hold) = hold {
        ingress = ingress.with_hold(hold);
    }
    let (socket, seen) = listening_on(ingress.clone(), dir.path()).await;
    let mut asking = Asking::open(&socket, line).await;
    let first = asking.line(PATIENCE).await.expect("the daemon answers at once");
    assert!(first.contains("hold_ms"), "held, not left to the keyboard at once: {first:?}");
    let mut id = None;
    for _ in 0..1200 {
        id = ingress.asks().held_on(terminal);
        if id.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let id = id.expect("the ledger holds it");
    Dialog { ingress, terminal, seen, asking, id, _dir: dir }
}

fn answers(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs.iter().map(|(q, a)| (q.to_string(), a.to_string())).collect()
}

async fn verdict(asking: &mut Asking) -> serde_json::Value {
    let line = asking.line(PATIENCE).await.expect("the verdict follows the hold");
    serde_json::from_str(line.trim()).expect("json")
}

/// A question is answered with its answers, in the input claude takes; a
/// phone's Allow can't answer it, and a second answer finds it gone.
#[tokio::test]
async fn a_held_question_is_answered_once_with_its_answers() {
    let line = a_dialog_request("AskUserQuestion", question_input());
    let Dialog { ingress, terminal, seen, mut asking, id, .. } = a_held_dialog("/wt/question", &line, None).await;
    assert!(!any_permission(&seen), "no Allow and Deny for a question, on any surface");

    let asks = ingress.asks();
    let none = HashMap::new();
    assert_eq!(asks.answer_with(terminal, &id, "allow", &none, "iPhone").await, Err(AnswerRefused::UnknownOption));
    let given = answers(&[("Which color should the button be?", "Blue")]);
    assert_eq!(asks.answer_with(terminal, &id, "answer", &given, "Mac").await, Ok(()), "the first answer wins");
    let mut expected = question_input();
    expected["answers"] = serde_json::json!({ "Which color should the button be?": "Blue" });
    assert_eq!(
        verdict(&mut asking).await,
        serde_json::json!({ "decision": { "behavior": "allow", "updatedInput": expected } })
    );

    let late = answers(&[("Which color should the button be?", "Red")]);
    assert_eq!(asks.answer_with(terminal, &id, "answer", &late, "iPhone").await, Err(AnswerRefused::NotHeld), "a late second answer");
    assert!(!asks.is_holding(terminal));
}

/// A plan is approved with claude's own input, once.
#[tokio::test]
async fn a_held_plan_is_approved_once() {
    let line = a_dialog_request("ExitPlanMode", plan_input());
    let Dialog { ingress, terminal, seen, mut asking, id, .. } = a_held_dialog("/wt/plan", &line, None).await;
    assert!(!any_permission(&seen), "no Allow and Deny for a plan on a lock screen");

    let asks = ingress.asks();
    assert_eq!(asks.answer(terminal, &id, "allow", "Mac").await, Ok(()));
    assert_eq!(
        verdict(&mut asking).await,
        serde_json::json!({ "decision": { "behavior": "allow", "updatedInput": plan_input() } })
    );
    assert_eq!(asks.answer(terminal, &id, "deny", "iPhone").await, Err(AnswerRefused::NotHeld), "a late second answer");
}

/// Keep Planning denies the exit, naming the device.
#[tokio::test]
async fn a_held_plan_kept_in_planning_names_the_device() {
    let line = a_dialog_request("ExitPlanMode", plan_input());
    let Dialog { ingress, terminal, mut asking, id, .. } = a_held_dialog("/wt/keep-planning", &line, None).await;
    assert_eq!(ingress.asks().answer(terminal, &id, "deny", "iPhone").await, Ok(()));
    let said = verdict(&mut asking).await;
    assert_eq!(said["decision"]["behavior"], "deny");
    let message = said["decision"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("Keep planning") && message.contains("iPhone"), "{message}");
}

/// Once the hold ends the dialog is the keyboard's, and an answer to it is
/// refused rather than written to a hook nobody reads.
#[tokio::test]
async fn an_answer_after_the_hold_ended_is_refused() {
    let line = a_dialog_request("AskUserQuestion", question_input());
    let Dialog { ingress, terminal, mut asking, id, .. } =
        a_held_dialog("/wt/stale", &line, Some(Duration::from_millis(300))).await;
    assert_eq!(asking.line(PATIENCE).await.as_deref(), Some("{}\n"), "no decision when the hold ends");
    let given = answers(&[("Which color should the button be?", "Blue")]);
    assert_eq!(
        ingress.asks().answer_with(terminal, &id, "answer", &given, "Mac").await,
        Err(AnswerRefused::NotHeld),
        "a stale ask"
    );
}
