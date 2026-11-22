//! Assemble Plan pages before any renderer resolves their cross-references.

use bytes::Bytes;
use farcooler_protocol::v1 as pb;

use crate::session::SessionError;

/// Collects a recent overview or scoped detail read.
pub struct PlanPages {
    workspace_id: Bytes,
    detail: Option<pb::PlanDetailRead>,
    offset: u32,
    restarted: bool,
    plan: pb::Plan,
}

impl PlanPages {
    pub fn overview(workspace_id: Bytes) -> Self {
        Self {
            workspace_id,
            detail: None,
            offset: 0,
            restarted: false,
            plan: pb::Plan::default(),
        }
    }

    pub fn detail(workspace_id: Bytes, subject: Option<pb::plan_detail_read::Subject>) -> Self {
        Self {
            detail: Some(pb::PlanDetailRead { offset: 0, subject }),
            ..Self::overview(workspace_id)
        }
    }

    pub fn request(&self) -> pb::PlanGetRequest {
        let detail = self.detail.as_ref().map(|query| pb::PlanDetailRead { offset: self.offset, ..query.clone() });
        pb::PlanGetRequest {
            workspace_id: self.workspace_id.clone(),
            include_closed: detail.is_some(),
            overview_offset: detail.is_none().then_some(self.offset),
            detail,
        }
    }

    /// Returns the assembled Plan on the last page. If the rows changed during
    /// the read, retry once from the start so references come from the same board.
    pub fn push(&mut self, mut page: pb::Plan) -> Result<Option<pb::Plan>, SessionError> {
        if self.offset != 0 && self.plan.page_revision != page.page_revision {
            if self.restarted {
                return Err(SessionError::Protocol("The plan changed while it was being read. Try again.".into()));
            }
            self.restarted = true;
            self.offset = 0;
            self.plan = pb::Plan::default();
            return Ok(None);
        }
        let next = page.next_offset.take();
        let themes = std::mem::take(&mut page.themes);
        if self.offset == 0 {
            self.plan = page;
        } else {
            self.plan.lines.extend(page.lines);
            self.plan.cards.extend(page.cards);
            self.plan.landings.extend(page.landings);
            self.plan.agents.extend(page.agents);
            self.plan.rulings.extend(page.rulings);
            self.plan.ci.extend(page.ci);
            self.plan.review_waits.extend(page.review_waits);
            self.plan.suggested_reviewers.extend(page.suggested_reviewers);
            if let (Some(cost), Some(next_cost)) = (&mut self.plan.cost, page.cost) {
                cost.compare.extend(next_cost.compare);
                cost.week.extend(next_cost.week);
            }
        }
        for fragment in themes {
            if let Some(theme) = self.plan.themes.last_mut()
                && let (Some(previous), Some(incoming)) = (&theme.theme, &fragment.theme)
                && previous.id == incoming.id
            {
                theme.task_ids.extend(fragment.task_ids);
            } else {
                self.plan.themes.push(fragment);
            }
        }
        if let Some(next) = next {
            if next <= self.offset {
                return Err(SessionError::Protocol("The runner returned an invalid plan page offset.".into()));
            }
            self.offset = next;
            return Ok(None);
        }
        self.plan.page_revision = None;
        Ok(Some(std::mem::take(&mut self.plan)))
    }
}
