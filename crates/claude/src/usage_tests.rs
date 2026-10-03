use super::*;

/// The `result` frame of a real turn (`tests/fixtures/turn_basic.jsonl`,
/// claude 2.1.x against claude-opus-5[1m]), and the assistant frame before it.
fn recorded(kind: &str) -> Value {
    include_str!("../tests/fixtures/turn_basic.jsonl")
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|f| f["type"] == kind)
        .expect("the fixture has one")
}

/// The same process's next result, as Claude Code writes it: `modelUsage`
/// and `total_cost_usd` grown by a second turn's spend (synthetic: no
/// two-turn capture exists), `usage` that second turn's alone.
fn second_result() -> Value {
    let mut frame = recorded("result");
    frame["uuid"] = "00000000-0000-4000-8000-0000000000aa".into();
    let opus = &mut frame["modelUsage"]["claude-opus-5[1m]"];
    opus["inputTokens"] = 5.into();
    opus["outputTokens"] = 104.into();
    opus["cacheReadInputTokens"] = 46842.into();
    opus["cacheCreationInputTokens"] = 7418.into();
    opus["costUSD"] = 0.1.into();
    frame["total_cost_usd"] = 0.1.into();
    frame["usage"] = serde_json::json!({ "input_tokens": 3, "output_tokens": 100,
        "cache_read_input_tokens": 26930, "cache_creation_input_tokens": 400 });
    frame
}

fn opus(input: u64, output: u64, cache_read: u64, cache_write: u64, cost: Option<i64>) -> ModelUsage {
    ModelUsage {
        model: Some("claude-opus-5[1m]".into()),
        input,
        output,
        cache_read,
        cache_write,
        cache_write_1h: 0,
        reported_cost_micros: cost,
    }
}

#[test]
fn a_processes_first_result_is_its_first_turn() {
    let usage = Ledger::default().observe(&recorded("result")).unwrap();
    assert_eq!(usage.key, "claude:9e766346-8b18-4f9f-b11d-575d07f891fd");
    assert_eq!(usage.active_ms, Some(3948));
    assert!(!usage.partial);
    assert_eq!(usage.models, Some(vec![opus(2, 4, 19912, 7018, Some(80_246))]));
}

/// The running totals of the second result include the first turn; the
/// second turn is only what they grew by.
#[test]
fn a_second_result_records_only_what_the_second_turn_added() {
    let mut ledger = Ledger::default();
    ledger.observe(&recorded("result"));
    let second = ledger.observe(&second_result()).unwrap();
    assert_eq!(second.models, Some(vec![opus(3, 100, 26930, 400, Some(19_754))]));
}

/// A resumed process's first totals carry the restored session's spend. The
/// turn is read from its own `usage` instead, on the model its message named,
/// with no reported cost, and marked partial; the next turn differences.
#[test]
fn a_resumed_processes_first_result_records_none_of_the_restored_spend() {
    let mut ledger = Ledger::resumed();
    ledger.observe(&recorded("assistant"));
    let first = ledger.observe(&second_result()).unwrap();
    assert!(first.partial);
    let models = first.models.unwrap();
    assert_eq!(models[0].model.as_deref(), Some("claude-opus-5"));
    assert_eq!((models[0].input, models[0].output, models[0].cache_read), (3, 100, 26930));
    assert_eq!(models[0].reported_cost_micros, None, "the running cost is not this turn's");

    let mut third = second_result();
    third["uuid"] = "00000000-0000-4000-8000-0000000000bb".into();
    third["modelUsage"]["claude-opus-5[1m]"]["outputTokens"] = 150.into();
    third["modelUsage"]["claude-opus-5[1m]"]["costUSD"] = 0.15.into();
    let next = ledger.observe(&third).unwrap();
    assert!(!next.partial);
    assert_eq!(next.models, Some(vec![opus(0, 46, 0, 0, Some(50_000))]));
}

/// A model that did no work this turn is not a row; one that appears for
/// the first time counts from zero.
#[test]
fn only_the_models_that_moved_are_this_turns() {
    let mut ledger = Ledger::default();
    ledger.observe(&recorded("result"));
    let mut next = recorded("result");
    next["modelUsage"]["claude-haiku-4-5-20251001"] =
        serde_json::json!({ "inputTokens": 10, "outputTokens": 2, "costUSD": 0.00002 });
    let models = ledger.observe(&next).unwrap().models.unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].model.as_deref(), Some("claude-haiku-4-5-20251001"));
    assert_eq!((models[0].input, models[0].reported_cost_micros), (10, Some(20)));
}

/// Without a split, the per-turn `usage` and the growth of `total_cost_usd`.
#[test]
fn without_a_split_the_turns_usage_and_its_share_of_the_cost_are_read() {
    let mut ledger = Ledger::default();
    let strip = |mut f: Value| {
        f.as_object_mut().unwrap().remove("modelUsage");
        f
    };
    let first = ledger.observe(&strip(recorded("result"))).unwrap().models.unwrap();
    assert_eq!((first[0].cache_write, first[0].cache_write_1h, first[0].reported_cost_micros), (7018, 7018, Some(80_246)));
    let second = ledger.observe(&strip(second_result())).unwrap().models.unwrap();
    assert_eq!((second[0].output, second[0].reported_cost_micros), (100, Some(19_754)));
}

/// A result that says nothing about spend is "not reported", not zero.
#[test]
fn a_result_missing_its_usage_is_not_reported() {
    let mut frame = recorded("result");
    let object = frame.as_object_mut().unwrap();
    object.remove("modelUsage");
    object.remove("usage");
    let usage = Ledger::default().observe(&frame).unwrap();
    assert_eq!(usage.models, None);
    assert_eq!(usage.active_ms, Some(3948), "the turn still ran for as long as it did");
}

#[test]
fn only_a_result_frame_is_a_turns_usage() {
    assert_eq!(Ledger::default().observe(&recorded("assistant")), None);
}

#[test]
fn a_result_without_an_id_is_keyed_the_same_every_time_it_is_heard() {
    let mut frame = recorded("result");
    frame.as_object_mut().unwrap().remove("uuid");
    assert_eq!(key(&frame), key(&frame.clone()));
}
