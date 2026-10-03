//! A turn's spend, read off the stream-json `result` frames of one process.
//!
//! A chat pane is one long-lived `claude --print` process running many turns,
//! and a `result`'s `modelUsage` and `total_cost_usd` are RUNNING totals for
//! that whole process, plus any spend restored when it resumed a session
//! (Agent SDK, "Track costs in streaming input mode": summing results
//! double-counts). So a turn is this result's totals less the previous
//! result's, per model, tokens and `costUSD` alike — the same difference the
//! codex ledger and ACP's running cost take. Only `usage` is per turn, and it
//! covers the main loop alone, without subagents.
//!
//! The first result of a resumed process has no previous one to subtract,
//! and its totals carry the restored spend. That turn is recorded from its
//! per-turn `usage`, on the model the turn's own messages named, with no
//! reported cost (the daemon estimates one), and marked partial: the
//! running total is never recorded as a turn.

use std::collections::BTreeMap;

use farcooler_agent_core::usage::{ModelUsage, TurnUsage, usd_to_micros};
use serde_json::Value;

/// One model's running totals.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Running {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    cost_micros: Option<i64>,
}

impl Running {
    fn read(u: &Value) -> Running {
        let n = |k: &str| u[k].as_u64().unwrap_or(0);
        Running {
            input: n("inputTokens"),
            output: n("outputTokens"),
            cache_read: n("cacheReadInputTokens"),
            cache_write: n("cacheCreationInputTokens"),
            cost_micros: u["costUSD"].as_f64().and_then(usd_to_micros),
        }
    }

    /// Whether `self` could have grown out of `earlier`. A total that went
    /// down is a different process's, so it is not a difference at all.
    fn follows(&self, earlier: &Running) -> bool {
        self.input >= earlier.input
            && self.output >= earlier.output
            && self.cache_read >= earlier.cache_read
            && self.cache_write >= earlier.cache_write
    }
}

/// One process's spend so far, for telling each turn's share from it.
#[derive(Debug, Default)]
pub struct Ledger {
    /// The running totals at the last result, by model. `None` before the
    /// first result.
    last: Option<BTreeMap<String, Running>>,
    /// `total_cost_usd` at the last result, for a frame with no split.
    last_total_cost: Option<i64>,
    /// The process resumed a session, so its first totals are not its own.
    resumed: bool,
    /// The model the current turn's messages named, for a turn read from
    /// its `usage`.
    model: Option<String>,
}

impl Ledger {
    /// A ledger for a process started with `--resume`.
    pub fn resumed() -> Ledger {
        Ledger { resumed: true, ..Ledger::default() }
    }

    /// Fold one frame in: the turn's usage on a `result`, else `None`.
    pub fn observe(&mut self, frame: &Value) -> Option<TurnUsage> {
        match frame["type"].as_str() {
            Some("assistant") => {
                if let Some(model) = frame["message"]["model"].as_str().filter(|m| *m != "<synthetic>") {
                    self.model = Some(model.to_string());
                }
                None
            }
            Some("result") => Some(self.result(frame)),
            _ => None,
        }
    }

    fn result(&mut self, frame: &Value) -> TurnUsage {
        let first = self.last.is_none();
        let split: Option<BTreeMap<String, Running>> = frame["modelUsage"]
            .as_object()
            .filter(|m| !m.is_empty())
            .map(|m| m.iter().map(|(model, u)| (model.clone(), Running::read(u))).collect());
        let total_cost = frame["total_cost_usd"].as_f64().and_then(usd_to_micros);
        let restored = first && self.resumed;

        let models = if restored {
            per_turn(frame, self.model.clone())
        } else if let Some(now) = &split {
            let before = self.last.clone().unwrap_or_default();
            Some(difference(now, &before))
        } else {
            per_turn(frame, None).map(|mut models| {
                let cost = match (total_cost, self.last_total_cost) {
                    (Some(now), Some(before)) if now >= before => Some(now - before),
                    (Some(now), None) => Some(now),
                    _ => None,
                };
                models[0].reported_cost_micros = cost;
                models
            })
        };

        if let Some(now) = split {
            self.last = Some(now);
        } else if self.last.is_none() {
            self.last = Some(BTreeMap::new());
        }
        self.last_total_cost = total_cost.or(self.last_total_cost);
        self.model = None;
        TurnUsage { key: key(frame), models, active_ms: frame["duration_ms"].as_u64(), partial: restored }
    }
}

/// Each model's growth since `before`. A model whose totals fell is a reset,
/// and counts from zero; a model that did not move did no work this turn.
fn difference(now: &BTreeMap<String, Running>, before: &BTreeMap<String, Running>) -> Vec<ModelUsage> {
    now.iter()
        .filter_map(|(model, n)| {
            let b = before.get(model).filter(|b| n.follows(b)).copied().unwrap_or_default();
            let cost = match (n.cost_micros, b.cost_micros) {
                (Some(n), Some(b)) if n >= b => Some(n - b),
                (Some(n), None) => Some(n),
                _ => None,
            };
            let usage = ModelUsage {
                model: Some(model.clone()),
                input: n.input - b.input,
                output: n.output - b.output,
                cache_read: n.cache_read - b.cache_read,
                cache_write: n.cache_write - b.cache_write,
                cache_write_1h: 0,
                reported_cost_micros: cost,
            };
            let moved = usage.input + usage.output + usage.cache_read + usage.cache_write > 0;
            moved.then_some(usage)
        })
        .collect()
}

/// The turn's own `usage` (main loop only), with no cost: the caller decides
/// what it knows about that.
fn per_turn(frame: &Value, model: Option<String>) -> Option<Vec<ModelUsage>> {
    let u = frame["usage"].as_object()?;
    let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
    Some(vec![ModelUsage {
        model,
        input: n("input_tokens"),
        output: n("output_tokens"),
        cache_read: n("cache_read_input_tokens"),
        cache_write: n("cache_creation_input_tokens"),
        cache_write_1h: u
            .get("cache_creation")
            .and_then(|c| c["ephemeral_1h_input_tokens"].as_u64())
            .unwrap_or(0),
        reported_cost_micros: None,
    }])
}

/// The result's own `uuid`, which a replay of the same frame repeats. A frame
/// without one is keyed by its content, which a replay also repeats.
fn key(frame: &Value) -> String {
    match frame["uuid"].as_str() {
        Some(uuid) if !uuid.is_empty() => format!("claude:{uuid}"),
        _ => format!("claude:{}", fnv(frame.to_string().as_bytes())),
    }
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
#[path = "usage_tests.rs"]
mod tests;
