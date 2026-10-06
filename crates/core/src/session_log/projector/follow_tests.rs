//! Pages and follows over a projection (ov-366).

use serde_json::json;

use super::fixtures::*;
use super::rows::*;
use super::Projection;

fn line(p: &mut Projection, v: serde_json::Value) {
    p.fold_line(v.to_string().as_bytes());
}

fn prompt(id: &str, text: &str) -> serde_json::Value {
    json!({"type":"user","promptId":id,"promptSource":"typed","timestamp":"2026-10-06T10:00:00Z","message":{"role":"user","content":text}})
}

fn said(uuid: &str, text: &str) -> serde_json::Value {
    json!({"type":"assistant","uuid":uuid,"timestamp":"2026-10-06T10:00:01Z","message":{"content":[{"type":"text","text":text}]}})
}

#[test]
fn a_follow_is_inserts_updates_and_removals_by_id() {
    let mut p = Projection::new();
    line(&mut p, prompt("p1", "go"));
    let rev = p.revision();
    p.hook("MessageDisplay", &json!({"prompt_id":"p1","message_id":"m1","index":0,"delta":"Shown first.\n"}), 1);
    line(&mut p, said("a1", "Written instead."));
    line(&mut p, json!({"type":"system","subtype":"turn_duration","durationMs":5,"timestamp":"2026-10-06T10:00:02Z"}));
    let changes = p.changes_since(rev, 100).unwrap();
    let kinds: Vec<(&str, &str)> = changes
        .iter()
        .map(|c| match c {
            Change::Insert(r) => ("insert", r.id.as_str()),
            Change::Update(r) => ("update", r.id.as_str()),
            Change::Remove { id, .. } => ("remove", *id),
        })
        .collect();
    assert_eq!(kinds, [("update", "turn:p1"), ("insert", "prose:a1:0")], "a row added and retracted since is nothing to this follower");

    // A follower that already held the hook's row is told to remove it.
    let mut p = Projection::new();
    line(&mut p, prompt("p1", "go"));
    p.hook("MessageDisplay", &json!({"prompt_id":"p1","message_id":"m1","index":0,"delta":"Shown first.\n"}), 1);
    let held = p.revision();
    line(&mut p, said("a1", "Written instead."));
    line(&mut p, json!({"type":"system","subtype":"turn_duration","durationMs":5,"timestamp":"2026-10-06T10:00:02Z"}));
    let changes = p.changes_since(held, 100).unwrap();
    assert!(changes.iter().any(|c| matches!(c, Change::Remove { id: "hprose:m1", .. })), "{changes:?}");
    assert!(changes.windows(2).all(|w| w[0].rev() != w[1].rev()), "each change at its own revision");
}

#[test]
fn a_page_skips_retracted_rows_and_still_holds_its_limit() {
    let mut p = Projection::new();
    line(&mut p, prompt("p1", "go"));
    p.hook("MessageDisplay", &json!({"prompt_id":"p1","message_id":"m1","index":0,"delta":"Shown.\n"}), 1);
    line(&mut p, said("a1", "Written."));
    line(&mut p, said("a2", "More."));
    line(&mut p, json!({"type":"system","subtype":"turn_duration","durationMs":5,"timestamp":"2026-10-06T10:00:02Z"}));
    assert!(p.rows().iter().any(|r| r.retracted), "the hook's copy is retracted");
    let page = p.page(None, 3);
    let ids: Vec<&str> = page.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["turn:p1", "prose:a1:0", "prose:a2:0"]);
    assert!(!p.any_before(page[0].ord));
    assert!(p.row("hprose:m1").is_none(), "gone from the index");
}

#[test]
fn a_follower_too_far_behind_is_told_to_page_instead() {
    let p = fold(EDITS);
    assert!(p.changes_since(0, 3).is_none(), "more than 3 changed");
    assert_eq!(p.changes_since(0, 10_000).unwrap().len(), p.rows().len());
    assert!(p.changes_since(p.revision(), 3).unwrap().is_empty());
}

/// The fold's cost a line does not grow with the session: the last 5,000
/// turns of 25,000 (50,000 rows) fold about as fast as the first 5,000.
/// Walking every row at each turn (activity, unrecorded turns, asks, prose)
/// made a 50,000-row attach take 30 s.
#[test]
fn a_late_turn_folds_as_fast_as_an_early_one() {
    let lines: Vec<String> = (0..25_000)
        .flat_map(|n| {
            [
                json!({"type":"user","promptId":format!("p{n}"),"promptSource":"typed","uuid":format!("u{n}"),"timestamp":"2026-10-06T10:00:00Z","message":{"content":"Another small change."}}).to_string(),
                json!({"type":"assistant","uuid":format!("a{n}"),"timestamp":"2026-10-06T10:00:01Z","message":{"content":[{"type":"text","text":"Done."}],"stop_reason":"end_turn"}}).to_string(),
            ]
        })
        .collect();
    let mut p = Projection::new();
    let mut fold = |range: std::ops::Range<usize>| {
        let started = std::time::Instant::now();
        for line in &lines[range] {
            p.fold_line(line.as_bytes());
        }
        started.elapsed()
    };
    let early = fold(0..10_000);
    fold(10_000..40_000);
    let late = fold(40_000..50_000);
    assert!(late < early * 3 + std::time::Duration::from_millis(50), "early {early:?}, late {late:?}");
}
