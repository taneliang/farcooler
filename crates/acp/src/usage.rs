//! A turn's spend, where the adapter reports one.
//!
//! Two places, both optional in ACP and both seen from
//! `@agentclientprotocol/claude-agent-acp` (`tests/fixtures/subagent_without_cap.jsonl`):
//! the `session/prompt` result's `usage` (`inputTokens`, `outputTokens`,
//! `cachedReadTokens`, `cachedWriteTokens`), and a `usage_update`'s `cost`,
//! which is the session's running total in a named currency. A turn's cost is
//! the difference across it. An adapter that sends neither reports nothing,
//! and nothing is what is recorded.

use farcooler_agent_core::usage::{ModelUsage, TurnUsage, usd_to_micros};
use serde_json::Value;

/// One session's running cost, for telling one turn's share from it.
#[derive(Debug, Default)]
pub struct Spend {
    /// The session's total as the latest `usage_update` stated it, in USD
    /// millionths. Only USD is read: summing two currencies is a guess.
    running: Option<i64>,
    /// The total when the last turn ended.
    settled: Option<i64>,
}

impl Spend {
    /// Fold one `session/update`'s params in.
    pub fn notice(&mut self, params: &Value) {
        let update = &params["update"];
        if update["sessionUpdate"] != "usage_update" || update["cost"]["currency"] != "USD" {
            return;
        }
        if let Some(micros) = update["cost"]["amount"].as_f64().and_then(usd_to_micros) {
            self.running = Some(micros);
        }
    }

    /// The turn a `session/prompt` result ends. `key` names it: ACP gives a
    /// turn no id, so the caller makes one that is new for every turn.
    pub fn ended(&mut self, key: String, result: &Value) -> TurnUsage {
        let cost = match (self.running, self.settled) {
            (Some(now), Some(before)) if now >= before => Some(now - before),
            (Some(now), None) => Some(now),
            _ => None,
        };
        self.settled = self.running;
        let models = result["usage"].as_object().map(|u| {
            let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
            vec![ModelUsage {
                model: None,
                input: n("inputTokens"),
                output: n("outputTokens"),
                cache_read: n("cachedReadTokens"),
                cache_write: n("cachedWriteTokens"),
                cache_write_1h: 0,
                reported_cost_micros: cost,
            }]
        });
        TurnUsage { key, models, active_ms: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(fixture: &str) -> Vec<Value> {
        let raw = std::fs::read_to_string(format!("{}/tests/fixtures/{fixture}", env!("CARGO_MANIFEST_DIR")))
            .unwrap();
        raw.lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    fn replay(fixture: &str) -> Vec<TurnUsage> {
        let mut spend = Spend::default();
        let mut turns = Vec::new();
        for frame in frames(fixture) {
            if frame["method"] == "session/update" {
                spend.notice(&frame["params"]);
            } else if frame["result"]["stopReason"].is_string() {
                turns.push(spend.ended(format!("t{}", turns.len()), &frame["result"]));
            }
        }
        turns
    }

    /// A recorded claude-agent-acp turn: its tokens and the cost its
    /// `usage_update` stated.
    #[test]
    fn a_recorded_turn_reports_its_tokens_and_cost() {
        let turns = replay("subagent_without_cap.jsonl");
        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].models,
            Some(vec![ModelUsage {
                model: None,
                input: 4,
                output: 383,
                cache_read: 40361,
                cache_write: 10351,
                cache_write_1h: 0,
                reported_cost_micros: Some(189_436),
            }])
        );
    }

    /// An adapter whose prompt result carries no usage reported none.
    #[test]
    fn a_turn_without_usage_is_not_reported() {
        let turns = replay("session_basic.jsonl");
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].models, None);
    }

    /// The session's running cost is split by turn, not charged whole to each.
    #[test]
    fn a_second_turn_is_charged_only_its_own_share_of_the_running_cost() {
        let mut spend = Spend::default();
        let cost = |amount: f64| {
            serde_json::json!({ "update": { "sessionUpdate": "usage_update", "used": 1, "size": 2,
                "cost": { "amount": amount, "currency": "USD" } } })
        };
        let result = serde_json::json!({ "stopReason": "end_turn", "usage": { "inputTokens": 1, "outputTokens": 1 } });
        spend.notice(&cost(0.25));
        spend.ended("a".into(), &result);
        spend.notice(&cost(0.40));
        let second = spend.ended("b".into(), &result);
        assert_eq!(second.models.unwrap()[0].reported_cost_micros, Some(150_000));
    }
}
