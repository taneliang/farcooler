//! `test/fixtures/task-usage.json`, the wording AgentKit and Android are
//! pinned to as well.

use super::*;

fn fixture() -> serde_json::Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../test/fixtures/task-usage.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("the shared fixture")).expect("fixture JSON")
}

fn text(v: &serde_json::Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

#[test]
fn token_counts_read_as_the_fixture_says() {
    let f = fixture();
    for case in f["tokens"].as_array().unwrap() {
        assert_eq!(tokens(case["n"].as_u64().unwrap()), case["text"].as_str().unwrap(), "{case}");
    }
}

#[test]
fn dollars_read_as_the_fixture_says() {
    let f = fixture();
    for case in f["dollars"].as_array().unwrap() {
        assert_eq!(dollars(case["micros"].as_i64().unwrap()), case["text"].as_str().unwrap(), "{case}");
    }
}

#[test]
fn durations_read_as_the_fixture_says() {
    let f = fixture();
    for case in f["durations"].as_array().unwrap() {
        assert_eq!(duration(case["ms"].as_i64().unwrap()), case["text"].as_str().unwrap(), "{case}");
    }
}

#[test]
fn every_case_is_worded_as_the_fixture_says() {
    let f = fixture();
    let cases = f["cases"].as_array().unwrap();
    assert!(cases.len() >= 9, "the fixture has its cases");
    for case in cases {
        let name = case["case"].as_str().unwrap();
        let usage: TaskSpend = serde_json::from_value(case["usage"].clone()).expect(name);
        let t = &usage.totals;
        assert_eq!(t.is_empty(), case["empty"].as_bool().unwrap(), "{name}: empty");
        if t.is_empty() {
            assert!(usage.rows().is_empty(), "{name}");
            continue;
        }
        assert_eq!(Some(t.tokens_line()), text(&case["tokens"]), "{name}: tokens");
        assert_eq!(t.token_detail(), text(&case["token_detail"]), "{name}: token detail");
        assert_eq!(Some(t.cost_line()), text(&case["cost"]), "{name}: cost");
        assert_eq!(t.time_line(), text(&case["time"]), "{name}: time");
        let rows: Vec<(String, String)> = usage.rows().iter().map(|r| (r.title(), r.detail())).collect();
        let expected: Vec<(String, String)> = case["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (r["title"].as_str().unwrap().to_string(), r["detail"].as_str().unwrap().to_string()))
            .collect();
        assert_eq!(rows, expected, "{name}: rows");
    }
}

#[test]
fn the_json_round_trips_with_the_fixture_field_names() {
    let f = fixture();
    let raw = f["cases"][1]["usage"].clone();
    let usage: TaskSpend = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(usage.totals.cost_reported_micros, 3_200_000);
    assert_eq!(serde_json::to_value(&usage).unwrap(), raw, "every field the apps read, by the same name");
}

#[test]
fn the_sections_sentences_are_the_fixtures() {
    let f = fixture();
    let w = &f["words"];
    assert_eq!(NOTHING_YET, w["nothing_yet"]);
    assert_eq!(NEEDS_UPDATE, w["needs_update"]);
    assert_eq!(COULDNT_READ, w["couldnt_read"]);
    assert_eq!(API_EQUIVALENT, w["api_equivalent"]);
    assert!(w.get("try_again").is_none(), "a button's label is each platform's own");
}
