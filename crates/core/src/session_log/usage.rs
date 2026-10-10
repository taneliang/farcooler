//! What a terminal agent's turns spent, read from the session log the daemon
//! already tails.
//!
//! A separate fold from `parse_line`, which stays one line to a few
//! `TurnEvent`s: spend needs the whole turn in view (claude repeats a message
//! across lines, codex states a running total), so this keeps its own state
//! across lines, and across files.
//!
//! **Claude** writes `message.usage` on every `assistant` line, and one model
//! call can be written as several lines with the same `message.id` (the
//! `thinking` block, then the `tool_use`), each carrying the SAME usage
//! (`claude-complete-turn.jsonl` lines 2 and 3). Summing lines counts that call
//! twice, so calls are kept by id. A turn opens on a `user` line carrying
//! `promptSource` and closes on `system`/`turn_duration`, or on the next turn
//! opening, for one that was interrupted.
//!
//! **Codex** writes `event_msg`/`token_count` with `total_token_usage`, the
//! session's running sum. A turn (`task_started` to `task_complete` or
//! `turn_aborted`) is the difference between the total at its end and at its
//! start, so a line read twice changes nothing. Its model is `turn_context`'s
//! `model`.
//!
//! A turn whose start was not read is not recorded at all: a log attached at
//! its end (`tail::READ_FROM_START_BYTES`) holds only the rest of the turn
//! in flight, and half a turn's tokens would be a number that is quietly
//! wrong. Re-reading a file from its start, which `Tail` does when a log
//! rotates, re-reads turns already finished; their keys are remembered and
//! they are not reported twice.

use std::collections::{HashMap, VecDeque};

use serde_json::Value;

use crate::usage::TokenCounts;

/// How completely a turn's tokens are known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageState {
    /// Every model call in the turn stated its usage.
    Reported,
    /// Some did and some did not: the counts are a floor.
    Partial,
    /// None did.
    NotReported,
}

impl UsageState {
    pub fn as_str(self) -> &'static str {
        match self {
            UsageState::Reported => "reported",
            UsageState::Partial => "partial",
            UsageState::NotReported => "not_reported",
        }
    }
}

/// One finished turn, as its log told it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggedTurn {
    /// Stable across re-reads: the turn's own id in its log.
    pub key: String,
    pub started_at_ms: Option<i64>,
    pub ended_at_ms: Option<i64>,
    /// The agent's own duration where it wrote one, else end less start.
    pub active_ms: Option<i64>,
    /// Tokens per model, in the order each model first appeared. A model of
    /// `None` is a codex turn before any `turn_context` named one.
    pub models: Vec<(Option<String>, TokenCounts)>,
    pub state: UsageState,
    /// A claude subagent's whole run, not a turn: its spend counts, and the
    /// turn count and active time leave it out (it runs inside, or after, a
    /// turn of the agent that started it).
    pub subagent: bool,
}

/// How many finished turns' keys are remembered, against a re-read. A
/// rotation re-reads one file, and no session file holds anywhere near this
/// many turns that a re-read could reach before the memory has moved on.
const REMEMBERED: usize = 512;

#[derive(Debug)]
struct ClaudeTurn {
    key: String,
    started_at_ms: Option<i64>,
    last_at_ms: Option<i64>,
    calls: Calls,
}

/// Claude model calls, each counted once however many lines repeat it.
#[derive(Debug, Default)]
struct Calls {
    /// By `message.id`: the model and the largest usage any of its lines
    /// stated. The field-wise maximum, so a line written mid-stream with a
    /// smaller count (a subagent's `thinking` line saying 2 output tokens,
    /// its `tool_use` line 658) cannot lower a later, complete one.
    by_id: HashMap<String, (Option<String>, TokenCounts)>,
    order: Vec<String>,
    unreported: std::collections::HashSet<String>,
}

impl Calls {
    /// One `message` of an `assistant` line.
    fn saw(&mut self, message: &Value) {
        let model = message["model"].as_str();
        // Claude Code's own stand-in for an API error or a limit notice: no
        // model ran, and its usage is all zeros.
        if model == Some("<synthetic>") {
            return;
        }
        let Some(id) = message["id"].as_str() else { return };
        match claude_counts(&message["usage"]) {
            Some(counts) => {
                self.unreported.remove(id);
                let entry = self.by_id.entry(id.to_string()).or_insert_with(|| {
                    self.order.push(id.to_string());
                    (model.map(str::to_string), TokenCounts::default())
                });
                entry.1 = max(entry.1, counts);
            }
            None if !self.by_id.contains_key(id) => {
                self.unreported.insert(id.to_string());
            }
            None => {}
        }
    }

    /// Tokens per model, in the order each model first appeared. A model that
    /// ran in fast mode and out of it is two entries: they bill differently.
    fn by_model(&self) -> Vec<(Option<String>, TokenCounts)> {
        let mut models: Vec<(Option<String>, TokenCounts)> = Vec::new();
        for id in &self.order {
            let (model, counts) = &self.by_id[id];
            match models.iter_mut().find(|(m, sum)| m == model && sum.fast == counts.fast) {
                Some((_, sum)) => sum.add(counts),
                None => models.push((model.clone(), *counts)),
            }
        }
        models
    }

    fn state(&self) -> UsageState {
        match (self.by_id.is_empty(), self.unreported.is_empty()) {
            (true, _) => UsageState::NotReported,
            (false, true) => UsageState::Reported,
            (false, false) => UsageState::Partial,
        }
    }
}

/// One subagent's transcript, as far as it has been read.
#[derive(Debug, Default)]
struct Subagent {
    calls: Calls,
    first_at_ms: Option<i64>,
    last_at_ms: Option<i64>,
    /// Read something new since it was last handed out.
    moved: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Running {
    input: u64,
    cached: u64,
    cache_write: u64,
    output: u64,
    total: u64,
}

#[derive(Debug)]
struct CodexTurn {
    key: String,
    started_at_ms: Option<i64>,
    baseline: Option<Running>,
    latest: Option<Running>,
}

/// One pane's spend, followed line by line. Kept on the pane, not the file,
/// so a turn that starts in one file and ends in the next is one turn.
#[derive(Debug, Default)]
pub struct LogUsage {
    claude: Option<ClaudeTurn>,
    codex: Option<CodexTurn>,
    codex_model: Option<String>,
    /// The highest running total read, from any line.
    codex_settled: Option<Running>,
    finished: Vec<LoggedTurn>,
    seen: VecDeque<String>,
    /// Claude subagents, by agent id.
    subagents: std::collections::BTreeMap<String, Subagent>,
}

impl LogUsage {
    /// The turns that finished since the last call, and every subagent
    /// whose transcript grew since then, as it stands now.
    pub fn take(&mut self) -> Vec<LoggedTurn> {
        let mut out = std::mem::take(&mut self.finished);
        for (agent, run) in self.subagents.iter_mut().filter(|(_, run)| run.moved) {
            run.moved = false;
            out.push(LoggedTurn {
                key: format!("claude-log:agent:{agent}"),
                started_at_ms: run.first_at_ms,
                ended_at_ms: run.last_at_ms,
                active_ms: run.first_at_ms.zip(run.last_at_ms).map(|(f, l)| l - f),
                models: run.calls.by_model(),
                state: run.calls.state(),
                subagent: true,
            });
        }
        out
    }

    fn seen(&self, key: &str) -> bool {
        self.seen.iter().any(|k| k == key)
    }

    fn finish(&mut self, turn: LoggedTurn) {
        if self.seen(&turn.key) {
            return;
        }
        self.seen.push_back(turn.key.clone());
        if self.seen.len() > REMEMBERED {
            self.seen.pop_front();
        }
        self.finished.push(turn);
    }

    /// One line of a claude transcript.
    pub fn claude_line(&mut self, line: &str) {
        let Ok(record) = serde_json::from_str::<Value>(line) else { return };
        let at = record["timestamp"].as_str().and_then(super::claude::parse_iso8601_millis);
        match record["type"].as_str() {
            Some("user") if record.get("promptSource").is_some() => {
                let id = record["uuid"].as_str().or(record["promptId"].as_str()).unwrap_or_default();
                let key = format!("claude-log:{id}");
                if self.claude.as_ref().is_some_and(|t| t.key == key) || self.seen(&key) {
                    return;
                }
                self.close_claude(None, None);
                self.claude = Some(ClaudeTurn { key, started_at_ms: at, last_at_ms: at, calls: Calls::default() });
            }
            Some("assistant") => {
                let Some(turn) = self.claude.as_mut() else { return };
                if at.is_some() {
                    turn.last_at_ms = at;
                }
                turn.calls.saw(&record["message"]);
            }
            Some("system") if record["subtype"] == "turn_duration" => {
                self.close_claude(at, record["durationMs"].as_i64());
            }
            _ => {}
        }
    }

    fn close_claude(&mut self, ended_at_ms: Option<i64>, duration_ms: Option<i64>) {
        let Some(turn) = self.claude.take() else { return };
        let ended_at_ms = ended_at_ms.or(turn.last_at_ms);
        let (models, state) = (turn.calls.by_model(), turn.calls.state());
        let active_ms = duration_ms.or_else(|| Some(ended_at_ms? - turn.started_at_ms?));
        self.finish(LoggedTurn {
            key: turn.key,
            started_at_ms: turn.started_at_ms,
            ended_at_ms,
            active_ms,
            models,
            state,
            subagent: false,
        });
    }

    /// One line of a claude subagent's own transcript,
    /// `<session>/subagents/agent-<agent>.jsonl`.
    ///
    /// A subagent's calls are not in the parent's file, and a background one
    /// goes on after the parent's turn has closed, so it is its own entry,
    /// handed out again by `take` each time it grows, under one key, for the
    /// store to keep the latest of. Its file opens no turn (no
    /// `promptSource`) and closes none, so the whole file is the entry.
    pub fn subagent_line(&mut self, agent: &str, line: &str) {
        let Ok(record) = serde_json::from_str::<Value>(line) else { return };
        let at = record["timestamp"].as_str().and_then(super::claude::parse_iso8601_millis);
        let run = self.subagents.entry(agent.to_string()).or_default();
        if let Some(at) = at {
            run.first_at_ms = Some(run.first_at_ms.map_or(at, |f| f.min(at)));
            run.last_at_ms = Some(run.last_at_ms.map_or(at, |l| l.max(at)));
        }
        if record["type"] == "assistant" {
            run.calls.saw(&record["message"]);
            run.moved = true;
        }
    }

    /// One line of a codex rollout.
    pub fn codex_line(&mut self, line: &str) {
        let Ok(record) = serde_json::from_str::<Value>(line) else { return };
        let at = record["timestamp"].as_str().and_then(super::claude::parse_iso8601_millis);
        let payload = &record["payload"];
        match (record["type"].as_str(), payload["type"].as_str()) {
            (Some("turn_context"), _) => {
                if let Some(model) = payload["model"].as_str() {
                    self.codex_model = Some(model.to_string());
                }
            }
            (Some("event_msg"), Some("task_started")) => {
                let key = format!("codex-log:{}", payload["turn_id"].as_str().unwrap_or_default());
                if self.codex.as_ref().is_some_and(|t| t.key == key) || self.seen(&key) {
                    return;
                }
                let started_at_ms = payload["started_at"].as_i64().map(|s| s * 1000).or(at);
                self.codex =
                    Some(CodexTurn { key, started_at_ms, baseline: self.codex_settled, latest: None });
            }
            (Some("event_msg"), Some("token_count")) => {
                let info = &payload["info"];
                let Some(total) = running(&info["total_token_usage"]) else { return };
                if self.codex_settled.is_none_or(|s| total.total >= s.total) {
                    self.codex_settled = Some(total);
                }
                let Some(turn) = self.codex.as_mut() else { return };
                if turn.baseline.is_none() {
                    let last = running(&info["last_token_usage"]).unwrap_or_default();
                    turn.baseline = Some(minus(total, last));
                }
                if turn.latest.is_none_or(|l| total.total >= l.total) {
                    turn.latest = Some(total);
                }
            }
            (Some("event_msg"), Some("task_complete" | "turn_aborted")) => {
                let Some(turn) = self.codex.take() else { return };
                let ended_at_ms = payload["completed_at"].as_i64().map(|s| s * 1000).or(at);
                let active_ms = payload["duration_ms"]
                    .as_i64()
                    .or_else(|| Some(ended_at_ms? - turn.started_at_ms?));
                let (models, state) = match (turn.latest, turn.baseline) {
                    (Some(latest), Some(baseline)) => {
                        let d = minus(latest, baseline);
                        let counts = TokenCounts {
                            input: d.input.saturating_sub(d.cached).saturating_sub(d.cache_write),
                            output: d.output,
                            cache_read: d.cached,
                            cache_write: d.cache_write,
                            cache_write_1h: 0,
                            fast: false,
                        };
                        (vec![(self.codex_model.clone(), counts)], UsageState::Reported)
                    }
                    _ => (Vec::new(), UsageState::NotReported),
                };
                self.finish(LoggedTurn {
                    key: turn.key,
                    started_at_ms: turn.started_at_ms,
                    ended_at_ms,
                    active_ms,
                    models,
                    state,
                    subagent: false,
                });
            }
            _ => {}
        }
    }
}

/// `message.usage`, or `None` where the line carries none.
fn claude_counts(usage: &Value) -> Option<TokenCounts> {
    let usage = usage.as_object()?;
    let n = |k: &str| usage.get(k).and_then(Value::as_u64).unwrap_or(0);
    Some(TokenCounts {
        input: n("input_tokens"),
        output: n("output_tokens"),
        cache_read: n("cache_read_input_tokens"),
        cache_write: n("cache_creation_input_tokens"),
        cache_write_1h: usage
            .get("cache_creation")
            .and_then(|c| c["ephemeral_1h_input_tokens"].as_u64())
            .unwrap_or(0),
        fast: usage.get("speed").and_then(Value::as_str) == Some("fast"),
    })
}

fn max(a: TokenCounts, b: TokenCounts) -> TokenCounts {
    TokenCounts {
        input: a.input.max(b.input),
        output: a.output.max(b.output),
        cache_read: a.cache_read.max(b.cache_read),
        cache_write: a.cache_write.max(b.cache_write),
        cache_write_1h: a.cache_write_1h.max(b.cache_write_1h),
        fast: a.fast || b.fast,
    }
}

fn running(v: &Value) -> Option<Running> {
    let n = |k: &str| v[k].as_u64();
    Some(Running {
        input: n("input_tokens")?,
        cached: n("cached_input_tokens").unwrap_or(0),
        cache_write: n("cache_write_input_tokens").unwrap_or(0),
        output: n("output_tokens")?,
        total: n("total_tokens").unwrap_or(0),
    })
}

fn minus(a: Running, b: Running) -> Running {
    Running {
        input: a.input.saturating_sub(b.input),
        cached: a.cached.saturating_sub(b.cached),
        cache_write: a.cache_write.saturating_sub(b.cache_write),
        output: a.output.saturating_sub(b.output),
        total: a.total.saturating_sub(b.total),
    }
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
