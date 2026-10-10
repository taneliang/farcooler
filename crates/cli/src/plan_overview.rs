//! The plan overview in text (ov-268), and the checks under "Worth a look".
//!
//! What the Mac's overview shows, so an orchestrator reads the same picture
//! the owner sees. Lanes and trains are named by what they do first and by
//! their slug second (ov-462): "Agents message the orchestrator (agent-msg)".
//! A child of `plan.rs`.

use farcooler_protocol::v1 as pb;

use super::train;
use super::{
    Keys, NOTHING_PLANNED, age, cost, count, is_live, is_open, ruling, spend_words, state_word, status_word, theme_row,
};

/// A lane as a person reads it, the slug second: "Agents message the
/// orchestrator (agent-msg)", or the name alone for a lane with no title.
pub(super) fn lane_label(l: &pb::Lane) -> String {
    if l.title.is_empty() || l.title.eq_ignore_ascii_case(&l.name) {
        l.name.clone()
    } else {
        format!("{} ({})", l.title, l.name)
    }
}

/// Whether `t` has not landed or been dropped.
pub(super) fn is_live_train(t: &pb::BoardTrain) -> bool {
    t.state != pb::BoardTrainState::Landed as i32 && t.state != pb::BoardTrainState::Dropped as i32
}

/// A live lane named like a live train (ov-461): the integrating agent modeled
/// as a lane of the train's own name. The train stands for it.
pub(super) fn shadows_a_train(plan: &pb::Plan, l: &pb::Lane) -> bool {
    is_live(l.state) && plan.trains.iter().any(|t| is_live_train(t) && t.name.eq_ignore_ascii_case(&l.name))
}

pub(super) fn lane_row(l: &pb::Lane, keys: &Keys, now: i64) -> String {
    let mut row = format!("{} · {}", lane_label(l), lane_status(l, now));
    if !l.cards.is_empty() {
        let listed: Vec<String> = l.cards.iter().map(|c| keys.of(&c.task_id)).collect();
        row.push_str(&format!(" · {}", listed.join(" ")));
    }
    row
}

/// "In review · in integ-9 · 5 cards · 470k tokens".
pub(super) fn lane_status(l: &pb::Lane, now: i64) -> String {
    let mut parts = vec![state_word(l.state).to_string()];
    if l.state == pb::LaneState::Fixing as i32 && l.fix_rounds > 0 {
        parts[0] = format!("Fixing · round {}", l.fix_rounds);
    }
    if let Some(rank) = l.plan_rank {
        parts.push(if rank == 1 { "next up".to_string() } else { format!("{} in the plan", ordinal(rank)) });
    }
    if let Some(train) = &l.train {
        parts.push(format!("in {train}"));
    }
    // Where its pull requests stand (ov-312). Building is the state word
    // already, so it isn't said twice.
    if let Some(stage) = l.stage.as_ref().filter(|s| s.kind != pb::PrStageKind::Building as i32) {
        parts.push(stage.label.clone());
    }
    parts.push(count(l.cards.len(), "card"));
    let spend = l.spend.unwrap_or_default();
    if spend.runs > 0 {
        parts.push(spend_words(&spend));
    }
    parts.extend(cost::budget_words(&spend, l.budget_tokens));
    if l.stale {
        parts.push(format!("stuck for {}", age(now - l.state_since)));
    }
    parts.join(" · ")
}

fn ordinal(n: u32) -> String {
    match n {
        1 => "1st".into(),
        2 => "2nd".into(),
        3 => "3rd".into(),
        n => format!("{n}th"),
    }
}

/// What the Mac's overview shows, in text, so an orchestrator reads the same
/// picture the owner sees.
pub(super) fn overview(plan: &pb::Plan, now: i64) -> String {
    if plan.themes.is_empty() && plan.lanes.is_empty() && plan.rulings.is_empty() && plan.trains.is_empty() {
        return NOTHING_PLANNED.to_string();
    }
    let keys = Keys::of_plan(plan);
    let mut out: Vec<String> = Vec::new();
    let lane_of = |id: &bytes::Bytes| plan.lanes.iter().find(|l| l.id == *id);

    let next: Vec<&pb::Lane> = plan.order.iter().filter_map(lane_of).collect();
    if !next.is_empty() {
        out.push("Next up".into());
        for (i, l) in next.iter().enumerate() {
            let cards: Vec<String> = l.cards.iter().map(|c| keys.of(&c.task_id)).collect();
            out.push(format!("  {}  {} · {}", i + 1, lane_label(l), cards.join(" ")));
            if !l.reason.is_empty() {
                out.push(format!("     {}", l.reason));
            }
        }
    }
    let now_lanes: Vec<&pb::Lane> = plan
        .lanes
        .iter()
        .filter(|l| is_live(l.state) && l.state != pb::LaneState::Queued as i32 && !shadows_a_train(plan, l))
        .collect();
    // Trains not yet landed head row groups of their lanes (ov-309); the
    // lanes on none follow.
    let trains: Vec<&pb::BoardTrain> = plan.trains.iter().filter(|t| is_live_train(t)).collect();
    if !now_lanes.is_empty() || !trains.is_empty() {
        out.push("Now".into());
        for t in &trains {
            out.push(format!("  {}", train::train_line(plan, t)));
            let on: Vec<&pb::Lane> = plan.lanes.iter().filter(|l| t.lane_ids.contains(&l.id)).collect();
            out.extend(on.iter().map(|l| format!("    {} · {}", lane_label(l), lane_status(l, now))));
        }
        let grouped = |l: &pb::Lane| trains.iter().any(|t| t.lane_ids.contains(&l.id));
        out.extend(now_lanes.iter().filter(|l| !grouped(l)).map(|l| format!("  {} · {}", lane_label(l), lane_status(l, now))));
    }
    let unplanned: Vec<&pb::Lane> = plan
        .lanes
        .iter()
        .filter(|l| l.state == pb::LaneState::Queued as i32 && l.plan_rank.is_none())
        .collect();
    if !unplanned.is_empty() {
        out.push("Queued, not in the plan".into());
        out.extend(unplanned.iter().map(|l| format!("  {} · {}", lane_label(l), l.reason)));
    }
    if !plan.themes.is_empty() {
        out.push("Themes".into());
        for view in &plan.themes {
            out.push(format!("  {}", theme_row(view)));
        }
    }
    out.extend(ruling::overview_lines(plan));
    out.extend(cost::overview_lines(plan));
    let day = 24 * 60 * 60 * 1000;
    let landed: Vec<&str> = plan
        .lanes
        .iter()
        .filter(|l| l.state == pb::LaneState::Landed as i32 && now - l.state_since < day)
        .map(|l| if l.title.is_empty() { l.name.as_str() } else { l.title.as_str() })
        .collect();
    if !landed.is_empty() {
        out.push("Landed today".into());
        out.push(format!("  {}", landed.join(", ")));
    }
    let checks = checks(plan, &keys);
    if !checks.is_empty() {
        out.push("Worth a look".into());
        out.extend(checks.into_iter().map(|c| format!("  {c}")));
    }
    out.join("\n")
}

/// What the reconciliation reports flag by hand, derived: a card whose lanes
/// have all landed while it isn't done, and one in progress with no lane. Only
/// for cards a theme or lane names; the layer knows no others.
pub(super) fn checks(plan: &pb::Plan, keys: &Keys) -> Vec<String> {
    let mut out = Vec::new();
    for l in plan.lanes.iter().filter(|l| shadows_a_train(plan, l)) {
        out.push(format!(
            "{}  is a lane and a train. Record its agent on the train (`plan train set {} --agent <id>`), then drop the lane.",
            l.name, l.name
        ));
    }
    for c in &plan.cards {
        if !is_open(c.status) {
            continue;
        }
        let cover = plan.coverage.iter().find(|v| v.task_id == c.task_id);
        let (live, landed) = cover.map_or((0, 0), |v| (v.live, v.landed));
        if live == 0 && landed > 0 {
            out.push(format!("{}  All its lanes have landed, and it's still {}.", keys.of(&c.task_id), status_word(c.status)));
        } else if live == 0
            && (c.status == pb::TaskStatus::InProgress as i32 || c.status == pb::TaskStatus::InReview as i32)
        {
            out.push(format!("{}  {}, and no lane is working it.", keys.of(&c.task_id), status_word(c.status)));
        }
    }
    out
}

