//! Byte-bounded pages for the overview and scoped Plan detail reads.

use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, plan_detail_read::Subject};
use prost::Message;
use uuid::Uuid;

use super::overview::{PLAN_BUDGET, row_bytes};

const THEME_MEMBERS_PER_ROW: usize = 256;
// Leave room inside the Plan for scalar fields, the cursor and the chunk wrapper.
const PAGE_PAYLOAD_BYTES: usize = PLAN_BUDGET - 256;

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
            plan.themes.retain_mut(|theme| {
                theme.task_ids.retain(|id| tasks.contains(id));
                !theme.task_ids.is_empty()
            });
            plan.rulings.retain(|ruling| ruling.task_ids.iter().any(|id| tasks.contains(id)));
            retain_tickets(&mut plan, &tasks);
            plan.landings.clear();
            plan.ci.clear();
        }
        None => {}
    }

    // Cost and reviewer comparisons belong to the overview.
    plan.cost = None;
    plan.review_waits.clear();
    plan.suggested_reviewers.clear();
    paginate(plan, query.offset)
}

pub(super) fn overview(plan: pb::Plan, offset: u32) -> Result<pb::Plan> {
    if offset == 0 && plan.encoded_len() <= PLAN_BUDGET {
        return Ok(plan);
    }
    paginate(plan, offset)
}

fn paginate(mut plan: pb::Plan, offset: u32) -> Result<pb::Plan> {
    // Page offsets are only meaningful against the same rows in the same
    // order. The read clock alone changes on every call.
    let now_ms = std::mem::take(&mut plan.now_ms);
    let mut hash = DefaultHasher::new();
    plan.encode_to_vec().hash(&mut hash);
    plan.page_revision = Some(hash.finish());
    plan.now_ms = now_ms;

    plan.themes = split_theme_memberships(plan.themes);
    let mut page = Page {
        skip: offset as usize,
        bytes_left: PAGE_PAYLOAD_BYTES,
        len: 0,
        more: false,
        chunk: None,
    };
    macro_rules! take {
        ($field:ident) => {
            page.take(&mut plan.$field, |row| pb::Plan { $field: vec![row], ..Default::default() });
        };
    }
    take!(themes);
    take!(lines);
    take!(cards);
    take!(landings);
    take!(agents);
    take!(rulings);
    take!(ci);
    take!(review_waits);
    take!(suggested_reviewers);
    if let Some(cost) = &mut plan.cost {
        page.take(&mut cost.compare, |row| pb::Plan {
            cost: Some(pb::PlanCost { compare: vec![row], ..Default::default() }),
            ..Default::default()
        });
        page.take(&mut cost.week, |row| pb::Plan {
            cost: Some(pb::PlanCost { week: vec![row], ..Default::default() }),
            ..Default::default()
        });
    }
    plan.row_chunk = page.chunk;
    if page.more {
        plan.next_offset = Some(offset.checked_add(page.len)
            .ok_or(DomainError::InvalidArgument { what: "plan_detail_offset" })?);
    }
    Ok(plan)
}

fn split_theme_memberships(themes: Vec<pb::BoardThemeView>) -> Vec<pb::BoardThemeView> {
    let mut rows = Vec::new();
    for mut theme in themes {
        if theme.task_ids.len() <= THEME_MEMBERS_PER_ROW {
            rows.push(theme);
            continue;
        }
        let task_ids = std::mem::take(&mut theme.task_ids);
        if row_bytes(&theme) + THEME_MEMBERS_PER_ROW * (16 + 10) > PAGE_PAYLOAD_BYTES {
            // The text alone is close to a frame. Chunk the whole row once
            // instead of duplicating its text in every membership fragment.
            theme.task_ids = task_ids;
            rows.push(theme);
            continue;
        }
        for members in task_ids.chunks(THEME_MEMBERS_PER_ROW) {
            rows.push(pb::BoardThemeView { task_ids: members.to_vec(), ..theme.clone() });
        }
    }
    rows
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
    chunk: Option<pb::PlanRowChunk>,
}

impl Page {
    fn take<M: Message + Clone>(&mut self, rows: &mut Vec<M>, pack: impl Fn(M) -> pb::Plan) {
        rows.retain(|row| {
            if self.more {
                return false;
            }
            let bytes = row_bytes(row);
            if bytes > PAGE_PAYLOAD_BYTES {
                self.take_chunk(pack(row.clone()).encode_to_vec());
                return false;
            }
            if self.skip > 0 {
                self.skip -= 1;
                return false;
            }
            if bytes > self.bytes_left {
                self.more = true;
                return false;
            }
            self.len += 1;
            self.bytes_left -= bytes;
            true
        });
    }

    // Each chunk occupies one cursor position and a page of its own so its
    // decoded row stays in order with the complete rows on neighboring pages.
    fn take_chunk(&mut self, encoded: Vec<u8>) {
        let count = encoded.len().div_ceil(PAGE_PAYLOAD_BYTES);
        if self.skip >= count {
            self.skip -= count;
            return;
        }
        if self.len != 0 {
            self.more = true;
            return;
        }
        let start = self.skip * PAGE_PAYLOAD_BYTES;
        let end = (start + PAGE_PAYLOAD_BYTES).min(encoded.len());
        let last = end == encoded.len();
        self.chunk = Some(pb::PlanRowChunk {
            data: bytes::Bytes::copy_from_slice(&encoded[start..end]),
            last,
        });
        self.skip = 0;
        self.bytes_left = 0;
        self.len = 1;
        self.more = !last;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_protocol::MAX_CONTROL_ENVELOPE_BYTES;

    #[test]
    fn a_live_landing_larger_than_one_frame_loads_in_overview_and_detail() {
        use farcooler_client::plan_pages::PlanPages;

        let landing = pb::Landing {
            id: bytes::Bytes::from_static(b"landing id"),
            number: 17,
            phase: pb::LandingPhase::ReadyToLand as i32,
            ci_subject: "sha:abc".into(),
            task_ids: (0u128..60_000)
                .map(|n| bytes::Bytes::copy_from_slice(&n.to_le_bytes()))
                .collect(),
            ..Default::default()
        };
        assert!(landing.encoded_len() > MAX_CONTROL_ENVELOPE_BYTES);
        let ci = pb::BoardCiRead {
            subject: landing.ci_subject.clone(),
            jobs: vec![pb::BoardCiJob { name: "Test on a matrix".into(), ..Default::default() }; 65_000],
            ..Default::default()
        };
        assert!(ci.encoded_len() > MAX_CONTROL_ENVELOPE_BYTES);
        let plan = pb::Plan {
            landings: vec![landing],
            ci: vec![ci],
            board_counts: Some(pb::PlanStatusCounts { in_progress: 60_000, ..Default::default() }),
            ..Default::default()
        };
        let subject = Subject::Landings(pb::PlanLandingQuery { number: Some(17) });
        let readers = [
            PlanPages::overview(bytes::Bytes::new()),
            PlanPages::detail(bytes::Bytes::new(), Some(subject)),
        ];
        for mut reader in readers {
            let mut pages = 0;
            loop {
                let request = reader.request();
                let reply = if let Some(query) = request.detail {
                    page(plan.clone(), query)
                } else {
                    overview(plan.clone(), request.overview_offset.unwrap())
                }.expect("even a single oversized landing must fit across pages");
                assert!(reply.encoded_len() <= PLAN_BUDGET);
                let reply = pb::Plan::decode(reply.encode_to_vec().as_slice()).unwrap();
                pages += 1;
                if let Some(read) = reader.push(reply).unwrap() {
                    assert_eq!(read, plan);
                    break;
                }
            }
            assert!(pages > 1);
        }
    }

    #[test]
    fn a_theme_larger_than_one_frame_can_be_read_in_order() {
        let theme_id = bytes::Bytes::from_static(b"theme identifier");
        let task_ids: Vec<_> = (0u128..60_000)
            .map(|n| bytes::Bytes::copy_from_slice(&n.to_le_bytes()))
            .collect();
        let theme = pb::BoardThemeView {
            theme: Some(pb::BoardTheme { id: theme_id.clone(), name: "Big theme".into(), ..Default::default() }),
            task_ids: task_ids.clone(),
            counts: Some(pb::PlanStatusCounts { backlog: 60_000, ..Default::default() }),
            ..Default::default()
        };
        assert!(theme.encoded_len() > MAX_CONTROL_ENVELOPE_BYTES);
        let plan = pb::Plan { themes: vec![theme], ..Default::default() };
        let mut query = pb::PlanDetailRead { offset: 0, subject: Some(Subject::ThemeName("Big theme".into())) };
        let mut read = Vec::new();
        loop {
            let page = page(plan.clone(), query.clone()).expect("theme membership fits in pages");
            assert!(page.encoded_len() < MAX_CONTROL_ENVELOPE_BYTES);
            for fragment in page.themes {
                assert_eq!(fragment.theme.unwrap().id, theme_id);
                assert_eq!(fragment.counts.unwrap().backlog, 60_000);
                read.extend(fragment.task_ids);
            }
            let Some(offset) = page.next_offset else { break };
            assert!(offset > query.offset);
            query.offset = offset;
        }
        assert!(query.offset > 0, "the theme actually spans frames");
        assert_eq!(read, task_ids);
    }
}
