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
pub const PRICE_TABLE: &str = "2026-10-10";

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
    /// The turn ran in fast mode (`usage.speed` was `fast`), which bills a
    /// premium on the models that offer it ([`FAST_MODE`]). Not a count:
    /// totals carry whether any part was fast, and only a priced row means it.
    pub fast: bool,
}

impl TokenCounts {
    pub fn add(&mut self, other: &TokenCounts) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.cache_write_1h += other.cache_write_1h;
        self.fast |= other.fast;
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

impl Rates {
    /// Every rate times `k`: fast mode's premium, which the docs apply to
    /// input and output and, on top of it, to the cache multipliers.
    fn times(self, k: f64) -> Rates {
        Rates {
            input: self.input * k,
            output: self.output * k,
            cache_read: self.cache_read * k,
            cache_write: self.cache_write * k,
            cache_write_1h: self.cache_write_1h * k,
        }
    }
}

/// One row of the docs table, in its column order: base input, 5m cache
/// write, 1h cache write, cache hit, output. Typed as the page prints them so
/// `scripts/claude-prices.py` can check them against it, one for one.
const fn rate(input: f64, cache_write: f64, cache_write_1h: f64, cache_read: f64, output: f64) -> Rates {
    Rates { input, output, cache_read, cache_write, cache_write_1h }
}

/// Anthropic's first-party list prices, as of [`PRICE_TABLE`], from
/// platform.claude.com/docs/en/about-claude/pricing.
///
/// `scripts/claude-prices.py` prints these rows from the page and `--check`s
/// them (a weekly CI job runs it), so the rows keep one shape: a model's id,
/// then `rate(..)`. Edit them by running the script, not by hand.
///
/// Two rows are checked against an agent's own arithmetic rather than taken
/// on trust: `claude-opus-5-5` reproduces Claude Code's `costUSD` for a turn
/// that wrote one-hour caches, and `claude-haiku-4-5` a session's `cost-state`
/// record (see the tests below).
///
/// Claude Haiku 5.5 is priced by prompt size, per request: up to 100,000
/// tokens, or over. A turn here is the sum of many requests and the store
/// keeps no request's size, so the row is the OVER-100,000 tier: an estimate
/// for it can be high, never low. (The cheaper tier is $0.10 in, $0.50 out,
/// $0.01 cache hit, $0.125 5m and $0.20 1h writes.)
///
/// No OpenAI model is listed. Codex runs gpt-5.5 and gpt-5.6 models whose
/// prices nobody here has a source for, and a rate typed from memory is a
/// guess wearing a number's clothes: their turns are recorded as unknown.
const TABLE: &[(&str, Rates)] = &[
    ("claude-fable-5-1", rate(10.0, 12.5, 20.0, 0.25, 50.0)),
    ("claude-mythos-5-1", rate(10.0, 12.5, 20.0, 0.25, 50.0)),
    ("claude-fable-5", rate(10.0, 12.5, 20.0, 1.0, 50.0)),
    ("claude-mythos-5", rate(10.0, 12.5, 20.0, 1.0, 50.0)),
    ("claude-opus-5-5", rate(4.0, 5.0, 8.0, 0.2, 20.0)),
    ("claude-opus-5", rate(5.0, 6.25, 10.0, 0.5, 25.0)),
    ("claude-opus-4-8", rate(5.0, 6.25, 10.0, 0.5, 25.0)),
    ("claude-opus-4-7", rate(5.0, 6.25, 10.0, 0.5, 25.0)),
    ("claude-opus-4-6", rate(5.0, 6.25, 10.0, 0.5, 25.0)),
    ("claude-opus-4-5", rate(5.0, 6.25, 10.0, 0.5, 25.0)),
    ("claude-opus-4-1", rate(15.0, 18.75, 30.0, 1.5, 75.0)),
    ("claude-opus-4", rate(15.0, 18.75, 30.0, 1.5, 75.0)),
    ("claude-sonnet-5-5", rate(2.0, 2.5, 4.0, 0.1, 10.0)),
    ("claude-sonnet-5", rate(2.0, 2.5, 4.0, 0.2, 10.0)),
    ("claude-sonnet-4-6", rate(3.0, 3.75, 6.0, 0.3, 15.0)),
    ("claude-sonnet-4-5", rate(3.0, 3.75, 6.0, 0.3, 15.0)),
    ("claude-sonnet-4", rate(3.0, 3.75, 6.0, 0.3, 15.0)),
    ("claude-haiku-5-5", rate(0.5, 0.625, 1.0, 0.05, 2.5)),
    ("claude-haiku-4-5", rate(1.0, 1.25, 2.0, 0.1, 5.0)),
    ("claude-haiku-3-5", rate(0.8, 1.0, 1.6, 0.08, 4.0)),
];

/// The models that offer fast mode, at twice the rates above (the docs'
/// fast-mode table: Opus 5.5 at $8 and $40, Opus 5 and 4.8 at $10 and $50).
/// Opus 4.6 runs at standard speed and standard rates when asked for fast.
///
/// Fast mode shows in a Claude Code transcript as `usage.speed: "fast"` on
/// each request. A harness that reports no such field (a chat pane's running
/// `modelUsage` totals; Codex; ACP) is priced as standard, which can be low.
const FAST_MODE: &[&str] = &["claude-opus-5-5", "claude-opus-5", "claude-opus-4-8"];

/// The rates for `model`, if the table has them.
///
/// Matched on the model's base name: the context-window tag Claude Code
/// appends (`claude-opus-5[1m]`) and a release date (`claude-haiku-4-5-20251001`)
/// are dropped. The 1M tag changes no rate at these prices; a long-context
/// premium, where one applies, is not modeled, so an estimate can be low.
pub fn rates(model: &str) -> Option<Rates> {
    let (rates, _) = lookup(model)?;
    Some(rates)
}

/// The rates and the model's table name, whose `FAST_MODE` entry (if any)
/// says whether fast mode bills a premium.
fn lookup(model: &str) -> Option<(Rates, &'static str)> {
    let base = model.split('[').next().unwrap_or(model).trim().to_ascii_lowercase();
    let base = match base.rsplit_once('-') {
        Some((head, date)) if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) => head.to_string(),
        _ => base,
    };
    // The id Anthropic gave Haiku 3.5 puts the version first.
    let base = if base == "claude-3-5-haiku" { "claude-haiku-3-5".to_string() } else { base };
    TABLE.iter().find(|(name, _)| *name == base).map(|(name, rates)| (*rates, *name))
}

/// What `tokens` would cost on `model` at list price, in millionths of a
/// dollar. `None` when there is no model or no rate for it.
pub fn estimate(model: Option<&str>, tokens: &TokenCounts) -> Option<i64> {
    let (r, name) = lookup(model?)?;
    let r = if tokens.fast && FAST_MODE.contains(&name) { r.times(2.0) } else { r };
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
            TokenCounts { input: 2, output: 4, cache_read: 19912, cache_write: 7018, cache_write_1h: 7018, fast: false };
        assert_eq!(estimate(Some("claude-opus-5[1m]"), &tokens), Some(80_246));
    }

    /// Claude Code's own `costUSD` for an Opus 5.5 turn was 0.2510482: 4 input,
    /// 70 output, 51,401 cache reads and 29,919 one-hour cache writes. Priced
    /// as five-minute writes it would have been $0.1613, a third low.
    #[test]
    fn an_opus_5_5_turn_with_one_hour_writes_reproduces_claude_codes_cost() {
        let tokens =
            TokenCounts { input: 4, output: 70, cache_read: 51401, cache_write: 29919, cache_write_1h: 29919, fast: false };
        assert_eq!(estimate(Some("claude-opus-5-5"), &tokens), Some(251_048));
        let short = TokenCounts { cache_write_1h: 0, ..tokens };
        assert_eq!(estimate(Some("claude-opus-5-5"), &short), Some(16 + 1400 + 10_280 + 149_595));
    }

    /// Every row of the docs table of 2026-10-10 (Haiku 5.5 at its over-100k
    /// tier), as the page prints it: input, 5m write, 1h write, hit, output.
    #[test]
    fn every_model_on_the_pricing_page_has_its_listed_rates() {
        let docs: &[(&str, [f64; 5])] = &[
            ("claude-fable-5-1", [10.0, 12.5, 20.0, 0.25, 50.0]),
            ("claude-mythos-5-1", [10.0, 12.5, 20.0, 0.25, 50.0]),
            ("claude-fable-5", [10.0, 12.5, 20.0, 1.0, 50.0]),
            ("claude-mythos-5", [10.0, 12.5, 20.0, 1.0, 50.0]),
            ("claude-opus-5-5", [4.0, 5.0, 8.0, 0.2, 20.0]),
            ("claude-opus-5", [5.0, 6.25, 10.0, 0.5, 25.0]),
            ("claude-opus-4-8", [5.0, 6.25, 10.0, 0.5, 25.0]),
            ("claude-opus-4-7", [5.0, 6.25, 10.0, 0.5, 25.0]),
            ("claude-opus-4-6", [5.0, 6.25, 10.0, 0.5, 25.0]),
            ("claude-opus-4-5", [5.0, 6.25, 10.0, 0.5, 25.0]),
            ("claude-opus-4-1", [15.0, 18.75, 30.0, 1.5, 75.0]),
            ("claude-opus-4", [15.0, 18.75, 30.0, 1.5, 75.0]),
            ("claude-sonnet-5-5", [2.0, 2.5, 4.0, 0.1, 10.0]),
            ("claude-sonnet-5", [2.0, 2.5, 4.0, 0.2, 10.0]),
            ("claude-sonnet-4-6", [3.0, 3.75, 6.0, 0.3, 15.0]),
            ("claude-sonnet-4-5", [3.0, 3.75, 6.0, 0.3, 15.0]),
            ("claude-sonnet-4", [3.0, 3.75, 6.0, 0.3, 15.0]),
            ("claude-haiku-5-5", [0.5, 0.625, 1.0, 0.05, 2.5]),
            ("claude-haiku-4-5", [1.0, 1.25, 2.0, 0.1, 5.0]),
            ("claude-haiku-3-5", [0.8, 1.0, 1.6, 0.08, 4.0]),
        ];
        assert_eq!(docs.len(), TABLE.len(), "a row in one list and not the other");
        for (model, [input, write, write_1h, read, output]) in docs {
            let r = rates(model).unwrap_or_else(|| panic!("{model} has no row"));
            assert_eq!(
                (r.input, r.cache_write, r.cache_write_1h, r.cache_read, r.output),
                (*input, *write, *write_1h, *read, *output),
                "{model}"
            );
        }
    }

    /// Fast mode, from the docs' own table: Opus 5.5 at $8 and $40, Opus 5
    /// and 4.8 at $10 and $50; the cache multipliers stack on top.
    #[test]
    fn fast_mode_doubles_the_models_that_offer_it_and_no_others() {
        let million = |fast| TokenCounts { input: 1_000_000, output: 1_000_000, fast, ..Default::default() };
        assert_eq!(estimate(Some("claude-opus-5-5"), &million(true)), Some(48_000_000));
        assert_eq!(estimate(Some("claude-opus-5-5"), &million(false)), Some(24_000_000));
        assert_eq!(estimate(Some("claude-opus-5"), &million(true)), Some(60_000_000));
        assert_eq!(estimate(Some("claude-opus-4-8[1m]"), &million(true)), Some(60_000_000));
        // Opus 4.6 and Sonnet run at standard speed and rates.
        assert_eq!(estimate(Some("claude-opus-4-6"), &million(true)), Some(30_000_000));
        assert_eq!(estimate(Some("claude-sonnet-5-5"), &million(true)), Some(12_000_000));
        let cached = TokenCounts { cache_read: 1_000_000, cache_write: 1_000_000, fast: true, ..Default::default() };
        assert_eq!(estimate(Some("claude-opus-5-5"), &cached), Some(400_000 + 10_000_000));
    }

    #[test]
    fn haiku_3_5_is_found_under_its_anthropic_id() {
        let tokens = TokenCounts { input: 1_000_000, ..Default::default() };
        assert_eq!(estimate(Some("claude-3-5-haiku-20241022"), &tokens), Some(800_000));
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
