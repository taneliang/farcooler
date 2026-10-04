//! The plan layer as JSON (ov-274): `plan.get` and `plan.events` in the shape
//! the apps decode, which is the shape `farcooler plan --json` prints, so the
//! Mac and the phones read one fixture, `test/fixtures/plan.json`.
//!
//! EXPERIMENTAL, behind `board_plan`, and removable with the rest of the layer
//! (design 8). The phones read these two answers and never write the plan: the
//! orchestrator is its one writer.
//!
//! The CLI builds the same objects from the same wire messages
//! (`crates/cli/src/plan.rs`). Both are held to the one fixture by a test, so
//! a key renamed on either side fails there instead of on a phone.

use farcooler_protocol::v1 as pb;
use serde_json::{Value, json};

use crate::session::{short, uuid_of};

fn id_text(id: &[u8]) -> String {
    uuid_of(id).to_string()
}

/// A card's status in the board's words, as the CLI says it.
fn status_word(status: i32) -> &'static str {
    match pb::TaskStatus::try_from(status) {
        Ok(pb::TaskStatus::Backlog) => "Backlog",
        Ok(pb::TaskStatus::Todo) => "To Do",
        Ok(pb::TaskStatus::NeedsDecision) => "Needs Decision",
        Ok(pb::TaskStatus::InProgress) => "In Progress",
        Ok(pb::TaskStatus::InReview) => "In Review",
        Ok(pb::TaskStatus::Done) => "Done",
        Ok(pb::TaskStatus::Cancelled) => "Cancelled",
        _ => "Unknown",
    }
}

fn theme_state_word(state: i32) -> &'static str {
    match pb::BoardThemeState::try_from(state) {
        Ok(pb::BoardThemeState::Active) => "active",
        Ok(pb::BoardThemeState::Paused) => "paused",
        Ok(pb::BoardThemeState::Done) => "done",
        Ok(pb::BoardThemeState::Dropped) => "dropped",
        _ => "unknown",
    }
}

fn lane_state_word(state: i32) -> &'static str {
    match pb::LaneState::try_from(state) {
        Ok(pb::LaneState::Queued) => "queued",
        Ok(pb::LaneState::Building) => "building",
        Ok(pb::LaneState::Review) => "review",
        Ok(pb::LaneState::Fixing) => "fixing",
        Ok(pb::LaneState::Landing) => "landing",
        Ok(pb::LaneState::Landed) => "landed",
        Ok(pb::LaneState::Dropped) => "dropped",
        _ => "unknown",
    }
}

/// A card's key from the plan's own cards, or its short id for one the plan
/// doesn't list.
fn key_of(plan: &pb::Plan, task: &[u8]) -> String {
    plan.cards.iter().find(|c| c.task_id == task).map_or_else(|| short(task), |c| c.key.clone())
}

fn theme_json(plan: &pb::Plan, view: &pb::BoardThemeView) -> Value {
    let t = view.theme.clone().unwrap_or_default();
    let c = view.counts.unwrap_or_default();
    json!({
        "id": id_text(&t.id),
        "short": short(&t.id),
        "name": t.name,
        "outcome": t.outcome,
        "story": t.story,
        "story_at": t.story_at,
        "next": t.next,
        "owner_ask": t.owner_ask,
        "state": theme_state_word(t.state),
        "ordinal": t.ordinal,
        "cards": view.task_ids.iter().map(|id| json!({ "task": id_text(id), "key": key_of(plan, id) })).collect::<Vec<_>>(),
        "counts": {
            "backlog": c.backlog, "todo": c.todo, "needs_decision": c.needs_decision,
            "in_progress": c.in_progress, "in_review": c.in_review, "done": c.done, "cancelled": c.cancelled,
        },
    })
}

fn lane_json(plan: &pb::Plan, l: &pb::Lane) -> Value {
    let spend = l.spend.unwrap_or_default();
    json!({
        "id": id_text(&l.id),
        "short": short(&l.id),
        "name": l.name,
        "state": lane_state_word(l.state),
        "reason": l.reason,
        "plan_rank": l.plan_rank,
        "worktree_path": l.worktree_path,
        "branch": l.branch,
        "harness": l.harness,
        "model": l.model,
        "train": l.train,
        "landed_sha": l.landed_sha,
        "state_since": l.state_since,
        "stale": l.stale,
        "fix_rounds": l.fix_rounds,
        "cards": l.cards.iter().map(|c| json!({
            "task": id_text(&c.task_id), "key": key_of(plan, &c.task_id), "slice": c.slice,
        })).collect::<Vec<_>>(),
        "agents": l.agents.iter().map(|a| json!({
            "harness": a.harness, "agent_id": a.agent_id,
            "role": match pb::LaneAgentRole::try_from(a.role) {
                Ok(pb::LaneAgentRole::Review) => "review",
                Ok(pb::LaneAgentRole::Fix) => "fix",
                _ => "build",
            },
            "model": a.model, "started_at": a.started_at, "ended_at": a.ended_at,
        })).collect::<Vec<_>>(),
        "spend": {
            "input_tokens": spend.input_tokens, "output_tokens": spend.output_tokens,
            "cache_read_tokens": spend.cache_read_tokens, "cache_write_tokens": spend.cache_write_tokens,
            "cost_micros": spend.cost_micros, "runs": spend.runs, "unmeasured_agents": spend.unmeasured_agents,
        },
    })
}

/// Cards that are neither done nor canceled.
fn is_open(status: i32) -> bool {
    status != pb::TaskStatus::Done as i32 && status != pb::TaskStatus::Cancelled as i32
}

/// `plan.get`'s answer: the whole plan, with the two flags the CLI's
/// reconciliation derives, `landed_not_closed` and `no_lane`.
pub fn plan_json(plan: &pb::Plan) -> Value {
    let flagged = |wanted: fn(&pb::PlanCard, u32, u32) -> bool| -> Vec<Value> {
        plan.cards
            .iter()
            .filter(|c| {
                let (live, landed) =
                    plan.coverage.iter().find(|v| v.task_id == c.task_id).map_or((0, 0), |v| (v.live, v.landed));
                is_open(c.status) && wanted(c, live, landed)
            })
            .map(|c| json!({ "task": id_text(&c.task_id), "key": c.key, "status": status_word(c.status) }))
            .collect()
    };
    json!({
        "now_ms": plan.now_ms,
        "themes": plan.themes.iter().map(|v| theme_json(plan, v)).collect::<Vec<_>>(),
        "lanes": plan.lanes.iter().map(|l| lane_json(plan, l)).collect::<Vec<_>>(),
        "order": plan.order.iter().map(|id| id_text(id)).collect::<Vec<_>>(),
        "cards": plan.cards.iter().map(|c| json!({
            "task": id_text(&c.task_id), "key": c.key, "title": c.title, "status": status_word(c.status),
        })).collect::<Vec<_>>(),
        "landed_not_closed": flagged(|_, live, landed| live == 0 && landed > 0),
        "no_lane": flagged(|c, live, landed| {
            live == 0
                && landed == 0
                && (c.status == pb::TaskStatus::InProgress as i32 || c.status == pb::TaskStatus::InReview as i32)
        }),
    })
}

fn event_json(e: &pb::PlanEvent) -> Value {
    json!({
        "at": e.at, "actor": e.actor, "kind": e.kind, "body": e.body,
        "extra": serde_json::from_str::<Value>(&e.extra_json).unwrap_or(Value::Null),
    })
}

/// `plan.events`' answer: `{"events": [...]}`, the record a page reads (the
/// CLI's `show --json` carries the same list beside its subject).
pub fn events_json(list: &pb::PlanEventList) -> Value {
    json!({ "events": list.events.iter().map(event_json).collect::<Vec<_>>() })
}

#[cfg(test)]
#[path = "plan_json_tests.rs"]
mod tests;
