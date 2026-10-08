//! The suggestion a turn row carries (ov-409): from the screen's box, as
//! claude draws it, through the real reader to the row's JSON.

use serde_json::json;

use super::rows::*;
use super::Projection;

fn line(p: &mut Projection, v: serde_json::Value) {
    p.fold_line(v.to_string().as_bytes());
}

fn prompt(id: &str, text: &str) -> serde_json::Value {
    json!({"type":"user","promptId":id,"promptSource":"typed","timestamp":"2026-10-06T10:00:00Z","message":{"role":"user","content":text}})
}

/// The newest turn's row as `agent.rows` carries it, with a suggestion.
const GOLDEN: &str = r#"{"ord":0,"rev":2,"id":"turn:p1","turn":null,"provisional":false,"kind":{"Turn":{"prompt":"first","origin":"Typed","started_ms":1791280800000,"ended_ms":null,"duration_ms":null,"outcome":null,"background_running":0,"activity":null,"suggestion":"wait for the background shell to finish"}}}"#;

fn turns(p: &Projection) -> Vec<(String, Option<String>)> {
    p.rows()
        .iter()
        .filter_map(|r| match &r.kind {
            RowKind::Turn(t) => Some((r.id.clone(), t.suggestion.clone())),
            _ => None,
        })
        .collect()
}

/// The screen is the source: the real reader's answer on a real capture is
/// what the row carries, down to the JSON a client decodes.
#[test]
fn the_screen_prediction_rides_the_newest_turn_and_clears_with_it() {
    let screen = std::fs::read_to_string(format!("{}/captures/claude-idle-nothing-running.txt", env!("CARGO_MANIFEST_DIR")))
        .unwrap()
        .replace("❯\u{a0}wait for the background shell to finish", "❯\u{a0}\x1b[2mwait for the background shell to finish\x1b[0m");
    let mut p = Projection::new();
    // No turn yet: nothing to carry it, and nothing breaks.
    p.set_suggestion(crate::composer::suggestion("claude", &screen));
    line(&mut p, prompt("p1", "first"));
    assert_eq!(turns(&p), [("turn:p1".to_string(), None)], "a prompt given after the screen was read is not offered one");

    let rev = p.revision();
    p.set_suggestion(crate::composer::suggestion("claude", &screen));
    let want = Some("wait for the background shell to finish".to_string());
    assert_eq!(turns(&p), [("turn:p1".to_string(), want.clone())]);
    assert!(p.changes_since(rev, 10).unwrap().iter().any(|c| c.id() == "turn:p1"), "a follower is told");
    let json = serde_json::to_string(p.rows().iter().find(|r| r.id == "turn:p1").unwrap()).unwrap();
    assert!(json.contains("\"suggestion\":\"wait for the background shell to finish\""), "{json}");
    // The bytes the apps decode: Swift's and Kotlin's row tests read this very string.
    assert_eq!(json, GOLDEN, "the turn row on the wire");

    // The same answer again changes nothing.
    let rev = p.revision();
    p.set_suggestion(want.clone());
    assert_eq!(p.revision(), rev);

    // A new prompt spends it, on the screen's next read or before.
    line(&mut p, prompt("p2", "second"));
    assert_eq!(turns(&p), [("turn:p1".to_string(), None), ("turn:p2".to_string(), None)]);
    p.set_suggestion(want.clone());
    assert_eq!(turns(&p), [("turn:p1".to_string(), None), ("turn:p2".to_string(), want)]);
    p.set_suggestion(None);
    assert_eq!(turns(&p)[1].1, None);
    let json = serde_json::to_string(p.rows().iter().find(|r| r.id == "turn:p2").unwrap()).unwrap();
    assert!(!json.contains("suggestion"), "absent, not null: {json}");
}
