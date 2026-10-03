//! A turn's spend, from codex's `thread/tokenUsage/updated` notifications.
//!
//! `ThreadTokenUsageUpdatedNotification` in the vendored schema carries two
//! `TokenUsageBreakdown`s: `total`, the thread's running sum, and `last`, the
//! most recent model call. A turn is the difference between the thread's total
//! at its end and at its start. Differences rather than a sum of `last`,
//! because a notification heard twice would be counted twice by a sum and not
//! at all by a difference.
//!
//! Codex reports no cost anywhere in the app-server protocol, so none is read.

use farcooler_agent_core::usage::{ModelUsage, TurnUsage};
use serde_json::Value;

/// The token counts in one `TokenUsageBreakdown`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Breakdown {
    /// Every input token, cached or not: OpenAI's convention, which codex
    /// keeps (`inputTokens` 18417 with `cachedInputTokens` 18176 in the
    /// recorded turn).
    input: u64,
    cached: u64,
    cache_write: u64,
    output: u64,
    total: u64,
}

impl Breakdown {
    fn read(v: &Value) -> Option<Breakdown> {
        let n = |k: &str| v[k].as_u64();
        Some(Breakdown {
            input: n("inputTokens")?,
            cached: n("cachedInputTokens").unwrap_or(0),
            cache_write: n("cacheWriteInputTokens").unwrap_or(0),
            output: n("outputTokens")?,
            total: n("totalTokens").unwrap_or(0),
        })
    }

    fn minus(self, earlier: Breakdown) -> Breakdown {
        Breakdown {
            input: self.input.saturating_sub(earlier.input),
            cached: self.cached.saturating_sub(earlier.cached),
            cache_write: self.cache_write.saturating_sub(earlier.cache_write),
            output: self.output.saturating_sub(earlier.output),
            total: self.total.saturating_sub(earlier.total),
        }
    }
}

#[derive(Debug)]
struct Open {
    turn: String,
    baseline: Breakdown,
    latest: Breakdown,
}

/// Follows one thread's running total across its turns.
#[derive(Debug, Default)]
pub struct TokenLedger {
    /// The thread's total when the last counted turn ended.
    settled: Option<Breakdown>,
    open: Option<Open>,
}

impl TokenLedger {
    /// Fold one notification in. Returns the turn's usage on its
    /// `turn/completed`, credited to `model`, the model the turn ran on.
    pub fn observe(&mut self, method: &str, params: &Value, model: Option<&str>) -> Option<TurnUsage> {
        match method {
            "thread/tokenUsage/updated" => {
                self.updated(params);
                None
            }
            "turn/completed" => Some(self.completed(params, model)),
            _ => None,
        }
    }

    fn updated(&mut self, params: &Value) {
        let (Some(turn), Some(total)) =
            (params["turnId"].as_str(), Breakdown::read(&params["tokenUsage"]["total"]))
        else {
            return;
        };
        match &mut self.open {
            Some(open) if open.turn == turn => {
                if total.total >= open.latest.total {
                    open.latest = total;
                }
            }
            _ => {
                // The thread's total before this turn's first call: what the
                // last counted turn left, or, on a pane that joined mid-thread,
                // this total less the call it reports.
                let last = Breakdown::read(&params["tokenUsage"]["last"]).unwrap_or_default();
                let baseline = self.settled.unwrap_or_else(|| total.minus(last));
                self.open = Some(Open { turn: turn.to_string(), baseline, latest: total });
            }
        }
    }

    fn completed(&mut self, params: &Value, model: Option<&str>) -> TurnUsage {
        let turn = params["turn"]["id"].as_str().unwrap_or_default();
        let open = self.open.take().filter(|open| open.turn == turn);
        let models = open.map(|open| {
            self.settled = Some(open.latest);
            let d = open.latest.minus(open.baseline);
            vec![ModelUsage {
                model: model.map(str::to_string),
                input: d.input.saturating_sub(d.cached).saturating_sub(d.cache_write),
                output: d.output,
                cache_read: d.cached,
                cache_write: d.cache_write,
                cache_write_1h: 0,
                reported_cost_micros: None,
            }]
        });
        TurnUsage {
            key: format!("codex:{turn}"),
            models,
            active_ms: params["turn"]["durationMs"].as_u64(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every notification of the recorded turn (`tests/fixtures/turn_basic.jsonl`,
    /// codex 0.146.0), in order.
    fn recorded() -> Vec<(String, Value)> {
        include_str!("../tests/fixtures/turn_basic.jsonl")
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .filter_map(|f| Some((f["method"].as_str()?.to_string(), f["params"].clone())))
            .collect()
    }

    fn run(ledger: &mut TokenLedger, frames: &[(String, Value)]) -> Vec<TurnUsage> {
        frames.iter().filter_map(|(m, p)| ledger.observe(m, p, Some("gpt-5.6-luna"))).collect()
    }

    #[test]
    fn a_recorded_turn_is_its_threads_total_split_into_cached_and_not() {
        let turns = run(&mut TokenLedger::default(), &recorded());
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].key, "codex:019fe879-657e-7b90-a8e8-007dfeec7a4a");
        assert_eq!(turns[0].active_ms, Some(1361));
        assert_eq!(
            turns[0].models,
            Some(vec![ModelUsage {
                model: Some("gpt-5.6-luna".into()),
                input: 241,
                output: 5,
                cache_read: 18176,
                cache_write: 0,
                cache_write_1h: 0,
                reported_cost_micros: None,
            }])
        );
    }

    /// The same usage notification heard twice is the same total, not twice
    /// the tokens.
    #[test]
    fn a_repeated_update_is_not_counted_twice() {
        let mut frames = recorded();
        let at = frames.iter().position(|(m, _)| m == "thread/tokenUsage/updated").unwrap();
        let again = frames[at].clone();
        frames.insert(at + 1, again);
        let turns = run(&mut TokenLedger::default(), &frames);
        assert_eq!(turns[0].models.as_ref().unwrap()[0].cache_read, 18176);
    }

    /// The second turn of a thread is its own spend, not the thread's.
    #[test]
    fn a_second_turn_counts_from_where_the_first_left_off() {
        let mut ledger = TokenLedger::default();
        run(&mut ledger, &recorded());
        let update = |turn: &str, total: u64, input: u64| {
            serde_json::json!({ "turnId": turn, "tokenUsage": {
                "total": { "totalTokens": total, "inputTokens": input, "cachedInputTokens": 18176, "outputTokens": total - input, "reasoningOutputTokens": 0 },
                "last": { "totalTokens": 1, "inputTokens": 1, "cachedInputTokens": 0, "outputTokens": 0, "reasoningOutputTokens": 0 }
            }})
        };
        let frames = vec![
            ("thread/tokenUsage/updated".to_string(), update("t2", 18522, 18507)),
            ("turn/completed".to_string(), serde_json::json!({ "turn": { "id": "t2", "durationMs": 10 } })),
        ];
        let turns = run(&mut ledger, &frames);
        let m = &turns[0].models.as_ref().unwrap()[0];
        assert_eq!((m.input, m.output, m.cache_read), (90, 10, 0));
    }

    /// A turn that ended with no usage notification reported nothing.
    #[test]
    fn a_turn_without_a_usage_notification_is_not_reported() {
        let frames: Vec<_> =
            recorded().into_iter().filter(|(m, _)| m != "thread/tokenUsage/updated").collect();
        let turns = run(&mut TokenLedger::default(), &frames);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].models, None);
    }
}
