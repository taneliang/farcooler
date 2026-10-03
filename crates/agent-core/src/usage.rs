//! What one finished turn cost, as the agent itself reported it.
//!
//! Spend, not context fill. `AgentEvent::Usage` is how full the window is for
//! the meter, and it is resent as a turn runs; this is said once, when the
//! turn is over, and it is what the runner's store records for reports.
//!
//! Nothing here is estimated. A backend that heard no usage says so with
//! `models: None`, and pricing a turn from tokens is the daemon's job, where
//! the price table and its date live, so an estimate is never mistaken for
//! the agent's own number.

/// One turn's usage, split by the model that did the work.
///
/// Claude reports a turn per model (`modelUsage`), because a turn can spend
/// tokens on more than one: a subagent on a smaller model, a title on Haiku.
/// Codex and ACP name at most one.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TurnUsage {
    /// Stable for this turn wherever it is heard again: a shim replaying its
    /// ring after a reconnect sends the same event, and the store keeps one
    /// row per key. The harness's own turn id where it has one.
    pub key: String,
    /// `None` when the harness reported no usage for this turn at all: "not
    /// reported", never zero.
    pub models: Option<Vec<ModelUsage>>,
    /// How long the turn ran, by the harness's own clock, in milliseconds.
    pub active_ms: Option<u64>,
    /// The counts are a floor, not the whole turn: the harness could state
    /// only part of it (Claude's first turn after `--resume`, whose running
    /// totals include the spend restored with the session).
    #[serde(default)]
    pub partial: bool,
}

/// One model's share of a turn.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelUsage {
    /// As the harness named it. `None` when it named none.
    pub model: Option<String>,
    /// Input tokens that were neither read from nor written to a cache.
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    /// Every cache write, whatever its lifetime.
    pub cache_write: u64,
    /// The part of `cache_write` written to Claude's one-hour cache, which is
    /// priced higher than the five-minute one. Zero where the harness does
    /// not split them.
    pub cache_write_1h: u64,
    /// The cost the AGENT reported, in millionths of a US dollar. `None`
    /// when it reported none, which is most of the time outside Claude.
    pub reported_cost_micros: Option<i64>,
}

/// Dollars as the agents write them, to the integer millionths the store
/// adds up exactly. `None` for anything that is not a finite, non-negative
/// number.
pub fn usd_to_micros(usd: f64) -> Option<i64> {
    (usd.is_finite() && usd >= 0.0).then(|| (usd * 1_000_000.0).round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dollars_become_exact_millionths() {
        assert_eq!(usd_to_micros(0.080246), Some(80_246));
        assert_eq!(usd_to_micros(0.0), Some(0));
        assert_eq!(usd_to_micros(-1.0), None);
        assert_eq!(usd_to_micros(f64::NAN), None);
    }
}
