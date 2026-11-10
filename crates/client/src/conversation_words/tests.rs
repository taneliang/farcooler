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

fn ask_item(item: Value) -> Value {
    let reply: Value = serde_json::from_str(&crate::plan_view::json(&json!({ "schema": 1, "item": item }).to_string())).unwrap();
    reply["item"].clone()
}

#[test]
fn an_items_logic_is_asked_of_the_core() {
    assert_eq!(ask_item(json!({ "agent_type": "general-purpose" }))["agent_type"], "General purpose");
    assert_eq!(ask_item(json!({ "agent_type": "" }))["agent_type"], "Agent");
    assert_eq!(ask_item(json!({ "agent_type": "éclair_x" }))["agent_type"], "Éclair x");
    assert_eq!(ask_item(json!({ "ms": 7_384_000 }))["ms"], "2:03:04");
    assert_eq!(ask_item(json!({ "ms": -5 }))["ms"], "0:00");
    assert_eq!(ask_item(json!({ "origin": "System" }))["origin"], true);
    assert_eq!(ask_item(json!({ "origin": "Scheduled" }))["origin"], false);
    assert_eq!(ask_item(json!({ "ms": 1 }))["agent_type"], Value::Null, "only what is asked");
}

/// What the web's and Android's tests read in place of the core, pinned here.
/// Rewrite with UPDATE_PLAN_VIEW_GOLDEN=1.
#[test]
fn the_item_answers_are_pinned() {
    let mut out = json!({ "agent_type": {}, "ms": {}, "origin": {} });
    for raw in ["general-purpose", "Explore", "web_search", ""] {
        out["agent_type"][raw] = ask_item(json!({ "agent_type": raw }))["agent_type"].clone();
    }
    for ms in [0i64, 42_000, 65_000, 220_000, 3_723_000, 7_384_000, -5] {
        out["ms"][ms.to_string()] = ask_item(json!({ "ms": ms }))["ms"].clone();
    }
    for origin in ["Notification", "System", "Typed", "Queued", "Scheduled"] {
        out["origin"][origin] = ask_item(json!({ "origin": origin }))["origin"].clone();
    }
    let actual = serde_json::to_string_pretty(&out).unwrap() + "\n";
    let path = format!("{}/../../test/fixtures/conversation-items.json", env!("CARGO_MANIFEST_DIR"));
    if std::env::var_os("UPDATE_PLAN_VIEW_GOLDEN").is_some() {
        std::fs::write(&path, &actual).unwrap();
    }
    assert_eq!(actual, std::fs::read_to_string(&path).unwrap_or_default(), "{path} is stale: UPDATE_PLAN_VIEW_GOLDEN=1");
}
