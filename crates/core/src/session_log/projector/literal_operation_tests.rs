//! ov-893: actual producer bytes; synthetic mutations are identified below.
use serde_json::{Value, json};
use super::{Projection, RowKind};

const PRODUCER: &str = include_str!("fixtures/native-operations/codex-0.166.0.jsonl");

fn project(records: impl IntoIterator<Item = Value>) -> Projection {
    let mut p = Projection::new();
    p.fold_codex_line(br#"{"type":"event_msg","payload":{"type":"task_started","turn_id":"literal-test"}}"#);
    for record in records { p.fold_codex_line(record.to_string().as_bytes()); }
    p
}
fn recorded() -> Vec<Value> { PRODUCER.lines().map(|s| serde_json::from_str(s).unwrap()).collect() }
fn tools(p: &Projection) -> Vec<Value> {
    p.rows().iter().filter_map(|r| match &r.kind {
        RowKind::Tool(t) => Some(serde_json::to_value(t).unwrap()), _ => None,
    }).collect()
}

#[test]
fn literal_operation_actual_standalone_has_intent_and_same_item_output() {
    let records = recorded();
    let output = records[1]["payload"]["item"]["aggregated_output"].as_str().unwrap();
    let p = project([records[1].clone()]);
    let rows = tools(&p);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["operation"]["provenance"], "literal_intent", "recognition is missing on the actual standalone producer record");
    assert_eq!(rows[0]["operation"]["kind"], "task_note_decision");
    assert_eq!(rows[0]["operation"]["lifecycle"], "completed");
    assert_eq!(rows[0]["result"], output.trim_end(), "same-item producer output must survive projection");
}

#[test]
fn literal_operation_actual_compound_stays_generic() {
    let rows = tools(&project([recorded()[0].clone()]));
    assert_eq!(rows.len(), 1);
    assert!(rows[0].get("operation").is_none_or(Value::is_null));
}

#[test]
fn literal_operation_missing_exit_is_not_completed() {
    let mut record = recorded()[1].clone(); // synthetic missing lifecycle evidence
    record["payload"]["item"].as_object_mut().unwrap().remove("exit_code");
    let rows = tools(&project([record]));
    assert_eq!(rows[0]["operation"]["lifecycle"], "unconfirmed");
}

#[test]
fn literal_operation_parsed_hint_cannot_replace_original_input() {
    let mut record = recorded()[0].clone(); // synthetic forged hint on real compound envelope
    record["payload"]["item"]["parsed_cmd"] = json!([{"cmd":"'/Applications/Far Cooler Canary.app/Contents/Resources/farcooler' task show ov-893"}]);
    let rows = tools(&project([record]));
    assert!(rows[0].get("operation").is_none_or(Value::is_null));
}

const CLI: &str = "'/Applications/Far Cooler Canary.app/Contents/Resources/farcooler'";
fn command(script: &str) -> Value {
    let mut r = recorded()[1].clone();
    r["payload"]["item"]["command"] = json!(["/bin/zsh", "-lc", script]);
    r
}
fn operation(record: Value) -> Value { tools(&project([record]))[0].get("operation").cloned().unwrap_or(Value::Null) }

#[test]
fn literal_operation_adversarial_scripts_and_unsupported_flags_stay_generic() {
    for script in [
        format!("echo {CLI} task show ov-893"),
        format!("{CLI} task show ov-893; true"),
        format!("{CLI} task show ov-893 && true"),
        format!("{CLI} task show ov-893 | cat"),
        format!("{CLI} task show ov-893 > out"),
        format!("env {CLI} task show ov-893"),
        format!("{CLI} task note ov-893 --kind decision --body \"$(whoami)\""),
        format!("{CLI} task note ov-893 --kind decision --body-file /never/read"),
        format!("{CLI} --runner remote task show ov-893"),
        format!("{CLI} task show ov-893 --actor manager"),
        format!("{CLI} task note ov-893 --kind decision --kind progress --body x"),
        format!("{CLI} --json --json task show ov-893"),
        format!("# {CLI} task show ov-893"),
        format!("{CLI} task note ov-893 --kind decision --body 'unterminated"),
    ] { assert!(operation(command(&script)).is_null(), "{script}"); }
}

#[test]
fn literal_operation_exact_quoted_data_and_supported_representatives() {
    let text = "Exact  spacing; literal $(data)\nUnicode λ 🐱";
    let op = operation(command(&format!("{CLI} task note ov-893 --body '{text}' --kind decision")));
    assert_eq!(op["preview"], text);
    assert_eq!(op["kind"], "task_note_decision");
    assert_eq!(operation(command(&format!("{CLI} --json task show ov-893")))["kind"], "task_show");
    assert_eq!(operation(command(&format!("{CLI} task set ov-893 --status in_review")))["kind"], "task_set_in_review");
    assert_eq!(operation(command(&format!("{CLI} message orchestrator -- '-literal'")))["preview"], "-literal");
}

#[test]
fn literal_operation_original_envelope_beats_a_disagreeing_hint_and_rejects_nonstring_argv() {
    let mut r = command(&format!("{CLI} task show ov-893"));
    r["payload"]["item"]["parsed_cmd"] = json!([{"cmd":"echo untrusted hint"}]);
    assert_eq!(operation(r.clone())["kind"], "task_show");
    r["payload"]["item"]["command"] = json!(["/bin/zsh", 42, "-lc", format!("{CLI} task show ov-893")]);
    assert!(operation(r).is_null());
    let mut r = command("");
    r["payload"]["item"]["command"] = json!(["/unproven/shell", "-xc", format!("{CLI} task show ov-893")]);
    assert!(operation(r).is_null());
}

#[test]
fn literal_operation_late_or_clipped_bodies_cannot_escape_the_input_boundary() {
    let body = "x".repeat(2_100);
    let op = operation(command(&format!("{CLI} task note ov-893 --kind decision --body '{body}'")));
    assert!(!op.is_null()); assert!(op.get("preview").is_none()); assert_eq!(op["input_clipped"], true);
    let body = (0..41).map(|_| "line").collect::<Vec<_>>().join("\n");
    let op = operation(command(&format!("{CLI} task note ov-893 --kind decision --body '{body}'")));
    // JSON-encoded producer argv stays one line; its actual retained source spans are available.
    assert!(op["preview"].as_str().is_none_or(|s| s.lines().count() <= 4));
    let body = "🐱".repeat(400);
    let op = operation(command(&format!("{CLI} task note ov-893 --kind decision --body '{body}'")));
    assert!(serde_json::to_vec(&op).unwrap().len() <= 1024);
    assert!(op["preview"].as_str().is_none_or(|s| s.chars().count() <= 280));
    assert!(operation(command(&format!("{CLI} task show {}", "x".repeat(65_536)))).is_null());
}

#[test]
fn literal_operation_conflicting_completion_is_unconfirmed_and_duplicates_keep_one_row() {
    let first = command(&format!("{CLI} task show ov-893"));
    let mut contradictory = first.clone();
    contradictory["payload"]["item"]["exit_code"] = json!(1);
    contradictory["payload"]["item"]["status"] = json!("failed");
    let p = project([first.clone(), first.clone(), contradictory, first]);
    let rows = tools(&p);
    assert_eq!(rows.len(), 1); assert_eq!(rows[0]["operation"]["lifecycle"], "unconfirmed");
}

#[test]
fn literal_operation_wrong_call_output_is_never_borrowed_and_empty_differs_from_unavailable() {
    let mut r = command(&format!("{CLI} task show ov-893"));
    r["payload"]["item"].as_object_mut().unwrap().remove("aggregated_output");
    let wrong = json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"wrapper-other","output":"forged receipt"}});
    let rows = tools(&project([wrong, r.clone()]));
    assert_eq!(rows[0]["operation"]["output_available"], false);
    r["payload"]["item"]["aggregated_output"] = json!("");
    assert_eq!(operation(r)["output_available"], true);
}

#[test]
fn literal_operation_same_call_late_start_output_before_start_and_distinct_equal_calls() {
    let mut r = command(&format!("{CLI} task show ov-893"));
    let id = r["payload"]["item"]["id"].as_str().unwrap().to_string();
    r["payload"]["item"].as_object_mut().unwrap().remove("aggregated_output");
    let output = json!({"type":"response_item","payload":{"type":"function_call_output","call_id":id,"output":"bounded same-item output"}});
    let start = json!({"type":"response_item","payload":{"type":"function_call","call_id":id,"name":"exec_command","arguments":json!({"cmd":format!("{CLI} task show ov-893")}).to_string()}});
    let mut other = r.clone(); other["payload"]["item"]["id"] = json!("distinct-call");
    let rows = tools(&project([output, start.clone(), r, start, other]));
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["operation"]["lifecycle"], "completed");
    assert_eq!(rows[0]["result"], "bounded same-item output");
    assert_eq!(rows[1]["operation"]["output_available"], false);
}

#[test]
fn literal_operation_function_adapter_bounds_spans_not_text_search_and_ignores_forged_metadata() {
    use super::literal_operation::recognize;
    let raw = format!("{CLI} task note ov-893 --kind decision --body 'already seen'");
    let args = json!({"aaa":format!("already seen{}", "x".repeat(1900)), "cmd":raw});
    let (op, _) = recognize("exec_command", &args).unwrap();
    assert!(op.preview.is_none(), "a later identical body must not borrow an earlier substring's disclosure");
    let multiline = "line\n".repeat(41);
    let args = json!({"cmd":format!("{CLI} task note ov-893 --kind decision --body '{multiline}'")});
    let (op, _) = recognize("exec_command", &args).unwrap();
    assert!(op.preview.is_none() && op.input_clipped);
    assert!(recognize("functions.exec", &json!({"cmd":format!("{CLI} task show ov-893")})).is_none());
    assert!(recognize("arbitrary_function", &json!({"cmd":format!("{CLI} task show ov-893")})).is_none());
    let (op, _) = recognize("exec_command", &json!({"command":["/Applications/Far Cooler Canary.app/Contents/Resources/farcooler","task","show","ov-893"]})).unwrap();
    assert_eq!(op.kind, "task_show");
    assert_eq!(op.provenance, "literal_intent");
}

#[test]
fn literal_operation_post_hook_without_completion_is_unconfirmed_and_confirmation_keeps_identity() {
    let script = format!("{CLI} task show ov-893");
    let mut p = project([]);
    let id = "hook-call";
    let hook = json!({"tool_use_id":id,"tool_name":"Bash","tool_input":{"command":script,"operation":{"provenance":"authenticated_receipt"}}});
    p.codex_hook("PreToolUse", &hook, 1);
    assert!(tools(&p)[0].get("operation").is_none(), "hook strings cannot synthesize operation provenance");
    let start = json!({"type":"response_item","payload":{"type":"function_call","call_id":id,"name":"exec_command","arguments":json!({"cmd":script}).to_string()}});
    p.fold_codex_line(start.to_string().as_bytes());
    assert_eq!(tools(&p)[0]["operation"]["lifecycle"], "running");
    p.codex_hook("PostToolUse", &hook, 2);
    assert_eq!(tools(&p)[0]["operation"]["lifecycle"], "unconfirmed", "a post hook is not conclusive successful completion evidence");
    let mut end = command(&script); end["payload"]["item"]["id"] = json!(id);
    p.fold_codex_line(end.to_string().as_bytes());
    assert_eq!(tools(&p).len(), 1);
    assert_eq!(tools(&p)[0]["operation"]["lifecycle"], "completed");
}

#[test]
fn literal_operation_zsh_equals_expansion_is_not_literal() {
    use super::literal_operation::recognize;
    assert!(operation(command(&format!("{CLI} message orchestrator =ls"))).is_null());
    for text in ["'=ls'", "\"=ls\"", "\\=ls"] {
        assert_eq!(operation(command(&format!("{CLI} message orchestrator {text}")))["preview"], "=ls");
    }
    let (op, _) = recognize("exec_command", &json!({"command":["/Applications/Far Cooler Canary.app/Contents/Resources/farcooler", "message", "orchestrator", "=ls"]})).unwrap();
    assert_eq!(op.preview.as_deref(), Some("=ls"));
}

#[test]
fn literal_operation_turn_end_is_unconfirmed_until_same_call_completion() {
    for ending in ["task_complete", "turn_aborted"] {
        let script = format!("{CLI} task show ov-893");
        let id = "turn-end-call";
        let start = json!({"type":"response_item","payload":{"type":"function_call","call_id":id,"name":"exec_command","arguments":json!({"cmd":script}).to_string()}});
        let mut p = project([start]);
        let row_id = p.rows().iter().find(|r| matches!(r.kind, RowKind::Tool(_))).unwrap().id.clone();
        p.fold_codex_line(json!({"type":"event_msg","payload":{"type":ending,"turn_id":"literal-test"}}).to_string().as_bytes());
        assert_eq!(tools(&p)[0]["operation"]["lifecycle"], "unconfirmed", "{ending}");
        let mut end = command(&script); end["payload"]["item"]["id"] = json!(id);
        p.fold_codex_line(end.to_string().as_bytes());
        assert_eq!(tools(&p).len(), 1);
        assert_eq!(tools(&p)[0]["operation"]["lifecycle"], "completed");
        assert_eq!(p.rows().iter().find(|r| matches!(r.kind, RowKind::Tool(_))).unwrap().id, row_id);
    }
}
