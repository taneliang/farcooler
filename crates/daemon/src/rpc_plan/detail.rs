//! Closed rows are available through detail reads, independently of what the
//! overview retained for display.

use std::collections::HashSet;

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, plan_detail_read::Subject};
use prost::Message;
use uuid::Uuid;

const PAGE_ROWS: u32 = 256;

pub(super) fn page(mut plan: pb::Plan, query: pb::PlanDetailRead) -> Result<pb::Plan> {
    match query.subject {
        Some(Subject::ThemeName(name)) => {
            let name = name.trim().to_lowercase();
            plan.themes.retain(|view| view.theme.as_ref().is_some_and(|theme|
                theme.name.to_lowercase().starts_with(&name) || short_id(&theme.id) == name));
            let tasks = plan.themes.iter().flat_map(|view| view.task_ids.iter().cloned()).collect();
            retain_tickets(&mut plan, &tasks);
            plan.landings.clear();
            plan.ci.clear();
            plan.rulings.clear();
        }
        Some(Subject::Landings(landings)) => {
            plan.landings.retain(|landing| landings.number.is_none_or(|number| landing.number == number));
            let tasks: HashSet<_> = plan.landings.iter().flat_map(|landing| &landing.task_ids).collect();
            let ci: HashSet<_> = plan.landings.iter().map(|landing| &landing.ci_subject).collect();
            plan.cards.retain(|card| tasks.contains(&card.task_id));
            plan.ci.retain(|read| ci.contains(&read.subject));
            plan.themes.clear();
            plan.lines.clear();
            plan.agents.retain(|agent| plan.landings.iter().any(|landing| landing.lander_id.as_ref() == Some(&agent.id)));
            plan.rulings.clear();
        }
        Some(Subject::Tickets(tickets)) => {
            let keys: HashSet<_> = tickets.keys.iter().map(|key| key.trim().to_lowercase()).collect();
            let tasks: HashSet<_> = plan.cards.iter()
                .filter(|card| keys.contains(&card.key.to_lowercase()) || keys.contains(&short_id(&card.task_id)))
                .map(|card| card.task_id.clone()).collect();
            plan.themes.retain(|theme| theme.task_ids.iter().any(|id| tasks.contains(id)));
            plan.rulings.retain(|ruling| ruling.task_ids.iter().any(|id| tasks.contains(id)));
            retain_tickets(&mut plan, &tasks);
            plan.landings.clear();
            plan.ci.clear();
        }
        None => {}
    }

    // Cost and reviewer comparisons belong to the overview. Reserve envelope
    // space and page by both row count and bytes, since text sizes vary.
    plan.cost = None;
    plan.review_waits.clear();
    plan.suggested_reviewers.clear();
    let mut page = Page {
        skip: query.offset as usize,
        bytes_left: farcooler_protocol::MAX_CONTROL_ENVELOPE_BYTES - 1024,
        len: 0,
        more: false,
    };
    page.take(&mut plan.themes);
    page.take(&mut plan.lines);
    page.take(&mut plan.cards);
    page.take(&mut plan.landings);
    page.take(&mut plan.agents);
    page.take(&mut plan.rulings);
    page.take(&mut plan.ci);
    if page.more {
        if page.len == 0 {
            return Err(DomainError::InvalidArgument { what: "plan_detail_row_too_large" });
        }
        plan.next_offset = Some(query.offset.checked_add(page.len)
            .ok_or(DomainError::InvalidArgument { what: "plan_detail_offset" })?);
    }
    Ok(plan)
}

fn short_id(bytes: &[u8]) -> String {
    let id = Uuid::from_slice(bytes).unwrap_or_default().simple().to_string();
    id[id.len() - 8..].to_string()
}

fn retain_tickets(plan: &mut pb::Plan, tasks: &HashSet<bytes::Bytes>) {
    let parents: HashSet<_> = plan.lines.iter().filter(|line| tasks.contains(&line.task_id))
        .filter_map(|line| line.stack_parent.clone()).collect();
    plan.lines.retain(|line| tasks.contains(&line.task_id) || parents.contains(&line.task_id));
    let agents: HashSet<_> = plan.lines.iter().flat_map(|line| line.work.iter().map(|work| &work.agent_id)).collect();
    plan.agents.retain(|agent| agents.contains(&agent.id));
    plan.cards.retain(|card| tasks.contains(&card.task_id) || parents.contains(&card.task_id));
}

struct Page {
    skip: usize,
    bytes_left: usize,
    len: u32,
    more: bool,
}

impl Page {
    fn take(&mut self, rows: &mut Vec<impl Message>) {
        rows.retain(|row| {
            if self.skip > 0 {
                self.skip -= 1;
                return false;
            }
            if self.more {
                return false;
            }
            let bytes = row.encoded_len() + 10;
            if self.len == PAGE_ROWS || bytes > self.bytes_left {
                self.more = true;
                return false;
            }
            self.len += 1;
            self.bytes_left -= bytes;
            true
        });
    }
}
