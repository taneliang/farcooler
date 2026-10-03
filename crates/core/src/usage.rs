//! Tokens, and what they would cost at API prices.
//!
//! An ESTIMATE, and labeled one everywhere it travels: a turn's cost is
//! "reported" when the agent stated it, "estimated" when it is these tokens
//! times the rates below, and "unknown" when there is no model or no rate. A
//! subscription (Claude Max, ChatGPT Pro) pays none of this per token, so for
//! most people every figure here is notional: what the same work would have
//! cost at the API's list price, never what was spent.
//!
//! The table is dated, and the date is written beside every estimate made
//! from it ([`PRICE_TABLE`]), so a row priced under old rates keeps them when
//! the table changes rather than being silently re-priced.

/// Which price table an estimate was made from: the date its rates were
/// read. Bump it whenever a rate changes or a model is added.
pub const PRICE_TABLE: &str = "2026-09-25";

/// One turn's tokens for one model, in the store's four buckets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenCounts {
    /// Input neither read from nor written to a cache.
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    /// Every cache write.
    pub cache_write: u64,
    /// The part of `cache_write` that went to the one-hour cache. Priced
    /// higher; zero where the harness does not split them.
    pub cache_write_1h: u64,
}

impl TokenCounts {
    pub fn add(&mut self, other: &TokenCounts) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.cache_write_1h += other.cache_write_1h;
    }
}

/// Where a cost came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CostSource {
    /// The agent stated it.
    Reported,
    /// Tokens times [`rates`], from the table named by [`PRICE_TABLE`].
    Estimated,
    /// No model was named, or the table has no rate for it.
    Unknown,
}

impl CostSource {
    pub fn as_str(self) -> &'static str {
        match self {
            CostSource::Reported => "reported",
            CostSource::Estimated => "estimated",
            CostSource::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> CostSource {
        match s {
            "reported" => CostSource::Reported,
            "estimated" => CostSource::Estimated,
            _ => CostSource::Unknown,
        }
    }
}

/// List prices in US dollars per million tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rates {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    /// The five-minute cache: 1.25 times input on every Claude model.
    pub cache_write: f64,
    /// The one-hour cache: twice input on every Claude model.
    pub cache_write_1h: f64,
}

const fn claude(input: f64, output: f64, cache_read: f64) -> Rates {
    Rates { input, output, cache_read, cache_write: input * 1.25, cache_write_1h: input * 2.0 }
}

/// Anthropic's first-party list prices, as of [`PRICE_TABLE`].
///
/// Two rows are checked against an agent's own arithmetic rather than taken
/// on trust: `claude-opus-5` reproduces the `costUSD` of the recorded turn in
/// `crates/claude/tests/fixtures/turn_basic.jsonl` to the millionth (see the
/// test below), and `claude-haiku-4-5` a session's `cost-state` record.
///
/// No OpenAI model is listed. Codex runs gpt-5.5 and gpt-5.6 models whose
/// prices nobody here has a source for, and a rate typed from memory is a
/// guess wearing a number's clothes: their turns are recorded as unknown.
const TABLE: &[(&str, Rates)] = &[
    ("claude-fable-5-1", claude(10.0, 50.0, 0.25)),
    ("claude-fable-5", claude(10.0, 50.0, 1.0)),
    ("claude-opus-5-5", claude(4.0, 20.0, 0.20)),
    ("claude-opus-5", claude(5.0, 25.0, 0.50)),
    ("claude-opus-4-8", claude(5.0, 25.0, 0.50)),
    ("claude-opus-4-7", claude(5.0, 25.0, 0.50)),
    ("claude-opus-4-6", claude(5.0, 25.0, 0.50)),
    ("claude-sonnet-5-5", claude(2.0, 10.0, 0.20)),
    ("claude-sonnet-5", claude(2.0, 10.0, 0.20)),
    ("claude-sonnet-4-6", claude(3.0, 15.0, 0.30)),
    ("claude-haiku-4-5", claude(1.0, 5.0, 0.10)),
];

/// The rates for `model`, if the table has them.
///
/// Matched on the model's base name: the context-window tag Claude Code
/// appends (`claude-opus-5[1m]`) and a release date (`claude-haiku-4-5-20251001`)
/// are dropped. The 1M tag changes no rate at these prices; a long-context
/// premium, where one applies, is not modeled, so an estimate can be low.
pub fn rates(model: &str) -> Option<Rates> {
    let base = model.split('[').next().unwrap_or(model).trim().to_ascii_lowercase();
    let base = match base.rsplit_once('-') {
        Some((head, date)) if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) => head.to_string(),
        _ => base,
    };
    TABLE.iter().find(|(name, _)| *name == base).map(|(_, rates)| *rates)
}

/// What `tokens` would cost on `model` at list price, in millionths of a
/// dollar. `None` when there is no model or no rate for it.
pub fn estimate(model: Option<&str>, tokens: &TokenCounts) -> Option<i64> {
    let r = rates(model?)?;
    let short_writes = tokens.cache_write.saturating_sub(tokens.cache_write_1h);
    let long_writes = tokens.cache_write_1h.min(tokens.cache_write);
    // Dollars per million tokens is exactly micro-dollars per token.
    let micros = tokens.input as f64 * r.input
        + tokens.output as f64 * r.output
        + tokens.cache_read as f64 * r.cache_read
        + short_writes as f64 * r.cache_write
        + long_writes as f64 * r.cache_write_1h;
    Some(micros.round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The recorded turn's own `costUSD` was 0.080246: 2 input, 4 output,
    /// 19,912 cache reads and 7,018 one-hour cache writes on claude-opus-5.
    #[test]
    fn the_table_reproduces_a_recorded_claude_cost() {
        let tokens =
            TokenCounts { input: 2, output: 4, cache_read: 19912, cache_write: 7018, cache_write_1h: 7018 };
        assert_eq!(estimate(Some("claude-opus-5[1m]"), &tokens), Some(80_246));
    }

    /// A session's `cost-state` record: 1,169 input and 13 output tokens on
    /// Haiku, `costUSD` 0.001234.
    #[test]
    fn a_dated_model_name_finds_its_rates() {
        let tokens = TokenCounts { input: 1169, output: 13, ..Default::default() };
        assert_eq!(estimate(Some("claude-haiku-4-5-20251001"), &tokens), Some(1234));
    }

    #[test]
    fn five_minute_writes_cost_less_than_one_hour_writes() {
        let short = TokenCounts { cache_write: 1_000_000, ..Default::default() };
        let long = TokenCounts { cache_write: 1_000_000, cache_write_1h: 1_000_000, ..Default::default() };
        assert_eq!(estimate(Some("claude-opus-5"), &short), Some(6_250_000));
        assert_eq!(estimate(Some("claude-opus-5"), &long), Some(10_000_000));
    }

    /// No model, or a model with no rate, is unknown: never a guess.
    #[test]
    fn an_unpriced_or_unnamed_model_has_no_estimate() {
        let tokens = TokenCounts { input: 100, ..Default::default() };
        assert_eq!(estimate(Some("gpt-5.6-luna"), &tokens), None);
        assert_eq!(estimate(None, &tokens), None);
        assert_eq!(estimate(Some("<synthetic>"), &tokens), None);
    }
}
