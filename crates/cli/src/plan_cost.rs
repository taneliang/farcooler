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
    // In integers: 4.1M is 4,100,000 tokens, which a float can't say exactly.
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    let all_digits = |t: &str| t.chars().all(|c| c.is_ascii_digit());
    if (whole.is_empty() && fraction.is_empty()) || !all_digits(whole) || !all_digits(fraction) {
        return Err(said.to_string());
    }
    // The fraction may not name a smaller unit than a token: 1.5k is 1,500, 1.0001k is not.
    let places = scale.ilog10() as usize;
    let fraction = fraction.trim_end_matches('0');
    if fraction.len() > places {
        return Err(said.to_string());
    }
    let padded = format!("{fraction:0<places$}");
    let whole: u64 = if whole.is_empty() { 0 } else { whole.parse().map_err(|_| said.to_string())? };
    let part: u64 = if padded.is_empty() { 0 } else { padded.parse().map_err(|_| said.to_string())? };
    let total = whole.checked_mul(scale).and_then(|w| w.checked_add(part)).ok_or_else(|| said.to_string())?;
    if !(1..=100_000_000_000_000).contains(&total) {
        return Err(said.to_string());
    }
    Ok(total)
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

/// The last seven UTC days, oldest first: "0, 0, 340K, 0, 1.2M, 0, 500K
/// tokens a day (UTC days), oldest first". Nothing when the runner sent none.
pub(super) fn trend_words(trend: &[u64]) -> Option<String> {
    if trend.is_empty() {
        return None;
    }
    let days: Vec<String> = trend.iter().map(|n| tokens(*n)).collect();
    Some(format!("{} tokens a day (UTC days), oldest first, today last", days.join(", ")))
}

/// The overview's cost section: the week's tokens, the comparison and what's
/// in flight. Empty from a runner that sent no cost.
pub(super) fn overview_lines(plan: &pb::Plan) -> Vec<String> {
    let Some(cost) = &plan.cost else { return Vec::new() };
    let mut out = vec!["Cost".to_string()];
    // No percentage: no harness reports the weekly limit to the runner. The
    // total comes first, then what each harness and model spent (ov-434).
    let week_dollars = if cost.week.is_empty() { String::new() } else { dollars_words(cost.week_cost_micros, false, "") };
    out.push(format!(
        "  Last 7 days  {} tokens{week_dollars} in this project (its weekly limit isn't known)",
        tokens(cost.week_tokens)
    ));
    for w in &cost.week {
        let model = if w.model.is_empty() { "no model named" } else { w.model.as_str() };
        out.push(format!("    {} {} · {} tokens{}", harness_name(&w.harness), model, tokens(w.tokens), dollars_words(w.cost_micros, false, &w.model)));
    }
    out.extend(compare_lines(cost));
    out
}

/// "4.2 finished cards": a card two pairs worked is shared out, so a pair's
/// number is a share and reads with its decimal when it isn't whole.
pub(super) fn cards_words(milli: u32) -> String {
    if milli % 1000 == 0 { format!("{} finished cards", milli / 1000) } else { format!("{:.1} finished cards", milli as f64 / 1000.0) }
}

/// " · about $2.50 a card API-equivalent"; where there is no figure,
/// " · No price listed for opus-9" when the row names a model (the table has
/// no rate for it), else " · API-equivalent dollars: Not reported".
fn dollars_words(micros: Option<i64>, a_card: bool, model: &str) -> String {
    match micros {
        Some(m) => format!(" · about {}{} API-equivalent", dollars(m), if a_card { " a card" } else { "" }),
        None if !model.is_empty() => format!(" · No price listed for {model}"),
        None => " · API-equivalent dollars: Not reported".to_string(),
    }
}

/// "Claude Code opus · 3 finished cards · 120K tokens a card · about $2.10 a card API-equivalent".
pub(super) fn compare_lines(cost: &pb::PlanCost) -> Vec<String> {
    let mut out = Vec::new();
    for p in &cost.compare {
        let model = if p.model.is_empty() { "no model named" } else { p.model.as_str() };
        // Cost per landed card: its spend on landed cards over its share of them.
        let each = p.tokens * 1000 / p.card_share_milli.max(1) as u64;
        let per_card = p.cost_micros.map(|m| m * 1000 / p.card_share_milli.max(1) as i64);
        out.push(format!(
            "  {} {} · {} · {} tokens a card{}",
            harness_name(&p.harness),
            model,
            cards_words(p.card_share_milli),
            tokens(each),
            dollars_words(per_card, true, &p.model)
        ));
    }
    if cost.compare_held_back > 0 {
        let pairs = count(cost.compare_held_back as usize, "other harness and model pair");
        out.push(format!("  {pairs} held back until three cards have landed"));
    }
    if cost.in_flight_tokens > 0 {
        out.push(format!(
            "  In flight  {} tokens on cards that haven't landed{}",
            tokens(cost.in_flight_tokens),
            dollars_words(cost.in_flight_cost_micros, false, "")
        ));
    }
    out
}
