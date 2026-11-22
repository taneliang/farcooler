//! The default Plan is an overview. Keep complete counters and a recent
//! sample of history. Detail commands request their own pages.

use std::cmp::Reverse;
use std::collections::HashSet;

use farcooler_protocol::{MAX_CONTROL_ENVELOPE_BYTES, v1 as pb};
use prost::Message;

// Leave space for the response, result and envelope around the Plan.
pub(super) const PLAN_BUDGET: usize = MAX_CONTROL_ENVELOPE_BYTES - 1024;
const HISTORY_BYTES: usize = 64 * 1024;
const RECENT_LINES: usize = 40;
const RECENT_LANDINGS: usize = 20;

pub(super) fn summarize(plan: &mut pb::Plan) {
    retain_recent(&mut plan.lines, RECENT_LINES, |line| {
        matches!(line.stage(), pb::TicketStage::Landed | pb::TicketStage::Dropped).then_some(line.stage_since)
    });
    retain_recent(&mut plan.landings, RECENT_LANDINGS, |landing| {
        matches!(landing.phase(), pb::LandingPhase::Landed | pb::LandingPhase::Dropped).then_some(landing.phase_since)
    });

    // A closed card with no visible line, landing or agent is represented by
    // its theme and board counts. Keep the cards that those rows still name.
    let shown: HashSet<_> = plan.lines.iter().flat_map(|line| std::iter::once(&line.task_id).chain(&line.stack_parent))
        .chain(plan.landings.iter().flat_map(|landing| &landing.task_ids))
        .chain(plan.agents.iter().flat_map(|agent| agent.tickets.iter().map(|ticket| &ticket.task_id)))
        .collect();
    plan.cards.retain(|card| shown.contains(&card.task_id)
        || !matches!(card.status(), pb::TaskStatus::Done | pb::TaskStatus::Cancelled));
    retain_card_refs(plan);
}

// Select by recency across legacy landings and engine groups, then retain
// the wire's display order. A single large history row cannot use the frame.
fn retain_recent<M: Message>(rows: &mut Vec<M>, limit: usize, since: impl Fn(&M) -> Option<i64>) {
    let mut recent: Vec<_> = rows.iter().enumerate()
        .filter_map(|(i, row)| since(row).map(|at| (Reverse(at), i)))
        .collect();
    recent.sort_unstable();
    let mut remaining = HISTORY_BYTES;
    let keep: HashSet<_> = recent.into_iter().take(limit).filter_map(|(_, i)| {
        let bytes = row_bytes(&rows[i]);
        remaining = remaining.checked_sub(bytes)?;
        Some(i)
    }).collect();
    let mut i = 0;
    rows.retain(|row| {
        let retain = since(row).is_none() || keep.contains(&i);
        i += 1;
        retain
    });
}

fn retain_card_refs(plan: &mut pb::Plan) {
    let cards: HashSet<_> = plan.cards.iter().map(|card| &card.task_id).collect();
    for theme in &mut plan.themes {
        theme.task_ids.retain(|id| cards.contains(id));
    }
}

// Released clients read one response. Reserve room for each section and keep
// the remaining rows usable without another read. Paging clients call summarize
// directly and fetch all the live rows.
pub(super) fn bound_legacy(plan: &mut pb::Plan) {
    if plan.encoded_len() <= PLAN_BUDGET {
        return;
    }
    plan.overview_truncated = true;
    let share = PLAN_BUDGET / 16;
    bound_rows(&mut plan.lines, 4 * share);
    bound_rows(&mut plan.cards, 3 * share);
    bound_rows(&mut plan.landings, 2 * share);
    bound_rows(&mut plan.agents, share);
    bound_rows(&mut plan.rulings, share / 2);
    bound_rows(&mut plan.ci, share / 2);
    bound_rows(&mut plan.review_waits, share / 4);
    bound_rows(&mut plan.suggested_reviewers, share / 4);
    if let Some(cost) = &mut plan.cost {
        bound_rows(&mut cost.compare, share / 4);
        bound_rows(&mut cost.week, share / 4);
    }
    let cards: HashSet<_> = plan.cards.iter().map(|card| &card.task_id).collect();
    plan.lines.retain(|line| cards.contains(&line.task_id));
    plan.landings.retain(|landing| landing.task_ids.iter().all(|id| cards.contains(id)));
    for agent in &mut plan.agents {
        agent.tickets.retain(|ticket| cards.contains(&ticket.task_id));
    }
    plan.suggested_reviewers.retain(|reviewers| cards.contains(&reviewers.task_id));
    retain_card_refs(plan);
    bound_rows(&mut plan.themes, 4 * share);
}

// Include a conservative allowance for the repeated field's tag and length.
pub(super) fn row_bytes(row: &impl Message) -> usize {
    row.encoded_len() + 10
}

fn bound_rows<M: Message>(rows: &mut Vec<M>, mut remaining: usize) {
    rows.retain(|row| {
        let Some(next) = remaining.checked_sub(row_bytes(row)) else { return false };
        remaining = next;
        true
    });
}
