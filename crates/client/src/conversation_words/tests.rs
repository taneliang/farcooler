use serde_json::{json, Value};

use super::*;

fn ask(style: &str, agent: &str) -> Value {
    let request = json!({ "schema": 1, "conversation": { "style": style, "agent": agent } });
    serde_json::from_str(&crate::plan_view::json(&request.to_string())).unwrap()
}

/// The table every app reads, pinned per style and agent. Swift, Kotlin and the
/// web read `test/fixtures/conversation-words.json`. Rewrite with
/// UPDATE_PLAN_VIEW_GOLDEN=1.
#[test]
fn the_conversation_words_are_pinned() {
    let mut out = serde_json::Map::new();
    for (style, agent) in [("apple", "Claude"), ("apple", "Codex"), ("material", "Claude"), ("material", "Codex")] {
        out.insert(format!("{style}_{}", agent.to_lowercase()), ask(style, agent));
    }
    let actual = serde_json::to_string_pretty(&Value::Object(out)).unwrap() + "\n";
    let path = format!("{}/../../test/fixtures/conversation-words.json", env!("CARGO_MANIFEST_DIR"));
    if std::env::var_os("UPDATE_PLAN_VIEW_GOLDEN").is_some() {
        std::fs::write(&path, &actual).unwrap();
    }
    assert_eq!(actual, std::fs::read_to_string(&path).unwrap_or_default(), "{path} is stale: UPDATE_PLAN_VIEW_GOLDEN=1");
}

#[test]
fn the_agent_is_named_and_the_button_is_in_the_platforms_case() {
    let codex = words(Style::Apple, "Codex");
    assert_eq!(codex.send["busy"].text.as_deref(), Some("Codex takes messages only between turns, so this one wasn’t sent. Send it again when the turn ends."));
    assert_eq!(codex.phrases["ask_Permission"], "Codex is asking for permission");
    let apple = words(Style::Apple, "Claude");
    let material = words(Style::Material, "Claude");
    assert_eq!(apple.send_now["settling"].text.as_deref(), Some("Claude is starting a step. Try Send Now again in a moment."));
    assert_eq!(material.send_now["settling"].text.as_deref(), Some("Claude is starting a step. Try Send now again in a moment."));
    assert_eq!(apple.stop["settling"].text.as_deref(), Some("Claude is starting a step. Try Stop again in a moment."));
}

#[test]
fn a_refusal_the_terminal_must_show_is_a_row_not_a_sentence() {
    let w = words(Style::Apple, "Claude");
    for (word, kind) in [("prompt", "handoff"), ("dialog", "handoff"), ("handoff", "panel"), ("draft", "draft")] {
        assert_eq!(w.send[word], Issue { kind, text: None }, "{word}");
    }
    assert_eq!(w.stop["idle"].kind, "quiet", "a turn that ended on its own says nothing");
    assert!(w.send.contains_key(""), "an unknown word has a sentence");
    assert!(w.stop.contains_key("") && w.send_now.contains_key("") && w.answer.contains_key(""));
}

#[test]
fn the_last_group_takes_any_tool() {
    let w = words(Style::Apple, "Claude");
    assert!(w.groups.last().unwrap().names.is_empty());
    assert!(w.groups[..w.groups.len() - 1].iter().all(|g| !g.names.is_empty()));
}

#[test]
fn another_schema_is_refused() {
    let bad: Value = serde_json::from_str(&crate::plan_view::json(r#"{"schema":2,"conversation":{"agent":"Claude"}}"#)).unwrap();
    assert_eq!(bad["error"], "invalid");
}

#[test]
fn a_span_is_said_in_a_timers_format() {
    let say = |ms: i64| -> Value { serde_json::from_str::<Value>(&crate::plan_view::json(&json!({ "schema": 1, "span": ms }).to_string())).unwrap()["span"].clone() };
    assert_eq!([say(-5), say(4_000), say(65_000), say(7_384_000)], ["0:00", "0:04", "1:05", "2:03:04"]);
    let bad: Value = serde_json::from_str(&crate::plan_view::json(r#"{"schema":2,"span":1}"#)).unwrap();
    assert_eq!(bad["error"], "invalid");
}
