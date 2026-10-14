//! `farcooler plan agents`: who is working, and for what (ov-503).
//!
//! ```text
//! farcooler plan agents [--json]
//! ```
//!
//! Every agent that still exists in this project, the orchestrator included:
//! working ones first, then the idle, each with the lane, train or card it
//! works for and its spend. Agents that finished or exited are not listed. The
//! header's spend is the last 24 hours of every agent, those too, so it is the
//! total the runner spent. Tokens come first and dollars second, and every
//! dollar is API-equivalent, as everywhere else on the plan. An agent no
//! record names says "Not on a lane" rather than being left out.
//!
//! A child of `plan.rs`. Behind `board_agents` as well as `board_plan`: a
//! runner with the plan and no agents list is told so before anything is read.

use farcooler_core::usage_words::{dollars, tokens};
use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb};
use serde_json::{Value, json};

use super::{Failed, ago, count, get_plan, id_text, spend_json};
use crate::tasks::{DispatchLink, Refused};

/// What a runner with the plan and no agents list is told.
const NEEDS_UPDATE: &str = "This runner needs an update to list agents.";

/// What an agent with no lane, train or card is working for.
const NOT_ON_A_LANE: &str = "Not on a lane";

fn needs_agents<L: DispatchLink>(link: &L) -> Result<(), Failed> {
    if link.capabilities().iter().any(|c| c == capability::BOARD_AGENTS_LIST) {
        return Ok(());
    }
    Err(Box::new(Refused::new(NEEDS_UPDATE.to_string(), Some(pb::ErrorCode::CapabilityUnsupported as i32))))
}

/// `plan agents`, printed.
pub(super) async fn agents<L: DispatchLink>(link: &mut L, workspace: bytes::Bytes, json: bool) -> Result<String, Failed> {
    needs_agents(link)?;
    let plan = get_plan(link, &workspace, false).await?;
    Ok(if json { agents_json(&plan).to_string() } else { agents_text(&plan) })
}

fn role_word(role: i32) -> &'static str {
    match pb::PlanAgentRole::try_from(role) {
        Ok(pb::PlanAgentRole::Orchestrator) => "Orchestrator",
        Ok(pb::PlanAgentRole::Build) => "Builder",
        Ok(pb::PlanAgentRole::Review) => "Reviewer",
        Ok(pb::PlanAgentRole::Fix) => "Fixer",
        Ok(pb::PlanAgentRole::Integrator) => "Integrator",
        Ok(pb::PlanAgentRole::Subagent) => "Subagent",
        _ => "Agent",
    }
}

fn role_key(role: i32) -> &'static str {
    match pb::PlanAgentRole::try_from(role) {
        Ok(pb::PlanAgentRole::Orchestrator) => "orchestrator",
        Ok(pb::PlanAgentRole::Build) => "build",
        Ok(pb::PlanAgentRole::Review) => "review",
        Ok(pb::PlanAgentRole::Fix) => "fix",
        Ok(pb::PlanAgentRole::Integrator) => "integrator",
        Ok(pb::PlanAgentRole::Subagent) => "subagent",
        _ => "agent",
    }
}

fn total(s: &pb::LaneSpend) -> u64 {
    s.input_tokens + s.output_tokens + s.cache_read_tokens + s.cache_write_tokens
}

/// What the agent works for, in a person's words: the plan, a lane, a train
/// or a card, else "Not on a lane".
fn works_for(a: &pb::PlanAgent) -> String {
    if a.role == pb::PlanAgentRole::Orchestrator as i32 {
        "The plan".to_string()
    } else if a.lane_id.is_some() {
        format!("Lane {}", a.lane_name)
    } else if a.train_id.is_some() {
        a.train_title.clone()
    } else if a.task_id.is_some() {
        format!("Card {}", a.task_key)
    } else {
        NOT_ON_A_LANE.to_string()
    }
}

/// "2.1M tokens · about $14.20 API-equivalent", tokens first.
fn spend_line(s: &pb::LaneSpend) -> String {
    match s.cost_micros {
        Some(m) => format!("{} tokens · about {} API-equivalent", tokens(total(s)), dollars(m)),
        None => format!("{} tokens", tokens(total(s))),
    }
}

fn agents_text(plan: &pb::Plan) -> String {
    // A runner before ov-518 sends no total: its list was every agent of the day.
    let all = plan.agents_spend.unwrap_or_else(|| pb::LaneSpend {
        input_tokens: plan.agents.iter().map(|a| a.spend.unwrap_or_default().input_tokens).sum(),
        output_tokens: plan.agents.iter().map(|a| a.spend.unwrap_or_default().output_tokens).sum(),
        cache_read_tokens: plan.agents.iter().map(|a| a.spend.unwrap_or_default().cache_read_tokens).sum(),
        cache_write_tokens: plan.agents.iter().map(|a| a.spend.unwrap_or_default().cache_write_tokens).sum(),
        // Dollars only when every agent is priced: a sum with a hole in it would read low.
        cost_micros: plan.agents.iter().try_fold(0i64, |sum, a| a.spend.as_ref()?.cost_micros.map(|m| sum + m)),
        ..Default::default()
    });
    // With none alive the day's spend still shows, as the apps' header does.
    if plan.agents.is_empty() && total(&all) == 0 {
        return "No agents running.".to_string();
    }
    let working = plan.agents.iter().filter(|a| a.working).count();
    let cost = all.cost_micros.map(|m| format!(" · about {} API-equivalent", dollars(m))).unwrap_or_default();
    let lead = if plan.agents.is_empty() {
        "No agents running".to_string()
    } else {
        format!("{working} working · {}", count(plan.agents.len(), "agent"))
    };
    let mut out = vec![format!("{lead} · {} tokens in the last 24 hours{cost}", tokens(total(&all)))];
    if plan.agents.is_empty() {
        return out.remove(0);
    }
    for a in &plan.agents {
        let spend = a.spend.unwrap_or_default();
        let mut who = format!("{} · {}", role_word(a.role), if a.model.is_empty() { &a.harness } else { &a.model });
        if !a.label.is_empty() {
            who.push_str(&format!(" \u{201c}{}\u{201d}", a.label));
        }
        let state = if a.working { "working".to_string() } else { if a.last_at > 0 { format!("idle · active {}", ago(plan.now_ms - a.last_at)) } else { "idle".to_string() } };
        out.push(format!(
            "  {} {who}  {}  {state}  {}",
            if a.working { "\u{25cf}" } else { "\u{25cb}" },
            works_for(a),
            spend_line(&spend)
        ));
    }
    out.join("\n")
}

pub(super) fn agent_json(a: &pb::PlanAgent) -> Value {
    json!({
        "id": a.id,
        "role": role_key(a.role),
        "harness": a.harness,
        "model": a.model,
        "label": a.label,
        "lane": a.lane_id.as_ref().map(|id| json!({ "id": id_text(id), "name": a.lane_name })),
        "train": a.train_id.as_ref().map(|id| json!({ "id": id_text(id), "title": a.train_title })),
        "card": a.task_id.as_ref().map(|id| json!({ "task": id_text(id), "key": a.task_key })),
        "terminal": a.terminal_id.as_ref().map(|id| id_text(id)),
        "working": a.working,
        "started_at": a.started_at,
        "last_at": a.last_at,
        "spend": spend_json(&a.spend.unwrap_or_default()),
    })
}

/// The `plan agents --json` object. `plan --json` carries the same `agents`
/// list under its own key, which is what the apps read.
fn agents_json(plan: &pb::Plan) -> Value {
    json!({ "now_ms": plan.now_ms, "agents": plan.agents.iter().map(agent_json).collect::<Vec<_>>() })
}

#[cfg(test)]
#[path = "plan_agents_tests.rs"]
mod tests;
