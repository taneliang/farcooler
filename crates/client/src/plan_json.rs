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

/// A spend as the JSON carries it: a lane's, or a theme's share (ov-306).
fn spend_json(s: &pb::LaneSpend) -> Value {
    json!({
        "input_tokens": s.input_tokens, "output_tokens": s.output_tokens,
        "cache_read_tokens": s.cache_read_tokens, "cache_write_tokens": s.cache_write_tokens,
        "cost_micros": s.cost_micros, "runs": s.runs, "unmeasured_agents": s.unmeasured_agents,
        "shared_agents": s.shared_agents,
    })
}

/// The week's tokens and the harness and model comparison as the JSON carries
/// them (ov-307). `null` from a runner without `board_cost`.
fn cost_json(c: &pb::PlanCost) -> Value {
    json!({
        "week_tokens": c.week_tokens,
        "compare": c.compare.iter().map(|p| json!({
            "harness": p.harness, "model": p.model, "card_share_milli": p.card_share_milli, "tokens": p.tokens,
            "cost_micros": p.cost_micros,
        })).collect::<Vec<_>>(),
        "compare_held_back": c.compare_held_back,
        "in_flight_tokens": c.in_flight_tokens,
        "in_flight_cost_micros": c.in_flight_cost_micros,
    })
}

/// Status counts as the JSON carries them: a theme's, or the board's (ov-306).
fn counts_json(c: &pb::PlanStatusCounts) -> Value {
    json!({
        "backlog": c.backlog, "todo": c.todo, "needs_decision": c.needs_decision,
        "in_progress": c.in_progress, "in_review": c.in_review, "done": c.done, "cancelled": c.cancelled,
    })
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
        "counts": counts_json(&c),
        "spend": spend_json(&view.spend.unwrap_or_default()),
        "budget_tokens": view.budget_tokens,
        "trend_tokens": view.trend_tokens,
    })
}

/// A pull request stage as the JSON carries it (ov-312): the words the runner
/// chose, and the facts behind them. Left out of the record when the runner
/// said nothing (`with_stage`), as every additive field on this wire is.
pub fn stage_json(s: &pb::PrStage) -> Value {
    let kind = match pb::PrStageKind::try_from(s.kind) {
        Ok(pb::PrStageKind::Building) => "building",
        Ok(pb::PrStageKind::AgentReview) => "agent_review",
        Ok(pb::PrStageKind::Fixing) => "fixing",
        Ok(pb::PrStageKind::WaitingOnReviewer) => "waiting_on_reviewer",
        Ok(pb::PrStageKind::ChangesRequested) => "changes_requested",
        Ok(pb::PrStageKind::ApprovedChecksRunning) => "approved_checks_running",
        Ok(pb::PrStageKind::ApprovedChecksFailing) => "approved_checks_failing",
        Ok(pb::PrStageKind::Approved) => "approved",
        Ok(pb::PrStageKind::ApprovedConflicts) => "approved_conflicts",
        Ok(pb::PrStageKind::Queued) => "queued",
        Ok(pb::PrStageKind::Merged) => "merged",
        Ok(pb::PrStageKind::Closed) => "closed",
        Ok(pb::PrStageKind::Unknown) => "unknown",
        _ => "unspecified",
    };
    json!({
        "kind": kind, "label": s.label, "reviewers": s.reviewers, "queue_position": s.queue_position,
        "checks": match pb::CheckState::try_from(s.checks) {
            Ok(pb::CheckState::Passing) => "passing",
            Ok(pb::CheckState::Failing) => "failing",
            Ok(pb::CheckState::Pending) => "pending",
            _ => "unknown",
        },
        "pr_number": s.pr_number, "pr_url": s.pr_url, "unresolved_threads": s.unresolved_threads,
        "read_at": s.read_at,
    })
}

pub fn with_stage(mut record: Value, stage: &Option<pb::PrStage>) -> Value {
    if let (Some(stage), Some(map)) = (stage, record.as_object_mut()) {
        map.insert("stage".into(), stage_json(stage));
    }
    record
}

fn lane_json(plan: &pb::Plan, l: &pb::Lane) -> Value {
    let spend = l.spend.unwrap_or_default();
    with_stage(json!({
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
        "budget_tokens": l.budget_tokens,
        "fix_rounds": l.fix_rounds,
        "cards": l.cards.iter().map(|c| with_stage(json!({
            "task": id_text(&c.task_id), "key": key_of(plan, &c.task_id), "slice": c.slice,
        }), &c.stage)).collect::<Vec<_>>(),
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
            "shared_agents": spend.shared_agents,
        },
    }), &l.stage)
}

fn ruling_state_word(state: i32) -> &'static str {
    match pb::BoardRulingState::try_from(state) {
        Ok(pb::BoardRulingState::Standing) => "standing",
        Ok(pb::BoardRulingState::Confirmed) => "confirmed",
        Ok(pb::BoardRulingState::Reversed) => "reversed",
        _ => "unknown",
    }
}

/// A ruling (ov-304), with its short id and its theme's name ("" when it
/// names none, or one the plan no longer lists).
fn ruling_json(plan: &pb::Plan, r: &pb::BoardRuling) -> Value {
    let theme = r.theme_id.as_ref().and_then(|id| {
        plan.themes.iter().filter_map(|v| v.theme.as_ref()).find(|t| t.id == *id).map(|t| t.name.clone())
    });
    json!({
        "id": id_text(&r.id),
        "short": format!("R-{}", r.number),
        "number": r.number,
        "decision": r.decision,
        "why": r.why,
        "reversal": r.reversal,
        "cards": r.task_ids.iter().enumerate().map(|(i, id)| json!({
            "task": id_text(id), "key": r.task_keys.get(i).cloned().unwrap_or_else(|| key_of(plan, id)),
        })).collect::<Vec<_>>(),
        "theme_id": r.theme_id.as_deref().map(id_text),
        "theme": theme.unwrap_or_default(),
        "state": ruling_state_word(r.state),
        "note": r.note,
        "actor": r.actor,
        "created_at": r.created_at,
        "settled_by": r.settled_by,
        "settled_at": r.settled_at,
    })
}

fn train_state_word(state: i32) -> &'static str {
    match pb::BoardTrainState::try_from(state) {
        Ok(pb::BoardTrainState::Integrating) => "integrating",
        Ok(pb::BoardTrainState::Gating) => "gating",
        Ok(pb::BoardTrainState::Pushed) => "pushed",
        Ok(pb::BoardTrainState::Green) => "green",
        Ok(pb::BoardTrainState::Red) => "red",
        Ok(pb::BoardTrainState::Landed) => "landed",
        Ok(pb::BoardTrainState::Dropped) => "dropped",
        _ => "unknown",
    }
}

fn ci_status_word(status: i32) -> &'static str {
    match pb::BoardCiStatus::try_from(status) {
        Ok(pb::BoardCiStatus::Passed) => "passed",
        Ok(pb::BoardCiStatus::Failed) => "failed",
        Ok(pb::BoardCiStatus::Running) => "running",
        Ok(pb::BoardCiStatus::Queued) => "queued",
        Ok(pb::BoardCiStatus::Superseded) => "superseded",
        Ok(pb::BoardCiStatus::None) => "none",
        _ => "unknown",
    }
}

/// A train (ov-309), with its lanes named ("" never: a lane the plan no longer
/// lists is named by its short id).
fn train_json(plan: &pb::Plan, t: &pb::BoardTrain) -> Value {
    let lane_name = |id: &[u8]| plan.lanes.iter().find(|l| l.id == id).map_or_else(|| short(id), |l| l.name.clone());
    json!({
        "id": id_text(&t.id),
        "short": short(&t.id),
        "name": t.name,
        "base": t.base,
        "pushed_sha": t.pushed_sha,
        "state": train_state_word(t.state),
        "state_since": t.state_since,
        "actor": t.actor,
        "created_at": t.created_at,
        "landed_at": t.landed_at,
        "lanes": t.lane_ids.iter().map(|id| json!({ "lane": id_text(id), "name": lane_name(id) })).collect::<Vec<_>>(),
        "ci_subject": t.ci_subject,
    })
}

/// What the runner last read of one CI subject (ov-309, ov-306).
fn ci_json(r: &pb::BoardCiRead) -> Value {
    json!({
        "subject": r.subject,
        "sha": r.sha,
        "status": ci_status_word(r.status),
        "url": r.url,
        "jobs": r.jobs.iter().map(|j| json!({ "name": j.name, "state": j.state, "url": j.url })).collect::<Vec<_>>(),
        "fetched_at": r.fetched_at,
        "changed_at": r.changed_at,
        "asked_at": r.asked_at,
    })
}

/// Cards that are neither done nor canceled.
fn is_open(status: i32) -> bool {
    status != pb::TaskStatus::Done as i32 && status != pb::TaskStatus::Cancelled as i32
}

/// `plan.get`'s answer: the whole plan, with the two flags the CLI's
/// reconciliation derives, `landed_not_closed` and `no_lane`, and the
/// rulings (ov-304) in the runner's order: standing first, newest first.
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
        "rulings": plan.rulings.iter().map(|r| ruling_json(plan, r)).collect::<Vec<_>>(),
        "trains": plan.trains.iter().map(|t| train_json(plan, t)).collect::<Vec<_>>(),
        "ci": plan.ci.iter().map(ci_json).collect::<Vec<_>>(),
        "board_counts": counts_json(&plan.board_counts.unwrap_or_default()),
        "cost": plan.cost.as_ref().map(cost_json),
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
