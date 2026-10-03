//! What agents spent, in words: the one wording `farcooler report`, the CLI's
//! `task usage` and the apps' Usage section share (ov-195).
//!
//! AgentKit's `TaskUsageFormat` (the Mac and iOS) and Android's `TaskUsage`
//! say the same things the same way, and all three are pinned by one fixture,
//! `test/fixtures/task-usage.json`. This side is en_US only, as the CLI is;
//! the apps format numbers for the reader's locale.
//!
//! **Every dollar is API-equivalent.** A cost the agent reported (Claude's
//! `costUSD`) and one estimated from the price table are both API list
//! prices: on a subscription that is notional, and on an API key it is what
//! was paid. Neither can be told from here, so a cost always says
//! "API-equivalent", says "estimated" or "partly estimated" when the table
//! made some of it, and "partly not reported" when some tokens have no known
//! price. A cost nobody knows is "Not reported", never a guessed number.

use serde::{Deserialize, Serialize};

/// Spend over some set of turns: `UsageTotals` on the wire, as JSON.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Spend {
    pub turns: u64,
    /// Turns whose tokens are a floor: some model call stated none.
    pub turns_partial: u64,
    /// Turns that stated no usage at all.
    pub turns_not_reported: u64,
    /// Claude subagent runs whose spend is included. Not turns.
    pub subagent_runs: u64,
    pub active_ms: i64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// Millionths of a US dollar, the agent's own figure.
    pub cost_reported_micros: i64,
    /// Millionths of a US dollar, from the price table.
    pub cost_estimated_micros: i64,
    /// Tokens with no known price.
    pub unpriced_tokens: u64,
    /// The price tables the estimates came from, oldest first.
    pub price_tables: Vec<String>,
}

/// One harness and model's share of a task's spend.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpendRow {
    pub harness: String,
    /// Empty when the harness named no model.
    pub model: String,
    pub totals: Spend,
}

/// One task's spend: what `farcooler task usage --json` prints and the
/// phones' `task.usage` route returns.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskSpend {
    /// The task's id.
    pub task: String,
    /// The runner's current price table.
    pub price_table: String,
    pub totals: Spend,
    pub by_harness_model: Vec<SpendRow>,
}

/// What an empty Usage section says.
pub const NOTHING_YET: &str = "No agent usage recorded yet.";

/// What a Usage section says for a runner too old to record spend.
pub const NEEDS_UPDATE: &str = "This runner needs an update to show spend.";

/// What a Usage section says when the read didn't come back, beside Try
/// Again.
pub const COULDNT_READ: &str = "Far Cooler couldn\u{2019}t read this task\u{2019}s usage.";

/// What "API-equivalent" means, said once under a task's spend total.
pub const API_EQUIVALENT: &str =
    "API-equivalent: what these tokens would cost at API list prices. On a subscription plan, you pay your plan\u{2019}s price instead.";

/// What a cost or a count nobody stated says.
pub const NOT_REPORTED: &str = "Not reported";

impl Spend {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_read_tokens + self.cache_write_tokens
    }

    /// Nothing recorded: no turn and no subagent run.
    pub fn is_empty(&self) -> bool {
        self.turns == 0 && self.subagent_runs == 0
    }

    fn priced_micros(&self) -> i64 {
        self.cost_reported_micros + self.cost_estimated_micros
    }

    /// Some of it has no known price, or no count at all.
    fn partly_unknown(&self) -> bool {
        self.unpriced_tokens > 0 || self.turns_partial > 0 || self.turns_not_reported > 0
    }

    /// "1.2M tokens", or "Not reported" when no turn stated any.
    pub fn tokens_line(&self) -> String {
        match self.total_tokens() {
            0 => NOT_REPORTED.to_string(),
            n => format!("{} tokens", tokens(n)),
        }
    }

    /// "12K input · 3.4K output · 1.1M cache", or nothing with no tokens.
    pub fn token_detail(&self) -> Option<String> {
        (self.total_tokens() > 0).then(|| {
            format!(
                "{} input · {} output · {} cache",
                tokens(self.input_tokens),
                tokens(self.output_tokens),
                tokens(self.cache_read_tokens + self.cache_write_tokens)
            )
        })
    }

    /// "$3.20 · API-equivalent", with "estimated", "partly estimated" and
    /// "partly not reported" as they apply; "Not reported" with no known
    /// cost.
    pub fn cost_line(&self) -> String {
        let priced = self.priced_micros();
        if priced <= 0 {
            return NOT_REPORTED.to_string();
        }
        let mut line = format!("{} · API-equivalent", dollars(priced));
        if self.cost_reported_micros == 0 {
            line.push_str(", estimated");
        } else if self.cost_estimated_micros > 0 {
            line.push_str(", partly estimated");
        }
        if self.partly_unknown() {
            line.push_str(", partly not reported");
        }
        line
    }

    /// A breakdown row's cost: "$3.20", "$0.42 estimated", "$3.20 partly
    /// not reported", or "Cost not reported". A row says what the total
    /// says about its own part, so a caveat is never only on the total.
    pub fn row_cost(&self) -> String {
        let priced = self.priced_micros();
        if priced <= 0 {
            return "Cost not reported".to_string();
        }
        let mut words = Vec::new();
        if self.cost_reported_micros == 0 {
            words.push("estimated");
        } else if self.cost_estimated_micros > 0 {
            words.push("partly estimated");
        }
        if self.partly_unknown() {
            words.push("partly not reported");
        }
        if words.is_empty() { dollars(priced) } else { format!("{} {}", dollars(priced), words.join(", ")) }
    }

    /// One line of a breakdown: "1.2M tokens · $3.20", or "Not reported"
    /// when it stated nothing.
    pub fn line_detail(&self) -> String {
        if self.total_tokens() == 0 && self.priced_micros() <= 0 {
            return NOT_REPORTED.to_string();
        }
        format!("{} tokens · {}", tokens(self.total_tokens()), self.row_cost())
    }

    /// "Agent time 3 h 10 min · 12 turns", either half alone, or nothing.
    pub fn time_line(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.active_ms > 0 {
            parts.push(format!("Agent time {}", duration(self.active_ms)));
        }
        match self.turns {
            0 => {}
            1 => parts.push("1 turn".to_string()),
            n => parts.push(format!("{n} turns")),
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

impl SpendRow {
    /// "claude · claude-opus-5", or the harness alone for an unnamed model.
    pub fn title(&self) -> String {
        if self.model.is_empty() { self.harness.clone() } else { format!("{} · {}", self.harness, self.model) }
    }

    /// "1.2M tokens · $3.20", or "Not reported" when it stated nothing.
    pub fn detail(&self) -> String {
        self.totals.line_detail()
    }
}

impl TaskSpend {
    /// The breakdown, most tokens first, then by title.
    pub fn rows(&self) -> Vec<&SpendRow> {
        let mut rows: Vec<&SpendRow> = self.by_harness_model.iter().collect();
        rows.sort_by(|a, b| {
            b.totals.total_tokens().cmp(&a.totals.total_tokens()).then_with(|| a.title().cmp(&b.title()))
        });
        rows
    }
}

/// A token count, short: "999", "1.2K", "12K", "1M", "2.1B". One decimal
/// below ten of a unit, none above; a count that rounds up to a thousand of
/// one unit is one of the next.
pub fn tokens(n: u64) -> String {
    const UNITS: [(f64, &str); 3] = [(1e3, "K"), (1e6, "M"), (1e9, "B")];
    if n < 1000 {
        return n.to_string();
    }
    let mut unit = UNITS.iter().rposition(|(scale, _)| n as f64 >= *scale).unwrap_or(0);
    loop {
        let (scale, suffix) = UNITS[unit];
        let v = n as f64 / scale;
        let digits = if v < 9.95 { 1 } else { 0 };
        let rounded = (v * 10f64.powi(digits)).round() / 10f64.powi(digits);
        if rounded >= 1000.0 && unit + 1 < UNITS.len() {
            unit += 1;
            continue;
        }
        let text = format!("{rounded:.*}", digits as usize);
        let text = text.strip_suffix(".0").unwrap_or(&text).to_string();
        return format!("{text}{suffix}");
    }
}

/// Millionths of a dollar as "$1,234.56"; above zero and below a cent,
/// "Under $0.01".
pub fn dollars(micros: i64) -> String {
    if micros > 0 && micros < 10_000 {
        return "Under $0.01".to_string();
    }
    let cents = (micros as f64 / 10_000.0).round() as i64;
    let whole = (cents / 100).to_string();
    let mut grouped = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("${grouped}.{:02}", cents % 100)
}

/// A total of agent time: "under a minute", "6 min", "3 h 10 min", and in
/// whole hours from ten hours on, since a sum of agent time is not a
/// stretch of calendar.
pub fn duration(ms: i64) -> String {
    const MINUTE: i64 = 60_000;
    let minutes = ms / MINUTE;
    match minutes {
        m if m < 1 => "under a minute".to_string(),
        m if m < 60 => format!("{m} min"),
        m if m < 600 => match m % 60 {
            0 => format!("{} h", m / 60),
            rest => format!("{} h {rest} min", m / 60),
        },
        m => format!("{} h", m / 60),
    }
}

#[cfg(test)]
#[path = "usage_words_tests.rs"]
mod tests;
