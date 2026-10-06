//! Hook rows matched to the records that confirm them: prose by its words,
//! a held permission by the call it holds (ov-366, from ov-363's reviews).
//! Synthetic records in the shapes claude 2.1.290 writes; every word invented.

use serde_json::{json, Value};

use super::fixtures::*;
use super::rows::*;
use super::Projection;

fn line(p: &mut Projection, v: Value) {
    p.fold_line(v.to_string().as_bytes());
}

fn prompt(id: &str) -> Value {
    json!({"type":"user","promptId":id,"promptSource":"typed","timestamp":"2026-10-06T10:00:00Z","message":{"role":"user","content":"go"}})
}

fn said(uuid: &str, text: &str) -> Value {
    json!({"type":"assistant","uuid":uuid,"timestamp":"2026-10-06T10:00:01Z","message":{"content":[{"type":"text","text":text}]}})
}

fn flush(message: &str, index: u64, delta: &str) -> Value {
    json!({"prompt_id":"p1","turn_id":"t1","message_id":message,"index":index,"delta":delta,"final":false})
}

fn timed() -> Value {
    json!({"type":"system","subtype":"turn_duration","durationMs":5,"timestamp":"2026-10-06T10:00:09Z"})
}

fn call(uuid: &str, id: &str, command: &str) -> Value {
    json!({"type":"assistant","uuid":uuid,"timestamp":"2026-10-06T10:00:02Z","message":{"content":[{"type":"tool_use","id":id,"name":"Bash","input":{"command":command}}]}})
}

fn result(uuid: &str, id: &str) -> Value {
    json!({"type":"user","uuid":uuid,"timestamp":"2026-10-06T10:00:03Z","message":{"content":[{"type":"tool_result","tool_use_id":id,"content":"ok"}]}})
}

fn ask(command: &str) -> Value {
    json!({"prompt_id":"p1","tool_name":"Bash","tool_input":{"command":command}})
}

fn perms(p: &Projection) -> Vec<(&Row, &Ask)> {
    p.rows()
        .iter()
        .filter_map(|r| match &r.kind {
            RowKind::Ask(a) if a.kind == AskKind::Permission => Some((r, a)),
            _ => None,
        })
        .collect()
}

/// A first flush of a few words is matched to a written row only when that
/// row BEGINS with them. Matching anywhere inside bound "I'll" to an earlier
/// row, and the message then showed nothing until its own record.
#[test]
fn a_short_first_flush_is_not_bound_to_an_earlier_row_that_contains_it() {
    let mut p = Projection::new();
    line(&mut p, prompt("p1"));
    line(&mut p, said("a1", "Then I'll check the logs."));
    p.hook("MessageDisplay", &flush("m2", 0, "I'll\n"), 1);
    let shown = p.row("hprose:m2").expect("the message is up at once, provisional");
    assert!(shown.provisional);
    p.hook("MessageDisplay", &flush("m2", 1, "run the tests.\n"), 2);
    line(&mut p, said("a2", "I'll\nrun the tests."));
    let rows = prose(&p);
    assert_eq!(rows.len(), 2, "{rows:?}");
    let confirmed = p.row("hprose:m2").unwrap();
    assert!(!confirmed.provisional);
    assert!(matches!(&confirmed.kind, RowKind::Prose(t) if t.text == "I'll\nrun the tests."));
}

/// Two written rows that begin alike: each hook message claims its own, in
/// order, and adds no row.
#[test]
fn messages_that_begin_alike_each_claim_their_own_written_row() {
    let mut p = Projection::new();
    line(&mut p, prompt("p1"));
    line(&mut p, said("a1", "Done with step one."));
    line(&mut p, said("a2", "Done with step two."));
    p.hook("MessageDisplay", &flush("m1", 0, "Done\n"), 1);
    p.hook("MessageDisplay", &flush("m2", 0, "Done\n"), 2);
    assert_eq!(prose(&p).len(), 2, "both already written");
    assert!(p.rows().iter().all(|r| !r.provisional));
}

/// The display drew words the record does not begin with. When the turn
/// closes, the hook's row is paired with the written one and retracted,
/// rather than waiting as a provisional copy for good.
#[test]
fn a_hook_row_no_record_begins_with_is_retracted_when_the_turn_closes() {
    let mut p = Projection::new();
    line(&mut p, prompt("p1"));
    p.hook("MessageDisplay", &flush("m1", 0, "Fixed it.\nAll green.\n"), 1);
    line(&mut p, said("a1", "Fixed it. All tests green."));
    assert_eq!(prose(&p).len(), 2, "unmatched while the turn is open");
    line(&mut p, timed());
    let live: Vec<&Row> = p.rows().iter().filter(|r| !r.retracted && matches!(r.kind, RowKind::Prose(_))).collect();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].id, "prose:a1:0");
    assert!(p.rows().iter().all(|r| !r.provisional), "nothing waits for good");
    p.hook("MessageDisplay", &flush("m1", 1, "late\n"), 3);
    p.hook("MessageDisplay", &flush("m9", 0, "Words no record holds\n"), 4);
    assert_eq!(p.rows().iter().filter(|r| !r.retracted && matches!(r.kind, RowKind::Prose(_))).count(), 1, "a late flush adds nothing");
}

/// Shown and never written (a reply cut off): it stands, confirmed.
#[test]
fn a_hook_row_with_nothing_written_to_pair_stands_when_the_turn_closes() {
    let mut p = Projection::new();
    line(&mut p, prompt("p1"));
    p.hook("MessageDisplay", &flush("m1", 0, "Half a thought\n"), 1);
    line(&mut p, json!({"type":"user","timestamp":"2026-10-06T10:00:02Z","message":{"content":[{"type":"text","text":"[Request interrupted by user]"}]}}));
    let row = p.row("hprose:m1").unwrap();
    assert!(!row.provisional && !row.retracted);
}

/// Two held Bash calls come back out of order: each ask is answered by its
/// own call. Settling by tool name answered the first ask with the second
/// call's result.
#[test]
fn each_permission_is_answered_by_its_own_call() {
    let mut p = Projection::new();
    line(&mut p, prompt("p1"));
    line(&mut p, call("c1", "toolu_ls", "ls"));
    p.hook("PermissionRequest", &ask("ls"), 1);
    line(&mut p, call("c2", "toolu_rm", "rm scratch.txt"));
    p.hook("PermissionRequest", &ask("rm scratch.txt"), 2);
    line(&mut p, result("r2", "toolu_rm"));
    let asks = perms(&p);
    assert_eq!(asks.len(), 2);
    assert!(!asks[0].1.answered, "the ls ask still waits");
    assert!(asks[1].1.answered, "the rm ask is answered");
    assert!(asks.iter().all(|(r, _)| !r.provisional), "each is tied to a written call, so confirmed");
}

/// A call that held no permission answers nothing.
#[test]
fn a_call_that_was_never_held_answers_no_ask() {
    let mut p = Projection::new();
    line(&mut p, prompt("p1"));
    p.hook("PermissionRequest", &ask("rm scratch.txt"), 1);
    line(&mut p, call("c1", "toolu_ls", "ls"));
    line(&mut p, result("r1", "toolu_ls"));
    let asks = perms(&p);
    assert!(!asks[0].1.answered);
    assert!(asks[0].0.provisional, "no call of its own written yet");
    line(&mut p, call("c2", "toolu_rm", "rm scratch.txt"));
    assert!(!perms(&p)[0].0.provisional, "its call is written: tied and confirmed");
    line(&mut p, result("r2", "toolu_rm"));
    assert!(perms(&p)[0].1.answered);
}

/// A subagent's call asks: tied through the subagent's own transcript.
#[test]
fn a_subagents_permission_is_answered_by_its_call_in_its_own_file() {
    let mut p = fold(&BACKGROUND.lines().take(4).collect::<Vec<_>>().join("\n"));
    let mut held = ask("cat notes.md");
    held["agent_id"] = json!("abg1");
    p.hook("PermissionRequest", &held, 1);
    p.fold_subagent_line("abg1", None, call("s1", "toolu_cat", "cat notes.md").to_string().as_bytes());
    assert!(!perms(&p)[0].0.provisional);
    assert!(!perms(&p)[0].1.answered);
    p.fold_subagent_line("abg1", None, result("s2", "toolu_cat").to_string().as_bytes());
    assert!(perms(&p)[0].1.answered);
}

/// An ask never tied to a call is over with its turn, and not provisional
/// after it.
#[test]
fn an_untied_ask_is_settled_with_its_turn() {
    let mut p = Projection::new();
    line(&mut p, prompt("p1"));
    p.hook("PermissionRequest", &ask("make"), 1);
    line(&mut p, timed());
    let asks = perms(&p);
    assert!(asks[0].1.answered && !asks[0].0.provisional);
}

/// Two calls alike (the same name and summary) in flight: claude asks in
/// call order, so the first ask holds the first call, and the first call's
/// result answers the first ask, not the second.
#[test]
fn twin_calls_answer_their_asks_in_call_order() {
    let mut p = Projection::new();
    line(&mut p, prompt("p1"));
    line(&mut p, call("c1", "toolu_a", "make"));
    line(&mut p, call("c2", "toolu_b", "make"));
    p.hook("PermissionRequest", &ask("make"), 1);
    p.hook("PermissionRequest", &ask("make"), 2);
    line(&mut p, result("r1", "toolu_a"));
    let answered: Vec<bool> = perms(&p).iter().map(|(_, a)| a.answered).collect();
    assert_eq!(answered, [true, false]);
}
