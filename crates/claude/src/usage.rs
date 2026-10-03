//! A turn's spend, read off the stream-json `result` frame that ends it.
//!
//! The `result` carries the whole turn's account twice: `usage`, summed over
//! every model call, and `modelUsage`, the same split by model with Claude
//! Code's own `costUSD` for each. The split is preferred, because a turn can
//! spend on more than one model and a report broken down by model needs it;
//! `usage` with `total_cost_usd` is the fallback for a frame without one.

use farcooler_agent_core::usage::{ModelUsage, TurnUsage, usd_to_micros};
use serde_json::Value;

/// The usage a `result` frame reports, or `None` for any other frame.
///
/// A result with neither `modelUsage` nor `usage` still yields a
/// `TurnUsage`, with `models: None`: the turn ended and said nothing about
/// what it spent, which is "not reported", not zero.
pub fn turn_usage(frame: &Value) -> Option<TurnUsage> {
    if frame["type"].as_str() != Some("result") {
        return None;
    }
    let models = by_model(frame).or_else(|| summed(frame));
    Some(TurnUsage { key: key(frame), models, active_ms: frame["duration_ms"].as_u64() })
}

/// The result's own `uuid`, which a replay of the same frame repeats. A frame
/// without one is keyed by its content, which a replay also repeats.
fn key(frame: &Value) -> String {
    match frame["uuid"].as_str() {
        Some(uuid) if !uuid.is_empty() => format!("claude:{uuid}"),
        _ => format!("claude:{}", fnv(frame.to_string().as_bytes())),
    }
}

fn by_model(frame: &Value) -> Option<Vec<ModelUsage>> {
    let split = frame["modelUsage"].as_object().filter(|m| !m.is_empty())?;
    Some(
        split
            .iter()
            .map(|(model, u)| ModelUsage {
                model: Some(model.clone()),
                input: u["inputTokens"].as_u64().unwrap_or(0),
                output: u["outputTokens"].as_u64().unwrap_or(0),
                cache_read: u["cacheReadInputTokens"].as_u64().unwrap_or(0),
                cache_write: u["cacheCreationInputTokens"].as_u64().unwrap_or(0),
                // Not split by lifetime per model. It only matters for an
                // estimate, and a model here carries its own `costUSD`.
                cache_write_1h: 0,
                reported_cost_micros: u["costUSD"].as_f64().and_then(usd_to_micros),
            })
            .collect(),
    )
}

fn summed(frame: &Value) -> Option<Vec<ModelUsage>> {
    let u = frame["usage"].as_object()?;
    let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
    Some(vec![ModelUsage {
        model: None,
        input: n("input_tokens"),
        output: n("output_tokens"),
        cache_read: n("cache_read_input_tokens"),
        cache_write: n("cache_creation_input_tokens"),
        cache_write_1h: u
            .get("cache_creation")
            .and_then(|c| c["ephemeral_1h_input_tokens"].as_u64())
            .unwrap_or(0),
        reported_cost_micros: frame["total_cost_usd"].as_f64().and_then(usd_to_micros),
    }])
}

/// FNV-1a, 64-bit: a stable name for a frame that carried no id. Nothing
/// adversarial depends on it.
fn fnv(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `result` frame of a real turn (`tests/fixtures/turn_basic.jsonl`,
    /// claude 2.1.x against claude-opus-5[1m]).
    fn recorded_result() -> Value {
        include_str!("../tests/fixtures/turn_basic.jsonl")
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .find(|f| f["type"] == "result")
            .expect("the fixture ends its turn")
    }

    #[test]
    fn a_recorded_result_reads_per_model_with_the_agents_own_cost() {
        let usage = turn_usage(&recorded_result()).unwrap();
        assert_eq!(usage.key, "claude:9e766346-8b18-4f9f-b11d-575d07f891fd");
        assert_eq!(usage.active_ms, Some(3948));
        assert_eq!(
            usage.models,
            Some(vec![ModelUsage {
                model: Some("claude-opus-5[1m]".into()),
                input: 2,
                output: 4,
                cache_read: 19912,
                cache_write: 7018,
                cache_write_1h: 0,
                reported_cost_micros: Some(80_246),
            }])
        );
    }

    #[test]
    fn without_a_split_the_summed_usage_and_total_cost_are_read() {
        let mut frame = recorded_result();
        frame.as_object_mut().unwrap().remove("modelUsage");
        let models = turn_usage(&frame).unwrap().models.unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model, None);
        assert_eq!((models[0].cache_read, models[0].cache_write, models[0].cache_write_1h), (19912, 7018, 7018));
        assert_eq!(models[0].reported_cost_micros, Some(80_246));
    }

    /// A result that says nothing about spend is "not reported", not zero.
    #[test]
    fn a_result_missing_its_usage_is_not_reported() {
        let mut frame = recorded_result();
        let object = frame.as_object_mut().unwrap();
        object.remove("modelUsage");
        object.remove("usage");
        let usage = turn_usage(&frame).unwrap();
        assert_eq!(usage.models, None);
        assert_eq!(usage.active_ms, Some(3948), "the turn still ran for as long as it did");
    }

    #[test]
    fn only_a_result_frame_is_a_turns_usage() {
        let assistant = include_str!("../tests/fixtures/turn_basic.jsonl")
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .find(|f| f["type"] == "assistant")
            .unwrap();
        assert_eq!(turn_usage(&assistant), None, "an assistant frame's usage is one call, not the turn");
    }

    #[test]
    fn a_result_without_an_id_is_keyed_the_same_every_time_it_is_heard() {
        let mut frame = recorded_result();
        frame.as_object_mut().unwrap().remove("uuid");
        assert_eq!(turn_usage(&frame).unwrap().key, turn_usage(&frame.clone()).unwrap().key);
    }
}
