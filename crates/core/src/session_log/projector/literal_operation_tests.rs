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
