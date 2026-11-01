//! The plan overview in text (ov-268, ov-872), and the checks under "Worth a look".
//!
//! What the Mac's overview shows, so an orchestrator reads the same picture
//! the owner sees: what's next up, the tickets being worked and who works
//! them, the landings on their way, the agents at work, the themes, and what
//! landed today. A ticket is named by its card's key and title. A child of
//! `plan.rs`.

use farcooler_protocol::v1 as pb;

use super::{
    NOTHING_PLANNED, age, agent, cost, count, is_live, is_open, landing, role_word, ruling, short_bytes, spend_words,
    stage_word, status_word, theme_row,
};

/// A card's key from the plan, or its short id for one the plan doesn't list.
pub(super) fn key_of(plan: &pb::Plan, task: &[u8]) -> String {
    plan.cards.iter().find(|c| c.task_id == task).map_or_else(|| short_bytes(task), |c| c.key.clone())
}

/// A ticket as a person reads it: "ov-1  Frosted window plane".
pub(super) fn ticket_label(plan: &pb::Plan, task: &[u8]) -> String {
    match plan.cards.iter().find(|c| c.task_id == task) {
        Some(c) if !c.title.is_empty() => format!("{}  {}", c.key, c.title),
        Some(c) => c.key.clone(),
        None => short_bytes(task),
    }
}

/// An agent as a row names it: "Reviewer opus (a1b2c3d4)", or its short id
/// alone when the plan no longer lists it.
pub(super) fn agent_name(plan: &pb::Plan, id: &[u8]) -> String {
    match plan.agents.iter().find(|a| a.id == id) {
        Some(a) => {
            let model = if a.model.is_empty() { a.harness.as_str() } else { a.model.as_str() };
            format!("{} {model} ({})", role_word(a.role), short_bytes(&a.id))
        }
        None => format!("agent {}", short_bytes(id)),
    }
}

/// Who works a ticket now: its open work, each agent once.
pub(super) fn workers(plan: &pb::Plan, l: &pb::TicketLine) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in l.work.iter().filter(|w| w.ended_at.is_none()) {
        let name = agent_name(plan, &w.agent_id);
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// "In review · PR 31 · Waiting on alice · Reviewer opus (a1b2c3d4) · 470K
/// tokens": where a ticket is, who has it, and what it spent.
pub(super) fn ticket_status(plan: &pb::Plan, l: &pb::TicketLine, now: i64) -> String {
    let mut parts = vec![stage_word(l.stage).to_string()];
    if l.stage == pb::TicketStage::Building as i32 && l.fix_rounds > 0 {
        parts[0] = format!("Fixing · round {}", l.fix_rounds);
    }
    if let Some(rank) = l.plan_rank {
        parts.push(if rank == 1 { "next up".to_string() } else { format!("{} in the plan", ordinal(rank)) });
    }
    if let Some(parent) = &l.stack_parent {
        let landed = plan.lines.iter().any(|p| p.task_id == *parent && p.stage == pb::TicketStage::Landed as i32);
        let key = key_of(plan, parent);
        parts.push(if landed { format!("stacked on {key}, which landed") } else { format!("stacked on {key}") });
    }
    if let Some(n) = l.landing {
        parts.push(format!("on Landing {n}"));
    } else if l.held_off {
        parts.push("taken off its landing".into());
    }
    if let Some(pr) = l.pr_number {
        parts.push(format!("PR {pr}"));
    }
    // Where its pull request stands (ov-312). Building is the stage word
    // already, so it isn't said twice.
    if let Some(stage) = l.pr_stage.as_ref().filter(|s| s.kind != pb::PrStageKind::Building as i32 && !s.label.is_empty()) {
        parts.push(stage.label.clone());
    }
    parts.extend(workers(plan, l));
    let spend = l.spend.unwrap_or_default();
    if spend.runs > 0 {
        parts.push(spend_words(&spend));
    }
    parts.extend(cost::budget_words(&spend, l.budget_tokens));
    if l.stale {
        parts.push(format!("stuck for {}", age(now - l.stage_since)));
    }
    parts.join(" · ")
}

/// A ticket's row: its key and title, then where it is.
pub(super) fn ticket_row(plan: &pb::Plan, l: &pb::TicketLine, now: i64) -> String {
    format!("{} · {}", ticket_label(plan, &l.task_id), ticket_status(plan, l, now))
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
    if plan.themes.is_empty()
        && plan.lines.is_empty()
        && plan.rulings.is_empty()
        && plan.landings.is_empty()
        && plan.agents.is_empty()
    {
        return NOTHING_PLANNED.to_string();
    }
    let mut out: Vec<String> = Vec::new();

    let mut next: Vec<&pb::TicketLine> = plan.lines.iter().filter(|l| l.plan_rank.is_some()).collect();
    next.sort_by_key(|l| l.plan_rank);
    if !next.is_empty() {
        out.push("Next up".into());
        for l in &next {
            out.push(format!("  {}  {}", l.plan_rank.unwrap_or_default(), ticket_label(plan, &l.task_id)));
            if !l.reason.is_empty() {
                out.push(format!("     {}", l.reason));
            }
        }
    }
    let working: Vec<&pb::TicketLine> =
        plan.lines.iter().filter(|l| is_live(l.stage) && l.stage != pb::TicketStage::Waiting as i32).collect();
    if !working.is_empty() {
        out.push("Now".into());
        out.extend(working.iter().map(|l| format!("  {}", ticket_row(plan, l, now))));
    }
    let landings: Vec<&pb::Landing> = plan.landings.iter().filter(|l| !landing::is_settled(l)).collect();
    if !landings.is_empty() {
        out.push("Landings".into());
        out.extend(landings.iter().map(|l| format!("  {}", landing::landing_line(plan, l))));
    }
    if !plan.agents.is_empty() {
        out.push("Agents".into());
        out.extend(plan.agents.iter().map(|a| agent::agent_row(plan, a)));
    }
    out.extend(waiting_lines(plan));
    out.extend(suggestion_lines(plan));
    let unplanned: Vec<&pb::TicketLine> =
        plan.lines.iter().filter(|l| l.stage == pb::TicketStage::Waiting as i32 && l.plan_rank.is_none()).collect();
    if !unplanned.is_empty() {
        out.push("Waiting, not in the plan".into());
        for l in unplanned {
            let reason = if l.reason.is_empty() { String::new() } else { format!(" · {}", l.reason) };
            out.push(format!("  {}{reason}", ticket_label(plan, &l.task_id)));
        }
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
    let landed: Vec<String> = plan
        .lines
        .iter()
        .filter(|l| l.stage == pb::TicketStage::Landed as i32 && now - l.stage_since < day)
        .map(|l| format!("  {}", ticket_label(plan, &l.task_id)))
        .collect();
    if !landed.is_empty() {
        out.push("Landed today".into());
        out.extend(landed);
    }
    let checks = checks(plan);
    if !checks.is_empty() {
        out.push("Worth a look".into());
        out.extend(checks.into_iter().map(|c| format!("  {c}")));
    }
    out.join("\n")
}

/// What the reconciliation reports flag by hand, derived: a card whose ticket
/// landed while it isn't done, and one in progress or in review that isn't on
/// the line. Only for cards the plan names; the layer knows no others. The
/// same two lists `plan --json` carries as `landed_not_closed` and
/// `off_the_line`.
pub(super) fn checks(plan: &pb::Plan) -> Vec<String> {
    let mut out = Vec::new();
    for c in plan.cards.iter().filter(|c| is_open(c.status)) {
        let stage = plan.lines.iter().find(|l| l.task_id == c.task_id).map(|l| l.stage);
        if stage == Some(pb::TicketStage::Landed as i32) {
            out.push(format!("{}  Landed, and it's still {}.", c.key, status_word(c.status)));
        } else if stage.is_none()
            && (c.status == pb::TaskStatus::InProgress as i32 || c.status == pb::TaskStatus::InReview as i32)
        {
            out.push(format!("{}  {}, and it isn't on the line.", c.key, status_word(c.status)));
        }
    }
    out
}

/// "Waiting for alice: 2 PRs, about 50 minutes" for each reviewer a live
/// ticket's ready pull request asks (ov-579). The minutes are that reviewer's
/// median time to a first review; "no history yet" when the runner has read
/// none of theirs. Nothing from a runner that sends no waits.
pub(super) fn waiting_lines(plan: &pb::Plan) -> Vec<String> {
    if plan.review_waits.is_empty() {
        return Vec::new();
    }
    let mut out = vec!["Waiting on reviewers".to_string()];
    out.extend(plan.review_waits.iter().map(|w| {
        let usual = w.median_wait_ms.map_or_else(|| "no history yet".to_string(), usual_wait);
        format!("  Waiting for {}: {}, {usual}", w.reviewer, count(w.prs as usize, "PR"))
    }));
    out
}

/// "about 50 minutes": a median, rounded to what a person plans around, to the
/// nearest five minutes under an hour and a half, then to the hour, then to the day.
pub(super) fn usual_wait(ms: i64) -> String {
    let minutes = ms.max(0) / 60_000;
    match minutes {
        0..=2 => "under 5 minutes".to_string(),
        3..=89 => format!("about {} minutes", ((minutes + 2) / 5 * 5).max(5)),
        90..=2879 => format!("about {} hours", (minutes + 30) / 60),
        _ => format!("about {} days", (minutes + 720) / 1440),
    }
}

/// "Suggested reviewers" and, for each live ticket's pull request that asks
/// nobody yet, whom to ask and why (ov-580). Nothing from a runner that sends
/// none, and nothing for a ticket the plan doesn't list.
pub(super) fn suggestion_lines(plan: &pb::Plan) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for s in &plan.suggested_reviewers {
        if !plan.lines.iter().any(|l| l.task_id == s.task_id) {
            continue;
        }
        let who: Vec<String> = s
            .reviewers
            .iter()
            .enumerate()
            .map(|(i, r)| match s.sources.get(i) {
                Some(why) => format!("{r} ({why})"),
                None => r.clone(),
            })
            .collect();
        if !who.is_empty() {
            lines.push(format!("  {}, PR {}: {}", key_of(plan, &s.task_id), s.pr, who.join(", ")));
        }
    }
    if lines.is_empty() {
        return lines;
    }
    lines.insert(0, "Suggested reviewers".to_string());
    lines
}
