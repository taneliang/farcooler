//! Cost on the plan (ov-307): a token budget on a theme or a lane, the
//! seven-day trend, and cost per finished card by harness and model, in words
//! an orchestrator reads as the owner sees them.
//!
//! Behind `board_cost` as well as the plan: `--budget` is refused before
//! anything is sent by a runner that doesn't keep budgets, and the reads simply
//! draw nothing from one that doesn't carry them.
//!
//! **Tokens first.** A budget is a number of tokens, counted as a lane's spend
//! counts them (input, output and both cache counts). Dollars follow, and
//! every one is API-equivalent. There is no weekly-limit percentage because
//! the runner can't know the limit: see `farcooler_store::plan_cost`.

use farcooler_core::usage_words::{dollars, tokens};
use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb};

use super::{DispatchLink, Failed, Refused, count};

/// A harness as a person names it.
pub(super) fn harness_name(harness: &str) -> String {
    match harness {
        "claude" => "Claude Code".into(),
        "codex" => "Codex".into(),
        "cursor" => "Cursor".into(),
        "opencode" => "opencode".into(),
        other => {
            let mut chars = other.chars();
            chars.next().map_or_else(|| "Unknown".into(), |c| c.to_uppercase().chain(chars).collect())
        }
    }
}

/// `--budget 5000000`, `--budget 800k` or `--budget 5M`: a whole number of
/// tokens, at least one, and a K or M or B at the end that means thousands,
/// millions or billions.
pub(super) fn parse_budget(raw: &str) -> Result<u64, String> {
    let raw = raw.trim().replace([',', '_'], "");
    let (digits, scale) = match raw.chars().last().map(|c| c.to_ascii_lowercase()) {
        Some('k') => (&raw[..raw.len() - 1], 1_000u64),
        Some('m') => (&raw[..raw.len() - 1], 1_000_000),
        Some('b') => (&raw[..raw.len() - 1], 1_000_000_000),
        _ => (raw.as_str(), 1),
    };
    let said = "A budget is a number of tokens, like 5000000 or 5M.";
    // A decimal such as 1.5M is fine; the product must still be whole tokens.
    let n: f64 = digits.parse().map_err(|_| said.to_string())?;
    let total = n * scale as f64;
    if !total.is_finite() || total < 1.0 || total.fract() != 0.0 || total > 1e14 {
        return Err(said.to_string());
    }
    Ok(total as u64)
}

/// Refused before anything is sent, so an older runner never drops the field.
pub(super) fn needs_cost<L: DispatchLink>(link: &L) -> Result<(), Failed> {
    if link.capabilities().iter().any(|c| c == capability::BOARD_COST) {
        return Ok(());
    }
    Err(Box::new(Refused::new(
        "This runner needs an update to keep budgets.".to_string(),
        Some(pb::ErrorCode::CapabilityUnsupported as i32),
    )))
}

/// What `--budget` and `--no-budget` ask: `Some(tokens)` to set, `Some(0)` to
/// take it away, `None` to leave it.
pub(super) fn asked(budget: Option<u64>, no_budget: bool) -> Option<u64> {
    if no_budget { Some(0) } else { budget }
}

fn total(s: &pb::LaneSpend) -> u64 {
    s.input_tokens + s.output_tokens + s.cache_read_tokens + s.cache_write_tokens
}

/// Whether `spend` is past its `budget`. Equal is within it.
pub(super) fn over(spend: &pb::LaneSpend, budget: Option<u64>) -> bool {
    budget.is_some_and(|b| total(spend) > b)
}

/// "1.2M of 5M tokens", or "Over budget: 6.1M of 5M tokens". Nothing without
/// a budget.
pub(super) fn budget_words(spend: &pb::LaneSpend, budget: Option<u64>) -> Option<String> {
    let b = budget?;
    let used = tokens(total(spend));
    Some(if over(spend, Some(b)) {
        format!("Over budget: {used} of {} tokens", tokens(b))
    } else {
        format!("{used} of {} tokens budgeted", tokens(b))
    })
}

/// The last seven days, oldest first: "0, 0, 340K, 0, 1.2M, 0, 500K tokens,
/// oldest first". Nothing when the runner sent none.
pub(super) fn trend_words(trend: &[u64]) -> Option<String> {
    if trend.is_empty() {
        return None;
    }
    let days: Vec<String> = trend.iter().map(|n| tokens(*n)).collect();
    Some(format!("{} tokens a day, oldest first, today last", days.join(", ")))
}

/// The overview's cost section: the week's tokens, and the comparison. Empty
/// from a runner that sent no cost.
pub(super) fn overview_lines(plan: &pb::Plan) -> Vec<String> {
    let Some(cost) = &plan.cost else { return Vec::new() };
    let mut out = vec!["Cost".to_string()];
    // No percentage: no harness reports the weekly limit to the runner.
    out.push(format!("  Last 7 days  {} tokens on this runner (its weekly limit isn't known)", tokens(cost.week_tokens)));
    out.extend(compare_lines(cost));
    out
}

/// "Claude Code opus-5 · 3 finished cards · 120K tokens a card · $2.10 a card".
pub(super) fn compare_lines(cost: &pb::PlanCost) -> Vec<String> {
    let mut out = Vec::new();
    for p in &cost.compare {
        let model = if p.model.is_empty() { "no model named" } else { p.model.as_str() };
        let mut line = format!(
            "  {} {} · {} · {} tokens a card",
            harness_name(&p.harness),
            model,
            count(p.cards as usize, "finished card"),
            tokens(p.tokens / p.cards.max(1) as u64)
        );
        if let Some(micros) = p.cost_micros {
            line.push_str(&format!(" · {} a card API-equivalent", dollars(micros / p.cards.max(1) as i64)));
        }
        out.push(line);
    }
    if cost.compare_held_back > 0 {
        let pairs = count(cost.compare_held_back as usize, "other harness and model pair");
        out.push(format!("  {pairs} held back until three cards have finished"));
    }
    out
}
