//! An agent at its provider's usage or rate limit (ov-1515). Codex prints one line and sits idle; Claude ends its
//! turn with `StopFailure`. Either way the step fails at once, saying until when, and the reason's shape is what
//! the engine reads back to pick a fallback model and to list one Needs You item per provider.

/// What every such reason starts with.
pub(crate) const PREFIX: &str = "usage limit until ";
/// The state report Codex writes to its terminal (OSC 7501). tmux keeps it out of the screen, so it is read from
/// the terminal's raw bytes.
const STATE: &[u8] = b"\x1b]7501;state=";
/// How each of Codex's limit errors starts, after its error mark, with a straight apostrophe.
const CODEX: [&str; 2] = ["You've hit your usage limit", "exceeded retry limit, last status: 429"];

fn reason(until: Option<&str>, harness: &str) -> String {
    let until: String = until.unwrap_or("an unknown time").chars().filter(|c| !c.is_control()).take(100).collect();
    format!("{PREFIX}{until} ({harness})")
}

/// The harness and the time a limit reason names.
pub(crate) fn parse(reason: &str) -> Option<(&'static str, &str)> {
    let rest = reason.strip_prefix(PREFIX)?;
    [("codex", " (Codex)"), ("claude", " (Claude)")].into_iter()
        .find_map(|(harness, tail)| rest.strip_suffix(tail).map(|until| (harness, until)))
}

/// A limit Codex is showing: its own error line on `screen`, and, where `raw` (the end of its terminal's bytes)
/// reports states, an error as the latest one. The words alone are not enough: a ticket or a file can quote them.
pub(crate) fn codex(screen: &str, raw: &[u8]) -> Option<String> {
    if let Some(at) = raw.windows(STATE.len()).rposition(|w| w == STATE)
        && !raw[at + STATE.len()..].starts_with(b"error") { return None; }
    let lines: Vec<&str> = screen.lines().collect();
    let said = lines.iter().enumerate().rev().find_map(|(at, line)| {
        let first = line.trim_start().strip_prefix("■ ")?.replace('’', "'");
        if !CODEX.iter().any(|known| first.starts_with(known)) { return None; }
        // The error wraps over the lines below it, up to a blank one.
        let rest = lines[at + 1..].iter().take(4).map(|l| l.trim()).take_while(|l| !l.is_empty());
        Some(std::iter::once(first.trim().to_owned()).chain(rest.map(str::to_owned)).collect::<Vec<_>>().join(" "))
    })?;
    let lower = said.to_ascii_lowercase();
    let after = |words: &str| lower.find(words).map(|at| {
        let tail = &said[at + words.len()..];
        // The time ends its sentence. "1:23 AM." and "4 days 3 hours." hold no ". " of their own.
        tail.split(". ").next().unwrap_or(tail).trim_end_matches('.').trim().to_owned()
    }).filter(|t| !t.is_empty());
    let until = after("try again at ").or_else(|| after("try again in ").map(|t| format!("{t} from now")));
    Some(reason(until.as_deref(), "Codex"))
}

/// A limit Claude ended its turn on, from its `StopFailure` hook. Claude retries a passing 429 itself, so one that
/// ends the turn has stopped the agent.
pub(crate) fn claude(payload: &serde_json::Value) -> Option<String> {
    let said = payload["last_assistant_message"].as_str().unwrap_or_default();
    let lower = said.to_ascii_lowercase();
    let limit = payload["error"].as_str() == Some("rate_limit")
        || lower.starts_with("you've hit your ") && lower.contains(" limit")
        || lower.contains("usage limit reached");
    if !limit { return None; }
    let until = said.split_once("resets ").map(|(_, t)| t.trim()).filter(|t| !t.is_empty());
    Some(reason(until, "Claude"))
}

#[cfg(test)]
#[path = "provider_limit_tests.rs"]
mod tests;
