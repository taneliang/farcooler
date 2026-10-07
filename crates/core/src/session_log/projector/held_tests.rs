//! A held ask's id on the row a view answers it from (ov-370), over the
//! hooks and transcripts real claude 2.1.290 wrote for a question and a plan
//! a hook answered (the ov-370 spike).

use serde_json::{json, Value};

use super::held::{ASK_ENDED, HELD_ASK};
use super::rows::*;
use super::Projection;

const QUESTION_HOOKS: &str = include_str!("fixtures/held-question-hooks.jsonl");
const QUESTION: &str = include_str!("fixtures/held-question.jsonl");
const PLAN_HOOKS: &str = include_str!("fixtures/held-plan-hooks.jsonl");
const PLAN: &str = include_str!("fixtures/held-plan.jsonl");

/// The recorded hooks, in order, up to and including `last`, the held
/// request carrying `held` as `hook_ingress` adds it.
fn hooks_until(recorded: &str, last: &str, held: &str) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for line in recorded.lines() {
        let hook: Value = serde_json::from_str(line).unwrap();
        let event = hook["event"].as_str().unwrap().to_string();
        let mut payload = hook["payload"].clone();
        if event == "PermissionRequest" {
            payload[HELD_ASK] = json!(held);
        }
        let done = event == last;
        out.push((event, payload));
        if done {
            break;
        }
    }
    out
}

fn the_ask(p: &Projection) -> (&Row, &Ask) {
    let asks: Vec<(&Row, &Ask)> =
        p.rows().iter().filter_map(|r| match &r.kind { RowKind::Ask(a) => Some((r, a)), _ => None }).collect();
    assert_eq!(asks.len(), 1, "one row for the dialog, never a second for its request: {asks:?}");
    asks[0]
}

fn fold_all(p: &mut Projection, transcript: &str) {
    for line in transcript.lines() {
        p.fold_line(line.as_bytes());
    }
}

fn ended(p: &mut Projection, id: &str, by: Option<&str>) {
    p.hook(ASK_ENDED, &json!({ "ask_id": id, "by": by }), 9);
}

/// A question's request puts its hold on the question's own row, with every
/// option, and its end takes it off again, naming the device that answered.
#[test]
fn a_held_question_is_answerable_from_its_row_until_its_hold_ends() {
    let mut p = Projection::new();
    for (event, payload) in hooks_until(QUESTION_HOOKS, "PermissionRequest", "hook-ask-q") {
        p.hook(&event, &payload, 1);
    }
    let (row, ask) = the_ask(&p);
    assert!(row.id.starts_with("ask:toolu_"), "{}", row.id);
    assert_eq!(ask.kind, AskKind::Question);
    assert_eq!(ask.held.as_deref(), Some("hook-ask-q"));
    assert_eq!(ask.questions.len(), 1, "read from the hook, before the record is in");
    let q = &ask.questions[0];
    assert_eq!((q.question.as_str(), q.header.as_str(), q.multi_select), ("Which color should the button be?", "Color", false));
    let labels: Vec<&str> = q.options.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(labels, ["Red", "Blue"]);
    assert_eq!(q.options[1].description, "Calm and quiet");

    fold_all(&mut p, QUESTION);
    let (row, ask) = the_ask(&p);
    assert!(!row.provisional, "the record confirms it");
    assert_eq!(ask.held.as_deref(), Some("hook-ask-q"), "the record doesn't take the hold off");

    ended(&mut p, "hook-ask-q", Some("iPhone"));
    let (_, ask) = the_ask(&p);
    assert_eq!(ask.held, None, "nothing to answer once the hold is over");
    assert_eq!(ask.answered_by.as_deref(), Some("iPhone"));
    assert!(ask.answered, "the hook's answer is in the record");
}

/// A plan's request lands on its row, which carries the whole plan.
#[test]
fn a_held_plan_carries_the_whole_plan_and_its_hold() {
    let mut p = Projection::new();
    for (event, payload) in hooks_until(PLAN_HOOKS, "PermissionRequest", "hook-ask-p") {
        p.hook(&event, &payload, 1);
    }
    let (_, ask) = the_ask(&p);
    assert_eq!(ask.kind, AskKind::PlanExit);
    assert_eq!(ask.held.as_deref(), Some("hook-ask-p"));
    assert_eq!(ask.plan.as_deref(), Some("# Plan\n\n1. Make the button blue.\n2. Ship it."), "line breaks kept");
    fold_all(&mut p, PLAN);
    let (row, ask) = the_ask(&p);
    assert!(!row.provisional);
    assert_eq!(ask.held.as_deref(), Some("hook-ask-p"));
    assert_eq!(ask.plan.as_deref(), Some("# Plan\n\n1. Make the button blue.\n2. Ship it."), "the record's plan");
    ended(&mut p, "hook-ask-p", None);
    let (_, ask) = the_ask(&p);
    assert_eq!((ask.held.as_deref(), ask.answered_by.as_deref()), (None, None), "the keyboard, or the clock");
}

/// The request is folded on another task than the hold's end, so it can
/// come second: a hold that has ended never goes back up.
#[test]
fn a_request_folded_after_its_hold_ended_offers_nothing() {
    let mut p = Projection::new();
    let hooks = hooks_until(QUESTION_HOOKS, "PermissionRequest", "hook-ask-late");
    let (request, before) = hooks.split_last().unwrap();
    for (event, payload) in before {
        p.hook(event, payload, 1);
    }
    ended(&mut p, "hook-ask-late", None);
    p.hook(&request.0, &request.1, 2);
    assert_eq!(the_ask(&p).1.held, None);
}

/// With no `PreToolUse` registered, the request comes before the record that
/// puts the row up, and waits for it.
#[test]
fn a_request_before_its_row_lands_when_the_row_does() {
    let mut p = Projection::new();
    let (_, request) = hooks_until(QUESTION_HOOKS, "PermissionRequest", "hook-ask-early").pop().unwrap();
    p.hook("PermissionRequest", &request, 1);
    assert!(!p.rows().iter().any(|r| matches!(r.kind, RowKind::Ask(_))), "no row of its own");
    fold_all(&mut p, QUESTION);
    assert_eq!(the_ask(&p).1.held.as_deref(), Some("hook-ask-early"));
}

/// A permission's row carries its hold the same way.
#[test]
fn a_held_permission_row_carries_its_hold() {
    let mut p = Projection::new();
    let request = json!({
        "hook_event_name": "PermissionRequest",
        "tool_name": "Bash",
        "tool_input": { "command": "touch spike-made-this.txt", "description": "Make a file" },
        HELD_ASK: "hook-ask-b",
    });
    p.hook("PermissionRequest", &request, 1);
    let (_, ask) = the_ask(&p);
    assert_eq!((ask.kind, ask.held.as_deref()), (AskKind::Permission, Some("hook-ask-b")));
    ended(&mut p, "hook-ask-b", Some("Mac"));
    let (_, ask) = the_ask(&p);
    assert_eq!((ask.held.as_deref(), ask.answered_by.as_deref()), (None, Some("Mac")));
}
