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

/// ov-743: the runner's note names whose answer it was and whose ask, and
/// the row keeps both once the record is in (here its turn settles the row;
/// `the_records_confirmation_keeps_who_answered` covers the confirmation);
/// an ending nobody answered names nobody, whatever its note says.
#[test]
fn a_held_ask_says_who_answered_it_and_for_whom() {
    let mut p = Projection::new();
    for (event, payload) in hooks_until(QUESTION_HOOKS, "PermissionRequest", "hook-ask-w") {
        p.hook(&event, &payload, 1);
    }
    let alice = json!({ "handle": "alice", "name": "Alice Chen" });
    let note = json!({ "ask_id": "hook-ask-w", "by": "iPhone", "person": alice, "for": { "handle": "", "name": "" } });
    p.hook(ASK_ENDED, &note, 9);
    assert!(the_ask(&p).0.provisional, "the hook's row, before its record");
    fold_all(&mut p, QUESTION);
    let (_, ask) = the_ask(&p);
    let person = |h: &str, n: &str| Some(RowPerson { handle: h.into(), name: n.into() });
    assert_eq!(ask.answered_by.as_deref(), Some("iPhone"));
    assert_eq!((ask.answered_person.clone(), ask.answered_for.clone()), (person("alice", "Alice Chen"), person("", "")), "Alice, for the owner");
    let wire = serde_json::to_value(ask).unwrap();
    assert_eq!(wire["answered_person"]["name"], "Alice Chen", "{wire}");

    let mut p = Projection::new();
    let request = json!({ "tool_name": "Bash", "tool_input": { "command": "ls" }, HELD_ASK: "hook-ask-k" });
    p.hook("PermissionRequest", &request, 1);
    p.hook(ASK_ENDED, &json!({ "ask_id": "hook-ask-k", "by": null, "person": alice }), 2);
    let (_, ask) = the_ask(&p);
    assert_eq!((ask.answered_by.as_deref(), ask.answered_person.as_ref()), (None, None), "the keyboard's answer is nobody's here");
    let wire = serde_json::to_value(ask).unwrap();
    assert!(wire.get("answered_person").is_none() && wire.get("answered_for").is_none(), "absent, as before: {wire}");
}

/// A plan left unanswered (claude killed mid-dialog, then resumed in the same
/// pane) is still an open `ExitPlanMode` row. A new plan's request that comes
/// before its own row never lands on the old one, whose plan the view would
/// show beside an Approve that approves the new one (review 1 M2).
#[test]
fn a_hold_lands_only_on_the_row_that_asks_what_it_holds() {
    let mut p = Projection::new();
    let stale = json!({"type":"assistant","uuid":"old","message":{"content":[{"type":"tool_use","id":"toolu_old","name":"ExitPlanMode","input":{"plan":"# Old plan"}}]}});
    p.fold_line(stale.to_string().as_bytes());
    let (_, request) = hooks_until(PLAN_HOOKS, "PermissionRequest", "hook-ask-new").pop().unwrap();
    p.hook("PermissionRequest", &request, 1);
    let held_on = |p: &Projection, id: &str| match &p.row(id).unwrap().kind {
        RowKind::Ask(a) => a.held.clone(),
        _ => None,
    };
    assert_eq!(held_on(&p, "ask:toolu_old"), None, "the old plan's row");
    // A row of another plan put up while the hold waits isn't its row either.
    let other = json!({"type":"assistant","uuid":"other","message":{"content":[{"type":"tool_use","id":"toolu_other","name":"ExitPlanMode","input":{"plan":"# Another plan"}}]}});
    p.fold_line(other.to_string().as_bytes());
    assert_eq!(held_on(&p, "ask:toolu_other"), None, "a row asking something else");
    fold_all(&mut p, PLAN);
    let new = p.rows().iter().find(|r| r.id.starts_with("ask:") && r.id != "ask:toolu_old" && r.id != "ask:toolu_other").expect("the new plan's row").id.clone();
    assert_eq!(held_on(&p, &new).as_deref(), Some("hook-ask-new"), "its own row, once it's up");
    assert_eq!(held_on(&p, "ask:toolu_old"), None);
}

/// Review finding 7 (ov-747): when the record confirms the hook's row (its
/// prompt line first, as claude writes it at most turns, so the hook's turn
/// is the record's), the row keeps the device, the person and whose ask it
/// was. The fixture's own order (the ask before its prompt) settles the
/// hook's turn first, and never reaches the confirmation's carry.
#[test]
fn the_records_confirmation_keeps_who_answered() {
    let mut p = Projection::new();
    for (event, payload) in hooks_until(QUESTION_HOOKS, "PermissionRequest", "hook-ask-c") {
        p.hook(&event, &payload, 1);
    }
    let alice = json!({ "handle": "alice", "name": "Alice Chen" });
    let bob = json!({ "handle": "bob", "name": "Bob Okafor" });
    p.hook(ASK_ENDED, &json!({ "ask_id": "hook-ask-c", "by": "iPhone", "person": alice, "for": bob }), 9);
    let lines: Vec<&str> = QUESTION.lines().collect();
    p.fold_line(lines[1].as_bytes()); // the prompt
    assert!(the_ask(&p).0.provisional, "the hook's turn is the record's, and its ask still the hook's");
    p.fold_line(lines[0].as_bytes()); // the ask
    let (row, ask) = the_ask(&p);
    assert!(!row.provisional, "the record confirms it");
    let person = |h: &str, n: &str| Some(RowPerson { handle: h.into(), name: n.into() });
    assert_eq!(ask.answered_by.as_deref(), Some("iPhone"), "the device");
    assert_eq!(ask.answered_person, person("alice", "Alice Chen"), "who answered");
    assert_eq!(ask.answered_for, person("bob", "Bob Okafor"), "whose ask it was");
    for line in &lines[2..] {
        p.fold_line(line.as_bytes());
    }
    let (_, ask) = the_ask(&p);
    assert_eq!((ask.answered_person.clone(), ask.answered_for.clone()), (person("alice", "Alice Chen"), person("bob", "Bob Okafor")), "and after its turn ends");
}
